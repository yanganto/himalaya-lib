//! Backend module.
//!
//! This module exposes the backend trait, which can be used to create
//! custom backend implementations.

use log::info;
use proc_lock::{lock, LockPath};
use std::{any::Any, borrow::Cow, io, result};
use thiserror::Error;

use crate::{
    account, backend, email, envelope, folder, id_mapper, AccountConfig, BackendConfig, Emails,
    Envelope, Envelopes, Flags, Folders, ImapBackendBuilder, MaildirBackendBuilder, MaildirConfig,
};

#[cfg(feature = "maildir-backend")]
use crate::MaildirBackend;

#[cfg(feature = "notmuch-backend")]
use crate::NotmuchBackend;

#[derive(Debug, Error)]
pub enum Error {
    #[error("cannot build backend with an empty config")]
    BuildBackendError,
    #[error("cannot lock synchronization for account {1}")]
    SyncAccountLockError(io::Error, String),
    #[error("synchronization not enabled for account {0}")]
    SyncNotEnabled(String),
    #[error(transparent)]
    EmailError(#[from] email::Error),
    #[error(transparent)]
    IdMapper(#[from] id_mapper::Error),
    #[error(transparent)]
    ConfigError(#[from] account::config::Error),
    #[error(transparent)]
    SyncFoldersError(#[from] folder::sync::Error),
    #[error(transparent)]
    SyncEnvelopesError(#[from] envelope::sync::Error),

    #[cfg(feature = "imap-backend")]
    #[error(transparent)]
    ImapBackendError(#[from] backend::imap::Error),
    #[cfg(feature = "maildir-backend")]
    #[error(transparent)]
    MaildirBackendError(#[from] backend::maildir::Error),
    #[cfg(feature = "notmuch-backend")]
    #[error(transparent)]
    NotmuchBackendError(#[from] backend::notmuch::Error),
}

pub type Result<T> = result::Result<T, Error>;

pub trait Backend: Sync + Send {
    fn name(&self) -> String;

    fn add_folder(&self, folder: &str) -> Result<()>;
    fn list_folders(&self) -> Result<Folders>;
    fn purge_folder(&self, folder: &str) -> Result<()>;
    fn delete_folder(&self, folder: &str) -> Result<()>;

    fn get_envelope(&self, folder: &str, id: &str) -> Result<Envelope>;
    fn get_envelope_internal(&self, folder: &str, internal_id: &str) -> Result<Envelope> {
        self.get_envelope(folder, internal_id)
    }

    fn list_envelopes(&self, folder: &str, page_size: usize, page: usize) -> Result<Envelopes>;
    fn search_envelopes(
        &self,
        folder: &str,
        query: &str,
        sort: &str,
        page_size: usize,
        page: usize,
    ) -> Result<Envelopes>;

    fn add_email(&self, folder: &str, email: &[u8], flags: &Flags) -> Result<String>;
    fn add_email_internal(&self, folder: &str, email: &[u8], flags: &Flags) -> Result<String> {
        self.add_email(folder, email, flags)
    }

    fn preview_emails(&self, folder: &str, ids: Vec<&str>) -> Result<Emails>;
    fn preview_emails_internal(&self, folder: &str, internal_ids: Vec<&str>) -> Result<Emails> {
        self.preview_emails(folder, internal_ids)
    }

    fn get_emails(&self, folder: &str, ids: Vec<&str>) -> Result<Emails>;
    fn get_emails_internal(&self, folder: &str, internal_ids: Vec<&str>) -> Result<Emails> {
        self.get_emails(folder, internal_ids)
    }

    fn copy_emails(&self, from_folder: &str, to_folder: &str, ids: Vec<&str>) -> Result<()>;
    fn copy_emails_internal(
        &self,
        from_folder: &str,
        to_folder: &str,
        internal_ids: Vec<&str>,
    ) -> Result<()> {
        self.copy_emails(from_folder, to_folder, internal_ids)
    }

    fn move_emails(&self, from_folder: &str, to_folder: &str, ids: Vec<&str>) -> Result<()>;
    fn move_emails_internal(
        &self,
        from_folder: &str,
        to_folder: &str,
        internal_ids: Vec<&str>,
    ) -> Result<()> {
        self.move_emails(from_folder, to_folder, internal_ids)
    }

    fn delete_emails(&self, folder: &str, ids: Vec<&str>) -> Result<()>;
    fn delete_emails_internal(&self, folder: &str, internal_ids: Vec<&str>) -> Result<()> {
        self.delete_emails(folder, internal_ids)
    }

    fn add_flags(&self, folder: &str, ids: Vec<&str>, flags: &Flags) -> Result<()>;
    fn add_flags_internal(
        &self,
        folder: &str,
        internal_ids: Vec<&str>,
        flags: &Flags,
    ) -> Result<()> {
        self.add_flags(folder, internal_ids, flags)
    }

    fn set_flags(&self, folder: &str, ids: Vec<&str>, flags: &Flags) -> Result<()>;
    fn set_flags_internal(
        &self,
        folder: &str,
        internal_ids: Vec<&str>,
        flags: &Flags,
    ) -> Result<()> {
        self.set_flags(folder, internal_ids, flags)
    }

    fn remove_flags(&self, folder: &str, ids: Vec<&str>, flags: &Flags) -> Result<()>;
    fn remove_flags_internal(
        &self,
        folder: &str,
        internal_ids: Vec<&str>,
        flags: &Flags,
    ) -> Result<()> {
        self.remove_flags(folder, internal_ids, flags)
    }

    fn close(&self) -> Result<()> {
        Ok(())
    }

    // INFO: for downcasting purpose
    fn as_any(&'static self) -> &(dyn Any);
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackendSyncProgressEvent {
    GetLocalCachedFolders,
    GetLocalFolders,
    GetRemoteCachedFolders,
    GetRemoteFolders,
    BuildFoldersPatch,
    ProcessFoldersPatch(usize),
    ProcessFolderHunk(String),

    StartEnvelopesSync(String, usize, usize),
    GetLocalCachedEnvelopes,
    GetLocalEnvelopes,
    GetRemoteCachedEnvelopes,
    GetRemoteEnvelopes,
    BuildEnvelopesPatch,
    ProcessEnvelopesPatch(usize),
    ProcessEnvelopeHunk(String),
}

pub struct BackendSyncBuilder<'a> {
    account_config: &'a AccountConfig,
    on_progress: Box<dyn Fn(BackendSyncProgressEvent) -> Result<()> + Sync + Send + 'a>,
    dry_run: bool,
}

impl<'a> BackendSyncBuilder<'a> {
    pub fn new(account_config: &'a AccountConfig) -> Self {
        Self {
            account_config,
            on_progress: Box::new(|_| Ok(())),
            dry_run: false,
        }
    }

    pub fn on_progress<F>(mut self, f: F) -> Self
    where
        F: Fn(BackendSyncProgressEvent) -> Result<()> + Sync + Send + 'a,
    {
        self.on_progress = Box::new(f);
        self
    }

    pub fn dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }

    pub fn sync(
        &self,
        remote: &dyn Backend,
    ) -> Result<(folder::sync::Patch, envelope::sync::Patch)> {
        let name = &self.account_config.name;
        if !self.account_config.sync {
            return Err(Error::SyncNotEnabled(name.clone()));
        }

        info!("starting synchronization");
        let progress = &self.on_progress;
        let sync_dir = self.account_config.sync_dir()?;
        let lock_path = LockPath::Tmp(format!("himalaya-sync-{}.lock", name));
        let guard =
            lock(&lock_path).map_err(|err| Error::SyncAccountLockError(err, name.to_owned()))?;

        let local = MaildirBackendBuilder::new()
            .db_path(sync_dir.join(name).join(".database.sqlite"))
            .build(
                Cow::Borrowed(self.account_config),
                Cow::Owned(MaildirConfig {
                    root_dir: sync_dir.join(name),
                }),
            )?;

        let (folders_patch, folders) = folder::SyncBuilder::new(self.account_config)
            .on_progress(|data| Ok(progress(data).map_err(Box::new)?))
            .dry_run(self.dry_run)
            .sync(&local, remote)?;

        let mut envelopes_patch: envelope::sync::Patch = vec![];
        let envelopes = envelope::SyncBuilder::new(self.account_config)
            .on_progress(|data| Ok(progress(data).map_err(Box::new)?))
            .dry_run(self.dry_run);

        for (folder_num, folder) in folders.iter().enumerate() {
            progress(BackendSyncProgressEvent::StartEnvelopesSync(
                folder.clone(),
                folder_num + 1,
                folders.len(),
            ))?;
            envelopes_patch.extend(envelopes.sync(folder, &local, remote)?);
        }

        drop(guard);

        Ok((folders_patch, envelopes_patch))
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BackendBuilder {
    sessions_pool_size: usize,
    disable_cache: bool,
}

impl<'a> BackendBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn sessions_pool_size(mut self, pool_size: usize) -> Self {
        self.sessions_pool_size = pool_size;
        self
    }

    pub fn disable_cache(mut self, disable_cache: bool) -> Self {
        self.disable_cache = disable_cache;
        self
    }

    pub fn build(
        &self,
        account_config: &'a AccountConfig,
        backend_config: &'a BackendConfig,
    ) -> Result<Box<dyn Backend + 'a>> {
        match backend_config {
            #[cfg(feature = "imap-backend")]
            BackendConfig::Imap(imap_config) if !account_config.sync || self.disable_cache => {
                Ok(Box::new(
                    ImapBackendBuilder::new()
                        .pool_size(self.sessions_pool_size)
                        .build(Cow::Borrowed(account_config), Cow::Borrowed(imap_config))?,
                ))
            }
            #[cfg(feature = "imap-backend")]
            BackendConfig::Imap(_) => Ok(Box::new(MaildirBackend::new(
                Cow::Borrowed(account_config),
                Cow::Owned(MaildirConfig {
                    root_dir: account_config.sync_dir()?.join(&account_config.name),
                }),
            )?)),
            #[cfg(feature = "maildir-backend")]
            BackendConfig::Maildir(maildir_config) => Ok(Box::new(MaildirBackend::new(
                Cow::Borrowed(account_config),
                Cow::Borrowed(maildir_config),
            )?)),
            #[cfg(feature = "notmuch-backend")]
            BackendConfig::Notmuch(notmuch_config) => Ok(Box::new(NotmuchBackend::new(
                Cow::Borrowed(account_config),
                Cow::Borrowed(notmuch_config),
            )?)),
            BackendConfig::None => Err(Error::BuildBackendError),
        }
    }
}
