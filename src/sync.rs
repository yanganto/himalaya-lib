use chrono::{DateTime, Local};
use dirs::data_dir;
use log::{debug, error, trace, warn};
use rayon::prelude::*;
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    fs::create_dir_all,
    io,
    path::PathBuf,
    result,
};
use thiserror::Error;

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
    LocalCache,
    Local,
    RemoteCache,
    Remote,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum HunkKindRestricted {
    Local,
    Remote,
}

pub type InternalId = String;
pub type Folder = String;
pub type Source = HunkKind;
pub type SourceRestricted = HunkKindRestricted;
pub type Target = HunkKind;
pub type TargetRestricted = HunkKindRestricted;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Hunk {
    CacheEnvelope(Folder, InternalId, SourceRestricted),
    CopyEmail(Folder, Envelope, SourceRestricted, TargetRestricted),
    RemoveEmail(Folder, InternalId, Target),
    SetFlags(Folder, InternalId, Flags, Target),
}

type Patch = Vec<Vec<Hunk>>;

pub const CREATE_ENVELOPES_TABLE: &str = "
CREATE TABLE IF NOT EXISTS envelopes (
    id          TEXT NOT NULL,
    internal_id TEXT NOT NULL,
    hash        TEXT NOT NULL,
    account     TEXT NOT NULL,
    folder      TEXT NOT NULL,
    flag        TEXT NOT NULL,
    message_id  TEXT NOT NULL,
    sender      TEXT NOT NULL,
    subject     TEXT NOT NULL,
    date        DATETIME
)";

const INSERT_ENVELOPE: &str = "INSERT INTO envelopes VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";
const DELETE_ENVELOPE: &str = "DELETE FROM envelopes WHERE account = ? AND folder = ? AND id = ?";
pub const SELECT_ENVELOPES: &str = "
    SELECT id, internal_id, hash, account, folder, GROUP_CONCAT(flag) AS flags, message_id, sender, subject, date
    FROM envelopes
    WHERE account = ?
    AND folder = ?
    GROUP BY hash
";

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
    cache.execute(CREATE_ENVELOPES_TABLE)?;

    let local = MaildirBackend::new(
        Cow::Borrowed(account),
        Cow::Owned(MaildirConfig {
            root_dir: sync_dir.join(&account.name),
        }),
    )?;

    let remote_envelopes: Envelopes = HashMap::from_iter(
        remote
            .list_envelopes("inbox", 0, 0)?
            .iter()
            .map(|envelope| (envelope.hash("inbox"), envelope.clone())),
    );

    let remote_envelopes_cached: Envelopes = HashMap::from_iter(
        cache
            .prepare(SELECT_ENVELOPES)?
            .into_iter()
            .bind((1, account.name.as_str()))?
            .bind((2, "inbox"))?
            .collect::<sqlite::Result<Vec<_>>>()?
            .iter()
            .map(|row| {
                let envelope = Envelope {
                    id: row.read::<&str, _>("id").into(),
                    internal_id: row.read::<&str, _>("internal_id").into(),
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

                (envelope.hash("inbox"), envelope)
            }),
    );

    let local_envelopes: Envelopes = HashMap::from_iter(
        local
            .list_envelopes("inbox", 0, 0)?
            .iter()
            .map(|envelope| (envelope.hash("inbox"), envelope.clone())),
    );

    let local_envelopes_cached: Envelopes = HashMap::from_iter(
        cache
            .prepare(SELECT_ENVELOPES)?
            .into_iter()
            .bind((1, format!("{}:cache", account.name).as_str()))?
            .bind((2, "inbox"))?
            .collect::<sqlite::Result<Vec<_>>>()?
            .iter()
            .map(|row| {
                let envelope = Envelope {
                    id: row.read::<&str, _>("id").into(),
                    internal_id: row.read::<&str, _>("internal_id").into(),
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

    println!("prev_left_envelopes: {:#?}", local_envelopes_cached);

    let patch = build_patch(
        "inbox",
        local_envelopes_cached,
        local_envelopes,
        remote_envelopes_cached,
        remote_envelopes,
    );

    println!("============= patch: {:#?}", patch);
    debug!("patch length: {}", patch.len());

    let process_hunk = |hunk: &Hunk| {
        match hunk {
            Hunk::CacheEnvelope(folder, internal_id, source) => match source {
                HunkKindRestricted::Local => {
                    let envelope = local.get_envelope_internal(&folder, &internal_id)?;
                    let mut statement = cache.prepare(INSERT_ENVELOPE)?;
                    for flag in envelope.flags.iter() {
                        statement.reset()?;
                        statement.bind((1, envelope.id.as_str()))?;
                        statement.bind((2, envelope.internal_id.as_str()))?;
                        statement.bind((3, envelope.hash(&folder).as_str()))?;
                        statement.bind((4, format!("{}:cache", account.name).as_str()))?;
                        statement.bind((5, folder.as_str()))?;
                        statement.bind((6, flag.to_string().as_str()))?;
                        statement.bind((7, envelope.message_id.as_str()))?;
                        statement.bind((8, envelope.sender.as_str()))?;
                        statement.bind((9, envelope.subject.as_str()))?;
                        statement.bind((
                            10,
                            match envelope.date {
                                Some(date) => date.to_rfc3339().into(),
                                None => sqlite::Value::Null,
                            },
                        ))?;
                        statement.next()?;
                    }
                }
                HunkKindRestricted::Remote => {
                    let envelope = remote.get_envelope_internal(&folder, &internal_id)?;
                    let mut statement = cache.prepare(INSERT_ENVELOPE)?;
                    for flag in envelope.flags.iter() {
                        statement.reset()?;
                        statement.bind((1, envelope.id.as_str()))?;
                        statement.bind((2, envelope.internal_id.as_str()))?;
                        statement.bind((3, envelope.hash(&folder).as_str()))?;
                        statement.bind((4, account.name.as_str()))?;
                        statement.bind((5, folder.as_str()))?;
                        statement.bind((6, flag.to_string().as_str()))?;
                        statement.bind((7, envelope.message_id.as_str()))?;
                        statement.bind((8, envelope.sender.as_str()))?;
                        statement.bind((9, envelope.subject.as_str()))?;
                        statement.bind((
                            10,
                            match envelope.date {
                                Some(date) => date.to_rfc3339().into(),
                                None => sqlite::Value::Null,
                            },
                        ))?;
                        statement.next()?;
                    }
                }
            },
            Hunk::CopyEmail(folder, envelope, source, target) => {
                let internal_ids = vec![envelope.internal_id.as_str()];
                let emails = match source {
                    HunkKindRestricted::Local => local.get_emails_internal(&folder, internal_ids),
                    HunkKindRestricted::Remote => remote.get_emails_internal(&folder, internal_ids),
                }?;
                let emails = emails.to_vec();
                let email = emails
                    .first()
                    .ok_or_else(|| Error::FindEmailError(envelope.internal_id.clone()))?;

                match target {
                    HunkKindRestricted::Local => {
                        let internal_id =
                            local.add_email_internal(&folder, email.raw()?, &envelope.flags)?;
                        let envelope = local.get_envelope_internal(&folder, &internal_id)?;

                        let mut statement = cache.prepare(INSERT_ENVELOPE)?;
                        for flag in envelope.flags.iter() {
                            statement.reset()?;
                            statement.bind((1, envelope.id.as_str()))?;
                            statement.bind((2, envelope.internal_id.as_str()))?;
                            statement.bind((3, envelope.hash(&folder).as_str()))?;
                            statement.bind((4, format!("{}:cache", account.name).as_str()))?;
                            statement.bind((5, folder.as_str()))?;
                            statement.bind((6, flag.to_string().as_str()))?;
                            statement.bind((7, envelope.message_id.as_str()))?;
                            statement.bind((8, envelope.sender.as_str()))?;
                            statement.bind((9, envelope.subject.as_str()))?;
                            statement.bind((
                                10,
                                match envelope.date {
                                    Some(date) => date.to_rfc3339().into(),
                                    None => sqlite::Value::Null,
                                },
                            ))?;
                            statement.next()?;
                        }
                    }
                    HunkKindRestricted::Remote => {
                        let internal_id =
                            remote.add_email_internal(&folder, email.raw()?, &envelope.flags)?;
                        let envelope = local.get_envelope_internal(&folder, &internal_id)?;

                        let mut statement = cache.prepare(INSERT_ENVELOPE)?;
                        for flag in envelope.flags.iter() {
                            statement.reset()?;
                            statement.bind((1, envelope.id.as_str()))?;
                            statement.bind((2, envelope.internal_id.as_str()))?;
                            statement.bind((3, envelope.hash(&folder).as_str()))?;
                            statement.bind((4, account.name.as_str()))?;
                            statement.bind((5, folder.as_str()))?;
                            statement.bind((6, flag.to_string().as_str()))?;
                            statement.bind((7, envelope.message_id.as_str()))?;
                            statement.bind((8, envelope.sender.as_str()))?;
                            statement.bind((9, envelope.subject.as_str()))?;
                            statement.bind((
                                10,
                                match envelope.date {
                                    Some(date) => date.to_rfc3339().into(),
                                    None => sqlite::Value::Null,
                                },
                            ))?;
                            statement.next()?;
                        }
                    }
                };
            }
            Hunk::RemoveEmail(folder, internal_id, target) => {
                let internal_ids = vec![internal_id.as_str()];

                match target {
                    HunkKind::LocalCache => {
                        let mut statement = cache.prepare(DELETE_ENVELOPE)?;
                        statement.bind((1, format!("{}:cache", account.name).as_str()))?;
                        statement.bind((2, folder.as_str()))?;
                        statement.bind((3, internal_id.as_str()))?;
                        statement.next()?;
                    }
                    HunkKind::Local => {
                        local.delete_emails_internal("inbox", internal_ids.clone())?;
                    }
                    HunkKind::RemoteCache => {
                        let mut statement = cache.prepare(DELETE_ENVELOPE)?;
                        statement.bind((1, account.name.as_str()))?;
                        statement.bind((2, folder.as_str()))?;
                        statement.bind((3, internal_id.as_str()))?;
                        statement.next()?;
                    }
                    HunkKind::Remote => {
                        remote.delete_emails_internal("inbox", internal_ids.clone())?;
                    }
                };
            }
            Hunk::SetFlags(folder, internal_id, flags, target) => {
                match target {
                    HunkKind::LocalCache => {
                        let mut statement = cache.prepare(DELETE_ENVELOPE)?;
                        statement.bind((1, format!("{}:cache", account.name).as_str()))?;
                        statement.bind((2, folder.as_str()))?;
                        statement.bind((3, internal_id.as_str()))?;
                        statement.next()?;

                        let envelope = local.get_envelope_internal(&folder, &internal_id)?;
                        let mut statement = cache.prepare(INSERT_ENVELOPE)?;
                        for flag in flags.iter() {
                            statement.bind((1, envelope.id.as_str()))?;
                            statement.bind((2, envelope.internal_id.as_str()))?;
                            statement.bind((3, envelope.hash(&folder).as_str()))?;
                            statement.bind((4, format!("{}:cache", account.name).as_str()))?;
                            statement.bind((5, folder.as_str()))?;
                            statement.bind((6, flag.to_string().as_str()))?;
                            statement.bind((7, envelope.message_id.as_str()))?;
                            statement.bind((8, envelope.sender.as_str()))?;
                            statement.bind((9, envelope.subject.as_str()))?;
                            statement.bind((
                                10,
                                match envelope.date {
                                    Some(date) => date.to_rfc3339().into(),
                                    None => sqlite::Value::Null,
                                },
                            ))?;
                            statement.next()?;
                        }
                    }
                    HunkKind::Local => {
                        local.set_flags_internal("inbox", vec![&internal_id], &flags)?;
                    }
                    HunkKind::RemoteCache => {
                        let mut statement = cache.prepare(DELETE_ENVELOPE)?;
                        statement.bind((1, account.name.as_str()))?;
                        statement.bind((2, folder.as_str()))?;
                        statement.bind((3, internal_id.as_str()))?;
                        statement.next()?;

                        let envelope = remote.get_envelope_internal(&folder, &internal_id)?;
                        let mut statement = cache.prepare(INSERT_ENVELOPE)?;
                        for flag in flags.iter() {
                            statement.bind((1, envelope.id.as_str()))?;
                            statement.bind((2, envelope.internal_id.as_str()))?;
                            statement.bind((3, envelope.hash(&folder).as_str()))?;
                            statement.bind((4, account.name.as_str()))?;
                            statement.bind((5, folder.as_str()))?;
                            statement.bind((6, flag.to_string().as_str()))?;
                            statement.bind((7, envelope.message_id.as_str()))?;
                            statement.bind((8, envelope.sender.as_str()))?;
                            statement.bind((9, envelope.subject.as_str()))?;
                            statement.bind((
                                10,
                                match envelope.date {
                                    Some(date) => date.to_rfc3339().into(),
                                    None => sqlite::Value::Null,
                                },
                            ))?;
                            statement.next()?;
                        }
                    }
                    HunkKind::Remote => {
                        remote.set_flags_internal("inbox", vec![internal_id], &flags)?;
                    }
                };
            }
        };

        Result::Ok(())
    };

    for (batch_num, batch) in patch.chunks(3).enumerate() {
        debug!("processing batch {}/{}", batch_num + 1, patch.len());

        batch
            .par_iter()
            .enumerate()
            .try_for_each(|(hunks_num, hunks)| {
                debug!("processing hunks group {}/{}", hunks_num + 1, batch.len());

                for (hunk_num, hunk) in hunks.iter().enumerate() {
                    debug!("processing hunk {}/{}", hunk_num + 1, hunks.len());
                    trace!("hunk: {:#?}", hunk);

                    if let Err(err) = process_hunk(hunk) {
                        warn!(
                            "error while processing hunk {:?}, skipping it: {}",
                            hunk, err
                        );
                    }
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
    let mut patch: Patch = vec![];
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
            // The hash exists nowhere, which cannot happen since
            // hashes has been built from all envelopes hash.
            (None, None, None, None) => (),

            // 0001
            //
            // The hash only exists in the remote side, which means a
            // new email has been added remote side and needs to be
            // cached remote side + copied local side.
            (None, None, None, Some(remote)) => patch.extend([
                vec![Hunk::CacheEnvelope(
                    folder.to_string(),
                    remote.internal_id.clone(),
                    HunkKindRestricted::Remote,
                )],
                vec![Hunk::CopyEmail(
                    folder.to_string(),
                    remote.clone(),
                    HunkKindRestricted::Remote,
                    HunkKindRestricted::Local,
                )],
            ]),

            // 0010
            //
            // The hash only exists in the remote cache, which means
            // an email is outdated and needs to be removed from the
            // remote cache.
            (None, None, Some(remote_cache), None) => patch.push(vec![Hunk::RemoveEmail(
                folder.to_string(),
                remote_cache.internal_id.clone(),
                HunkKind::RemoteCache,
            )]),

            // 0011
            //
            // The hash exists in the remote side but not in the local
            // side, which means there is a conflict. Since we cannot
            // determine which side (local removed or remote added) is
            // the most up-to-date, it is safer to consider the remote
            // added side up-to-date in order not to lose data.
            //
            // TODO: make this behaviour customizable.
            (None, None, Some(remote_cache), Some(remote)) => {
                patch.push(vec![Hunk::CopyEmail(
                    folder.to_string(),
                    remote.clone(),
                    HunkKindRestricted::Remote,
                    HunkKindRestricted::Local,
                )]);

                if remote_cache.flags != remote.flags {
                    patch.push(vec![Hunk::SetFlags(
                        folder.to_string(),
                        remote_cache.internal_id.clone(),
                        remote.flags.clone(),
                        HunkKind::RemoteCache,
                    )])
                }
            }

            // 0100
            //
            // The hash only exists in the local side, which means a
            // new email has been added local side and needs to be
            // added cached local side + added remote sides.
            (None, Some(local), None, None) => patch.extend([
                vec![Hunk::CacheEnvelope(
                    folder.to_string(),
                    local.internal_id.clone(),
                    HunkKindRestricted::Local,
                )],
                vec![Hunk::CopyEmail(
                    folder.to_string(),
                    local.clone(),
                    HunkKindRestricted::Local,
                    HunkKindRestricted::Remote,
                )],
            ]),

            // 0101
            //
            // The hash exists in both local and remote sides, which
            // means a new (same) email has been added both sides and
            // the most recent needs to be kept.
            //
            // NOTE: this case should never happen: new emails
            // internal identifier are unique and should (in theory)
            // never conflict, but we implement this case for the sake
            // of exhaustiveness.
            (None, Some(local), None, Some(remote)) => {
                match (local.date.as_ref(), remote.date.as_ref()) {
                    // The date exists only on the local side, so we
                    // keep the local side and remove the remote side.
                    (Some(_), None) => patch.push(vec![
                        Hunk::RemoveEmail(
                            folder.to_string(),
                            remote.internal_id.clone(),
                            HunkKind::Remote,
                        ),
                        Hunk::CopyEmail(
                            folder.to_string(),
                            local.clone(),
                            HunkKindRestricted::Local,
                            HunkKindRestricted::Remote,
                        ),
                        Hunk::CacheEnvelope(
                            folder.to_string(),
                            local.internal_id.clone(),
                            HunkKindRestricted::Local,
                        ),
                    ]),

                    // The date exists in both sides and the local
                    // date is greater than the remote date, so we
                    // keep the local side.
                    (Some(date_left), Some(date_right)) if date_left > date_right => {
                        patch.push(vec![
                            Hunk::RemoveEmail(
                                folder.to_string(),
                                remote.internal_id.clone(),
                                HunkKind::Remote,
                            ),
                            Hunk::CopyEmail(
                                folder.to_string(),
                                local.clone(),
                                HunkKindRestricted::Local,
                                HunkKindRestricted::Remote,
                            ),
                            Hunk::CacheEnvelope(
                                folder.to_string(),
                                local.internal_id.clone(),
                                HunkKindRestricted::Local,
                            ),
                        ])
                    }

                    // For all other cases we keep the remote side.
                    _ => patch.push(vec![
                        Hunk::RemoveEmail(
                            folder.to_string(),
                            local.internal_id.clone(),
                            HunkKind::Local,
                        ),
                        Hunk::CopyEmail(
                            folder.to_string(),
                            remote.clone(),
                            HunkKindRestricted::Remote,
                            HunkKindRestricted::Local,
                        ),
                        Hunk::CacheEnvelope(
                            folder.to_string(),
                            remote.internal_id.clone(),
                            HunkKindRestricted::Remote,
                        ),
                    ]),
                }
            }

            // 0110
            //
            // The hash exists in the local side and in the remote
            // cache side, which means a new (same) email has been
            // added local side but removed remote side. Since we
            // cannot determine which side (local added or remote
            // removed) is the most up-to-date, it is safer to
            // consider the remote added side up-to-date in order not
            // to lose data.
            //
            // TODO: make this behaviour customizable.
            (None, Some(local), Some(remote_cache), None) => patch.push(vec![
                Hunk::RemoveEmail(
                    folder.to_string(),
                    remote_cache.internal_id.clone(),
                    HunkKind::RemoteCache,
                ),
                Hunk::CacheEnvelope(
                    folder.to_string(),
                    local.internal_id.clone(),
                    HunkKindRestricted::Local,
                ),
                Hunk::CopyEmail(
                    folder.to_string(),
                    local.clone(),
                    HunkKindRestricted::Local,
                    HunkKindRestricted::Remote,
                ),
            ]),

            // 0111
            //
            // The hash exists everywhere except in the local cache,
            // which means the local cache misses an email and needs
            // to be updated. Flags also need to be synchronized.
            (None, Some(local), Some(remote_cache), Some(remote)) => {
                patch.push(vec![Hunk::CacheEnvelope(
                    folder.to_string(),
                    local.internal_id.clone(),
                    HunkKindRestricted::Local,
                )]);

                let flags = sync_flags(None, Some(local), Some(remote_cache), Some(remote));

                if local.flags != flags {
                    patch.push(vec![Hunk::SetFlags(
                        folder.to_string(),
                        local.internal_id.clone(),
                        flags.clone(),
                        HunkKind::Local,
                    )]);
                }

                if remote_cache.flags != flags {
                    patch.push(vec![Hunk::SetFlags(
                        folder.to_string(),
                        remote_cache.internal_id.clone(),
                        flags.clone(),
                        HunkKind::RemoteCache,
                    )]);
                }

                if remote.flags != flags {
                    patch.push(vec![Hunk::SetFlags(
                        folder.to_string(),
                        remote.internal_id.clone(),
                        flags.clone(),
                        HunkKind::Remote,
                    )]);
                }
            }

            // 1000
            //
            // The hash only exists in the local cache, which means
            // the local cache has an outdated email and need to be
            // cleaned.
            (Some(local_cache), None, None, None) => patch.push(vec![Hunk::RemoveEmail(
                folder.to_string(),
                local_cache.internal_id.clone(),
                HunkKind::LocalCache,
            )]),

            // 1001
            //
            // The hash exists in the local cache and in the remote,
            // which means a new (same) email has been removed local
            // side but added remote side. Since we cannot determine
            // which side (local removed or remote added) is the most
            // up-to-date, it is safer to consider the remote added
            // side up-to-date in order not to lose data.
            //
            // TODO: make this behaviour customizable.
            (Some(local_cache), None, None, Some(remote)) => patch.push(vec![
                Hunk::RemoveEmail(
                    folder.to_string(),
                    local_cache.internal_id.clone(),
                    HunkKind::LocalCache,
                ),
                Hunk::CacheEnvelope(
                    folder.to_string(),
                    remote.internal_id.clone(),
                    HunkKindRestricted::Remote,
                ),
                Hunk::CopyEmail(
                    folder.to_string(),
                    remote.clone(),
                    HunkKindRestricted::Remote,
                    HunkKindRestricted::Local,
                ),
            ]),

            // 1010
            //
            // The hash only exists in both caches, which means caches
            // have an outdated email and need to be cleaned up.
            (Some(local_cache), None, Some(remote_cache), None) => patch.extend([
                vec![Hunk::RemoveEmail(
                    folder.to_string(),
                    local_cache.internal_id.clone(),
                    HunkKind::LocalCache,
                )],
                vec![Hunk::RemoveEmail(
                    folder.to_string(),
                    remote_cache.internal_id.clone(),
                    HunkKind::RemoteCache,
                )],
            ]),

            // 1011
            //
            // The hash exists everywhere except in local side, which
            // means an email has been removed local side and needs to
            // be removed everywhere else.
            (Some(local_cache), None, Some(remote_cache), Some(remote)) => patch.extend([
                vec![Hunk::RemoveEmail(
                    folder.to_string(),
                    local_cache.internal_id.clone(),
                    HunkKind::LocalCache,
                )],
                vec![Hunk::RemoveEmail(
                    folder.to_string(),
                    remote_cache.internal_id.clone(),
                    HunkKind::RemoteCache,
                )],
                vec![Hunk::RemoveEmail(
                    folder.to_string(),
                    remote.internal_id.clone(),
                    HunkKind::Remote,
                )],
            ]),

            // 1100
            //
            // The hash exists in local side but not in remote side,
            // which means there is a conflict. Since we cannot
            // determine which side (local updated or remote removed)
            // is the most up-to-date, it is safer to consider the
            // local updated side up-to-date in order not to lose
            // data.
            //
            // TODO: make this behaviour customizable.
            (Some(local_cache), Some(local), None, None) => {
                patch.push(vec![Hunk::CopyEmail(
                    folder.to_string(),
                    local.clone(),
                    HunkKindRestricted::Local,
                    HunkKindRestricted::Remote,
                )]);

                if local_cache.flags != local.flags {
                    patch.push(vec![Hunk::SetFlags(
                        folder.to_string(),
                        local_cache.internal_id.clone(),
                        local.flags.clone(),
                        HunkKind::LocalCache,
                    )]);
                }
            }

            // 1101
            //
            // The hash exists everywhere except in remote cache side,
            // which means an email is missing remote cache side and
            // needs to be updated. Flags also need to be
            // synchronized.
            (Some(local_cache), Some(local), None, Some(remote)) => {
                let flags = sync_flags(Some(local_cache), Some(local), None, Some(remote));

                if local_cache.flags != flags {
                    patch.push(vec![Hunk::SetFlags(
                        folder.to_string(),
                        local_cache.internal_id.clone(),
                        flags.clone(),
                        HunkKind::LocalCache,
                    )]);
                }

                if local.flags != flags {
                    patch.push(vec![Hunk::SetFlags(
                        folder.to_string(),
                        local.internal_id.clone(),
                        flags.clone(),
                        HunkKind::Local,
                    )]);
                }

                if remote.flags != flags {
                    patch.push(vec![Hunk::SetFlags(
                        folder.to_string(),
                        remote.internal_id.clone(),
                        flags.clone(),
                        HunkKind::Remote,
                    )]);
                }

                patch.push(vec![Hunk::CacheEnvelope(
                    folder.to_string(),
                    remote.internal_id.clone(),
                    HunkKindRestricted::Remote,
                )]);
            }

            // 1110
            //
            // The hash exists everywhere except in remote side, which
            // means an email has been removed remote side and needs
            // to be removed everywhere else.
            (Some(local_cache), Some(local), Some(remote_cache), None) => patch.extend([
                vec![Hunk::RemoveEmail(
                    folder.to_string(),
                    local_cache.internal_id.clone(),
                    HunkKind::LocalCache,
                )],
                vec![Hunk::RemoveEmail(
                    folder.to_string(),
                    local.internal_id.clone(),
                    HunkKind::Local,
                )],
                vec![Hunk::RemoveEmail(
                    folder.to_string(),
                    remote_cache.internal_id.clone(),
                    HunkKind::RemoteCache,
                )],
            ]),

            // 1111
            //
            // The hash exists everywhere, which means all flags need
            // to be synchronized.
            (Some(local_cache), Some(local), Some(remote_cache), Some(remote)) => {
                let flags = sync_flags(
                    Some(local_cache),
                    Some(local),
                    Some(remote_cache),
                    Some(remote),
                );

                if local_cache.flags != flags {
                    patch.push(vec![Hunk::SetFlags(
                        folder.to_string(),
                        local_cache.internal_id.clone(),
                        flags.clone(),
                        HunkKind::LocalCache,
                    )]);
                }

                if local.flags != flags {
                    patch.push(vec![Hunk::SetFlags(
                        folder.to_string(),
                        local.internal_id.clone(),
                        flags.clone(),
                        HunkKind::Local,
                    )]);
                }

                if remote_cache.flags != flags {
                    patch.push(vec![Hunk::SetFlags(
                        folder.to_string(),
                        remote_cache.internal_id.clone(),
                        flags.clone(),
                        HunkKind::RemoteCache,
                    )]);
                }

                if remote.flags != flags {
                    patch.push(vec![Hunk::SetFlags(
                        folder.to_string(),
                        remote.internal_id.clone(),
                        flags.clone(),
                        HunkKind::Remote,
                    )]);
                }
            }
        }
    }

    patch
}

pub fn sync_flags(
    local_cache: Option<&Envelope>,
    local: Option<&Envelope>,
    remote_cache: Option<&Envelope>,
    remote: Option<&Envelope>,
) -> Flags {
    let mut synchronized_flags: HashSet<Flag> = HashSet::default();

    let mut all_flags: HashSet<Flag> = HashSet::default();
    all_flags.extend(local_cache.map(|e| e.flags.clone().0).unwrap_or_default());
    all_flags.extend(local.map(|e| e.flags.clone().0).unwrap_or_default());
    all_flags.extend(remote_cache.map(|e| e.flags.clone().0).unwrap_or_default());
    all_flags.extend(remote.map(|e| e.flags.clone().0).unwrap_or_default());

    for flag in all_flags {
        match (
            local_cache.and_then(|e| e.flags.get(&flag)),
            local.and_then(|e| e.flags.get(&flag)),
            remote_cache.and_then(|e| e.flags.get(&flag)),
            remote.and_then(|e| e.flags.get(&flag)),
        ) {
            // The flag exists nowhere, which cannot happen since the
            // flags hashset is built from envelopes flags.
            (None, None, None, None) => (),

            // The flag only exists in remote side, which means a new
            // flag has been added.
            (None, None, None, Some(_)) => {
                synchronized_flags.insert(flag.clone());
            }

            // The flag only exists in remote cache, which means an
            // outdated flag needs to be removed.
            (None, None, Some(_), None) => {
                synchronized_flags.remove(&flag);
            }

            // The flag exists in remote side but not in local side,
            // which means there is a conflict. Since we cannot
            // determine which side (local removed or remote added) is
            // the most up-to-date, it is safer to consider the remote
            // added side up-to-date (or local removed in case of
            // [`Flag::Deleted`]) in order not to lose data.
            //
            // TODO: make this behaviour customizable.
            (None, None, Some(_), Some(_)) if flag == Flag::Deleted => {
                synchronized_flags.remove(&flag);
            }
            (None, None, Some(_), Some(_)) => {
                synchronized_flags.insert(flag.clone());
            }

            // The flag only exists in local side, which means a new
            // flag has been added.
            (None, Some(_), None, None) => {
                synchronized_flags.insert(flag.clone());
            }

            // The flag exists in local and remote sides, which means
            // a new (same) flag has been added both sides at the same
            // time.
            (None, Some(_), None, Some(_)) => {
                synchronized_flags.insert(flag.clone());
            }

            // The flag exists in local side and remote cache side,
            // which means a new (same) flag has been added local side
            // but removed remote side. Since we cannot determine
            // which side (local added or remote removed) is the most
            // up-to-date, it is safer to consider the local added
            // side up-to-date (or remote removed in case of
            // [`Flag::Deleted`]) in order not to lose data.
            //
            // TODO: make this behaviour customizable.
            (None, Some(_), Some(_), None) if flag == Flag::Deleted => {
                synchronized_flags.remove(&flag);
            }
            (None, Some(_), Some(_), None) => {
                synchronized_flags.insert(flag.clone());
            }

            // The flag exists everywhere except in local cache, which
            // means the local cache misses a flag.
            (None, Some(_), Some(_), Some(_)) => {
                synchronized_flags.insert(flag.clone());
            }

            // The flag only exists in local cache side, which means
            // the local cache has an outdated flag.
            (Some(_), None, None, None) => {
                synchronized_flags.remove(&flag);
            }

            // The flag exists in local cache side and remote side,
            // which means a new (same) flag has been removed local
            // cache side but added remote side. Since we cannot
            // determine which side (local removed or remote added) is
            // the most up-to-date, it is safer to consider the remote
            // added side up-to-date (or local removed in case of
            // [`Flag::Deleted`]) in order not to lose data.
            //
            // TODO: make this behaviour customizable.
            (Some(_), None, None, Some(_)) if flag == Flag::Deleted => {
                synchronized_flags.remove(&flag);
            }
            (Some(_), None, None, Some(_)) => {
                synchronized_flags.insert(flag.clone());
            }

            // The flag exists in both caches, which means a old flag
            // needs to be removed everywhere.
            (Some(_), None, Some(_), None) => {
                synchronized_flags.remove(&flag);
            }

            // The flag exists everywhere except in local side, which
            // means a flag has been removed local side and needs to
            // be removed everywhere else.
            (Some(_), None, Some(_), Some(_)) => {
                synchronized_flags.remove(&flag);
            }

            // The flag exists in the local sides but not in remote
            // sides, which means there is a conflict. Since we cannot
            // determine which side is the most up-to-date, it is
            // safer to consider the local side side up-to-date (or
            // remote side in case of [`Flag::Deleted`]) in order not
            // to lose data.
            //
            // TODO: make this behaviour customizable.
            (Some(_), Some(_), None, None) if flag == Flag::Deleted => {
                synchronized_flags.remove(&flag);
            }
            (Some(_), Some(_), None, None) => {
                synchronized_flags.insert(flag.clone());
            }

            // The flag exists everywhere except in remote cache side,
            // which means the remote cache misses a flag.
            (Some(_), Some(_), None, Some(_)) => {
                synchronized_flags.insert(flag.clone());
            }

            // The flag exists everywhere except in remote side, which
            // means a flag has been removed remote side and needs to
            // be removed everywhere else.
            (Some(_), Some(_), Some(_), None) => {
                synchronized_flags.remove(&flag);
            }

            // The flag exists everywhere, which means the flag needs
            // to be added.
            (Some(_), Some(_), Some(_), Some(_)) => {
                synchronized_flags.insert(flag.clone());
            }
        }
    }

    Flags::from_iter(synchronized_flags)
}

#[cfg(test)]
mod sync {
    use crate::{Envelope, Flag};

    use super::{Envelopes, Flags, Hunk, HunkKind, HunkKindRestricted, Patch};

    #[test]
    fn sync_flags() {
        assert_eq!(super::sync_flags(None, None, None, None), Flags::default());

        assert_eq!(
            super::sync_flags(
                None,
                None,
                None,
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
            ),
            Flags::from_iter([Flag::Seen]),
        );

        assert_eq!(
            super::sync_flags(
                Some(&Envelope::default()),
                Some(&Envelope::default()),
                Some(&Envelope::default()),
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
            ),
            Flags::from_iter([Flag::Seen]),
        );

        assert_eq!(
            super::sync_flags(
                Some(&Envelope::default()),
                Some(&Envelope::default()),
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
                Some(&Envelope::default()),
            ),
            Flags::default()
        );

        assert_eq!(
            super::sync_flags(
                Some(&Envelope::default()),
                Some(&Envelope::default()),
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
            ),
            Flags::from_iter([Flag::Seen]),
        );

        assert_eq!(
            super::sync_flags(
                Some(&Envelope::default()),
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
                Some(&Envelope::default()),
                Some(&Envelope::default()),
            ),
            Flags::from_iter([Flag::Seen]),
        );

        assert_eq!(
            super::sync_flags(
                Some(&Envelope::default()),
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
                Some(&Envelope::default()),
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
            ),
            Flags::from_iter([Flag::Seen]),
        );

        assert_eq!(
            super::sync_flags(
                Some(&Envelope::default()),
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
                Some(&Envelope::default()),
            ),
            Flags::from_iter([Flag::Seen]),
        );

        assert_eq!(
            super::sync_flags(
                Some(&Envelope::default()),
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
            ),
            Flags::from_iter([Flag::Seen]),
        );

        assert_eq!(
            super::sync_flags(
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
                Some(&Envelope::default()),
                Some(&Envelope::default()),
                Some(&Envelope::default()),
            ),
            Flags::default()
        );

        assert_eq!(
            super::sync_flags(
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
                Some(&Envelope::default()),
                Some(&Envelope::default()),
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
            ),
            Flags::from_iter([Flag::Seen]),
        );

        assert_eq!(
            super::sync_flags(
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
                Some(&Envelope::default()),
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
                Some(&Envelope::default()),
            ),
            Flags::default(),
        );

        assert_eq!(
            super::sync_flags(
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
                Some(&Envelope::default()),
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
            ),
            Flags::default(),
        );

        assert_eq!(
            super::sync_flags(
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
                Some(&Envelope::default()),
                Some(&Envelope::default()),
            ),
            Flags::from_iter([Flag::Seen]),
        );

        assert_eq!(
            super::sync_flags(
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
                Some(&Envelope::default()),
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
            ),
            Flags::from_iter([Flag::Seen]),
        );

        assert_eq!(
            super::sync_flags(
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen]),
                    ..Envelope::default()
                }),
                Some(&Envelope::default()),
            ),
            Flags::default(),
        );

        assert_eq!(
            super::sync_flags(
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen, Flag::Flagged]),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen, Flag::Flagged]),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen, Flag::Flagged]),
                    ..Envelope::default()
                }),
                Some(&Envelope {
                    flags: Flags::from_iter([Flag::Seen, Flag::Flagged]),
                    ..Envelope::default()
                }),
            ),
            Flags::from_iter([Flag::Seen, Flag::Flagged]),
        );
    }

    #[test]
    fn build_patch_0000() {
        let local_cache = Envelopes::default();
        let local = Envelopes::default();
        let remote_cache = Envelopes::default();
        let remote = Envelopes::default();

        assert_eq!(
            super::build_patch("inbox", local_cache, local, remote_cache, remote),
            vec![] as Patch
        );
    }

    #[test]
    fn build_patch_0001() {
        let local_cache = Envelopes::default();
        let local = Envelopes::default();
        let remote_cache = Envelopes::default();
        let remote = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "remote-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);

        assert_eq!(
            super::build_patch("inbox", local_cache, local, remote_cache, remote),
            vec![
                vec![Hunk::CacheEnvelope(
                    "inbox".into(),
                    "remote-id".into(),
                    HunkKindRestricted::Remote,
                )],
                vec![Hunk::CopyEmail(
                    "inbox".into(),
                    Envelope {
                        internal_id: "remote-id".into(),
                        flags: "seen".into(),
                        ..Envelope::default()
                    },
                    HunkKindRestricted::Remote,
                    HunkKindRestricted::Local
                )],
            ],
        );
    }

    #[test]
    fn build_patch_0010() {
        let local_cache = Envelopes::default();
        let local = Envelopes::default();
        let remote_cache = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "remote-cache-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let remote = Envelopes::default();

        assert_eq!(
            super::build_patch("inbox", local_cache, local, remote_cache, remote),
            vec![vec![Hunk::RemoveEmail(
                "inbox".into(),
                "remote-cache-id".into(),
                HunkKind::RemoteCache
            )]],
        );
    }

    #[test]
    fn build_patch_0011_same_flags() {
        let local_cache = Envelopes::default();
        let local = Envelopes::default();
        let remote_cache = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "remote-cache-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let remote = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "remote-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);

        assert_eq!(
            super::build_patch("inbox", local_cache, local, remote_cache, remote),
            vec![vec![Hunk::CopyEmail(
                "inbox".into(),
                Envelope {
                    internal_id: "remote-id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                },
                HunkKindRestricted::Remote,
                HunkKindRestricted::Local,
            )]],
        );
    }

    #[test]
    fn build_patch_0011_different_flags() {
        let local_cache = Envelopes::default();
        let local = Envelopes::default();
        let remote_cache = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "remote-cache-id".into(),
                flags: "seen replied".into(),
                ..Envelope::default()
            },
        )]);
        let remote = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "remote-id".into(),
                flags: "seen flagged deleted".into(),
                ..Envelope::default()
            },
        )]);

        assert_eq!(
            super::build_patch("inbox", local_cache, local, remote_cache, remote),
            vec![
                vec![Hunk::CopyEmail(
                    "inbox".into(),
                    Envelope {
                        internal_id: "remote-id".into(),
                        flags: "seen flagged deleted".into(),
                        ..Envelope::default()
                    },
                    HunkKindRestricted::Remote,
                    HunkKindRestricted::Local,
                )],
                vec![Hunk::SetFlags(
                    "inbox".into(),
                    "remote-cache-id".into(),
                    "seen flagged deleted".into(),
                    HunkKind::RemoteCache,
                )]
            ]
        );
    }

    #[test]
    fn build_patch_0100() {
        let local_cache = Envelopes::default();
        let local = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "local-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let remote_cache = Envelopes::default();
        let remote = Envelopes::default();

        assert_eq!(
            super::build_patch("inbox", local_cache, local, remote_cache, remote),
            vec![
                vec![Hunk::CacheEnvelope(
                    "inbox".into(),
                    "local-id".into(),
                    HunkKindRestricted::Local
                )],
                vec![Hunk::CopyEmail(
                    "inbox".into(),
                    Envelope {
                        internal_id: "local-id".into(),
                        flags: "seen".into(),
                        ..Envelope::default()
                    },
                    HunkKindRestricted::Local,
                    HunkKindRestricted::Remote,
                )],
            ],
        );
    }

    #[test]
    fn build_patch_0101() {
        let local_cache = Envelopes::default();
        let local = Envelopes::from_iter([
            (
                "hash-1".into(),
                Envelope {
                    internal_id: "local-id-1".into(),
                    flags: "seen".into(),
                    date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
                    ..Envelope::default()
                },
            ),
            (
                "hash-2".into(),
                Envelope {
                    internal_id: "local-id-2".into(),
                    flags: "seen".into(),
                    date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
                    ..Envelope::default()
                },
            ),
            (
                "hash-3".into(),
                Envelope {
                    internal_id: "local-id-3".into(),
                    flags: "seen".into(),
                    date: None,
                    ..Envelope::default()
                },
            ),
            (
                "hash-4".into(),
                Envelope {
                    internal_id: "local-id-4".into(),
                    flags: "seen".into(),
                    date: None,
                    ..Envelope::default()
                },
            ),
            (
                "hash-5".into(),
                Envelope {
                    internal_id: "local-id-5".into(),
                    flags: "seen".into(),
                    date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
                    ..Envelope::default()
                },
            ),
        ]);
        let remote_cache = Envelopes::default();
        let remote = Envelopes::from_iter([
            (
                "hash-1".into(),
                Envelope {
                    internal_id: "remote-id-1".into(),
                    flags: "seen".into(),
                    date: None,
                    ..Envelope::default()
                },
            ),
            (
                "hash-2".into(),
                Envelope {
                    internal_id: "remote-id-2".into(),
                    flags: "seen".into(),
                    date: Some("2021-01-01T00:00:00-00:00".parse().unwrap()),
                    ..Envelope::default()
                },
            ),
            (
                "hash-3".into(),
                Envelope {
                    internal_id: "remote-id-3".into(),
                    flags: "seen".into(),
                    date: None,
                    ..Envelope::default()
                },
            ),
            (
                "hash-4".into(),
                Envelope {
                    internal_id: "remote-id-4".into(),
                    flags: "seen".into(),
                    date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
                    ..Envelope::default()
                },
            ),
            (
                "hash-5".into(),
                Envelope {
                    internal_id: "remote-id-5".into(),
                    flags: "seen".into(),
                    date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
                    ..Envelope::default()
                },
            ),
        ]);

        let patch = super::build_patch("inbox", local_cache, local, remote_cache, remote)
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();

        assert_eq!(patch.len(), 15);
        assert!(patch.contains(&Hunk::RemoveEmail(
            "inbox".into(),
            "remote-id-1".into(),
            HunkKind::Remote
        )));
        assert!(patch.contains(&Hunk::CacheEnvelope(
            "inbox".into(),
            "local-id-1".into(),
            HunkKindRestricted::Local,
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "inbox".into(),
            Envelope {
                internal_id: "local-id-1".into(),
                flags: "seen".into(),
                date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
                ..Envelope::default()
            },
            HunkKindRestricted::Local,
            HunkKindRestricted::Remote
        )));
        assert!(patch.contains(&Hunk::RemoveEmail(
            "inbox".into(),
            "remote-id-2".into(),
            HunkKind::Remote
        )));
        assert!(patch.contains(&Hunk::CacheEnvelope(
            "inbox".into(),
            "local-id-2".into(),
            HunkKindRestricted::Local,
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "inbox".into(),
            Envelope {
                internal_id: "local-id-2".into(),
                flags: "seen".into(),
                date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
                ..Envelope::default()
            },
            HunkKindRestricted::Local,
            HunkKindRestricted::Remote
        )));
        assert!(patch.contains(&Hunk::RemoveEmail(
            "inbox".into(),
            "local-id-3".into(),
            HunkKind::Local
        )));
        assert!(patch.contains(&Hunk::CacheEnvelope(
            "inbox".into(),
            "remote-id-3".into(),
            HunkKindRestricted::Remote,
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "inbox".into(),
            Envelope {
                internal_id: "remote-id-3".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
            HunkKindRestricted::Remote,
            HunkKindRestricted::Local
        )));
        assert!(patch.contains(&Hunk::RemoveEmail(
            "inbox".into(),
            "local-id-4".into(),
            HunkKind::Local
        )));
        assert!(patch.contains(&Hunk::CacheEnvelope(
            "inbox".into(),
            "remote-id-4".into(),
            HunkKindRestricted::Remote,
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "inbox".into(),
            Envelope {
                internal_id: "remote-id-4".into(),
                flags: "seen".into(),
                date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
                ..Envelope::default()
            },
            HunkKindRestricted::Remote,
            HunkKindRestricted::Local
        )));
        assert!(patch.contains(&Hunk::RemoveEmail(
            "inbox".into(),
            "local-id-5".into(),
            HunkKind::Local
        )));
        assert!(patch.contains(&Hunk::CacheEnvelope(
            "inbox".into(),
            "remote-id-5".into(),
            HunkKindRestricted::Remote,
        )));
        assert!(patch.contains(&Hunk::CopyEmail(
            "inbox".into(),
            Envelope {
                internal_id: "remote-id-5".into(),
                flags: "seen".into(),
                date: Some("2022-01-01T00:00:00-00:00".parse().unwrap()),
                ..Envelope::default()
            },
            HunkKindRestricted::Remote,
            HunkKindRestricted::Local
        )));
    }

    #[test]
    fn build_patch_0110() {
        let local_cache = Envelopes::default();
        let local = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "local-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let remote_cache = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "remote-id".into(),
                flags: "flagged".into(),
                ..Envelope::default()
            },
        )]);
        let remote = Envelopes::default();

        assert_eq!(
            super::build_patch("inbox", local_cache, local, remote_cache, remote),
            vec![vec![
                Hunk::RemoveEmail("inbox".into(), "remote-id".into(), HunkKind::RemoteCache),
                Hunk::CacheEnvelope("inbox".into(), "local-id".into(), HunkKindRestricted::Local,),
                Hunk::CopyEmail(
                    "inbox".into(),
                    Envelope {
                        internal_id: "local-id".into(),
                        flags: "seen".into(),
                        ..Envelope::default()
                    },
                    HunkKindRestricted::Local,
                    HunkKindRestricted::Remote
                )
            ]],
        );
    }

    #[test]
    fn build_patch_0111() {
        let local_cache = Envelopes::default();
        let local = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "local-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let remote_cache = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "remote-cache-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let remote = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "remote-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);

        assert_eq!(
            super::build_patch("inbox", local_cache, local, remote_cache, remote),
            vec![vec![Hunk::CacheEnvelope(
                "inbox".into(),
                "local-id".into(),
                HunkKindRestricted::Local,
            )]]
        );
    }

    #[test]
    fn build_patch_1000() {
        let local_cache = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "local-cache-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let local = Envelopes::default();
        let remote_cache = Envelopes::default();
        let remote = Envelopes::default();

        assert_eq!(
            super::build_patch("inbox", local_cache, local, remote_cache, remote),
            vec![vec![Hunk::RemoveEmail(
                "inbox".into(),
                "local-cache-id".into(),
                HunkKind::LocalCache
            )]]
        );
    }

    #[test]
    fn build_patch_1001() {
        let local_cache = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "local-cache-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let local = Envelopes::default();
        let remote_cache = Envelopes::default();
        let remote = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "remote-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);

        assert_eq!(
            super::build_patch("inbox", local_cache, local, remote_cache, remote),
            vec![vec![
                Hunk::RemoveEmail(
                    "inbox".into(),
                    "local-cache-id".into(),
                    HunkKind::LocalCache
                ),
                Hunk::CacheEnvelope(
                    "inbox".into(),
                    "remote-id".into(),
                    HunkKindRestricted::Remote,
                ),
                Hunk::CopyEmail(
                    "inbox".into(),
                    Envelope {
                        internal_id: "remote-id".into(),
                        flags: "seen".into(),
                        ..Envelope::default()
                    },
                    HunkKindRestricted::Remote,
                    HunkKindRestricted::Local,
                ),
            ]]
        );
    }

    #[test]
    fn build_patch_1010() {
        let local_cache = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "local-cache-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let local = Envelopes::default();
        let remote_cache = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "remote-cache-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let remote = Envelopes::default();

        assert_eq!(
            super::build_patch("inbox", local_cache, local, remote_cache, remote),
            vec![
                vec![Hunk::RemoveEmail(
                    "inbox".into(),
                    "local-cache-id".into(),
                    HunkKind::LocalCache
                )],
                vec![Hunk::RemoveEmail(
                    "inbox".into(),
                    "remote-cache-id".into(),
                    HunkKind::RemoteCache
                )],
            ]
        );
    }

    #[test]
    fn build_patch_1011() {
        let local_cache = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "local-cache-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let local = Envelopes::default();
        let remote_cache = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "remote-cache-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let remote = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "remote-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);

        assert_eq!(
            super::build_patch("inbox", local_cache, local, remote_cache, remote),
            vec![
                vec![Hunk::RemoveEmail(
                    "inbox".into(),
                    "local-cache-id".into(),
                    HunkKind::LocalCache,
                )],
                vec![Hunk::RemoveEmail(
                    "inbox".into(),
                    "remote-cache-id".into(),
                    HunkKind::RemoteCache,
                )],
                vec![Hunk::RemoveEmail(
                    "inbox".into(),
                    "remote-id".into(),
                    HunkKind::Remote
                )],
            ]
        );
    }

    #[test]
    fn build_patch_1100_same_flags() {
        let local_cache = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "local-cache-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let local = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "local-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let remote_cache = Envelopes::default();
        let remote = Envelopes::default();

        assert_eq!(
            super::build_patch("inbox", local_cache, local, remote_cache, remote),
            vec![vec![Hunk::CopyEmail(
                "inbox".into(),
                Envelope {
                    internal_id: "local-id".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                },
                HunkKindRestricted::Local,
                HunkKindRestricted::Remote,
            )]]
        );
    }

    #[test]
    fn build_patch_1100_different_flags() {
        let local_cache = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "local-cache-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let local = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "local-id".into(),
                flags: "flagged".into(),
                ..Envelope::default()
            },
        )]);
        let remote_cache = Envelopes::default();
        let remote = Envelopes::default();

        assert_eq!(
            super::build_patch("inbox", local_cache, local, remote_cache, remote),
            vec![
                vec![Hunk::CopyEmail(
                    "inbox".into(),
                    Envelope {
                        internal_id: "local-id".into(),
                        flags: "flagged".into(),
                        ..Envelope::default()
                    },
                    HunkKindRestricted::Local,
                    HunkKindRestricted::Remote,
                )],
                vec![Hunk::SetFlags(
                    "inbox".into(),
                    "local-cache-id".into(),
                    "flagged".into(),
                    HunkKind::LocalCache,
                )]
            ]
        );
    }

    #[test]
    fn build_patch_1101() {
        let local_cache = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "local-cache-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let local = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "local-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let remote_cache = Envelopes::default();
        let remote = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "remote-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);

        assert_eq!(
            super::build_patch("inbox", local_cache, local, remote_cache, remote),
            vec![vec![Hunk::CacheEnvelope(
                "inbox".into(),
                "remote-id".into(),
                HunkKindRestricted::Remote,
            )]],
        );
    }

    #[test]
    fn build_patch_1110() {
        let local_cache = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "local-cache-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let local = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "local-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let remote_cache = Envelopes::from_iter([(
            "hash".into(),
            Envelope {
                internal_id: "remote-cache-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let remote = Envelopes::default();

        assert_eq!(
            super::build_patch("inbox", local_cache, local, remote_cache, remote),
            vec![
                vec![Hunk::RemoveEmail(
                    "inbox".into(),
                    "local-cache-id".into(),
                    HunkKind::LocalCache,
                )],
                vec![Hunk::RemoveEmail(
                    "inbox".into(),
                    "local-id".into(),
                    HunkKind::Local
                )],
                vec![Hunk::RemoveEmail(
                    "inbox".into(),
                    "remote-cache-id".into(),
                    HunkKind::RemoteCache,
                )],
            ]
        );
    }
}
