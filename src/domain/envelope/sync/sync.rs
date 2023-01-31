use log::{debug, info, trace, warn};
use rayon::prelude::*;
use std::{
    collections::{HashMap, HashSet},
    fmt,
};

use crate::{flag, AccountConfig, Backend, BackendSyncProgressEvent, Envelope, MaildirBackend};

use super::{Cache, Error, Result};

pub type Envelopes = HashMap<String, Envelope>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HunkKind {
    LocalCache,
    Local,
    RemoteCache,
    Remote,
}

impl fmt::Display for HunkKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LocalCache => write!(f, "local cache"),
            Self::Local => write!(f, "local backend"),
            Self::RemoteCache => write!(f, "remote cache"),
            Self::Remote => write!(f, "remote backend"),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum HunkKindRestricted {
    Local,
    Remote,
}

impl fmt::Display for HunkKindRestricted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local => write!(f, "local"),
            Self::Remote => write!(f, "remote"),
        }
    }
}

type FolderName = String;
type InternalId = String;
type SourceRestricted = HunkKindRestricted;
type Target = HunkKind;
type TargetRestricted = HunkKindRestricted;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Hunk {
    CacheEnvelope(FolderName, InternalId, SourceRestricted),
    CopyEmail(FolderName, Envelope, SourceRestricted, TargetRestricted),
    RemoveEmail(FolderName, InternalId, Target),
    SetFlags(FolderName, Envelope, Target),
}

impl fmt::Display for Hunk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CacheEnvelope(folder, id, source) => {
                write!(f, "Adding envelope {id} to {source} cache folder {folder}",)
            }
            Self::CopyEmail(folder, envelope, source, target) => {
                write!(
                    f,
                    "Copying {source} envelope {id} to {target} folder {folder}",
                    id = envelope.id,
                )
            }
            Self::RemoveEmail(folder, id, target) => {
                write!(f, "Removing envelope {id} from {target} folder {folder}")
            }
            Self::SetFlags(folder, envelope, target) => {
                write!(
                    f,
                    "Setting flags {flags} to {target} envelope from folder {folder}",
                    flags = envelope.flags.to_string(),
                )
            }
        }
    }
}

pub type Patch = Vec<Vec<Hunk>>;

pub struct SyncBuilder<'a> {
    account_config: &'a AccountConfig,
    dry_run: bool,
    on_progress: Box<dyn Fn(BackendSyncProgressEvent) -> Result<()> + Sync + Send + 'a>,
}

impl<'a> SyncBuilder<'a> {
    pub fn new(account_config: &'a AccountConfig) -> Self {
        Self {
            account_config,
            dry_run: false,
            on_progress: Box::new(|_| Ok(())),
        }
    }

    pub fn dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }

    pub fn on_progress<F>(mut self, f: F) -> Self
    where
        F: Fn(BackendSyncProgressEvent) -> Result<()> + Sync + Send + 'a,
    {
        self.on_progress = Box::new(f);
        self
    }

    pub fn sync<F>(&self, folder: F, local: &MaildirBackend, remote: &dyn Backend) -> Result<Patch>
    where
        F: ToString,
    {
        let folder = folder.to_string();
        info!("synchronizing envelopes from folder {folder}");

        let progress = &self.on_progress;
        let cache = Cache::new(self.account_config)?;

        progress(BackendSyncProgressEvent::GetLocalCachedEnvelopes)?;

        let local_envelopes_cached: Envelopes = HashMap::from_iter(
            cache
                .list_local_envelopes(&folder)?
                .iter()
                .map(|envelope| (envelope.message_id.clone(), envelope.clone())),
        );

        trace!("local envelopes cached: {:#?}", local_envelopes_cached);

        progress(BackendSyncProgressEvent::GetLocalEnvelopes)?;

        let local_envelopes: Envelopes = HashMap::from_iter(
            local
                .list_envelopes(&folder, 0, 0)
                .or_else(|err| {
                    if self.dry_run {
                        Ok(Default::default())
                    } else {
                        Err(Box::new(err))
                    }
                })?
                .iter()
                .map(|envelope| {
                    (
                        envelope.message_id.clone(),
                        envelope.clone_without_custom_flags(),
                    )
                }),
        );

        trace!("local envelopes: {:#?}", local_envelopes);

        progress(BackendSyncProgressEvent::GetRemoteCachedEnvelopes)?;

        let remote_envelopes_cached: Envelopes = HashMap::from_iter(
            cache
                .list_remote_envelopes(&folder)?
                .iter()
                .map(|envelope| (envelope.message_id.clone(), envelope.clone())),
        );

        trace!("remote envelopes cached: {:#?}", remote_envelopes_cached);

        progress(BackendSyncProgressEvent::GetRemoteEnvelopes)?;

        let remote_envelopes: Envelopes = HashMap::from_iter(
            remote
                .list_envelopes(&folder, 0, 0)
                .or_else(|err| {
                    if self.dry_run {
                        Ok(Default::default())
                    } else {
                        Err(Box::new(err))
                    }
                })?
                .iter()
                .map(|envelope| {
                    (
                        envelope.message_id.clone(),
                        envelope.clone_without_custom_flags(),
                    )
                }),
        );

        trace!("remote envelopes: {:#?}", remote_envelopes);

        progress(BackendSyncProgressEvent::BuildEnvelopesPatch)?;

        let patch = build_patch(
            &folder,
            local_envelopes_cached,
            local_envelopes,
            remote_envelopes_cached,
            remote_envelopes,
        );

        progress(BackendSyncProgressEvent::ProcessEnvelopesPatch(patch.len()))?;

        debug!("envelopes patch: {:#?}", patch);

        if self.dry_run {
            info!("dry run activated, skipping envelopes patch");
        } else {
            let process_hunk = |hunk: &Hunk| {
                match hunk {
                    Hunk::CacheEnvelope(folder, internal_id, HunkKindRestricted::Local) => {
                        let envelope = local
                            .get_envelope_internal(folder, &internal_id)
                            .map_err(Box::new)?;
                        cache
                            .insert_local_envelope(folder, envelope.clone_without_custom_flags())?;
                    }
                    Hunk::CacheEnvelope(folder, internal_id, HunkKindRestricted::Remote) => {
                        let envelope = remote
                            .get_envelope_internal(&folder, &internal_id)
                            .map_err(Box::new)?;
                        cache.insert_remote_envelope(
                            folder,
                            envelope.clone_without_custom_flags(),
                        )?;
                    }
                    Hunk::CopyEmail(folder, envelope, source, target) => {
                        let internal_ids = vec![envelope.internal_id.as_str()];
                        let emails = match source {
                            HunkKindRestricted::Local => {
                                local.preview_emails_internal(folder, internal_ids)
                            }
                            HunkKindRestricted::Remote => {
                                remote.preview_emails_internal(folder, internal_ids)
                            }
                        }
                        .map_err(Box::new)?;
                        let emails = emails.to_vec();
                        let email = emails
                            .first()
                            .ok_or_else(|| Error::FindEmailError(envelope.internal_id.clone()))?;

                        match target {
                            HunkKindRestricted::Local => {
                                let internal_id = local
                                    .add_email_internal(folder, email.raw()?, &envelope.flags)
                                    .map_err(Box::new)?;
                                let envelope = local
                                    .get_envelope_internal(folder, &internal_id)
                                    .map_err(Box::new)?;
                                cache.insert_local_envelope(
                                    folder,
                                    envelope.clone_without_custom_flags(),
                                )?;
                            }
                            HunkKindRestricted::Remote => {
                                let internal_id = remote
                                    .add_email_internal(&folder, email.raw()?, &envelope.flags)
                                    .map_err(Box::new)?;
                                let envelope = remote
                                    .get_envelope_internal(&folder, &internal_id)
                                    .map_err(Box::new)?;
                                cache.insert_remote_envelope(
                                    folder,
                                    envelope.clone_without_custom_flags(),
                                )?;
                            }
                        };
                    }
                    Hunk::RemoveEmail(folder, internal_id, HunkKind::LocalCache) => {
                        cache.delete_local_envelope(folder, internal_id)?;
                    }
                    Hunk::RemoveEmail(folder, internal_id, HunkKind::Local) => {
                        local
                            .delete_emails_internal(folder, vec![internal_id])
                            .map_err(Box::new)?;
                    }
                    Hunk::RemoveEmail(folder, internal_id, HunkKind::RemoteCache) => {
                        cache.delete_remote_envelope(folder, internal_id)?;
                    }
                    Hunk::RemoveEmail(folder, internal_id, HunkKind::Remote) => {
                        remote
                            .delete_emails_internal(folder, vec![internal_id])
                            .map_err(Box::new)?;
                    }
                    Hunk::SetFlags(folder, envelope, HunkKind::LocalCache) => {
                        cache.delete_local_envelope(folder, &envelope.internal_id)?;
                        cache.insert_local_envelope(folder, envelope.clone())?;
                    }
                    Hunk::SetFlags(folder, envelope, HunkKind::Local) => {
                        local
                            .set_flags_internal(
                                folder,
                                vec![&envelope.internal_id],
                                &envelope.flags,
                            )
                            .map_err(Box::new)?;
                    }
                    Hunk::SetFlags(folder, envelope, HunkKind::RemoteCache) => {
                        cache.delete_remote_envelope(folder, &envelope.internal_id)?;
                        cache.insert_remote_envelope(folder, envelope.clone())?;
                    }
                    Hunk::SetFlags(folder, envelope, HunkKind::Remote) => {
                        remote
                            .set_flags_internal(
                                folder,
                                vec![&envelope.internal_id],
                                &envelope.flags,
                            )
                            .map_err(Box::new)?;
                    }
                };

                Result::Ok(())
            };

            let patch_chunks = patch.chunks(10);
            let patch_chunks_len = patch_chunks.len();

            for (batch_num, batch) in patch_chunks.enumerate() {
                debug!(
                    "processing envelopes patch, batch {}/{}",
                    batch_num + 1,
                    patch_chunks_len
                );

                batch.par_iter().try_for_each(|hunks| {
                    for hunk in hunks {
                        trace!("processing hunk: {hunk:#?}");
                        progress(BackendSyncProgressEvent::ProcessEnvelopeHunk(
                            hunk.to_string(),
                        ))?;

                        match process_hunk(hunk) {
                            Ok(()) => (),
                            Err(err) => {
                                warn!(
                                    "error while processing hunk {:?}, skipping it: {:?}",
                                    hunk, err
                                );
                            }
                        }
                    }

                    Result::Ok(())
                })?;
            }
        }

        Ok(patch)
    }
}

pub fn build_patch<F>(
    folder: F,
    local_cache: Envelopes,
    local: Envelopes,
    remote_cache: Envelopes,
    remote: Envelopes,
) -> Patch
where
    F: Clone + ToString,
{
    let mut patch: Patch = vec![];
    let mut message_ids = HashSet::new();

    // Gathers all existing hashes found in all envelopes.
    message_ids.extend(local_cache.iter().map(|(id, _)| id.as_str()));
    message_ids.extend(local.iter().map(|(id, _)| id.as_str()));
    message_ids.extend(remote_cache.iter().map(|(id, _)| id.as_str()));
    message_ids.extend(remote.iter().map(|(id, _)| id.as_str()));

    // Given the matrice local_cache × local × remote_cache × remote,
    // checks every 2⁴ = 16 possibilities:
    for message_id in message_ids {
        let local_cache = local_cache.get(message_id);
        let local = local.get(message_id);
        let remote_cache = remote_cache.get(message_id);
        let remote = remote.get(message_id);

        match (local_cache, local, remote_cache, remote) {
            // 0000
            //
            // The message_id exists nowhere, which cannot happen since
            // message_ides has been built from all envelopes message_id.
            (None, None, None, None) => (),

            // 0001
            //
            // The message_id only exists in the remote side, which means a
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
            // The message_id only exists in the remote cache, which means
            // an email is outdated and needs to be removed from the
            // remote cache.
            (None, None, Some(remote_cache), None) => patch.push(vec![Hunk::RemoveEmail(
                folder.to_string(),
                remote_cache.internal_id.clone(),
                HunkKind::RemoteCache,
            )]),

            // 0011
            //
            // The message_id exists in the remote side but not in the local
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
                        Envelope {
                            flags: remote.flags.clone(),
                            ..remote_cache.clone()
                        },
                        HunkKind::RemoteCache,
                    )])
                }
            }

            // 0100
            //
            // The message_id only exists in the local side, which means a
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
            // The message_id exists in both local and remote sides, which
            // means a new (same) email has been added both sides and
            // the most recent needs to be kept.
            //
            // NOTE: this case should never happen: new emails
            // internal identifier are unique and should (in theory)
            // never conflict, but we implement this case for the sake
            // of exhaustiveness.
            (None, Some(local), None, Some(remote)) => {
                if local.date > remote.date {
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
                } else {
                    patch.push(vec![
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
                    ])
                }
            }

            // 0110
            //
            // The message_id exists in the local side and in the remote
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
            // The message_id exists everywhere except in the local cache,
            // which means the local cache misses an email and needs
            // to be updated. Flags also need to be synchronized.
            (None, Some(local), Some(remote_cache), Some(remote)) => {
                patch.push(vec![Hunk::CacheEnvelope(
                    folder.to_string(),
                    local.internal_id.clone(),
                    HunkKindRestricted::Local,
                )]);

                let flags = flag::sync_all(None, Some(local), Some(remote_cache), Some(remote));

                if local.flags != flags {
                    patch.push(vec![Hunk::SetFlags(
                        folder.to_string(),
                        Envelope {
                            flags: flags.clone(),
                            ..local.clone()
                        },
                        HunkKind::Local,
                    )]);
                }

                if remote_cache.flags != flags {
                    patch.push(vec![Hunk::SetFlags(
                        folder.to_string(),
                        Envelope {
                            flags: flags.clone(),
                            ..remote_cache.clone()
                        },
                        HunkKind::RemoteCache,
                    )]);
                }

                if remote.flags != flags {
                    patch.push(vec![Hunk::SetFlags(
                        folder.to_string(),
                        Envelope {
                            flags: flags.clone(),
                            ..remote.clone()
                        },
                        HunkKind::Remote,
                    )]);
                }
            }

            // 1000
            //
            // The message_id only exists in the local cache, which means
            // the local cache has an outdated email and need to be
            // cleaned.
            (Some(local_cache), None, None, None) => patch.push(vec![Hunk::RemoveEmail(
                folder.to_string(),
                local_cache.internal_id.clone(),
                HunkKind::LocalCache,
            )]),

            // 1001
            //
            // The message_id exists in the local cache and in the remote,
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
            // The message_id only exists in both caches, which means caches
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
            // The message_id exists everywhere except in local side, which
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
            // The message_id exists in local side but not in remote side,
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
                        Envelope {
                            flags: local.flags.clone(),
                            ..local_cache.clone()
                        },
                        HunkKind::LocalCache,
                    )]);
                }
            }

            // 1101
            //
            // The message_id exists everywhere except in remote cache side,
            // which means an email is missing remote cache side and
            // needs to be updated. Flags also need to be
            // synchronized.
            (Some(local_cache), Some(local), None, Some(remote)) => {
                let flags = flag::sync_all(Some(local_cache), Some(local), None, Some(remote));

                if local_cache.flags != flags {
                    patch.push(vec![Hunk::SetFlags(
                        folder.to_string(),
                        Envelope {
                            flags: flags.clone(),
                            ..local_cache.clone()
                        },
                        HunkKind::LocalCache,
                    )]);
                }

                if local.flags != flags {
                    patch.push(vec![Hunk::SetFlags(
                        folder.to_string(),
                        Envelope {
                            flags: flags.clone(),
                            ..local.clone()
                        },
                        HunkKind::Local,
                    )]);
                }

                if remote.flags != flags {
                    patch.push(vec![Hunk::SetFlags(
                        folder.to_string(),
                        Envelope {
                            flags: flags.clone(),
                            ..remote.clone()
                        },
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
            // The message_id exists everywhere except in remote side, which
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
            // The message_id exists everywhere, which means all flags need
            // to be synchronized.
            (Some(local_cache), Some(local), Some(remote_cache), Some(remote)) => {
                let flags = flag::sync_all(
                    Some(local_cache),
                    Some(local),
                    Some(remote_cache),
                    Some(remote),
                );

                if local_cache.flags != flags {
                    patch.push(vec![Hunk::SetFlags(
                        folder.to_string(),
                        Envelope {
                            flags: flags.clone(),
                            ..local_cache.clone()
                        },
                        HunkKind::LocalCache,
                    )]);
                }

                if local.flags != flags {
                    patch.push(vec![Hunk::SetFlags(
                        folder.to_string(),
                        Envelope {
                            flags: flags.clone(),
                            ..local.clone()
                        },
                        HunkKind::Local,
                    )]);
                }

                if remote_cache.flags != flags {
                    patch.push(vec![Hunk::SetFlags(
                        folder.to_string(),
                        Envelope {
                            flags: flags.clone(),
                            ..remote_cache.clone()
                        },
                        HunkKind::RemoteCache,
                    )]);
                }

                if remote.flags != flags {
                    patch.push(vec![Hunk::SetFlags(
                        folder.to_string(),
                        Envelope {
                            flags: flags.clone(),
                            ..remote.clone()
                        },
                        HunkKind::Remote,
                    )]);
                }
            }
        }
    }

    patch
}

#[cfg(test)]
mod envelopes_sync {
    use crate::{Envelope, Flag, Flags};

    use super::{Envelopes, Hunk, HunkKind, HunkKindRestricted, Patch};

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
            "message_id".into(),
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
            "message_id".into(),
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
            "message_id".into(),
            Envelope {
                internal_id: "remote-cache-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let remote = Envelopes::from_iter([(
            "message_id".into(),
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
            "message_id".into(),
            Envelope {
                internal_id: "remote-cache-id".into(),
                flags: "seen replied".into(),
                ..Envelope::default()
            },
        )]);
        let remote = Envelopes::from_iter([(
            "message_id".into(),
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
                    Envelope {
                        internal_id: "remote-cache-id".into(),
                        flags: Flags::from_iter([Flag::Seen, Flag::Flagged, Flag::Deleted]),
                        ..Envelope::default()
                    },
                    HunkKind::RemoteCache,
                )]
            ]
        );
    }

    #[test]
    fn build_patch_0100() {
        let local_cache = Envelopes::default();
        let local = Envelopes::from_iter([(
            "message_id".into(),
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
                "message_id-1".into(),
                Envelope {
                    internal_id: "local-id-1".into(),
                    flags: "seen".into(),
                    date: "2022-01-01T00:00:00-00:00".parse().unwrap(),
                    ..Envelope::default()
                },
            ),
            (
                "message_id-2".into(),
                Envelope {
                    internal_id: "local-id-2".into(),
                    flags: "seen".into(),
                    date: "2022-01-01T00:00:00-00:00".parse().unwrap(),
                    ..Envelope::default()
                },
            ),
            (
                "message_id-3".into(),
                Envelope {
                    internal_id: "local-id-3".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                },
            ),
            (
                "message_id-4".into(),
                Envelope {
                    internal_id: "local-id-4".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                },
            ),
            (
                "message_id-5".into(),
                Envelope {
                    internal_id: "local-id-5".into(),
                    flags: "seen".into(),
                    date: "2022-01-01T00:00:00-00:00".parse().unwrap(),
                    ..Envelope::default()
                },
            ),
        ]);
        let remote_cache = Envelopes::default();
        let remote = Envelopes::from_iter([
            (
                "message_id-1".into(),
                Envelope {
                    internal_id: "remote-id-1".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                },
            ),
            (
                "message_id-2".into(),
                Envelope {
                    internal_id: "remote-id-2".into(),
                    flags: "seen".into(),
                    date: "2021-01-01T00:00:00-00:00".parse().unwrap(),
                    ..Envelope::default()
                },
            ),
            (
                "message_id-3".into(),
                Envelope {
                    internal_id: "remote-id-3".into(),
                    flags: "seen".into(),
                    ..Envelope::default()
                },
            ),
            (
                "message_id-4".into(),
                Envelope {
                    internal_id: "remote-id-4".into(),
                    flags: "seen".into(),
                    date: "2022-01-01T00:00:00-00:00".parse().unwrap(),
                    ..Envelope::default()
                },
            ),
            (
                "message_id-5".into(),
                Envelope {
                    internal_id: "remote-id-5".into(),
                    flags: "seen".into(),
                    date: "2022-01-01T00:00:00-00:00".parse().unwrap(),
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
                date: "2022-01-01T00:00:00-00:00".parse().unwrap(),
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
                date: "2022-01-01T00:00:00-00:00".parse().unwrap(),
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
                date: "2022-01-01T00:00:00-00:00".parse().unwrap(),
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
                date: "2022-01-01T00:00:00-00:00".parse().unwrap(),
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
            "message_id".into(),
            Envelope {
                internal_id: "local-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let remote_cache = Envelopes::from_iter([(
            "message_id".into(),
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
            "message_id".into(),
            Envelope {
                internal_id: "local-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let remote_cache = Envelopes::from_iter([(
            "message_id".into(),
            Envelope {
                internal_id: "remote-cache-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let remote = Envelopes::from_iter([(
            "message_id".into(),
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
            "message_id".into(),
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
            "message_id".into(),
            Envelope {
                internal_id: "local-cache-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let local = Envelopes::default();
        let remote_cache = Envelopes::default();
        let remote = Envelopes::from_iter([(
            "message_id".into(),
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
            "message_id".into(),
            Envelope {
                internal_id: "local-cache-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let local = Envelopes::default();
        let remote_cache = Envelopes::from_iter([(
            "message_id".into(),
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
            "message_id".into(),
            Envelope {
                internal_id: "local-cache-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let local = Envelopes::default();
        let remote_cache = Envelopes::from_iter([(
            "message_id".into(),
            Envelope {
                internal_id: "remote-cache-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let remote = Envelopes::from_iter([(
            "message_id".into(),
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
            "message_id".into(),
            Envelope {
                internal_id: "local-cache-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let local = Envelopes::from_iter([(
            "message_id".into(),
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
            "message_id".into(),
            Envelope {
                internal_id: "local-cache-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let local = Envelopes::from_iter([(
            "message_id".into(),
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
                    Envelope {
                        internal_id: "local-cache-id".into(),
                        flags: Flags::from_iter([Flag::Flagged]),
                        ..Envelope::default()
                    },
                    HunkKind::LocalCache,
                )]
            ]
        );
    }

    #[test]
    fn build_patch_1101() {
        let local_cache = Envelopes::from_iter([(
            "message_id".into(),
            Envelope {
                internal_id: "local-cache-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let local = Envelopes::from_iter([(
            "message_id".into(),
            Envelope {
                internal_id: "local-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let remote_cache = Envelopes::default();
        let remote = Envelopes::from_iter([(
            "message_id".into(),
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
            "message_id".into(),
            Envelope {
                internal_id: "local-cache-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let local = Envelopes::from_iter([(
            "message_id".into(),
            Envelope {
                internal_id: "local-id".into(),
                flags: "seen".into(),
                ..Envelope::default()
            },
        )]);
        let remote_cache = Envelopes::from_iter([(
            "message_id".into(),
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
