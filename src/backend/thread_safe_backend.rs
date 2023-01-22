use log::info;
use proc_lock::{lock, LockPath};
use std::{borrow::Cow, io, result};
use thiserror::Error;

use crate::{
    account, envelope, folder, AccountConfig, Backend, MaildirBackendBuilder, MaildirConfig,
};

use super::maildir;

#[derive(Debug, Error)]
pub enum Error {
    #[error("cannot lock synchronization of account {1}")]
    SyncAccountLockError(io::Error, String),

    #[error(transparent)]
    ConfigError(#[from] account::config::Error),
    #[error(transparent)]
    MaildirError(#[from] maildir::Error),
    #[error(transparent)]
    SyncFoldersError(#[from] folder::sync::Error),
    #[error(transparent)]
    SyncEnvelopesError(#[from] envelope::sync::Error),
}

pub type Result<T> = result::Result<T, Error>;

pub trait ThreadSafeBackend: Backend + Send + Sync {
    fn sync(&self, account: &AccountConfig, dry_run: bool) -> Result<()> {
        info!("starting synchronization");

        if !account.sync {
            info!(
                "synchronization not enabled for account {}, exiting",
                account.name
            );
            return Ok(());
        }

        let lock_path = LockPath::Tmp(format!("himalaya-sync-{}.lock", self.name()));
        let guard =
            lock(&lock_path).map_err(|err| Error::SyncAccountLockError(err, self.name()))?;

        let sync_dir = account.sync_dir()?;

        let local = MaildirBackendBuilder::new()
            .url_encoded_folders(true)
            .build(
                Cow::Borrowed(account),
                Cow::Owned(MaildirConfig {
                    root_dir: sync_dir.join(&account.name),
                }),
            )?;

        let cache = folder::sync::Cache::new(Cow::Borrowed(account), &sync_dir)?;
        let folders = folder::sync_all(&cache, &local, self, dry_run)?;

        let cache = envelope::sync::Cache::new(Cow::Borrowed(account), &sync_dir)?;
        for folder in &folders {
            envelope::sync_all(folder, &cache, &local, self, dry_run)?;
        }

        drop(guard);

        Ok(())
    }
}
