//! Backend module.
//!
//! This module exposes the backend trait, which can be used to create
//! custom backend implementations.

use std::{any::Any, result};
use thiserror::Error;

use crate::{
    account, backend, email, id_mapper, AccountConfig, BackendConfig, Email, Envelopes, Folders,
};

#[cfg(feature = "imap-backend")]
use crate::ImapBackend;

#[cfg(feature = "maildir-backend")]
use crate::MaildirBackend;

#[cfg(feature = "notmuch-backend")]
use crate::NotmuchBackend;

#[derive(Debug, Error)]
pub enum Error {
    #[error("cannot build backend with an empty config")]
    BuildBackendError,

    #[error(transparent)]
    EmailError(#[from] email::Error),
    #[error(transparent)]
    IdMapper(#[from] id_mapper::Error),
    #[error(transparent)]
    ConfigError(#[from] account::config::Error),

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

pub trait Backend {
    fn add_folder(&self, folder: &str) -> Result<()>;
    fn list_folder(&self) -> Result<Folders>;
    fn delete_folder(&self, folder: &str) -> Result<()>;

    fn list_envelope(&self, folder: &str, page_size: usize, page: usize) -> Result<Envelopes>;
    fn search_envelope(
        &self,
        folder: &str,
        query: &str,
        sort: &str,
        page_size: usize,
        page: usize,
    ) -> Result<Envelopes>;

    fn add_email(&self, folder: &str, email: &[u8], flags: &str) -> Result<String>;
    fn get_email(&self, folder: &str, id: &str) -> Result<Email<'_>>;
    fn copy_email(&self, folder: &str, folder_target: &str, ids: &str) -> Result<()>;
    fn move_email(&self, folder: &str, folder_target: &str, ids: &str) -> Result<()>;
    fn delete_email(&self, folder: &str, ids: &str) -> Result<()>;

    fn add_flags(&self, folder: &str, ids: &str, flags: &str) -> Result<()>;
    fn set_flags(&self, folder: &str, ids: &str, flags: &str) -> Result<()>;
    fn remove_flags(&self, folder: &str, ids: &str, flags: &str) -> Result<()>;

    // only for downcasting
    fn as_any(&'static self) -> &(dyn Any);
}

#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct BackendBuilder;

impl<'a> BackendBuilder {
    pub fn build(
        account_config: &'a AccountConfig,
        backend_config: &'a BackendConfig<'a>,
    ) -> Result<Box<dyn Backend + 'a>> {
        match backend_config {
            #[cfg(feature = "imap-backend")]
            BackendConfig::Imap(imap_config) => Ok(Box::new(ImapBackend::new(imap_config)?)),
            #[cfg(feature = "maildir-backend")]
            BackendConfig::Maildir(maildir_config) => Ok(Box::new(MaildirBackend::new(
                account_config,
                maildir_config,
            ))),
            #[cfg(feature = "notmuch-backend")]
            BackendConfig::Notmuch(config) => {
                Ok(Box::new(NotmuchBackend::new(account_config, config)?))
            }
            BackendConfig::None => Err(Error::BuildBackendError),
        }
    }
}
