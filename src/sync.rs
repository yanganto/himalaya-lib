use dirs::data_dir;
use log::{debug, error, warn};
use rayon::prelude::*;
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    fs::OpenOptions,
    io::{self, prelude::*, BufReader},
    path::{Path, PathBuf},
    result,
    sync::Mutex,
};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    backend, email, AccountConfig, Backend, Envelope, Flag, Flags, MaildirBackend, MaildirConfig,
    ThreadSafeBackend,
};

#[derive(Debug, Error)]
pub enum Error {
    #[error("cannot get backend lock")]
    GetBackendLockError(String),
    #[error("cannot get id mapper lock")]
    GetIdMapperLockError(String),

    #[error("cannot get XDG_DATA_HOME directory")]
    GetXdgDataDirError,
    #[error("cannot create maildir directories: {1}")]
    CreateMaildirDirError(#[source] io::Error, PathBuf),
    #[error("cannot find email {0}")]
    FindEmailError(String),
    #[error("cannot open sync id mapper file at {1}")]
    OpenHashMapFileError(#[source] io::Error, PathBuf),
    #[error("cannot read line from sync id mapper")]
    ReadHashMapFileLineError(#[source] io::Error),
    #[error("cannot write sync id mapper file at {1}")]
    WriteHashMapFileError(#[source] io::Error, PathBuf),
    #[error("cannot parse line from sync id mapper: {0}")]
    ParseLineError(String),

    #[error(transparent)]
    BackendError(#[from] backend::Error),
    #[error(transparent)]
    EmailError(#[from] email::Error),
    #[error(transparent)]
    MaildirError(#[from] backend::maildir::Error),
}

pub type Result<T> = result::Result<T, Error>;

pub type Envelopes = HashMap<String, Envelope>;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum HunkKind {
    PrevLeft,
    NextLeft,
    PrevRight,
    NextRight,
}

pub type Id = String;
pub type Source = HunkKind;
pub type Target = HunkKind;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Hunk {
    CopyEmail(Id, Flags, Source, Target),
    RemoveEmail(Id, Target),
    AddFlag(Id, Flag, Target),
    RemoveFlag(Id, Flag, Target),
}

type Patch = Vec<Hunk>;

pub fn sync<B: ThreadSafeBackend>(config: &AccountConfig, next_right: &B) -> Result<()> {
    if !config.sync {
        return Ok(());
    }

    let sync_dir = match config.sync_dir.as_ref().filter(|dir| dir.is_dir()) {
        Some(path) => path.clone(),
        None => {
            warn!("sync dir not set or invalid, falling back to $XDG_DATA_HOME");
            data_dir()
                .map(|dir| dir.join(next_right.name()))
                .ok_or(Error::GetXdgDataDirError)?
        }
    };

    let next_right_envelopes = HashMap::from_iter(
        next_right
            .list_envelopes("inbox", 0, 0)?
            .iter()
            .map(|e| (e.internal_id.clone(), e.clone())),
    );

    let prev_left_dir = sync_dir.join(".PrevCache");
    let prev_left = MaildirBackend::new(
        Cow::Borrowed(config),
        Cow::Owned(MaildirConfig {
            root_dir: prev_left_dir.clone(),
        }),
    )?;
    let prev_left_id_mapper = SyncIdMapper::new(prev_left_dir)?;
    let prev_left_envelopes = HashMap::from_iter(
        prev_left
            .list_envelopes("inbox", 0, 0)?
            .iter()
            .map(|e| (e.internal_id.clone(), e.clone())),
    );
    let prev_left_envelopes = prev_left_id_mapper.map_ids(prev_left_envelopes);

    let next_left_dir = sync_dir.join(".Cache");
    let next_left = MaildirBackend::new(
        Cow::Borrowed(config),
        Cow::Owned(MaildirConfig {
            root_dir: next_left_dir.clone(),
        }),
    )?;
    let next_left_id_mapper = SyncIdMapper::new(next_left_dir)?;
    let next_left_envelopes = HashMap::from_iter(
        next_left
            .list_envelopes("inbox", 0, 0)?
            .iter()
            .map(|e| (e.internal_id.clone(), e.clone())),
    );
    let next_left_envelopes = next_left_id_mapper.map_ids(next_left_envelopes);

    let prev_right_dir = sync_dir.clone();
    let prev_right = MaildirBackend::new(
        Cow::Borrowed(config),
        Cow::Owned(MaildirConfig {
            root_dir: prev_right_dir.clone(),
        }),
    )?;
    let prev_right_id_mapper = SyncIdMapper::new(prev_right_dir)?;
    let prev_right_envelopes = HashMap::from_iter(
        prev_right
            .list_envelopes("inbox", 0, 0)?
            .iter()
            .map(|e| (e.internal_id.clone(), e.clone())),
    );
    let prev_right_envelopes = prev_right_id_mapper.map_ids(prev_right_envelopes);

    let patch = build_patch(
        prev_left_envelopes,
        next_left_envelopes,
        prev_right_envelopes,
        next_right_envelopes,
    );

    let prev_left = Mutex::new(prev_left);
    let next_left = Mutex::new(next_left);
    let prev_right = Mutex::new(prev_right);
    let next_right = Mutex::new(next_right);

    let prev_left_id_mapper = Mutex::new(prev_left_id_mapper);
    let next_left_id_mapper = Mutex::new(next_left_id_mapper);
    let prev_right_id_mapper = Mutex::new(prev_right_id_mapper);

    for chunks in patch.chunks(10) {
        chunks.par_iter().try_for_each(|hunk| {
            debug!("processing hunk {:?}…", hunk);

            let op = || {
                let prev_left = prev_left
                    .lock()
                    .map_err(|err| Error::GetBackendLockError(err.to_string()))?;
                let next_left = next_left
                    .lock()
                    .map_err(|err| Error::GetBackendLockError(err.to_string()))?;
                let prev_right = prev_right
                    .lock()
                    .map_err(|err| Error::GetBackendLockError(err.to_string()))?;
                let next_right = next_right
                    .lock()
                    .map_err(|err| Error::GetBackendLockError(err.to_string()))?;

                let mut prev_left_id_mapper = prev_left_id_mapper
                    .lock()
                    .map_err(|err| Error::GetIdMapperLockError(err.to_string()))?;
                let mut next_left_id_mapper = next_left_id_mapper
                    .lock()
                    .map_err(|err| Error::GetIdMapperLockError(err.to_string()))?;
                let mut prev_right_id_mapper = prev_right_id_mapper
                    .lock()
                    .map_err(|err| Error::GetIdMapperLockError(err.to_string()))?;

                match hunk {
                    Hunk::CopyEmail(internal_id, flags, source, target) => {
                        let internal_ids = vec![internal_id.as_str()];
                        let emails = match source {
                            HunkKind::PrevLeft => {
                                prev_left.get_emails_internal("inbox", internal_ids)
                            }
                            HunkKind::NextLeft => {
                                next_left.get_emails_internal("inbox", internal_ids)
                            }
                            HunkKind::PrevRight => {
                                prev_right.get_emails_internal("inbox", internal_ids)
                            }
                            HunkKind::NextRight => {
                                next_right.get_emails_internal("inbox", internal_ids)
                            }
                        }?;
                        let emails = emails.to_vec();
                        let email = emails
                            .first()
                            .ok_or_else(|| Error::FindEmailError(internal_id.clone()))?;

                        match target {
                            HunkKind::PrevLeft => prev_left_id_mapper.insert([(
                                prev_left.add_email_internal("inbox", email.raw()?, &flags)?,
                                internal_id,
                            )]),
                            HunkKind::NextLeft => next_left_id_mapper.insert([(
                                next_left.add_email_internal("inbox", email.raw()?, &flags)?,
                                internal_id,
                            )]),
                            HunkKind::PrevRight => prev_right_id_mapper.insert([(
                                prev_right.add_email_internal("inbox", email.raw()?, &flags)?,
                                internal_id,
                            )]),
                            HunkKind::NextRight => {
                                next_right.add_email_internal("inbox", email.raw()?, &flags)?;
                            }
                        };
                    }
                    Hunk::RemoveEmail(internal_id, target) => {
                        let internal_ids = vec![internal_id.as_str()];

                        match target {
                            HunkKind::PrevLeft => {
                                prev_left_id_mapper.remove(&internal_id);
                                prev_left.delete_emails_internal("inbox", internal_ids.clone())
                            }
                            HunkKind::NextLeft => {
                                next_left_id_mapper.remove(&internal_id);
                                next_left.delete_emails_internal("inbox", internal_ids.clone())
                            }
                            HunkKind::PrevRight => {
                                prev_right_id_mapper.remove(&internal_id);
                                prev_right.delete_emails_internal("inbox", internal_ids.clone())
                            }
                            HunkKind::NextRight => {
                                next_right.delete_emails_internal("inbox", internal_ids.clone())
                            }
                        }?;
                    }
                    Hunk::AddFlag(internal_id, flag, target) => {
                        let internal_ids = vec![internal_id.as_str()];
                        let flags = Flags::from_iter([flag.clone()]);
                        match target {
                            HunkKind::PrevLeft => {
                                prev_left.add_flags_internal("inbox", internal_ids.clone(), &flags)
                            }
                            HunkKind::NextLeft => {
                                next_left.add_flags_internal("inbox", internal_ids.clone(), &flags)
                            }
                            HunkKind::PrevRight => {
                                prev_right.add_flags_internal("inbox", internal_ids.clone(), &flags)
                            }
                            HunkKind::NextRight => {
                                next_right.add_flags_internal("inbox", internal_ids.clone(), &flags)
                            }
                        }?;
                    }
                    Hunk::RemoveFlag(internal_id, flag, target) => {
                        let internal_ids = vec![internal_id.as_str()];
                        let flags = Flags::from_iter([flag.clone()]);
                        match target {
                            HunkKind::PrevLeft => prev_left.remove_flags_internal(
                                "inbox",
                                internal_ids.clone(),
                                &flags,
                            ),
                            HunkKind::NextLeft => next_left.remove_flags_internal(
                                "inbox",
                                internal_ids.clone(),
                                &flags,
                            ),
                            HunkKind::PrevRight => prev_right.remove_flags_internal(
                                "inbox",
                                internal_ids.clone(),
                                &flags,
                            ),
                            HunkKind::NextRight => next_right.remove_flags_internal(
                                "inbox",
                                internal_ids.clone(),
                                &flags,
                            ),
                        }?;
                    }
                };

                Result::Ok(())
            };

            if let Err(err) = op() {
                warn!("error while processing hunk {:?}, skipping it", hunk);
                error!("{}", err.to_string());
            }

            Result::Ok(())
        })?;
    }

    prev_left_id_mapper.lock().unwrap().save()?;
    next_left_id_mapper.lock().unwrap().save()?;
    prev_right_id_mapper.lock().unwrap().save()?;

    Ok(())
}

pub fn build_patch(
    prev_left: Envelopes,
    next_left: Envelopes,
    prev_right: Envelopes,
    next_right: Envelopes,
) -> Patch {
    let mut patch = vec![];
    let mut ids = HashSet::new();

    // Gathers all existing ids found in all envelopes.
    ids.extend(prev_left.iter().map(|(id, _)| id.as_str()));
    ids.extend(next_left.iter().map(|(id, _)| id.as_str()));
    ids.extend(prev_right.iter().map(|(id, _)| id.as_str()));
    ids.extend(next_right.iter().map(|(id, _)| id.as_str()));

    // Given the matrice prev_left × next_left × prev_right × next_right,
    // checks every 2⁴ = 16 possibilities:
    for id in ids {
        let prev_left = prev_left.get(id);
        let next_left = next_left.get(id);
        let prev_right = prev_right.get(id);
        let next_right = next_right.get(id);

        match (prev_left, next_left, prev_right, next_right) {
            // 0000
            //
            // The id exists nowhere, which cannot happen since the
            // ids hashset has been built from envelopes id.
            (None, None, None, None) => (),

            // 0001
            //
            // The id only exists in the next right side, which means
            // a new email has been added next right and needs to be
            // synchronized prev left, next left and prev right sides.
            (None, None, None, Some(next_right)) => patch.extend([
                Hunk::CopyEmail(
                    next_right.internal_id.clone(),
                    next_right.flags.clone(),
                    HunkKind::NextRight,
                    HunkKind::PrevLeft,
                ),
                Hunk::CopyEmail(
                    next_right.internal_id.clone(),
                    next_right.flags.clone(),
                    HunkKind::NextRight,
                    HunkKind::NextLeft,
                ),
                Hunk::CopyEmail(
                    next_right.internal_id.clone(),
                    next_right.flags.clone(),
                    HunkKind::NextRight,
                    HunkKind::PrevRight,
                ),
            ]),

            // 0010
            //
            // The id only exists in the prev right side, which means
            // an email is outdated prev right side and needs to be
            // removed.
            (None, None, Some(prev_right), None) => patch.push(Hunk::RemoveEmail(
                prev_right.internal_id.clone(),
                HunkKind::PrevRight,
            )),

            // 0011
            //
            // The id exists in the right side but not in the left
            // side, which means there is a conflict. Since we cannot
            // determine which side (left removed or right added) is
            // the most up-to-date, it is safer to consider the right
            // added side up-to-date in order not to lose data.
            //
            // TODO: make this behaviour customizable.
            (None, None, Some(prev_right), Some(next_right)) => {
                let mut all_flags: HashSet<Flag> = HashSet::default();
                all_flags.extend(prev_right.flags.0.clone());
                all_flags.extend(next_right.flags.0.clone());

                let mut flags = Flags::default();

                for flag in all_flags {
                    match (prev_right.flags.get(&flag), next_right.flags.get(&flag)) {
                        // The flag exists nowhere, which cannot
                        // happen since the flags hashset has been
                        // built from envelopes flags.
                        (None, None) => (),

                        // The [`Flag::Deleted`] exists next right
                        // side but not prev right side, which means
                        // there is a conflict. Since we cannot
                        // determine which side is the most
                        // up-to-date, it is safer to remove the flag
                        // next right side in order not to lose data.
                        //
                        // TODO: make this behaviour customizable.
                        (None, Some(&Flag::Deleted)) => patch.push(Hunk::RemoveFlag(
                            next_right.internal_id.clone(),
                            Flag::Deleted,
                            HunkKind::NextRight,
                        )),

                        // The flag exists next right side but not
                        // prev right side, which means there is a
                        // conflict. Since we cannot determine which
                        // side is the most up-to-date, it is safer to
                        // add the flag prev right side in order not
                        // to lose data.
                        //
                        // TODO: make this behaviour customizable.
                        (None, Some(flag)) => {
                            flags.insert(flag.clone());
                            patch.push(Hunk::AddFlag(
                                prev_right.internal_id.clone(),
                                flag.clone(),
                                HunkKind::PrevRight,
                            ));
                        }

                        // The [`Flag::Deleted`] exists prev right
                        // side but not next right side, which means
                        // there is a conflict. Since we cannot
                        // determine which side is the most
                        // up-to-date, it is safer to remove the flag
                        // prev right side in order not to lose data.
                        //
                        // TODO: make this behaviour customizable.
                        (Some(&Flag::Deleted), None) => patch.push(Hunk::RemoveFlag(
                            prev_right.internal_id.clone(),
                            Flag::Deleted,
                            HunkKind::PrevRight,
                        )),

                        // The flag exists prev right side but not
                        // next right side, which means there is a
                        // conflict. Since we cannot determine which
                        // side is the most up-to-date, it is safer to
                        // add the flag next right side in order not
                        // to lose data.
                        //
                        // TODO: make this behaviour customizable.
                        (Some(flag), None) => {
                            flags.insert(flag.clone());
                            patch.push(Hunk::AddFlag(
                                next_right.internal_id.clone(),
                                flag.clone(),
                                HunkKind::NextRight,
                            ))
                        }

                        // The flag exists everywhere, nothing to do.
                        (Some(_), Some(flag)) => {
                            flags.insert(flag.clone());
                        }
                    }
                }

                patch.extend([
                    Hunk::CopyEmail(
                        next_right.internal_id.clone(),
                        flags.clone(),
                        HunkKind::NextRight,
                        HunkKind::PrevLeft,
                    ),
                    Hunk::CopyEmail(
                        next_right.internal_id.clone(),
                        flags.clone(),
                        HunkKind::NextRight,
                        HunkKind::NextLeft,
                    ),
                ]);
            }

            // 0100
            //
            // The id only exists in the next left side, which means a
            // new email has been added next left side and needs to be
            // added prev left, prev right and next right sides.
            (None, Some(next_left), None, None) => patch.extend([
                Hunk::CopyEmail(
                    next_left.internal_id.clone(),
                    next_left.flags.clone(),
                    HunkKind::NextLeft,
                    HunkKind::PrevLeft,
                ),
                Hunk::CopyEmail(
                    next_left.internal_id.clone(),
                    next_left.flags.clone(),
                    HunkKind::NextLeft,
                    HunkKind::PrevRight,
                ),
                Hunk::CopyEmail(
                    next_left.internal_id.clone(),
                    next_left.flags.clone(),
                    HunkKind::NextLeft,
                    HunkKind::NextRight,
                ),
            ]),

            // 0101
            //
            // The id exists in both next sides, which means a new
            // (same) email has been added both sides and the most
            // recent needs to by kept.
            //
            // NOTE: this case should never happen: new emails
            // internal identifier are feeded with auto-generated UUID
            // v4 and should (in theory) never conflict, but we
            // implement this case for the sake of exhaustiveness.
            (None, Some(next_left), None, Some(next_right)) => {
                match (next_left.date.as_ref(), next_right.date.as_ref()) {
                    // The date exists only on the next left side, so
                    // we keep the next left side and remove the next
                    // right side.
                    (Some(_), None) => patch.extend([
                        Hunk::RemoveEmail(id.to_owned(), HunkKind::NextRight),
                        Hunk::CopyEmail(
                            next_left.internal_id.clone(),
                            next_left.flags.clone(),
                            HunkKind::NextLeft,
                            HunkKind::PrevLeft,
                        ),
                        Hunk::CopyEmail(
                            next_left.internal_id.clone(),
                            next_left.flags.clone(),
                            HunkKind::NextLeft,
                            HunkKind::PrevRight,
                        ),
                        Hunk::CopyEmail(
                            next_left.internal_id.clone(),
                            next_left.flags.clone(),
                            HunkKind::NextLeft,
                            HunkKind::NextRight,
                        ),
                    ]),

                    // The date exists in both next side and the left
                    // date is greater than the right date, so we keep
                    // the next left side.
                    (Some(date_left), Some(date_right)) if date_left > date_right => {
                        patch.extend([
                            Hunk::RemoveEmail(id.to_owned(), HunkKind::NextRight),
                            Hunk::CopyEmail(
                                next_left.internal_id.clone(),
                                next_left.flags.clone(),
                                HunkKind::NextLeft,
                                HunkKind::PrevLeft,
                            ),
                            Hunk::CopyEmail(
                                next_left.internal_id.clone(),
                                next_left.flags.clone(),
                                HunkKind::NextLeft,
                                HunkKind::PrevRight,
                            ),
                            Hunk::CopyEmail(
                                next_left.internal_id.clone(),
                                next_left.flags.clone(),
                                HunkKind::NextLeft,
                                HunkKind::NextRight,
                            ),
                        ])
                    }

                    // For all other cases we keep the next right side.
                    (None, None) | (None, Some(_)) | (Some(_), Some(_)) => patch.extend([
                        Hunk::RemoveEmail(id.to_owned(), HunkKind::NextLeft),
                        Hunk::CopyEmail(
                            next_right.internal_id.clone(),
                            next_right.flags.clone(),
                            HunkKind::NextRight,
                            HunkKind::PrevLeft,
                        ),
                        Hunk::CopyEmail(
                            next_right.internal_id.clone(),
                            next_right.flags.clone(),
                            HunkKind::NextRight,
                            HunkKind::NextLeft,
                        ),
                        Hunk::CopyEmail(
                            next_right.internal_id.clone(),
                            next_right.flags.clone(),
                            HunkKind::NextRight,
                            HunkKind::PrevRight,
                        ),
                    ]),
                }
            }

            // 0110
            //
            // The id exists in the next left and prev right sides,
            // which means a new (same) email has been added left side
            // but removed right side. Since we cannot determine which
            // side (left added or right removed) is the most
            // up-to-date, it is safer to consider the left added side
            // up-to-date in order not to lose data.
            //
            // TODO: make this behaviour customizable.
            (None, Some(next_left), Some(_), None) => patch.extend([
                Hunk::RemoveEmail(id.to_owned(), HunkKind::PrevRight),
                Hunk::CopyEmail(
                    next_left.internal_id.clone(),
                    next_left.flags.clone(),
                    HunkKind::NextLeft,
                    HunkKind::PrevLeft,
                ),
                Hunk::CopyEmail(
                    next_left.internal_id.clone(),
                    next_left.flags.clone(),
                    HunkKind::NextLeft,
                    HunkKind::PrevRight,
                ),
                Hunk::CopyEmail(
                    next_left.internal_id.clone(),
                    next_left.flags.clone(),
                    HunkKind::NextLeft,
                    HunkKind::NextRight,
                ),
            ]),

            // 0111
            //
            // The id exists everywhere except in prev left side,
            // which means the prev left side is outdated and needs to
            // be updated.
            (None, Some(next_left), Some(prev_right), Some(next_right)) => {
                patch.push(Hunk::CopyEmail(
                    next_left.internal_id.clone(),
                    Flags::default(),
                    HunkKind::NextLeft,
                    HunkKind::PrevLeft,
                ));
                patch.extend(build_flags_patch(
                    None,
                    Some(next_left),
                    Some(prev_right),
                    Some(next_right),
                ))
            }

            // 1000
            //
            // The id only exists in the prev left side, which means
            // an old email is not up-to-date and needs to be deleted.
            (Some(prev_left), None, None, None) => patch.push(Hunk::RemoveEmail(
                prev_left.internal_id.clone(),
                HunkKind::PrevLeft,
            )),

            // 1001
            //
            // The id exists in the prev left and next right sides,
            // which means a new (same) email has been removed left
            // side but added right side. Since we cannot determine
            // which side (left removed or right added) is the most
            // up-to-date, it is safer to consider the right added
            // side up-to-date in order not to lose data.
            //
            // TODO: make this behaviour customizable.
            (Some(_), None, None, Some(next_right)) => patch.extend([
                Hunk::RemoveEmail("id".into(), HunkKind::PrevLeft),
                Hunk::CopyEmail(
                    next_right.internal_id.clone(),
                    next_right.flags.clone(),
                    HunkKind::NextRight,
                    HunkKind::PrevLeft,
                ),
                Hunk::CopyEmail(
                    next_right.internal_id.clone(),
                    next_right.flags.clone(),
                    HunkKind::NextRight,
                    HunkKind::NextLeft,
                ),
                Hunk::CopyEmail(
                    next_right.internal_id.clone(),
                    next_right.flags.clone(),
                    HunkKind::NextRight,
                    HunkKind::PrevRight,
                ),
            ]),

            // 1010
            //
            // The id exists in prev sides but not next sides, which
            // means an outdated email needs to be removed everywhere.
            (Some(prev_left), None, Some(prev_right), None) => patch.extend([
                Hunk::RemoveEmail(prev_left.internal_id.clone(), HunkKind::PrevLeft),
                Hunk::RemoveEmail(prev_right.internal_id.clone(), HunkKind::PrevRight),
            ]),

            // 1011
            //
            // The id exists everywhere except in next left side,
            // which means an email has been removed next left side
            // and needs to be removed everywhere else.
            (Some(prev_left), None, Some(prev_right), Some(next_right)) => patch.extend([
                Hunk::RemoveEmail(prev_left.internal_id.clone(), HunkKind::PrevLeft),
                Hunk::RemoveEmail(prev_right.internal_id.clone(), HunkKind::PrevRight),
                Hunk::RemoveEmail(next_right.internal_id.clone(), HunkKind::NextRight),
            ]),

            // 1100
            //
            // The id exists in the left side but not in the right
            // side, which means there is a conflict. Since we cannot
            // determine which side (left added or right removed) is
            // the most up-to-date, it is safer to consider the left
            // added side up-to-date in order not to lose data.
            //
            // TODO: make this behaviour customizable.
            (Some(prev_left), Some(next_left), None, None) => {
                patch.extend([
                    Hunk::CopyEmail(
                        next_left.internal_id.clone(),
                        Flags::default(),
                        HunkKind::NextLeft,
                        HunkKind::PrevRight,
                    ),
                    Hunk::CopyEmail(
                        next_left.internal_id.clone(),
                        Flags::default(),
                        HunkKind::NextLeft,
                        HunkKind::NextRight,
                    ),
                ]);
                patch.extend(build_flags_patch(
                    Some(prev_left),
                    Some(next_left),
                    None,
                    None,
                ))
            }

            // 1101
            //
            // The id exists everywhere except in prev right side,
            // which means an email is missing prev right side and
            // needs to be synchronized.
            (Some(prev_left), Some(next_left), None, Some(next_right)) => {
                patch.push(Hunk::CopyEmail(
                    next_right.internal_id.clone(),
                    Flags::default(),
                    HunkKind::NextRight,
                    HunkKind::PrevRight,
                ));
                patch.extend(build_flags_patch(
                    Some(prev_left),
                    Some(next_left),
                    None,
                    Some(next_right),
                ))
            }

            // 1110
            //
            // The id exists everywhere except in next right side,
            // which means an email has been removed next right side
            // and needs to be removed everywhere else.
            (Some(prev_left), Some(next_left), Some(prev_right), None) => patch.extend([
                Hunk::RemoveEmail(prev_left.internal_id.clone(), HunkKind::PrevLeft),
                Hunk::RemoveEmail(next_left.internal_id.clone(), HunkKind::NextLeft),
                Hunk::RemoveEmail(prev_right.internal_id.clone(), HunkKind::PrevRight),
            ]),

            // 1111
            //
            // The id exists everywhere, which means flags need to be
            // synchronized.
            (Some(prev_left), Some(next_left), Some(prev_right), Some(next_right)) => {
                patch.extend(build_flags_patch(
                    Some(prev_left),
                    Some(next_left),
                    Some(prev_right),
                    Some(next_right),
                ));
            }
        }
    }

    patch
}

pub fn build_flags_patch(
    prev_left: Option<&Envelope>,
    next_left: Option<&Envelope>,
    prev_right: Option<&Envelope>,
    next_right: Option<&Envelope>,
) -> Patch {
    let mut patch = vec![];

    let mut all_flags: HashSet<Flag> = HashSet::default();
    all_flags.extend(prev_left.map(|e| e.flags.clone().0).unwrap_or_default());
    all_flags.extend(next_left.map(|e| e.flags.clone().0).unwrap_or_default());
    all_flags.extend(prev_right.map(|e| e.flags.clone().0).unwrap_or_default());
    all_flags.extend(next_right.map(|e| e.flags.clone().0).unwrap_or_default());

    for flag in all_flags {
        match (
            prev_left.and_then(|e| e.flags.get(&flag)),
            next_left.and_then(|e| e.flags.get(&flag)),
            prev_right.and_then(|e| e.flags.get(&flag)),
            next_right.and_then(|e| e.flags.get(&flag)),
        ) {
            // The flag exists nowhere, which cannot happen since the
            // flags hashset is built from envelopes flags.
            (None, None, None, None) => (),

            // The flag only exists in the next right side, which
            // means a new flag needs to be added prev left, next left
            // and prev right sides.
            (None, None, None, Some(_)) => {
                if let Some(prev_left) = prev_left {
                    patch.push(Hunk::AddFlag(
                        prev_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevLeft,
                    ))
                }
                if let Some(next_left) = next_left {
                    patch.push(Hunk::AddFlag(
                        next_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::NextLeft,
                    ))
                }
                if let Some(prev_right) = prev_right {
                    patch.push(Hunk::AddFlag(
                        prev_right.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevRight,
                    ))
                }
            }

            // The flag only exists in the prev right, which means it
            // is an old flag that needs to be removed.
            (None, None, Some(_), None) => {
                if let Some(prev_right) = prev_right {
                    patch.push(Hunk::RemoveFlag(
                        prev_right.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevRight,
                    ))
                }
            }

            // The flag exists in right sides but not in left sides,
            // which means there is a conflict. Since we cannot
            // determine which side (left removed or right added) is
            // the most up-to-date, it is safer to consider the right
            // added side up-to-date (or left removed in case of
            // [`Flag::Deleted`]) in order not to lose data.
            //
            // TODO: make this behaviour customizable.
            (None, None, Some(_), Some(_)) if flag == Flag::Deleted => {
                if let Some(prev_right) = prev_right {
                    patch.push(Hunk::RemoveFlag(
                        prev_right.internal_id.clone(),
                        Flag::Deleted,
                        HunkKind::PrevRight,
                    ))
                }
                if let Some(next_right) = next_right {
                    patch.push(Hunk::RemoveFlag(
                        next_right.internal_id.clone(),
                        Flag::Deleted,
                        HunkKind::NextRight,
                    ))
                }
            }
            (None, None, Some(_), Some(_)) => {
                if let Some(prev_left) = prev_left {
                    patch.push(Hunk::AddFlag(
                        prev_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevLeft,
                    ))
                }
                if let Some(next_left) = next_left {
                    patch.push(Hunk::AddFlag(
                        next_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::NextLeft,
                    ))
                }
            }

            // The flag only exists in the next left side, which means
            // a new flag has been added and needs to be synchronized
            // prev left, prev right and next right sides.
            (None, Some(_), None, None) => {
                if let Some(prev_left) = prev_left {
                    patch.push(Hunk::AddFlag(
                        prev_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevLeft,
                    ))
                }
                if let Some(prev_right) = prev_right {
                    patch.push(Hunk::AddFlag(
                        prev_right.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevRight,
                    ))
                }
                if let Some(next_right) = next_right {
                    patch.push(Hunk::AddFlag(
                        next_right.internal_id.clone(),
                        flag.clone(),
                        HunkKind::NextRight,
                    ))
                }
            }

            // The flag exists in next sides but not in prev sides,
            // which means a new (same) flag has been added both sides
            // at the same time and needs to be synchronized with prev
            // sides.
            (None, Some(_), None, Some(_)) => {
                if let Some(prev_left) = prev_left {
                    patch.push(Hunk::AddFlag(
                        prev_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevLeft,
                    ))
                }
                if let Some(prev_right) = prev_right {
                    patch.push(Hunk::AddFlag(
                        prev_right.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevRight,
                    ))
                }
            }

            // The flag exists in the next left and prev right sides,
            // which means a new (same) flag has been added left side
            // but removed right side. Since we cannot determine which
            // side (left added or right removed) is the most
            // up-to-date, it is safer to consider the left added side
            // up-to-date (or right removed in case of
            // [`Flag::Deleted`]) in order not to lose data.
            //
            // TODO: make this behaviour customizable.
            (None, Some(_), Some(_), None) if flag == Flag::Deleted => {
                if let Some(next_left) = next_left {
                    patch.push(Hunk::RemoveFlag(
                        next_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::NextLeft,
                    ))
                }
                if let Some(prev_right) = prev_right {
                    patch.push(Hunk::RemoveFlag(
                        prev_right.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevRight,
                    ))
                }
            }
            (None, Some(_), Some(_), None) => {
                if let Some(prev_left) = prev_left {
                    patch.push(Hunk::AddFlag(
                        prev_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevLeft,
                    ))
                }
                if let Some(next_right) = next_right {
                    patch.push(Hunk::AddFlag(
                        next_right.internal_id.clone(),
                        flag.clone(),
                        HunkKind::NextRight,
                    ))
                }
            }

            // The flag exists everywhere except in prev left side,
            // which means the prev left is outdated and needs to be
            // updated.
            (None, Some(_), Some(_), Some(_)) => {
                if let Some(prev_left) = prev_left {
                    patch.push(Hunk::AddFlag(
                        prev_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevLeft,
                    ))
                }
            }

            // The flag exists only in prev left side, which means the
            // prev left is outdated and needs to be updated.
            (Some(_), None, None, None) => {
                if let Some(prev_left) = prev_left {
                    patch.push(Hunk::RemoveFlag(
                        prev_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevLeft,
                    ))
                }
            }

            // The flag exists in the prev left and next right side,
            // which means a new (same) flag has been removed left
            // side but added right side. Since we cannot determine
            // which side (left removed or right added) is the most
            // up-to-date, it is safer to consider the right added
            // side up-to-date (or left removed in case of
            // [`Flag::Deleted`]) in order not to lose data.
            //
            // TODO: make this behaviour customizable.
            (Some(_), None, None, Some(_)) if flag == Flag::Deleted => {
                if let Some(prev_left) = prev_left {
                    patch.push(Hunk::RemoveFlag(
                        prev_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevLeft,
                    ))
                }
                if let Some(next_right) = next_right {
                    patch.push(Hunk::RemoveFlag(
                        next_right.internal_id.clone(),
                        flag.clone(),
                        HunkKind::NextRight,
                    ))
                }
            }
            (Some(_), None, None, Some(_)) => {
                if let Some(next_left) = next_left {
                    patch.push(Hunk::AddFlag(
                        next_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::NextLeft,
                    ))
                }
                if let Some(prev_right) = prev_right {
                    patch.push(Hunk::AddFlag(
                        prev_right.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevRight,
                    ))
                }
            }

            // The flag exists in prev sides, which means a old flag
            // needs to be removed everywhere.
            (Some(_), None, Some(_), None) => {
                if let Some(prev_left) = prev_left {
                    patch.push(Hunk::RemoveFlag(
                        prev_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevLeft,
                    ))
                }
                if let Some(prev_right) = prev_right {
                    patch.push(Hunk::RemoveFlag(
                        prev_right.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevRight,
                    ))
                }
            }

            // The flag exists everywhere except in next left, which
            // means a flag has been removed next left and needs to be
            // removed everywhere else.
            (Some(_), None, Some(_), Some(_)) => {
                if let Some(prev_left) = prev_left {
                    patch.push(Hunk::RemoveFlag(
                        prev_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevLeft,
                    ))
                }
                if let Some(prev_right) = prev_right {
                    patch.push(Hunk::RemoveFlag(
                        prev_right.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevRight,
                    ))
                }
                if let Some(next_right) = next_right {
                    patch.push(Hunk::RemoveFlag(
                        next_right.internal_id.clone(),
                        flag.clone(),
                        HunkKind::NextRight,
                    ))
                }
            }

            // The flag exists in the prev left and next left sides,
            // which means there is a conflict. Since we cannot
            // determine which side (left added or right removed) is
            // the most up-to-date, it is safer to consider the left
            // added side up-to-date (or right removed in case of
            // [`Flag::Deleted`]) in order not to lose data.
            //
            // TODO: make this behaviour customizable.
            (Some(_), Some(_), None, None) if flag == Flag::Deleted => {
                if let Some(prev_left) = prev_left {
                    patch.push(Hunk::RemoveFlag(
                        prev_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevLeft,
                    ))
                }
                if let Some(next_left) = next_left {
                    patch.push(Hunk::RemoveFlag(
                        next_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::NextLeft,
                    ))
                }
            }
            (Some(_), Some(_), None, None) => {
                if let Some(prev_right) = prev_right {
                    patch.push(Hunk::AddFlag(
                        prev_right.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevRight,
                    ))
                }
                if let Some(next_right) = next_right {
                    patch.push(Hunk::AddFlag(
                        next_right.internal_id.clone(),
                        flag.clone(),
                        HunkKind::NextRight,
                    ))
                }
            }

            // The flag exists everywhere except in prev right side,
            // which means the prev right flag is outdated and needs
            // to be updated.
            (Some(_), Some(_), None, Some(_)) => {
                if let Some(prev_right) = prev_right {
                    patch.push(Hunk::AddFlag(
                        prev_right.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevRight,
                    ))
                }
            }

            // The flag exists everywhere except in next right side,
            // which means a flag has been removed next right side and
            // needs to be removed everywhere else.
            (Some(_), Some(_), Some(_), None) => {
                if let Some(prev_left) = prev_left {
                    patch.push(Hunk::RemoveFlag(
                        prev_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevLeft,
                    ))
                }
                if let Some(next_left) = next_left {
                    patch.push(Hunk::RemoveFlag(
                        next_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::NextLeft,
                    ))
                }
                if let Some(prev_right) = prev_right {
                    patch.push(Hunk::RemoveFlag(
                        prev_right.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevRight,
                    ))
                }
            }

            // The flag exists everything, nothing to do.
            (Some(_), Some(_), Some(_), Some(_)) => (),
        }
    }

    patch
}

#[derive(Debug, Default)]
pub struct SyncIdMapper {
    path: PathBuf,
    pub map: HashMap<String, String>,
}

impl SyncIdMapper {
    pub fn new<P: AsRef<Path>>(dir: P) -> Result<Self> {
        let mut mapper = Self::default();
        mapper.path = dir.as_ref().join(".himalaya-sync-id-map");

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&mapper.path)
            .map_err(|err| Error::OpenHashMapFileError(err, mapper.path.clone()))?;
        let reader = BufReader::new(file);

        for line in reader.lines() {
            let line = line.map_err(Error::ReadHashMapFileLineError)?;
            let (left_id, right_id) = line
                .split_once(' ')
                .ok_or_else(|| Error::ParseLineError(line.clone()))?;
            mapper.map.insert(left_id.to_string(), right_id.to_string());
        }

        Ok(mapper)
    }

    pub fn map_ids(&self, envelopes: Envelopes) -> Envelopes {
        envelopes
            .into_iter()
            .map(|(internal_id, mut envelope)| {
                envelope.id = if let Some(id) = self.map.get(&internal_id) {
                    id.clone()
                } else {
                    Uuid::new_v4().to_string()
                };
                (envelope.id.clone(), envelope)
            })
            .collect()
    }

    pub fn insert<L: ToString, R: ToString, T: IntoIterator<Item = (L, R)>>(&mut self, lines: T) {
        for (left, right) in lines {
            self.map.insert(left.to_string(), right.to_string());
        }
    }

    pub fn remove<K: AsRef<str>>(&mut self, key: K) {
        self.map.remove(key.as_ref());
    }

    pub fn save(&mut self) -> Result<()> {
        let mut buffer = String::default();

        for (left, right) in &self.map {
            buffer.push_str(left);
            buffer.push(' ');
            buffer.push_str(right);
            buffer.push('\n');
        }

        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.path)
            .map_err(|err| Error::OpenHashMapFileError(err, self.path.clone()))?
            .write(buffer.as_bytes())
            .map_err(|err| Error::WriteHashMapFileError(err, self.path.clone()))?;

        Ok(())
    }
}

impl<'a> FromIterator<(&'a str, &'a str)> for SyncIdMapper {
    fn from_iter<T: IntoIterator<Item = (&'a str, &'a str)>>(iter: T) -> Self {
        let mut id_mapper = Self::default();

        for (left, right) in iter {
            id_mapper.map.insert(left.to_string(), right.to_string());
        }

        id_mapper
    }
}

#[cfg(test)]
mod sync {
    use crate::{Envelope, Flag, Flags};

    use super::{Envelopes, Hunk, HunkKind, Patch};

    #[test]
    fn build_flags_patch() {
        assert_eq!(
            super::build_flags_patch(None, None, None, None),
            vec![] as Patch,
        );

        assert_eq!(
            super::build_flags_patch(
                None,
                None,
                None,
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
            ),
            vec![] as Patch,
        );

        assert_eq!(
            super::build_flags_patch(
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
            ),
            vec![
                Hunk::AddFlag("id".into(), Flag::Seen, HunkKind::PrevLeft),
                Hunk::AddFlag("id".into(), Flag::Seen, HunkKind::NextLeft),
                Hunk::AddFlag("id".into(), Flag::Seen, HunkKind::PrevRight),
            ],
        );

        assert_eq!(
            super::build_flags_patch(
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
            ),
            vec![Hunk::RemoveFlag(
                "id".into(),
                Flag::Seen,
                HunkKind::PrevRight,
            )],
        );

        assert_eq!(
            super::build_flags_patch(
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
            ),
            vec![
                Hunk::AddFlag("id".into(), Flag::Seen, HunkKind::PrevLeft),
                Hunk::AddFlag("id".into(), Flag::Seen, HunkKind::NextLeft),
            ],
        );

        assert_eq!(
            super::build_flags_patch(
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
            ),
            vec![
                Hunk::AddFlag("id".into(), Flag::Seen, HunkKind::PrevLeft),
                Hunk::AddFlag("id".into(), Flag::Seen, HunkKind::PrevRight),
                Hunk::AddFlag("id".into(), Flag::Seen, HunkKind::NextRight),
            ],
        );

        assert_eq!(
            super::build_flags_patch(
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
            ),
            vec![
                Hunk::AddFlag("id".into(), Flag::Seen, HunkKind::PrevLeft),
                Hunk::AddFlag("id".into(), Flag::Seen, HunkKind::PrevRight),
            ],
        );

        assert_eq!(
            super::build_flags_patch(
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
            ),
            vec![
                Hunk::AddFlag("id".into(), Flag::Seen, HunkKind::PrevLeft),
                Hunk::AddFlag("id".into(), Flag::Seen, HunkKind::NextRight),
            ],
        );

        assert_eq!(
            super::build_flags_patch(
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
            ),
            vec![Hunk::AddFlag("id".into(), Flag::Seen, HunkKind::PrevLeft)],
        );

        assert_eq!(
            super::build_flags_patch(
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
            ),
            vec![Hunk::RemoveFlag(
                "id".into(),
                Flag::Seen,
                HunkKind::PrevLeft
            )],
        );

        assert_eq!(
            super::build_flags_patch(
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
            ),
            vec![
                Hunk::AddFlag("id".into(), Flag::Seen, HunkKind::NextLeft),
                Hunk::AddFlag("id".into(), Flag::Seen, HunkKind::PrevRight),
            ],
        );

        assert_eq!(
            super::build_flags_patch(
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
            ),
            vec![
                Hunk::RemoveFlag("id".into(), Flag::Seen, HunkKind::PrevLeft),
                Hunk::RemoveFlag("id".into(), Flag::Seen, HunkKind::PrevRight),
            ],
        );

        assert_eq!(
            super::build_flags_patch(
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
            ),
            vec![
                Hunk::RemoveFlag("id".into(), Flag::Seen, HunkKind::PrevLeft),
                Hunk::RemoveFlag("id".into(), Flag::Seen, HunkKind::PrevRight),
                Hunk::RemoveFlag("id".into(), Flag::Seen, HunkKind::NextRight),
            ],
        );

        assert_eq!(
            super::build_flags_patch(
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
            ),
            vec![
                Hunk::AddFlag("id".into(), Flag::Seen, HunkKind::PrevRight),
                Hunk::AddFlag("id".into(), Flag::Seen, HunkKind::NextRight),
            ],
        );

        assert_eq!(
            super::build_flags_patch(
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
            ),
            vec![Hunk::AddFlag("id".into(), Flag::Seen, HunkKind::PrevRight,)],
        );

        assert_eq!(
            super::build_flags_patch(
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                }),
            ),
            vec![
                Hunk::RemoveFlag("id".into(), Flag::Seen, HunkKind::PrevLeft),
                Hunk::RemoveFlag("id".into(), Flag::Seen, HunkKind::NextLeft),
                Hunk::RemoveFlag("id".into(), Flag::Seen, HunkKind::PrevRight),
            ],
        );

        assert_eq!(
            super::build_flags_patch(
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    internal_id: "id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                }),
            ),
            vec![] as Patch,
        );
    }

    #[test]
    fn build_patch_0000() {
        let prev_left = Envelopes::default();
        let next_left = Envelopes::default();
        let prev_right = Envelopes::default();
        let next_right = Envelopes::default();

        let patch = super::build_patch(prev_left, next_left, prev_right, next_right);
        assert_eq!(patch, vec![] as Patch);
    }

    #[test]
    fn build_patch_0001() {
        let prev_left = Envelopes::default();
        let next_left = Envelopes::default();
        let prev_right = Envelopes::default();
        let next_right = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);

        let patch = super::build_patch(prev_left, next_left, prev_right, next_right);
        assert_eq!(
            patch,
            vec![
                Hunk::CopyEmail(
                    "id".into(),
                    "seen".into(),
                    HunkKind::NextRight,
                    HunkKind::PrevLeft
                ),
                Hunk::CopyEmail(
                    "id".into(),
                    "seen".into(),
                    HunkKind::NextRight,
                    HunkKind::NextLeft
                ),
                Hunk::CopyEmail(
                    "id".into(),
                    "seen".into(),
                    HunkKind::NextRight,
                    HunkKind::PrevRight
                ),
            ],
        );
    }

    #[test]
    fn build_patch_0010() {
        let prev_left = Envelopes::default();
        let next_left = Envelopes::default();
        let prev_right = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let next_right = Envelopes::default();

        let patch = super::build_patch(prev_left, next_left, prev_right, next_right);
        assert_eq!(
            patch,
            vec![Hunk::RemoveEmail("id".into(), HunkKind::PrevRight)],
        );
    }

    #[test]
    fn build_patch_0011_same_flags() {
        let prev_left = Envelopes::default();
        let next_left = Envelopes::default();
        let prev_right = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let next_right = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);

        let patch = super::build_patch(prev_left, next_left, prev_right, next_right);
        assert_eq!(
            patch,
            vec![
                Hunk::CopyEmail(
                    "id".into(),
                    "seen".into(),
                    HunkKind::NextRight,
                    HunkKind::PrevLeft
                ),
                Hunk::CopyEmail(
                    "id".into(),
                    "seen".into(),
                    HunkKind::NextRight,
                    HunkKind::NextLeft
                ),
            ],
        );
    }

    #[test]
    fn build_patch_0011_different_flags() {
        let prev_left = Envelopes::default();
        let next_left = Envelopes::default();
        let prev_right = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen replied".into(),
                ..Envelope::default()
            },
        )]);
        let next_right = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen flagged deleted".into(),
                ..Envelope::default()
            },
        )]);

        let patch = super::build_patch(prev_left, next_left, prev_right, next_right);

        assert_eq!(patch.len(), 5);
        assert!(patch.contains(&Hunk::AddFlag(
            "id".into(),
            Flag::Answered,
            HunkKind::NextRight
        )));
        assert!(patch.contains(&Hunk::AddFlag(
            "id".into(),
            Flag::Flagged,
            HunkKind::PrevRight
        )));
        assert!(patch.contains(&Hunk::RemoveFlag(
            "id".into(),
            Flag::Deleted,
            HunkKind::NextRight
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "id".into(),
            "seen replied flagged".into(),
            HunkKind::NextRight,
            HunkKind::PrevLeft,
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "id".into(),
            "seen replied flagged".into(),
            HunkKind::NextRight,
            HunkKind::NextLeft
        )));
    }

    #[test]
    fn build_patch_0100() {
        let prev_left = Envelopes::default();
        let next_left = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let prev_right = Envelopes::default();
        let next_right = Envelopes::default();

        let patch = super::build_patch(prev_left, next_left, prev_right, next_right);

        assert_eq!(
            patch,
            vec![
                Hunk::CopyEmail(
                    "id".into(),
                    "seen".into(),
                    HunkKind::NextLeft,
                    HunkKind::PrevLeft,
                ),
                Hunk::CopyEmail(
                    "id".into(),
                    "seen".into(),
                    HunkKind::NextLeft,
                    HunkKind::PrevRight,
                ),
                Hunk::CopyEmail(
                    "id".into(),
                    "seen".into(),
                    HunkKind::NextLeft,
                    HunkKind::NextRight,
                )
            ],
        );
    }

    #[test]
    fn build_patch_0101() {
        let prev_left = Envelopes::default();
        let next_left = Envelopes::from_iter([
            (
                "id-1".into(),
                Envelope {
                    id: "id-1".into(),
                    internal_id: "id-1".into(),
                    flags: "seen".into(),
                    date: Some("2022-01-01 00:00:00".into()),
                    ..Envelope::default()
                },
            ),
            (
                "id-2".into(),
                Envelope {
                    id: "id-2".into(),
                    internal_id: "id-2".into(),
                    flags: "seen".into(),
                    date: Some("2022-01-01 00:00:00".into()),
                    ..Envelope::default()
                },
            ),
            (
                "id-3".into(),
                Envelope {
                    id: "id-3".into(),
                    internal_id: "id-3".into(),
                    flags: "seen".into(),
                    date: None,
                    ..Envelope::default()
                },
            ),
            (
                "id-4".into(),
                Envelope {
                    id: "id-4".into(),
                    internal_id: "id-4".into(),
                    flags: "seen".into(),
                    date: None,
                    ..Envelope::default()
                },
            ),
            (
                "id-5".into(),
                Envelope {
                    id: "id-5".into(),
                    internal_id: "id-5".into(),
                    flags: "seen".into(),
                    date: Some("2022-01-01 00:00:00".into()),
                    ..Envelope::default()
                },
            ),
        ]);
        let prev_right = Envelopes::default();
        let next_right = Envelopes::from_iter([
            (
                "id-1".into(),
                Envelope {
                    id: "id-1".into(),
                    internal_id: "id-1".into(),
                    flags: "seen".into(),
                    date: None,
                    ..Envelope::default()
                },
            ),
            (
                "id-2".into(),
                Envelope {
                    id: "id-1".into(),
                    internal_id: "id-2".into(),
                    flags: "seen".into(),
                    date: Some("2021-01-01 00:00:00".into()),
                    ..Envelope::default()
                },
            ),
            (
                "id-3".into(),
                Envelope {
                    id: "id-3".into(),
                    internal_id: "id-3".into(),
                    flags: "seen".into(),
                    date: None,
                    ..Envelope::default()
                },
            ),
            (
                "id-4".into(),
                Envelope {
                    id: "id-4".into(),
                    internal_id: "id-4".into(),
                    flags: "seen".into(),
                    date: Some("2022-01-01 00:00:00".into()),
                    ..Envelope::default()
                },
            ),
            (
                "id-5".into(),
                Envelope {
                    id: "id-5".into(),
                    internal_id: "id-5".into(),
                    flags: "seen".into(),
                    date: Some("2022-01-01 00:00:00".into()),
                    ..Envelope::default()
                },
            ),
        ]);

        let patch = super::build_patch(prev_left, next_left, prev_right, next_right);

        assert_eq!(patch.len(), 20);
        assert!(patch.contains(&Hunk::RemoveEmail("id-1".into(), HunkKind::NextRight)));
        assert!(patch.contains(&Hunk::CopyEmail(
            "id-1".into(),
            "seen".into(),
            HunkKind::NextLeft,
            HunkKind::PrevLeft
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "id-1".into(),
            "seen".into(),
            HunkKind::NextLeft,
            HunkKind::PrevRight
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "id-1".into(),
            "seen".into(),
            HunkKind::NextLeft,
            HunkKind::NextRight
        )));
        assert!(patch.contains(&Hunk::RemoveEmail("id-2".into(), HunkKind::NextRight)));
        assert!(patch.contains(&Hunk::CopyEmail(
            "id-2".into(),
            "seen".into(),
            HunkKind::NextLeft,
            HunkKind::PrevLeft
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "id-2".into(),
            "seen".into(),
            HunkKind::NextLeft,
            HunkKind::PrevRight
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "id-2".into(),
            "seen".into(),
            HunkKind::NextLeft,
            HunkKind::NextRight
        )));
        assert!(patch.contains(&Hunk::RemoveEmail("id-3".into(), HunkKind::NextLeft)));
        assert!(patch.contains(&Hunk::CopyEmail(
            "id-3".into(),
            "seen".into(),
            HunkKind::NextRight,
            HunkKind::PrevLeft
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "id-3".into(),
            "seen".into(),
            HunkKind::NextRight,
            HunkKind::NextLeft
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "id-3".into(),
            "seen".into(),
            HunkKind::NextRight,
            HunkKind::PrevRight
        )));
        assert!(patch.contains(&Hunk::RemoveEmail("id-4".into(), HunkKind::NextLeft)));
        assert!(patch.contains(&Hunk::CopyEmail(
            "id-4".into(),
            "seen".into(),
            HunkKind::NextRight,
            HunkKind::PrevLeft
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "id-4".into(),
            "seen".into(),
            HunkKind::NextRight,
            HunkKind::NextLeft
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "id-4".into(),
            "seen".into(),
            HunkKind::NextRight,
            HunkKind::PrevRight
        )));
        assert!(patch.contains(&Hunk::RemoveEmail("id-5".into(), HunkKind::NextLeft)));
        assert!(patch.contains(&Hunk::CopyEmail(
            "id-5".into(),
            "seen".into(),
            HunkKind::NextRight,
            HunkKind::PrevLeft
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "id-5".into(),
            "seen".into(),
            HunkKind::NextRight,
            HunkKind::NextLeft
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "id-5".into(),
            "seen".into(),
            HunkKind::NextRight,
            HunkKind::PrevRight
        )));
    }

    #[test]
    fn build_patch_0110() {
        let prev_left = Envelopes::default();
        let next_left = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let prev_right = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "flagged".into(),
                ..Envelope::default()
            },
        )]);
        let next_right = Envelopes::default();

        let patch = super::build_patch(prev_left, next_left, prev_right, next_right);

        assert_eq!(
            patch,
            vec![
                Hunk::RemoveEmail("id".into(), HunkKind::PrevRight),
                Hunk::CopyEmail(
                    "id".into(),
                    "seen".into(),
                    HunkKind::NextLeft,
                    HunkKind::PrevLeft
                ),
                Hunk::CopyEmail(
                    "id".into(),
                    "seen".into(),
                    HunkKind::NextLeft,
                    HunkKind::PrevRight
                ),
                Hunk::CopyEmail(
                    "id".into(),
                    "seen".into(),
                    HunkKind::NextLeft,
                    HunkKind::NextRight
                )
            ],
        );
    }

    #[test]
    fn build_patch_0111() {
        let prev_left = Envelopes::default();
        let next_left = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let prev_right = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let next_right = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);

        let patch = super::build_patch(prev_left, next_left, prev_right, next_right);

        assert_eq!(
            patch,
            vec![Hunk::CopyEmail(
                "id".into(),
                Flags::default(),
                HunkKind::NextLeft,
                HunkKind::PrevLeft,
            ),]
        );
    }

    #[test]
    fn build_patch_1000() {
        let prev_left = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let next_left = Envelopes::default();
        let prev_right = Envelopes::default();
        let next_right = Envelopes::default();

        let patch = super::build_patch(prev_left, next_left, prev_right, next_right);

        assert_eq!(
            patch,
            vec![Hunk::RemoveEmail("id".into(), HunkKind::PrevLeft)]
        );
    }

    #[test]
    fn build_patch_1001() {
        let prev_left = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let next_left = Envelopes::default();
        let prev_right = Envelopes::default();
        let next_right = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);

        let patch = super::build_patch(prev_left, next_left, prev_right, next_right);

        assert_eq!(
            patch,
            vec![
                Hunk::RemoveEmail("id".into(), HunkKind::PrevLeft),
                Hunk::CopyEmail(
                    "id".into(),
                    "seen".into(),
                    HunkKind::NextRight,
                    HunkKind::PrevLeft,
                ),
                Hunk::CopyEmail(
                    "id".into(),
                    "seen".into(),
                    HunkKind::NextRight,
                    HunkKind::NextLeft,
                ),
                Hunk::CopyEmail(
                    "id".into(),
                    "seen".into(),
                    HunkKind::NextRight,
                    HunkKind::PrevRight,
                ),
            ]
        );
    }

    #[test]
    fn build_patch_1010() {
        let prev_left = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let next_left = Envelopes::default();
        let prev_right = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let next_right = Envelopes::default();

        let patch = super::build_patch(prev_left, next_left, prev_right, next_right);

        assert_eq!(
            patch,
            vec![
                Hunk::RemoveEmail("id".into(), HunkKind::PrevLeft),
                Hunk::RemoveEmail("id".into(), HunkKind::PrevRight),
            ]
        );
    }

    #[test]
    fn build_patch_1011() {
        let prev_left = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let next_left = Envelopes::default();
        let prev_right = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let next_right = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);

        let patch = super::build_patch(prev_left, next_left, prev_right, next_right);

        assert_eq!(
            patch,
            vec![
                Hunk::RemoveEmail("id".into(), HunkKind::PrevLeft),
                Hunk::RemoveEmail("id".into(), HunkKind::PrevRight),
                Hunk::RemoveEmail("id".into(), HunkKind::NextRight),
            ]
        );
    }

    #[test]
    fn build_patch_1100() {
        let prev_left = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let next_left = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let prev_right = Envelopes::default();
        let next_right = Envelopes::default();

        let patch = super::build_patch(prev_left, next_left, prev_right, next_right);

        assert_eq!(
            patch,
            vec![
                Hunk::CopyEmail(
                    "id".into(),
                    Flags::default(),
                    HunkKind::NextLeft,
                    HunkKind::PrevRight,
                ),
                Hunk::CopyEmail(
                    "id".into(),
                    Flags::default(),
                    HunkKind::NextLeft,
                    HunkKind::NextRight,
                ),
            ]
        );
    }

    #[test]
    fn build_patch_1101() {
        let prev_left = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let next_left = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let prev_right = Envelopes::default();
        let next_right = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);

        let patch = super::build_patch(prev_left, next_left, prev_right, next_right);

        assert_eq!(
            patch,
            vec![Hunk::CopyEmail(
                "id".into(),
                Flags::default(),
                HunkKind::NextRight,
                HunkKind::PrevRight,
            ),]
        );
    }

    #[test]
    fn build_patch_1110() {
        let prev_left = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let next_left = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let prev_right = Envelopes::from_iter([(
            "id".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let next_right = Envelopes::default();

        let patch = super::build_patch(prev_left, next_left, prev_right, next_right);

        assert_eq!(
            patch,
            vec![
                Hunk::RemoveEmail("id".into(), HunkKind::PrevLeft),
                Hunk::RemoveEmail("id".into(), HunkKind::NextLeft),
                Hunk::RemoveEmail("id".into(), HunkKind::PrevRight),
            ]
        );
    }
}
