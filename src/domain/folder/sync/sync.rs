use log::{debug, info, trace};
use std::{collections::HashSet, fmt};

use crate::{AccountConfig, Backend, BackendSyncProgressEvent, MaildirBackend};

use super::{Cache, Result};

pub type FoldersName = HashSet<FolderName>;
pub type FolderName = String;
pub type Patch = Vec<Hunk>;
pub type Target = HunkKind;

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Hunk {
    CreateFolder(FolderName, Target),
    DeleteFolder(FolderName, Target),
}

impl fmt::Display for Hunk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CreateFolder(name, target) => write!(f, "Adding folder {name} to {target}"),
            Self::DeleteFolder(name, target) => write!(f, "Removing folder {name} from {target}"),
        }
    }
}

pub struct SyncBuilder<'a> {
    account_config: &'a AccountConfig,
    dry_run: bool,
    on_progress: Box<dyn Fn(BackendSyncProgressEvent) -> Result<()> + 'a>,
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
        F: Fn(BackendSyncProgressEvent) -> Result<()> + 'a,
    {
        self.on_progress = Box::new(f);
        self
    }

    pub fn sync(
        &self,
        local: &MaildirBackend,
        remote: &dyn Backend,
    ) -> Result<(Patch, FoldersName)> {
        info!("starting folders sync");

        let progress = &self.on_progress;
        let cache = Cache::new(self.account_config)?;

        progress(BackendSyncProgressEvent::GetLocalCachedFolders)?;

        let local_folders_cached: FoldersName =
            HashSet::from_iter(cache.list_local_folders()?.iter().cloned());

        trace!("local folders cached: {:#?}", local_folders_cached);

        progress(BackendSyncProgressEvent::GetLocalFolders)?;

        let local_folders: FoldersName = HashSet::from_iter(
            local
                .list_folders()
                .map_err(Box::new)?
                .iter()
                .map(|folder| folder.name.clone()),
        );

        trace!("local folders: {:#?}", local_folders);

        progress(BackendSyncProgressEvent::GetRemoteCachedFolders)?;

        let remote_folders_cached: FoldersName =
            HashSet::from_iter(cache.list_remote_folders()?.iter().cloned());

        trace!("remote folders cached: {:#?}", remote_folders_cached);

        progress(BackendSyncProgressEvent::GetRemoteFolders)?;

        let remote_folders: FoldersName = HashSet::from_iter(
            remote
                .list_folders()
                .map_err(Box::new)?
                .iter()
                .map(|folder| folder.name.clone()),
        );

        trace!("remote folders: {:#?}", remote_folders);

        progress(BackendSyncProgressEvent::BuildFoldersPatch)?;

        let (patch, folders) = build_patch(
            local_folders_cached,
            local_folders,
            remote_folders_cached,
            remote_folders,
        );

        progress(BackendSyncProgressEvent::ProcessFoldersPatch(patch.len()))?;

        debug!("folders patch: {:#?}", patch);

        if self.dry_run {
            info!("dry run enabled, skipping folders patch");
        } else {
            let patch_len = patch.len();

            for (hunk_num, hunk) in patch.iter().enumerate() {
                debug!(
                    "applying folders patch, hunk {}/{}",
                    hunk_num + 1,
                    patch_len
                );

                trace!("processing hunk: {hunk:#?}");

                progress(BackendSyncProgressEvent::ProcessFolderHunk(
                    hunk.to_string(),
                ))?;

                match hunk {
                    Hunk::CreateFolder(ref folder, HunkKind::LocalCache) => {
                        cache.insert_local_folder(folder)?;
                    }
                    Hunk::CreateFolder(ref folder, HunkKind::Local) => {
                        local.add_folder(folder).map_err(Box::new)?;
                    }
                    Hunk::CreateFolder(ref folder, HunkKind::RemoteCache) => {
                        cache.insert_remote_folder(folder)?;
                    }
                    Hunk::CreateFolder(ref folder, HunkKind::Remote) => {
                        remote.add_folder(&folder).map_err(Box::new)?;
                    }
                    Hunk::DeleteFolder(ref folder, HunkKind::LocalCache) => {
                        cache.delete_local_folder(folder)?;
                    }
                    Hunk::DeleteFolder(ref folder, HunkKind::Local) => {
                        local.delete_folder(folder).map_err(Box::new)?;
                    }
                    Hunk::DeleteFolder(ref folder, HunkKind::RemoteCache) => {
                        cache.delete_remote_folder(folder)?;
                    }
                    Hunk::DeleteFolder(ref folder, HunkKind::Remote) => {
                        remote.delete_folder(&folder).map_err(Box::new)?;
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

        Ok((patch, folders))
    }
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
