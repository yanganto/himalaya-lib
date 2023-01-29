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

    #[error(transparent)]
    EmailError(#[from] email::Error),
    #[error(transparent)]
    IdMapper(#[from] id_mapper::Error),
    #[error(transparent)]
    ConfigError(#[from] account::config::Error),
    #[error(transparent)]
    SyncFoldersError(#[from] Box<folder::sync::Error>),
    #[error(transparent)]
    SyncEnvelopesError(#[from] Box<envelope::sync::Error>),

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
            .db_path(sync_dir.join(&account.name).join(".database.sqlite"))
            .build(
                Cow::Borrowed(account),
                Cow::Owned(MaildirConfig {
                    root_dir: sync_dir.join(&account.name),
                }),
            )?;

        let cache = folder::sync::Cache::new(Cow::Borrowed(account), &sync_dir);
        let folders = folder::sync_all(&cache, &local, self, dry_run).map_err(Box::new)?;

        let cache = envelope::sync::Cache::new(Cow::Borrowed(account), &sync_dir);
        for folder in &folders {
            envelope::sync_all(folder, &cache, &local, self, dry_run).map_err(Box::new)?;
        }

        drop(guard);

        Ok(())
    }

    // INFO: for downcasting purpose
    fn as_any(&'static self) -> &(dyn Any);
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
