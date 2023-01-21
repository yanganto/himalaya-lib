use log::{debug, trace, warn};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};

use crate::{flag, Backend, Envelope, MaildirBackend, ThreadSafeBackend};

use super::{Cache, Error, Result};

pub type Envelopes = HashMap<String, Envelope>;

#[derive(Clone, Debug, Eq, PartialEq)]
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

type Patch = Vec<Vec<Hunk>>;

pub fn sync_all<F, B>(
    folder: F,
    cache: &Cache,
    local: &MaildirBackend,
    remote: &B,
    dry_run: bool,
) -> Result<()>
where
    F: AsRef<str>,
    B: ThreadSafeBackend + ?Sized,
{
    debug!("synchronizing envelopes from folder: {}", folder.as_ref());

    let local_envelopes_cached: Envelopes = HashMap::from_iter(
        cache
            .list_local_envelopes(folder.as_ref())?
            .iter()
            .map(|envelope| (envelope.hash(folder.as_ref().clone()), envelope.clone())),
    );

    let local_envelopes: Envelopes = HashMap::from_iter(
        local
            .list_envelopes(folder.as_ref(), 0, 0)?
            .iter()
            .map(|envelope| (envelope.hash(folder.as_ref().clone()), envelope.clone())),
    );

    let remote_envelopes_cached: Envelopes = HashMap::from_iter(
        cache
            .list_remote_envelopes(folder.as_ref())?
            .iter()
            .map(|envelope| (envelope.hash(folder.as_ref().clone()), envelope.clone())),
    );

    let remote_envelopes: Envelopes = HashMap::from_iter(
        remote
            .list_envelopes(folder.as_ref(), 0, 0)?
            .iter()
            .map(|envelope| (envelope.hash(folder.as_ref().clone()), envelope.clone())),
    );

    let patch = build_patch(
        folder.as_ref(),
        local_envelopes_cached,
        local_envelopes,
        remote_envelopes_cached,
        remote_envelopes,
    );

    debug!("patch length: {}", patch.len());
    trace!("patch: {:#?}", patch);

    if !dry_run {
        let process_hunk = |hunk: &Hunk| {
            match hunk {
                Hunk::CacheEnvelope(folder, internal_id, HunkKindRestricted::Local) => {
                    let envelope = local.get_envelope_internal(folder, &internal_id)?;
                    cache.insert_local_envelope(folder, envelope)?;
                }
                Hunk::CacheEnvelope(folder, internal_id, HunkKindRestricted::Remote) => {
                    let envelope = remote.get_envelope_internal(&folder, &internal_id)?;
                    cache.insert_remote_envelope(folder, envelope)?;
                }
                Hunk::CopyEmail(folder, envelope, source, target) => {
                    let internal_ids = vec![envelope.internal_id.as_str()];
                    let emails = match source {
                        HunkKindRestricted::Local => {
                            local.get_emails_internal(folder, internal_ids)
                        }
                        HunkKindRestricted::Remote => {
                            remote.get_emails_internal(folder, internal_ids)
                        }
                    }?;
                    let emails = emails.to_vec();
                    let email = emails
                        .first()
                        .ok_or_else(|| Error::FindEmailError(envelope.internal_id.clone()))?;

                    match target {
                        HunkKindRestricted::Local => {
                            let internal_id =
                                local.add_email_internal(folder, email.raw()?, &envelope.flags)?;
                            let envelope = local.get_envelope_internal(folder, &internal_id)?;
                            cache.insert_local_envelope(folder, envelope)?;
                        }
                        HunkKindRestricted::Remote => {
                            let internal_id = remote.add_email_internal(
                                &folder,
                                email.raw()?,
                                &envelope.flags,
                            )?;
                            let envelope = local.get_envelope_internal(&folder, &internal_id)?;
                            cache.insert_remote_envelope(folder, envelope)?;
                        }
                    };
                }
                Hunk::RemoveEmail(folder, internal_id, HunkKind::LocalCache) => {
                    cache.delete_local_envelope(folder, internal_id)?;
                }
                Hunk::RemoveEmail(folder, internal_id, HunkKind::Local) => {
                    local.delete_emails_internal(folder, vec![internal_id])?;
                }
                Hunk::RemoveEmail(folder, internal_id, HunkKind::RemoteCache) => {
                    cache.delete_remote_envelope(folder, internal_id)?;
                }
                Hunk::RemoveEmail(folder, internal_id, HunkKind::Remote) => {
                    remote.delete_emails_internal(folder, vec![internal_id])?;
                }
                Hunk::SetFlags(folder, envelope, HunkKind::LocalCache) => {
                    cache.delete_local_envelope(folder, &envelope.internal_id)?;
                    cache.insert_local_envelope(folder, envelope.clone())?;
                }
                Hunk::SetFlags(folder, envelope, HunkKind::Local) => {
                    local.set_flags_internal(
                        folder,
                        vec![&envelope.internal_id],
                        &envelope.flags,
                    )?;
                }
                Hunk::SetFlags(folder, envelope, HunkKind::RemoteCache) => {
                    cache.delete_remote_envelope(folder, &envelope.internal_id)?;
                    cache.insert_remote_envelope(folder, envelope.clone())?;
                }
                Hunk::SetFlags(folder, envelope, HunkKind::Remote) => {
                    remote.set_flags_internal(
                        folder,
                        vec![&envelope.internal_id],
                        &envelope.flags,
                    )?;
                }
            };

            Result::Ok(())
        };

        for (batch_num, batch) in patch.chunks(3).enumerate() {
            debug!("processing batch {}/{}", batch_num + 1, patch.len() / 3);

            batch.par_iter().try_for_each(|hunks| {
                trace!("processing hunks: {:#?}", hunks);

                for hunk in hunks {
                    if let Err(err) = process_hunk(hunk) {
                        warn!(
                            "error while processing hunk {:?}, skipping it: {:?}",
                            hunk, err
                        );
                    }
                }

                Result::Ok(())
            })?;
        }
    }

    Ok(())
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
    let mut hashes = HashSet::new();

    // Gathers all existing hashes found in all envelopes.
    hashes.extend(local_cache.iter().map(|(hash, _)| hash.as_str()));
    hashes.extend(local.iter().map(|(hash, _)| hash.as_str()));
    hashes.extend(remote_cache.iter().map(|(hash, _)| hash.as_str()));
    hashes.extend(remote.iter().map(|(hash, _)| hash.as_str()));

    // Given the matrice local_cache × local × remote_cache × remote,
    // checks every 2⁴ = 16 possibilities:
    for hash in hashes {
        let local_cache = local_cache.get(hash);
        let local = local.get(hash);
        let remote_cache = remote_cache.get(hash);
        let remote = remote.get(hash);

        match (local_cache, local, remote_cache, remote) {
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
            // The hash exists everywhere except in remote cache side,
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
