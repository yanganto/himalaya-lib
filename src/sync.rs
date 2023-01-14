use chrono::{DateTime, Local};
use dirs::data_dir;
use log::{debug, error, warn};
use rayon::prelude::*;
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    fs::{create_dir_all, OpenOptions},
    io::{self, prelude::*, BufReader},
    path::{Path, PathBuf},
    result,
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
    #[error("cannot get backend lock")]
    CreateXdgDataDirsError(#[source] io::Error),
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

    #[error(transparent)]
    CacheError(#[from] sqlite::Error),
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
pub type Folder = String;
pub type Source = HunkKind;
pub type Target = HunkKind;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Hunk {
    CopyEmail(Folder, Envelope, Source, Target),
    RemoveEmail(Folder, Id, Target),
    AddFlag(Folder, Envelope, Flag, Target),
    RemoveFlag(Folder, Id, Flag, Target),
}

type Patch = Vec<Hunk>;

const SELECT: &str = "
    SELECT id, hash, account, folder, GROUP_CONCAT(flag) AS flags, message_id, sender, subject, date
    FROM envelopes
    WHERE account = ?
    AND folder = ?
    GROUP BY hash
";
const INSERT: &str = "INSERT INTO envelopes VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)";
const DELETE_EMAIL: &str = "DELETE FROM envelopes WHERE account = ? AND folder = ? AND id = ?";
const DELETE_FLAG: &str =
    "DELETE FROM envelopes WHERE account = ? AND folder = ? AND id = ? AND flag = ?";

pub fn sync<B: ThreadSafeBackend>(account: &AccountConfig, remote: &B) -> Result<()> {
    debug!("starting synchronization");

    if !account.sync {
        debug!(
            "synchronization not enabled for account {}, exiting",
            account.name
        );
        return Ok(());
    }

    let sync_dir = match account.sync_dir.as_ref().filter(|dir| dir.is_dir()) {
        Some(dir) => dir.clone(),
        None => {
            warn!("sync dir not set or invalid, falling back to $XDG_DATA_HOME/himalaya");
            data_dir()
                .map(|dir| dir.join("himalaya"))
                .ok_or(Error::GetXdgDataDirError)?
        }
    };

    create_dir_all(&sync_dir).map_err(Error::CreateXdgDataDirsError)?;

    let cache = sqlite::Connection::open_with_full_mutex(sync_dir.join("database.sqlite"))?;

    cache.execute(
        "CREATE TABLE IF NOT EXISTS envelopes (
            id         TEXT NOT NULL,
            hash       TEXT NOT NULL,
            account    TEXT NOT NULL,
            folder     TEXT NOT NULL,
            flag       TEXT NOT NULL,
            message_id TEXT NOT NULL,
            sender     TEXT NOT NULL,
            subject    TEXT NOT NULL,
            date       DATETIME
        );",
    )?;

    let local = MaildirBackend::new(
        Cow::Borrowed(account),
        Cow::Owned(MaildirConfig {
            root_dir: sync_dir.join(&account.name),
        }),
    )?;

    let next_right_envelopes: Envelopes = HashMap::from_iter(
        remote
            .list_envelopes("inbox", 0, 0)?
            .iter()
            .map(|envelope| (envelope.hash("inbox"), envelope.clone())),
    );

    println!("next_right_envelopes: {:#?}", next_right_envelopes);

    let prev_right_envelopes: Envelopes = HashMap::from_iter(
        cache
            .prepare(SELECT)?
            .into_iter()
            .bind((1, account.name.as_str()))?
            .bind((2, "inbox"))?
            .collect::<sqlite::Result<Vec<_>>>()?
            .iter()
            .map(|row| {
                let envelope = Envelope {
                    id: row.read::<&str, _>("id").into(),
                    internal_id: row.read::<&str, _>("id").into(),
                    flags: Flags::from_iter(
                        row.read::<&str, _>("flags").split(",").map(Flag::from),
                    ),
                    message_id: row.read::<&str, _>("message_id").into(),
                    subject: row.read::<&str, _>("subject").into(),
                    sender: row.read::<&str, _>("sender").into(),
                    date: {
                        let date_str = row.read::<&str, _>("date");
                        match DateTime::parse_from_rfc3339(date_str) {
                            Ok(date) => Some(date.with_timezone(&Local)),
                            Err(err) => {
                                warn!("invalid date {}, skipping it: {}", date_str, err);
                                None
                            }
                        }
                    },
                };

                (envelope.hash("inbox"), envelope)
            }),
    );

    println!("prev_right_envelopes: {:#?}", prev_right_envelopes);

    let next_left_envelopes: Envelopes = HashMap::from_iter(
        local
            .list_envelopes("inbox", 0, 0)?
            .iter()
            .map(|envelope| (envelope.hash("inbox"), envelope.clone())),
    );

    println!("next_left_envelopes: {:#?}", next_left_envelopes);

    let prev_left_envelopes: Envelopes = HashMap::from_iter(
        cache
            .prepare(SELECT)?
            .into_iter()
            .bind((1, format!("{}:cache", account.name).as_str()))?
            .bind((2, "inbox"))?
            .collect::<sqlite::Result<Vec<_>>>()?
            .iter()
            .map(|row| {
                let envelope = Envelope {
                    id: row.read::<&str, _>("id").into(),
                    internal_id: row.read::<&str, _>("id").into(),
                    flags: Flags::from_iter(
                        row.read::<&str, _>("flags").split(",").map(Flag::from),
                    ),
                    message_id: row.read::<&str, _>("message_id").into(),
                    sender: row.read::<&str, _>("sender").into(),
                    subject: row.read::<&str, _>("subject").into(),
                    date: {
                        let date_str = row.read::<&str, _>("date");
                        match DateTime::parse_from_rfc3339(date_str) {
                            Ok(date) => Some(date.with_timezone(&Local)),
                            Err(err) => {
                                warn!("invalid date {}, skipping it: {}", date_str, err);
                                None
                            }
                        }
                    },
                };

                (row.read::<&str, _>("hash").into(), envelope)
            }),
    );

    println!("prev_left_envelopes: {:#?}", prev_left_envelopes);

    let patch = build_patch(
        "inbox",
        prev_left_envelopes,
        next_left_envelopes,
        prev_right_envelopes,
        next_right_envelopes,
    );

    println!("patch: {:#?}", patch);
    debug!("patch length: {}", patch.len());

    for (batch_num, chunks) in patch.chunks(3).enumerate() {
        debug!("processing batch {}", batch_num + 1);

        chunks
            .par_iter()
            .enumerate()
            .try_for_each(|(hunk_num, hunk)| {
                debug!("processing hunk {}: {:?}", hunk_num + 1, hunk);

                let op = || {
                    match hunk {
                        Hunk::CopyEmail(folder, envelope, source, target) => {
                            let internal_ids = vec![envelope.internal_id.as_str()];
                            let emails = match source {
                                HunkKind::PrevLeft => {
                                    panic!("prev left");
                                }
                                HunkKind::NextLeft => {
                                    local.get_emails_internal(&folder, internal_ids)
                                }
                                HunkKind::PrevRight => {
                                    panic!("prev right");
                                }
                                HunkKind::NextRight => {
                                    remote.get_emails_internal(&folder, internal_ids)
                                }
                            }?;
                            let emails = emails.to_vec();
                            let email = emails.first().ok_or_else(|| {
                                Error::FindEmailError(envelope.internal_id.clone())
                            })?;

                            match target {
                                HunkKind::PrevLeft => {
                                    let mut statement = cache.prepare(INSERT)?;
                                    for flag in envelope.flags.iter() {
                                        statement.reset()?;
                                        statement.bind((1, envelope.internal_id.as_str()))?;
                                        statement.bind((2, envelope.hash(&folder).as_str()))?;
                                        statement.bind((
                                            3,
                                            format!("{}:cache", account.name).as_str(),
                                        ))?;
                                        statement.bind((4, folder.as_str()))?;
                                        statement.bind((5, flag.to_string().as_str()))?;
                                        statement.bind((6, envelope.message_id.as_str()))?;
                                        statement.bind((7, envelope.sender.as_str()))?;
                                        statement.bind((8, envelope.subject.as_str()))?;
                                        statement.bind((
                                            9,
                                            match envelope.date {
                                                Some(date) => date.to_rfc3339().into(),
                                                None => sqlite::Value::Null,
                                            },
                                        ))?;
                                        statement.next()?;
                                    }
                                }
                                HunkKind::NextLeft => {
                                    local.add_email_internal(
                                        &folder,
                                        email.raw()?,
                                        &envelope.flags,
                                    )?;
                                }
                                HunkKind::PrevRight => {
                                    let mut statement = cache.prepare(INSERT)?;
                                    for flag in envelope.flags.iter() {
                                        statement.reset()?;
                                        statement.bind((1, envelope.internal_id.as_str()))?;
                                        statement.bind((2, envelope.hash(&folder).as_str()))?;
                                        statement.bind((3, account.name.as_str()))?;
                                        statement.bind((4, folder.as_str()))?;
                                        statement.bind((5, flag.to_string().as_str()))?;
                                        statement.bind((6, envelope.message_id.as_str()))?;
                                        statement.bind((7, envelope.sender.as_str()))?;
                                        statement.bind((8, envelope.subject.as_str()))?;
                                        statement.bind((
                                            9,
                                            match envelope.date {
                                                Some(date) => date.to_rfc3339().into(),
                                                None => sqlite::Value::Null,
                                            },
                                        ))?;
                                        statement.next()?;
                                    }
                                }
                                HunkKind::NextRight => {
                                    remote.add_email_internal(
                                        &folder,
                                        email.raw()?,
                                        &envelope.flags,
                                    )?;
                                }
                            };
                        }
                        Hunk::RemoveEmail(folder, internal_id, target) => {
                            let internal_ids = vec![internal_id.as_str()];

                            match target {
                                HunkKind::PrevLeft | HunkKind::PrevRight => {
                                    let mut statement = cache.prepare(DELETE_EMAIL)?;
                                    statement.bind((1, account.name.as_str()))?;
                                    statement.bind((2, folder.as_str()))?;
                                    statement.bind((3, internal_id.as_str()))?;
                                    statement.next()?;
                                }
                                HunkKind::NextLeft => {
                                    local.delete_emails_internal("inbox", internal_ids.clone())?;
                                }
                                HunkKind::NextRight => {
                                    remote.delete_emails_internal("inbox", internal_ids.clone())?;
                                }
                            };
                        }
                        Hunk::AddFlag(folder, envelope, flag, target) => {
                            let internal_ids = vec![envelope.internal_id.as_str()];
                            let flags = Flags::from_iter([flag.clone()]);

                            match target {
                                HunkKind::PrevLeft | HunkKind::PrevRight => {
                                    let mut statement = cache.prepare(INSERT)?;
                                    statement.bind((1, envelope.internal_id.as_str()))?;
                                    statement.bind((2, envelope.hash(&folder).as_str()))?;
                                    statement.bind((3, account.name.as_str()))?;
                                    statement.bind((4, folder.as_str()))?;
                                    statement.bind((5, flag.to_string().as_str()))?;
                                    statement.bind((6, envelope.message_id.as_str()))?;
                                    statement.bind((7, envelope.sender.as_str()))?;
                                    statement.bind((8, envelope.subject.as_str()))?;
                                    statement.bind((
                                        9,
                                        match envelope.date {
                                            Some(date) => date.to_rfc3339().into(),
                                            None => sqlite::Value::Null,
                                        },
                                    ))?;
                                    statement.next()?;
                                }
                                HunkKind::NextLeft => {
                                    local.add_flags_internal(
                                        "inbox",
                                        internal_ids.clone(),
                                        &flags,
                                    )?;
                                }
                                HunkKind::NextRight => {
                                    remote.add_flags_internal(
                                        "inbox",
                                        internal_ids.clone(),
                                        &flags,
                                    )?;
                                }
                            };
                        }
                        Hunk::RemoveFlag(folder, internal_id, flag, target) => {
                            let internal_ids = vec![internal_id.as_str()];
                            let flags = Flags::from_iter([flag.clone()]);

                            match target {
                                HunkKind::PrevLeft | HunkKind::PrevRight => {
                                    let mut statement = cache.prepare(DELETE_FLAG)?;
                                    statement.bind((1, account.name.as_str()))?;
                                    statement.bind((2, folder.as_str()))?;
                                    statement.bind((3, internal_id.as_str()))?;
                                    statement.bind((4, flag.to_string().as_str()))?;
                                    statement.next()?;
                                }
                                HunkKind::NextLeft => {
                                    local.remove_flags_internal(
                                        "inbox",
                                        internal_ids.clone(),
                                        &flags,
                                    )?;
                                }
                                HunkKind::NextRight => {
                                    remote.remove_flags_internal(
                                        "inbox",
                                        internal_ids.clone(),
                                        &flags,
                                    )?;
                                }
                            };
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

    Ok(())
}

pub fn build_patch<F: ToString + Clone>(
    folder: F,
    prev_left: Envelopes,
    next_left: Envelopes,
    prev_right: Envelopes,
    next_right: Envelopes,
) -> Patch {
    let mut patch = vec![];
    let mut hashes = HashSet::new();

    // Gathers all existing hashes found in all envelopes.
    hashes.extend(prev_left.iter().map(|(hash, _)| hash.as_str()));
    hashes.extend(next_left.iter().map(|(hash, _)| hash.as_str()));
    hashes.extend(prev_right.iter().map(|(hash, _)| hash.as_str()));
    hashes.extend(next_right.iter().map(|(hash, _)| hash.as_str()));

    // Given the matrice prev_left × next_left × prev_right × next_right,
    // checks every 2⁴ = 16 possibilities:
    for hash in hashes {
        let prev_left = prev_left.get(hash);
        let next_left = next_left.get(hash);
        let prev_right = prev_right.get(hash);
        let next_right = next_right.get(hash);

        match (prev_left, next_left, prev_right, next_right) {
            // 0000
            //
            // The hash exists nowhere, which cannot happen since the
            // hash hashset has been built from envelopes hash.
            (None, None, None, None) => (),

            // 0001
            //
            // The id only exists in the next right side, which means
            // a new email has been added next right and needs to be
            // synchronized prev left, next left and prev right sides.
            (None, None, None, Some(next_right)) => patch.extend([
                Hunk::CopyEmail(
                    folder.to_string(),
                    next_right.clone(),
                    HunkKind::NextRight,
                    HunkKind::PrevLeft,
                ),
                Hunk::CopyEmail(
                    folder.to_string(),
                    next_right.clone(),
                    HunkKind::NextRight,
                    HunkKind::NextLeft,
                ),
                Hunk::CopyEmail(
                    folder.to_string(),
                    next_right.clone(),
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
                folder.to_string(),
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
                            folder.to_string(),
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
                                folder.to_string(),
                                prev_right.clone(),
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
                            folder.to_string(),
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
                                folder.to_string(),
                                next_right.clone(),
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
                        folder.to_string(),
                        Envelope {
                            flags: flags.clone(),
                            ..next_right.clone()
                        },
                        HunkKind::NextRight,
                        HunkKind::PrevLeft,
                    ),
                    Hunk::CopyEmail(
                        folder.to_string(),
                        Envelope {
                            flags: flags.clone(),
                            ..next_right.clone()
                        },
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
                    folder.to_string(),
                    next_left.clone(),
                    HunkKind::NextLeft,
                    HunkKind::PrevLeft,
                ),
                Hunk::CopyEmail(
                    folder.to_string(),
                    next_left.clone(),
                    HunkKind::NextLeft,
                    HunkKind::PrevRight,
                ),
                Hunk::CopyEmail(
                    folder.to_string(),
                    next_left.clone(),
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
                        Hunk::RemoveEmail(
                            folder.to_string(),
                            next_right.internal_id.clone(),
                            HunkKind::NextRight,
                        ),
                        Hunk::CopyEmail(
                            folder.to_string(),
                            next_left.clone(),
                            HunkKind::NextLeft,
                            HunkKind::PrevLeft,
                        ),
                        Hunk::CopyEmail(
                            folder.to_string(),
                            next_left.clone(),
                            HunkKind::NextLeft,
                            HunkKind::PrevRight,
                        ),
                        Hunk::CopyEmail(
                            folder.to_string(),
                            next_left.clone(),
                            HunkKind::NextLeft,
                            HunkKind::NextRight,
                        ),
                    ]),

                    // The date exists in both next side and the left
                    // date is greater than the right date, so we keep
                    // the next left side.
                    (Some(date_left), Some(date_right)) if date_left > date_right => {
                        patch.extend([
                            Hunk::RemoveEmail(
                                folder.to_string(),
                                next_right.internal_id.clone(),
                                HunkKind::NextRight,
                            ),
                            Hunk::CopyEmail(
                                folder.to_string(),
                                next_left.clone(),
                                HunkKind::NextLeft,
                                HunkKind::PrevLeft,
                            ),
                            Hunk::CopyEmail(
                                folder.to_string(),
                                next_left.clone(),
                                HunkKind::NextLeft,
                                HunkKind::PrevRight,
                            ),
                            Hunk::CopyEmail(
                                folder.to_string(),
                                next_left.clone(),
                                HunkKind::NextLeft,
                                HunkKind::NextRight,
                            ),
                        ])
                    }

                    // For all other cases we keep the next right side.
                    (None, None) | (None, Some(_)) | (Some(_), Some(_)) => patch.extend([
                        Hunk::RemoveEmail(
                            folder.to_string(),
                            next_left.internal_id.clone(),
                            HunkKind::NextLeft,
                        ),
                        Hunk::CopyEmail(
                            folder.to_string(),
                            next_right.clone(),
                            HunkKind::NextRight,
                            HunkKind::PrevLeft,
                        ),
                        Hunk::CopyEmail(
                            folder.to_string(),
                            next_right.clone(),
                            HunkKind::NextRight,
                            HunkKind::NextLeft,
                        ),
                        Hunk::CopyEmail(
                            folder.to_string(),
                            next_right.clone(),
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
                Hunk::RemoveEmail(folder.to_string(), hash.to_owned(), HunkKind::PrevRight),
                Hunk::CopyEmail(
                    folder.to_string(),
                    next_left.clone(),
                    HunkKind::NextLeft,
                    HunkKind::PrevLeft,
                ),
                Hunk::CopyEmail(
                    folder.to_string(),
                    next_left.clone(),
                    HunkKind::NextLeft,
                    HunkKind::PrevRight,
                ),
                Hunk::CopyEmail(
                    folder.to_string(),
                    next_left.clone(),
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
                    folder.to_string(),
                    Envelope {
                        flags: Flags::default(),
                        ..next_left.clone()
                    },
                    HunkKind::NextLeft,
                    HunkKind::PrevLeft,
                ));
                patch.extend(build_flags_patch(
                    folder.clone(),
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
                folder.to_string(),
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
            (Some(prev_left), None, None, Some(next_right)) => patch.extend([
                Hunk::RemoveEmail(
                    folder.to_string(),
                    prev_left.internal_id.clone(),
                    HunkKind::PrevLeft,
                ),
                Hunk::CopyEmail(
                    folder.to_string(),
                    next_right.clone(),
                    HunkKind::NextRight,
                    HunkKind::PrevLeft,
                ),
                Hunk::CopyEmail(
                    folder.to_string(),
                    next_right.clone(),
                    HunkKind::NextRight,
                    HunkKind::NextLeft,
                ),
                Hunk::CopyEmail(
                    folder.to_string(),
                    next_right.clone(),
                    HunkKind::NextRight,
                    HunkKind::PrevRight,
                ),
            ]),

            // 1010
            //
            // The id exists in prev sides but not next sides, which
            // means an outdated email needs to be removed everywhere.
            (Some(prev_left), None, Some(prev_right), None) => patch.extend([
                Hunk::RemoveEmail(
                    folder.to_string(),
                    prev_left.internal_id.clone(),
                    HunkKind::PrevLeft,
                ),
                Hunk::RemoveEmail(
                    folder.to_string(),
                    prev_right.internal_id.clone(),
                    HunkKind::PrevRight,
                ),
            ]),

            // 1011
            //
            // The id exists everywhere except in next left side,
            // which means an email has been removed next left side
            // and needs to be removed everywhere else.
            (Some(prev_left), None, Some(prev_right), Some(next_right)) => patch.extend([
                Hunk::RemoveEmail(
                    folder.to_string(),
                    prev_left.internal_id.clone(),
                    HunkKind::PrevLeft,
                ),
                Hunk::RemoveEmail(
                    folder.to_string(),
                    prev_right.internal_id.clone(),
                    HunkKind::PrevRight,
                ),
                Hunk::RemoveEmail(
                    folder.to_string(),
                    next_right.internal_id.clone(),
                    HunkKind::NextRight,
                ),
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
                        folder.to_string(),
                        Envelope {
                            flags: Flags::default(),
                            ..next_left.clone()
                        },
                        HunkKind::NextLeft,
                        HunkKind::PrevRight,
                    ),
                    Hunk::CopyEmail(
                        folder.to_string(),
                        Envelope {
                            flags: Flags::default(),
                            ..next_left.clone()
                        },
                        HunkKind::NextLeft,
                        HunkKind::NextRight,
                    ),
                ]);
                patch.extend(build_flags_patch(
                    folder.clone(),
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
                    folder.to_string(),
                    Envelope {
                        flags: Flags::default(),
                        ..next_right.clone()
                    },
                    HunkKind::NextRight,
                    HunkKind::PrevRight,
                ));
                patch.extend(build_flags_patch(
                    folder.clone(),
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
                Hunk::RemoveEmail(
                    folder.to_string(),
                    prev_left.internal_id.clone(),
                    HunkKind::PrevLeft,
                ),
                Hunk::RemoveEmail(
                    folder.to_string(),
                    next_left.internal_id.clone(),
                    HunkKind::NextLeft,
                ),
                Hunk::RemoveEmail(
                    folder.to_string(),
                    prev_right.internal_id.clone(),
                    HunkKind::PrevRight,
                ),
            ]),

            // 1111
            //
            // The id exists everywhere, which means flags need to be
            // synchronized.
            (Some(prev_left), Some(next_left), Some(prev_right), Some(next_right)) => {
                patch.extend(build_flags_patch(
                    folder.clone(),
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

pub fn build_flags_patch<F: ToString>(
    folder: F,
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
                        folder.to_string(),
                        prev_left.clone(),
                        flag.clone(),
                        HunkKind::PrevLeft,
                    ))
                }
                if let Some(next_left) = next_left {
                    patch.push(Hunk::AddFlag(
                        folder.to_string(),
                        next_left.clone(),
                        flag.clone(),
                        HunkKind::NextLeft,
                    ))
                }
                if let Some(prev_right) = prev_right {
                    patch.push(Hunk::AddFlag(
                        folder.to_string(),
                        prev_right.clone(),
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
                        folder.to_string(),
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
                        folder.to_string(),
                        prev_right.internal_id.clone(),
                        Flag::Deleted,
                        HunkKind::PrevRight,
                    ))
                }
                if let Some(next_right) = next_right {
                    patch.push(Hunk::RemoveFlag(
                        folder.to_string(),
                        next_right.internal_id.clone(),
                        Flag::Deleted,
                        HunkKind::NextRight,
                    ))
                }
            }
            (None, None, Some(_), Some(_)) => {
                if let Some(prev_left) = prev_left {
                    patch.push(Hunk::AddFlag(
                        folder.to_string(),
                        prev_left.clone(),
                        flag.clone(),
                        HunkKind::PrevLeft,
                    ))
                }
                if let Some(next_left) = next_left {
                    patch.push(Hunk::AddFlag(
                        folder.to_string(),
                        next_left.clone(),
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
                        folder.to_string(),
                        prev_left.clone(),
                        flag.clone(),
                        HunkKind::PrevLeft,
                    ))
                }
                if let Some(prev_right) = prev_right {
                    patch.push(Hunk::AddFlag(
                        folder.to_string(),
                        prev_right.clone(),
                        flag.clone(),
                        HunkKind::PrevRight,
                    ))
                }
                if let Some(next_right) = next_right {
                    patch.push(Hunk::AddFlag(
                        folder.to_string(),
                        next_right.clone(),
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
                        folder.to_string(),
                        prev_left.clone(),
                        flag.clone(),
                        HunkKind::PrevLeft,
                    ))
                }
                if let Some(prev_right) = prev_right {
                    patch.push(Hunk::AddFlag(
                        folder.to_string(),
                        prev_right.clone(),
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
                        folder.to_string(),
                        next_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::NextLeft,
                    ))
                }
                if let Some(prev_right) = prev_right {
                    patch.push(Hunk::RemoveFlag(
                        folder.to_string(),
                        prev_right.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevRight,
                    ))
                }
            }
            (None, Some(_), Some(_), None) => {
                if let Some(prev_left) = prev_left {
                    patch.push(Hunk::AddFlag(
                        folder.to_string(),
                        prev_left.clone(),
                        flag.clone(),
                        HunkKind::PrevLeft,
                    ))
                }
                if let Some(next_right) = next_right {
                    patch.push(Hunk::AddFlag(
                        folder.to_string(),
                        next_right.clone(),
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
                        folder.to_string(),
                        prev_left.clone(),
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
                        folder.to_string(),
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
                        folder.to_string(),
                        prev_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevLeft,
                    ))
                }
                if let Some(next_right) = next_right {
                    patch.push(Hunk::RemoveFlag(
                        folder.to_string(),
                        next_right.internal_id.clone(),
                        flag.clone(),
                        HunkKind::NextRight,
                    ))
                }
            }
            (Some(_), None, None, Some(_)) => {
                if let Some(next_left) = next_left {
                    patch.push(Hunk::AddFlag(
                        folder.to_string(),
                        next_left.clone(),
                        flag.clone(),
                        HunkKind::NextLeft,
                    ))
                }
                if let Some(prev_right) = prev_right {
                    patch.push(Hunk::AddFlag(
                        folder.to_string(),
                        prev_right.clone(),
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
                        folder.to_string(),
                        prev_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevLeft,
                    ))
                }
                if let Some(prev_right) = prev_right {
                    patch.push(Hunk::RemoveFlag(
                        folder.to_string(),
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
                        folder.to_string(),
                        prev_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevLeft,
                    ))
                }
                if let Some(prev_right) = prev_right {
                    patch.push(Hunk::RemoveFlag(
                        folder.to_string(),
                        prev_right.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevRight,
                    ))
                }
                if let Some(next_right) = next_right {
                    patch.push(Hunk::RemoveFlag(
                        folder.to_string(),
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
                        folder.to_string(),
                        prev_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevLeft,
                    ))
                }
                if let Some(next_left) = next_left {
                    patch.push(Hunk::RemoveFlag(
                        folder.to_string(),
                        next_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::NextLeft,
                    ))
                }
            }
            (Some(_), Some(_), None, None) => {
                if let Some(prev_right) = prev_right {
                    patch.push(Hunk::AddFlag(
                        folder.to_string(),
                        prev_right.clone(),
                        flag.clone(),
                        HunkKind::PrevRight,
                    ))
                }
                if let Some(next_right) = next_right {
                    patch.push(Hunk::AddFlag(
                        folder.to_string(),
                        next_right.clone(),
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
                        folder.to_string(),
                        prev_right.clone(),
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
                        folder.to_string(),
                        prev_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::PrevLeft,
                    ))
                }
                if let Some(next_left) = next_left {
                    patch.push(Hunk::RemoveFlag(
                        folder.to_string(),
                        next_left.internal_id.clone(),
                        flag.clone(),
                        HunkKind::NextLeft,
                    ))
                }
                if let Some(prev_right) = prev_right {
                    patch.push(Hunk::RemoveFlag(
                        folder.to_string(),
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
    use crate::{Envelope, Flag};

    use super::{Envelopes, Hunk, HunkKind, Patch};

    #[test]
    fn build_flags_patch() {
        assert_eq!(
            super::build_flags_patch("inbox", None, None, None, None),
            vec![] as Patch,
        );

        assert_eq!(
            super::build_flags_patch(
                "inbox",
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
                "inbox",
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
                Hunk::AddFlag(
                    "inbox".into(),
                    Envelope {
                        internal_id: "id".into(),
                        ..Envelope::default()
                    },
                    Flag::Seen,
                    HunkKind::PrevLeft
                ),
                Hunk::AddFlag(
                    "inbox".into(),
                    Envelope {
                        internal_id: "id".into(),
                        ..Envelope::default()
                    },
                    Flag::Seen,
                    HunkKind::NextLeft
                ),
                Hunk::AddFlag(
                    "inbox".into(),
                    Envelope {
                        internal_id: "id".into(),
                        ..Envelope::default()
                    },
                    Flag::Seen,
                    HunkKind::PrevRight
                ),
            ],
        );

        assert_eq!(
            super::build_flags_patch(
                "inbox",
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
                "inbox".into(),
                "id".into(),
                Flag::Seen,
                HunkKind::PrevRight,
            )],
        );

        assert_eq!(
            super::build_flags_patch(
                "inbox",
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
                Hunk::AddFlag(
                    "inbox".into(),
                    Envelope {
                        internal_id: "id".into(),
                        ..Envelope::default()
                    },
                    Flag::Seen,
                    HunkKind::PrevLeft
                ),
                Hunk::AddFlag(
                    "inbox".into(),
                    Envelope {
                        internal_id: "id".into(),
                        ..Envelope::default()
                    },
                    Flag::Seen,
                    HunkKind::NextLeft
                ),
            ],
        );

        assert_eq!(
            super::build_flags_patch(
                "inbox",
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
                Hunk::AddFlag(
                    "inbox".into(),
                    Envelope {
                        internal_id: "id".into(),
                        ..Envelope::default()
                    },
                    Flag::Seen,
                    HunkKind::PrevLeft
                ),
                Hunk::AddFlag(
                    "inbox".into(),
                    Envelope {
                        internal_id: "id".into(),
                        ..Envelope::default()
                    },
                    Flag::Seen,
                    HunkKind::PrevRight
                ),
                Hunk::AddFlag(
                    "inbox".into(),
                    Envelope {
                        internal_id: "id".into(),
                        ..Envelope::default()
                    },
                    Flag::Seen,
                    HunkKind::NextRight
                ),
            ],
        );

        assert_eq!(
            super::build_flags_patch(
                "inbox",
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
                Hunk::AddFlag(
                    "inbox".into(),
                    Envelope {
                        internal_id: "id".into(),
                        ..Envelope::default()
                    },
                    Flag::Seen,
                    HunkKind::PrevLeft
                ),
                Hunk::AddFlag(
                    "inbox".into(),
                    Envelope {
                        internal_id: "id".into(),
                        ..Envelope::default()
                    },
                    Flag::Seen,
                    HunkKind::PrevRight
                ),
            ],
        );

        assert_eq!(
            super::build_flags_patch(
                "inbox",
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
                Hunk::AddFlag(
                    "inbox".into(),
                    Envelope {
                        internal_id: "id".into(),
                        ..Envelope::default()
                    },
                    Flag::Seen,
                    HunkKind::PrevLeft
                ),
                Hunk::AddFlag(
                    "inbox".into(),
                    Envelope {
                        internal_id: "id".into(),
                        ..Envelope::default()
                    },
                    Flag::Seen,
                    HunkKind::NextRight
                ),
            ],
        );

        assert_eq!(
            super::build_flags_patch(
                "inbox",
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
            vec![Hunk::AddFlag(
                "inbox".into(),
                Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                },
                Flag::Seen,
                HunkKind::PrevLeft
            )],
        );

        assert_eq!(
            super::build_flags_patch(
                "inbox",
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
                "inbox".into(),
                "id".into(),
                Flag::Seen,
                HunkKind::PrevLeft
            )],
        );

        assert_eq!(
            super::build_flags_patch(
                "inbox",
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
                Hunk::AddFlag(
                    "inbox".into(),
                    Envelope {
                        internal_id: "id".into(),
                        ..Envelope::default()
                    },
                    Flag::Seen,
                    HunkKind::NextLeft
                ),
                Hunk::AddFlag(
                    "inbox".into(),
                    Envelope {
                        internal_id: "id".into(),
                        ..Envelope::default()
                    },
                    Flag::Seen,
                    HunkKind::PrevRight
                ),
            ],
        );

        assert_eq!(
            super::build_flags_patch(
                "inbox",
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
                Hunk::RemoveFlag("inbox".into(), "id".into(), Flag::Seen, HunkKind::PrevLeft),
                Hunk::RemoveFlag("inbox".into(), "id".into(), Flag::Seen, HunkKind::PrevRight),
            ],
        );

        assert_eq!(
            super::build_flags_patch(
                "inbox",
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
                Hunk::RemoveFlag("inbox".into(), "id".into(), Flag::Seen, HunkKind::PrevLeft),
                Hunk::RemoveFlag("inbox".into(), "id".into(), Flag::Seen, HunkKind::PrevRight),
                Hunk::RemoveFlag("inbox".into(), "id".into(), Flag::Seen, HunkKind::NextRight),
            ],
        );

        assert_eq!(
            super::build_flags_patch(
                "inbox",
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
                Hunk::AddFlag(
                    "inbox".into(),
                    Envelope {
                        internal_id: "id".into(),
                        ..Envelope::default()
                    },
                    Flag::Seen,
                    HunkKind::PrevRight
                ),
                Hunk::AddFlag(
                    "inbox".into(),
                    Envelope {
                        internal_id: "id".into(),
                        ..Envelope::default()
                    },
                    Flag::Seen,
                    HunkKind::NextRight
                ),
            ],
        );

        assert_eq!(
            super::build_flags_patch(
                "inbox",
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
            vec![Hunk::AddFlag(
                "inbox".into(),
                Envelope {
                    internal_id: "id".into(),
                    ..Envelope::default()
                },
                Flag::Seen,
                HunkKind::PrevRight,
            )],
        );

        assert_eq!(
            super::build_flags_patch(
                "inbox",
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
                Hunk::RemoveFlag("inbox".into(), "id".into(), Flag::Seen, HunkKind::PrevLeft),
                Hunk::RemoveFlag("inbox".into(), "id".into(), Flag::Seen, HunkKind::NextLeft),
                Hunk::RemoveFlag("inbox".into(), "id".into(), Flag::Seen, HunkKind::PrevRight),
            ],
        );

        assert_eq!(
            super::build_flags_patch(
                "inbox",
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

        let patch = super::build_patch("inbox", prev_left, next_left, prev_right, next_right);
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

        let patch = super::build_patch("inbox", prev_left, next_left, prev_right, next_right);
        assert_eq!(
            patch,
            vec![
                Hunk::CopyEmail(
                    "inbox".into(),
                    Envelope {
                        id: "id".into(),
                        internal_id: "id".into(),
                        flags: "seen".into(),
                        ..Envelope::default()
                    },
                    HunkKind::NextRight,
                    HunkKind::PrevLeft
                ),
                Hunk::CopyEmail(
                    "inbox".into(),
                    Envelope {
                        id: "id".into(),
                        internal_id: "id".into(),
                        flags: "seen".into(),
                        ..Envelope::default()
                    },
                    HunkKind::NextRight,
                    HunkKind::NextLeft
                ),
                Hunk::CopyEmail(
                    "inbox".into(),
                    Envelope {
                        id: "id".into(),
                        internal_id: "id".into(),
                        flags: "seen".into(),
                        ..Envelope::default()
                    },
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

        let patch = super::build_patch("inbox", prev_left, next_left, prev_right, next_right);
        assert_eq!(
            patch,
            vec![Hunk::RemoveEmail(
                "inbox".into(),
                "id".into(),
                HunkKind::PrevRight
            )],
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

        let patch = super::build_patch("inbox", prev_left, next_left, prev_right, next_right);
        assert_eq!(
            patch,
            vec![
                Hunk::CopyEmail(
                    "inbox".into(),
                    Envelope {
                        id: "id".into(),
                        internal_id: "id".into(),
                        flags: "seen".into(),
                        ..Envelope::default()
                    },
                    HunkKind::NextRight,
                    HunkKind::PrevLeft
                ),
                Hunk::CopyEmail(
                    "inbox".into(),
                    Envelope {
                        id: "id".into(),
                        internal_id: "id".into(),
                        flags: "seen".into(),
                        ..Envelope::default()
                    },
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

        let patch = super::build_patch("inbox", prev_left, next_left, prev_right, next_right);

        assert_eq!(patch.len(), 5);
        assert!(patch.contains(&Hunk::AddFlag(
            "inbox".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen flagged deleted".into(),
                ..Envelope::default()
            },
            Flag::Answered,
            HunkKind::NextRight
        )));
        assert!(patch.contains(&Hunk::AddFlag(
            "inbox".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen replied".into(),
                ..Envelope::default()
            },
            Flag::Flagged,
            HunkKind::PrevRight
        )));
        assert!(patch.contains(&Hunk::RemoveFlag(
            "inbox".into(),
            "id".into(),
            Flag::Deleted,
            HunkKind::NextRight
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "inbox".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen replied flagged".into(),
                ..Envelope::default()
            },
            HunkKind::NextRight,
            HunkKind::PrevLeft,
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "inbox".into(),
            Envelope {
                id: "id".into(),
                internal_id: "id".into(),
                flags: "seen replied flagged".into(),
                ..Envelope::default()
            },
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

        let patch = super::build_patch("inbox", prev_left, next_left, prev_right, next_right);

        assert_eq!(
            patch,
            vec![
                Hunk::CopyEmail(
                    "inbox".into(),
                    Envelope {
                        id: "id".into(),
                        internal_id: "id".into(),
                        flags: "seen".into(),
                        ..Envelope::default()
                    },
                    HunkKind::NextLeft,
                    HunkKind::PrevLeft,
                ),
                Hunk::CopyEmail(
                    "inbox".into(),
                    Envelope {
                        id: "id".into(),
                        internal_id: "id".into(),
                        flags: "seen".into(),
                        ..Envelope::default()
                    },
                    HunkKind::NextLeft,
                    HunkKind::PrevRight,
                ),
                Hunk::CopyEmail(
                    "inbox".into(),
                    Envelope {
                        id: "id".into(),
                        internal_id: "id".into(),
                        flags: "seen".into(),
                        ..Envelope::default()
                    },
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
                    date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
                    ..Envelope::default()
                },
            ),
            (
                "id-2".into(),
                Envelope {
                    id: "id-2".into(),
                    internal_id: "id-2".into(),
                    flags: "seen".into(),
                    date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
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
                    date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
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
                    date: Some("2021-01-01T00:00:00-00:00".parse().unwrap()),
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
                    date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
                    ..Envelope::default()
                },
            ),
            (
                "id-5".into(),
                Envelope {
                    id: "id-5".into(),
                    internal_id: "id-5".into(),
                    flags: "seen".into(),
                    date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
                    ..Envelope::default()
                },
            ),
        ]);

        let patch = super::build_patch("inbox", prev_left, next_left, prev_right, next_right);

        assert_eq!(patch.len(), 20);
        assert!(patch.contains(&Hunk::RemoveEmail(
            "inbox".into(),
            "id-1".into(),
            HunkKind::NextRight
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "inbox".into(),
            Envelope {
                id: "id-1".into(),
                internal_id: "id-1".into(),
                flags: "seen".into(),
                date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
                ..Envelope::default()
            },
            HunkKind::NextLeft,
            HunkKind::PrevLeft
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "inbox".into(),
            Envelope {
                id: "id-1".into(),
                internal_id: "id-1".into(),
                flags: "seen".into(),
                date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
                ..Envelope::default()
            },
            HunkKind::NextLeft,
            HunkKind::PrevRight
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "inbox".into(),
            Envelope {
                id: "id-1".into(),
                internal_id: "id-1".into(),
                flags: "seen".into(),
                date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
                ..Envelope::default()
            },
            HunkKind::NextLeft,
            HunkKind::NextRight
        )));
        assert!(patch.contains(&Hunk::RemoveEmail(
            "inbox".into(),
            "id-2".into(),
            HunkKind::NextRight
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "inbox".into(),
            Envelope {
                id: "id-2".into(),
                internal_id: "id-2".into(),
                flags: "seen".into(),
                date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
                ..Envelope::default()
            },
            HunkKind::NextLeft,
            HunkKind::PrevLeft
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "inbox".into(),
            Envelope {
                id: "id-2".into(),
                internal_id: "id-2".into(),
                flags: "seen".into(),
                date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
                ..Envelope::default()
            },
            HunkKind::NextLeft,
            HunkKind::PrevRight
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "inbox".into(),
            Envelope {
                id: "id-2".into(),
                internal_id: "id-2".into(),
                flags: "seen".into(),
                date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
                ..Envelope::default()
            },
            HunkKind::NextLeft,
            HunkKind::NextRight
        )));
        assert!(patch.contains(&Hunk::RemoveEmail(
            "inbox".into(),
            "id-3".into(),
            HunkKind::NextLeft
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "inbox".into(),
            Envelope {
                id: "id-3".into(),
                internal_id: "id-3".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
            HunkKind::NextRight,
            HunkKind::PrevLeft
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "inbox".into(),
            Envelope {
                id: "id-3".into(),
                internal_id: "id-3".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
            HunkKind::NextRight,
            HunkKind::NextLeft
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "inbox".into(),
            Envelope {
                id: "id-3".into(),
                internal_id: "id-3".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
            HunkKind::NextRight,
            HunkKind::PrevRight
        )));
        assert!(patch.contains(&Hunk::RemoveEmail(
            "inbox".into(),
            "id-4".into(),
            HunkKind::NextLeft
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "inbox".into(),
            Envelope {
                id: "id-4".into(),
                internal_id: "id-4".into(),
                flags: "seen".into(),
                date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
                ..Envelope::default()
            },
            HunkKind::NextRight,
            HunkKind::PrevLeft
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "inbox".into(),
            Envelope {
                id: "id-4".into(),
                internal_id: "id-4".into(),
                flags: "seen".into(),
                date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
                ..Envelope::default()
            },
            HunkKind::NextRight,
            HunkKind::NextLeft
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "inbox".into(),
            Envelope {
                id: "id-4".into(),
                internal_id: "id-4".into(),
                flags: "seen".into(),
                date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
                ..Envelope::default()
            },
            HunkKind::NextRight,
            HunkKind::PrevRight
        )));
        assert!(patch.contains(&Hunk::RemoveEmail(
            "inbox".into(),
            "id-5".into(),
            HunkKind::NextLeft
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "inbox".into(),
            Envelope {
                id: "id-5".into(),
                internal_id: "id-5".into(),
                flags: "seen".into(),
                date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
                ..Envelope::default()
            },
            HunkKind::NextRight,
            HunkKind::PrevLeft
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "inbox".into(),
            Envelope {
                id: "id-5".into(),
                internal_id: "id-5".into(),
                flags: "seen".into(),
                date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
                ..Envelope::default()
            },
            HunkKind::NextRight,
            HunkKind::NextLeft
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "inbox".into(),
            Envelope {
                id: "id-5".into(),
                internal_id: "id-5".into(),
                flags: "seen".into(),
                date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
                ..Envelope::default()
            },
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

        let patch = super::build_patch("inbox", prev_left, next_left, prev_right, next_right);

        assert_eq!(
            patch,
            vec![
                Hunk::RemoveEmail("inbox".into(), "id".into(), HunkKind::PrevRight),
                Hunk::CopyEmail(
                    "inbox".into(),
                    Envelope {
                        id: "id".into(),
                        internal_id: "id".into(),
                        flags: "seen".into(),
                        ..Envelope::default()
                    },
                    HunkKind::NextLeft,
                    HunkKind::PrevLeft
                ),
                Hunk::CopyEmail(
                    "inbox".into(),
                    Envelope {
                        id: "id".into(),
                        internal_id: "id".into(),
                        flags: "seen".into(),
                        ..Envelope::default()
                    },
                    HunkKind::NextLeft,
                    HunkKind::PrevRight
                ),
                Hunk::CopyEmail(
                    "inbox".into(),
                    Envelope {
                        id: "id".into(),
                        internal_id: "id".into(),
                        flags: "seen".into(),
                        ..Envelope::default()
                    },
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

        let patch = super::build_patch("inbox", prev_left, next_left, prev_right, next_right);

        assert_eq!(
            patch,
            vec![Hunk::CopyEmail(
                "inbox".into(),
                Envelope {
                    id: "id".into(),
                    internal_id: "id".into(),
                    ..Envelope::default()
                },
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

        let patch = super::build_patch("inbox", prev_left, next_left, prev_right, next_right);

        assert_eq!(
            patch,
            vec![Hunk::RemoveEmail(
                "inbox".into(),
                "id".into(),
                HunkKind::PrevLeft
            )]
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

        let patch = super::build_patch("inbox", prev_left, next_left, prev_right, next_right);

        assert_eq!(
            patch,
            vec![
                Hunk::RemoveEmail("inbox".into(), "id".into(), HunkKind::PrevLeft),
                Hunk::CopyEmail(
                    "inbox".into(),
                    Envelope {
                        id: "id".into(),
                        internal_id: "id".into(),
                        flags: "seen".into(),
                        ..Envelope::default()
                    },
                    HunkKind::NextRight,
                    HunkKind::PrevLeft,
                ),
                Hunk::CopyEmail(
                    "inbox".into(),
                    Envelope {
                        id: "id".into(),
                        internal_id: "id".into(),
                        flags: "seen".into(),
                        ..Envelope::default()
                    },
                    HunkKind::NextRight,
                    HunkKind::NextLeft,
                ),
                Hunk::CopyEmail(
                    "inbox".into(),
                    Envelope {
                        id: "id".into(),
                        internal_id: "id".into(),
                        flags: "seen".into(),
                        ..Envelope::default()
                    },
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

        let patch = super::build_patch("inbox", prev_left, next_left, prev_right, next_right);

        assert_eq!(
            patch,
            vec![
                Hunk::RemoveEmail("inbox".into(), "id".into(), HunkKind::PrevLeft),
                Hunk::RemoveEmail("inbox".into(), "id".into(), HunkKind::PrevRight),
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

        let patch = super::build_patch("inbox", prev_left, next_left, prev_right, next_right);

        assert_eq!(
            patch,
            vec![
                Hunk::RemoveEmail("inbox".into(), "id".into(), HunkKind::PrevLeft),
                Hunk::RemoveEmail("inbox".into(), "id".into(), HunkKind::PrevRight),
                Hunk::RemoveEmail("inbox".into(), "id".into(), HunkKind::NextRight),
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

        let patch = super::build_patch("inbox", prev_left, next_left, prev_right, next_right);

        assert_eq!(
            patch,
            vec![
                Hunk::CopyEmail(
                    "inbox".into(),
                    Envelope {
                        id: "id".into(),
                        internal_id: "id".into(),
                        ..Envelope::default()
                    },
                    HunkKind::NextLeft,
                    HunkKind::PrevRight,
                ),
                Hunk::CopyEmail(
                    "inbox".into(),
                    Envelope {
                        id: "id".into(),
                        internal_id: "id".into(),
                        ..Envelope::default()
                    },
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

        let patch = super::build_patch("inbox", prev_left, next_left, prev_right, next_right);

        assert_eq!(
            patch,
            vec![Hunk::CopyEmail(
                "inbox".into(),
                Envelope {
                    id: "id".into(),
                    internal_id: "id".into(),
                    ..Envelope::default()
                },
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

        let patch = super::build_patch("inbox", prev_left, next_left, prev_right, next_right);

        assert_eq!(
            patch,
            vec![
                Hunk::RemoveEmail("inbox".into(), "id".into(), HunkKind::PrevLeft),
                Hunk::RemoveEmail("inbox".into(), "id".into(), HunkKind::NextLeft),
                Hunk::RemoveEmail("inbox".into(), "id".into(), HunkKind::PrevRight),
            ]
        );
    }
}
