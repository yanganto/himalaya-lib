use log::{info, trace};
use std::collections::HashSet;

use crate::{Backend, MaildirBackend, ThreadSafeBackend};

use super::{Cache, Result};

pub(super) type FoldersName = HashSet<FolderName>;
type FolderName = String;
type Patch = Vec<Hunk>;
type Target = HunkKind;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HunkKind {
    LocalCache,
    Local,
    RemoteCache,
    Remote,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Hunk {
    CreateFolder(FolderName, Target),
    DeleteFolder(FolderName, Target),
}

pub fn sync_all<B>(
    cache: &Cache,
    local: &MaildirBackend,
    remote: &B,
    dry_run: bool,
) -> Result<FoldersName>
where
    B: ThreadSafeBackend + ?Sized,
{
    info!("starting folders sync");

    let local_folders_cached: FoldersName =
        HashSet::from_iter(cache.list_local_folders()?.iter().cloned());

    trace!("local folders cached: {:#?}", local_folders_cached);

    // local Maildir folders are already encoded
    let local_folders: FoldersName = HashSet::from_iter(
        local
            .list_folders()?
            .iter()
            .map(|folder| folder.name.clone()),
    );

    trace!("local folders: {:#?}", local_folders);

    let remote_folders_cached: FoldersName =
        HashSet::from_iter(cache.list_remote_folders()?.iter().cloned());

    trace!("remote folders cached: {:#?}", remote_folders_cached);

    let remote_folders: FoldersName = HashSet::from_iter(
        remote
            .list_folders()?
            .iter()
            .map(|folder| folder.name.clone()),
    );

    trace!("remote folders: {:#?}", remote_folders);

    let (patch, folders) = build_patch(
        local_folders_cached,
        local_folders,
        remote_folders_cached,
        remote_folders,
    );

    info!("folders patch length: {}", patch.len());
    trace!("folders patch: {:#?}", patch);

    if dry_run {
        info!("dry run activated, skipping folders patch");
    } else {
        let patch_len = patch.len();
        for (hunk_num, hunk) in patch.into_iter().enumerate() {
            info!(
                "applying folders patch, hunk {}/{}",
                hunk_num + 1,
                patch_len
            );

            match hunk {
                Hunk::CreateFolder(ref folder, HunkKind::LocalCache) => {
                    cache.insert_local_folder(folder)?;
                }
                Hunk::CreateFolder(ref folder, HunkKind::Local) => {
                    local.add_folder(folder)?;
                }
                Hunk::CreateFolder(ref folder, HunkKind::RemoteCache) => {
                    cache.insert_remote_folder(folder)?;
                }
                Hunk::CreateFolder(ref folder, HunkKind::Remote) => {
                    remote.add_folder(&folder)?;
                }
                Hunk::DeleteFolder(ref folder, HunkKind::LocalCache) => {
                    cache.delete_local_folder(folder)?;
                }
                Hunk::DeleteFolder(ref folder, HunkKind::Local) => {
                    local.delete_folder(folder)?;
                }
                Hunk::DeleteFolder(ref folder, HunkKind::RemoteCache) => {
                    cache.delete_remote_folder(folder)?;
                }
                Hunk::DeleteFolder(ref folder, HunkKind::Remote) => {
                    remote.delete_folder(&folder)?;
                }
            }
        }
    }

    let folders = folders
        .into_iter()
        .map(|folder| {
            urlencoding::decode(&folder)
                .map(|folder| folder.to_string())
                .unwrap_or_else(|_| folder)
        })
        .collect();

    trace!("folders: {:#?}", folders);

    Ok(folders)
}

pub fn build_patch(
    local_cache: FoldersName,
    local: FoldersName,
    remote_cache: FoldersName,
    remote: FoldersName,
) -> (Patch, FoldersName) {
    let mut patch: Patch = vec![];
    let mut folders: FoldersName = HashSet::new();

    // Gathers all existing folders name.
    folders.extend(local_cache.clone());
    folders.extend(local.clone());
    folders.extend(remote_cache.clone());
    folders.extend(remote.clone());

    // Given the matrice local_cache × local × remote_cache × remote,
    // checks every 2⁴ = 16 possibilities:
    for folder in &folders {
        let local_cache = local_cache.get(folder);
        let local = local.get(folder);
        let remote_cache = remote_cache.get(folder);
        let remote = remote.get(folder);

        match (local_cache, local, remote_cache, remote) {
            // 0000
            (None, None, None, None) => (),

            // 0001
            (None, None, None, Some(_)) => patch.extend([
                Hunk::CreateFolder(folder.clone(), HunkKind::LocalCache),
                Hunk::CreateFolder(folder.clone(), HunkKind::Local),
                Hunk::CreateFolder(folder.clone(), HunkKind::RemoteCache),
            ]),

            // 0010
            (None, None, Some(_), None) => {
                patch.push(Hunk::DeleteFolder(folder.clone(), HunkKind::RemoteCache))
            }

            // 0011
            (None, None, Some(_), Some(_)) => patch.extend([
                Hunk::CreateFolder(folder.clone(), HunkKind::LocalCache),
                Hunk::CreateFolder(folder.clone(), HunkKind::Local),
            ]),

            // 0100
            //
            (None, Some(_), None, None) => patch.extend([
                Hunk::CreateFolder(folder.clone(), HunkKind::LocalCache),
                Hunk::CreateFolder(folder.clone(), HunkKind::RemoteCache),
                Hunk::CreateFolder(folder.clone(), HunkKind::Remote),
            ]),

            // 0101
            (None, Some(_), None, Some(_)) => patch.extend([
                Hunk::CreateFolder(folder.clone(), HunkKind::LocalCache),
                Hunk::CreateFolder(folder.clone(), HunkKind::RemoteCache),
            ]),

            // 0110
            (None, Some(_), Some(_), None) => patch.extend([
                Hunk::CreateFolder(folder.clone(), HunkKind::LocalCache),
                Hunk::CreateFolder(folder.clone(), HunkKind::Remote),
            ]),

            // 0111
            (None, Some(_), Some(_), Some(_)) => {
                patch.push(Hunk::CreateFolder(folder.clone(), HunkKind::LocalCache))
            }

            // 1000
            (Some(_), None, None, None) => {
                patch.push(Hunk::DeleteFolder(folder.clone(), HunkKind::LocalCache))
            }

            // 1001
            (Some(_), None, None, Some(_)) => patch.extend([
                Hunk::CreateFolder(folder.clone(), HunkKind::Local),
                Hunk::CreateFolder(folder.clone(), HunkKind::RemoteCache),
            ]),

            // 1010
            (Some(_), None, Some(_), None) => patch.extend([
                Hunk::DeleteFolder(folder.clone(), HunkKind::LocalCache),
                Hunk::DeleteFolder(folder.clone(), HunkKind::RemoteCache),
            ]),

            // 1011
            (Some(_), None, Some(_), Some(_)) => patch.extend([
                Hunk::DeleteFolder(folder.clone(), HunkKind::LocalCache),
                Hunk::DeleteFolder(folder.clone(), HunkKind::RemoteCache),
                Hunk::DeleteFolder(folder.clone(), HunkKind::Remote),
            ]),

            // 1100
            (Some(_), Some(_), None, None) => patch.extend([
                Hunk::CreateFolder(folder.clone(), HunkKind::RemoteCache),
                Hunk::CreateFolder(folder.clone(), HunkKind::Remote),
            ]),

            // 1101
            (Some(_), Some(_), None, Some(_)) => {
                patch.push(Hunk::CreateFolder(folder.clone(), HunkKind::RemoteCache))
            }

            // 1110
            (Some(_), Some(_), Some(_), None) => patch.extend([
                Hunk::DeleteFolder(folder.clone(), HunkKind::LocalCache),
                Hunk::DeleteFolder(folder.clone(), HunkKind::Local),
                Hunk::DeleteFolder(folder.clone(), HunkKind::RemoteCache),
            ]),

            // 1111
            (Some(_), Some(_), Some(_), Some(_)) => (),
        }
    }

    (patch, folders)
}

#[cfg(test)]
mod folders_sync {
    use super::{FoldersName, Hunk, HunkKind, Patch};

    #[test]
    fn build_folder_patch() {
        // 0000
        assert_eq!(
            super::build_patch(
                FoldersName::default(),
                FoldersName::default(),
                FoldersName::default(),
                FoldersName::default(),
            ),
            (vec![] as Patch, FoldersName::default()),
        );

        // 0001
        assert_eq!(
            super::build_patch(
                FoldersName::default(),
                FoldersName::default(),
                FoldersName::default(),
                FoldersName::from_iter(["folder".into()]),
            ),
            (
                vec![
                    Hunk::CreateFolder("folder".into(), HunkKind::LocalCache),
                    Hunk::CreateFolder("folder".into(), HunkKind::Local),
                    Hunk::CreateFolder("folder".into(), HunkKind::RemoteCache),
                ],
                FoldersName::from_iter(["folder".into()]),
            ),
        );

        // 0010
        assert_eq!(
            super::build_patch(
                FoldersName::default(),
                FoldersName::default(),
                FoldersName::from_iter(["folder".into()]),
                FoldersName::default(),
            ),
            (
                vec![Hunk::DeleteFolder("folder".into(), HunkKind::RemoteCache)],
                FoldersName::from_iter(["folder".into()]),
            ),
        );

        // 0011
        assert_eq!(
            super::build_patch(
                FoldersName::default(),
                FoldersName::default(),
                FoldersName::from_iter(["folder".into()]),
                FoldersName::from_iter(["folder".into()]),
            ),
            (
                vec![
                    Hunk::CreateFolder("folder".into(), HunkKind::LocalCache),
                    Hunk::CreateFolder("folder".into(), HunkKind::Local),
                ],
                FoldersName::from_iter(["folder".into()]),
            ),
        );

        // 0100
        assert_eq!(
            super::build_patch(
                FoldersName::default(),
                FoldersName::from_iter(["folder".into()]),
                FoldersName::default(),
                FoldersName::default(),
            ),
            (
                vec![
                    Hunk::CreateFolder("folder".into(), HunkKind::LocalCache),
                    Hunk::CreateFolder("folder".into(), HunkKind::RemoteCache),
                    Hunk::CreateFolder("folder".into(), HunkKind::Remote),
                ],
                FoldersName::from_iter(["folder".into()]),
            ),
        );

        // 0101
        assert_eq!(
            super::build_patch(
                FoldersName::default(),
                FoldersName::from_iter(["folder".into()]),
                FoldersName::default(),
                FoldersName::from_iter(["folder".into()]),
            ),
            (
                vec![
                    Hunk::CreateFolder("folder".into(), HunkKind::LocalCache),
                    Hunk::CreateFolder("folder".into(), HunkKind::RemoteCache),
                ],
                FoldersName::from_iter(["folder".into()]),
            ),
        );

        // 0110
        assert_eq!(
            super::build_patch(
                FoldersName::default(),
                FoldersName::from_iter(["folder".into()]),
                FoldersName::from_iter(["folder".into()]),
                FoldersName::default(),
            ),
            (
                vec![
                    Hunk::CreateFolder("folder".into(), HunkKind::LocalCache),
                    Hunk::CreateFolder("folder".into(), HunkKind::Remote),
                ],
                FoldersName::from_iter(["folder".into()]),
            ),
        );

        // 0111
        assert_eq!(
            super::build_patch(
                FoldersName::default(),
                FoldersName::from_iter(["folder".into()]),
                FoldersName::from_iter(["folder".into()]),
                FoldersName::from_iter(["folder".into()]),
            ),
            (
                vec![Hunk::CreateFolder("folder".into(), HunkKind::LocalCache)],
                FoldersName::from_iter(["folder".into()]),
            ),
        );

        // 1000
        assert_eq!(
            super::build_patch(
                FoldersName::from_iter(["folder".into()]),
                FoldersName::default(),
                FoldersName::default(),
                FoldersName::default(),
            ),
            (
                vec![Hunk::DeleteFolder("folder".into(), HunkKind::LocalCache)],
                FoldersName::from_iter(["folder".into()]),
            ),
        );

        // 1001
        assert_eq!(
            super::build_patch(
                FoldersName::from_iter(["folder".into()]),
                FoldersName::default(),
                FoldersName::default(),
                FoldersName::from_iter(["folder".into()]),
            ),
            (
                vec![
                    Hunk::CreateFolder("folder".into(), HunkKind::Local),
                    Hunk::CreateFolder("folder".into(), HunkKind::RemoteCache),
                ],
                FoldersName::from_iter(["folder".into()]),
            ),
        );

        // 1010
        assert_eq!(
            super::build_patch(
                FoldersName::from_iter(["folder".into()]),
                FoldersName::default(),
                FoldersName::from_iter(["folder".into()]),
                FoldersName::default(),
            ),
            (
                vec![
                    Hunk::DeleteFolder("folder".into(), HunkKind::LocalCache),
                    Hunk::DeleteFolder("folder".into(), HunkKind::RemoteCache),
                ],
                FoldersName::from_iter(["folder".into()]),
            ),
        );

        // 1011
        assert_eq!(
            super::build_patch(
                FoldersName::from_iter(["folder".into()]),
                FoldersName::default(),
                FoldersName::from_iter(["folder".into()]),
                FoldersName::from_iter(["folder".into()]),
            ),
            (
                vec![
                    Hunk::DeleteFolder("folder".into(), HunkKind::LocalCache),
                    Hunk::DeleteFolder("folder".into(), HunkKind::RemoteCache),
                    Hunk::DeleteFolder("folder".into(), HunkKind::Remote),
                ],
                FoldersName::from_iter(["folder".into()]),
            ),
        );

        // 1100
        assert_eq!(
            super::build_patch(
                FoldersName::from_iter(["folder".into()]),
                FoldersName::from_iter(["folder".into()]),
                FoldersName::default(),
                FoldersName::default(),
            ),
            (
                vec![
                    Hunk::CreateFolder("folder".into(), HunkKind::RemoteCache),
                    Hunk::CreateFolder("folder".into(), HunkKind::Remote),
                ],
                FoldersName::from_iter(["folder".into()]),
            ),
        );

        // 1101
        assert_eq!(
            super::build_patch(
                FoldersName::from_iter(["folder".into()]),
                FoldersName::from_iter(["folder".into()]),
                FoldersName::default(),
                FoldersName::from_iter(["folder".into()]),
            ),
            (
                vec![Hunk::CreateFolder("folder".into(), HunkKind::RemoteCache)],
                FoldersName::from_iter(["folder".into()]),
            ),
        );

        // 1110
        assert_eq!(
            super::build_patch(
                FoldersName::from_iter(["folder".into()]),
                FoldersName::from_iter(["folder".into()]),
                FoldersName::from_iter(["folder".into()]),
                FoldersName::default(),
            ),
            (
                vec![
                    Hunk::DeleteFolder("folder".into(), HunkKind::LocalCache),
                    Hunk::DeleteFolder("folder".into(), HunkKind::Local),
                    Hunk::DeleteFolder("folder".into(), HunkKind::RemoteCache),
                ],
                FoldersName::from_iter(["folder".into()]),
            ),
        );

        // 1111
        assert_eq!(
            super::build_patch(
                FoldersName::from_iter(["folder".into()]),
                FoldersName::from_iter(["folder".into()]),
                FoldersName::from_iter(["folder".into()]),
                FoldersName::from_iter(["folder".into()]),
            ),
            (vec![] as Patch, FoldersName::from_iter(["folder".into()])),
        );
    }
}
