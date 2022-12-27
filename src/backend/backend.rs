//! Backend module.
//!
//! This module exposes the backend trait, which can be used to create
//! custom backend implementations.

use std::{any::Any, result};
use thiserror::Error;

use crate::{
    account, backend, email, id_mapper, AccountConfig, BackendConfig, Emails, Envelopes, Flags,
    Folders,
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
    fn purge_folder(&self, folder: &str) -> Result<()>;
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

    fn add_email(&self, folder: &str, email: &[u8], flags: &Flags) -> Result<String>;
    fn get_emails(&self, folder: &str, ids: Vec<&str>) -> Result<Emails>;
    fn copy_emails(&self, from_folder: &str, to_folder: &str, ids: Vec<&str>) -> Result<()>;
    fn move_emails(&self, from_folder: &str, to_folder: &str, ids: Vec<&str>) -> Result<()>;
    fn delete_emails(&self, folder: &str, ids: Vec<&str>) -> Result<()>;

    fn add_flags(&self, folder: &str, ids: Vec<&str>, flags: &Flags) -> Result<()>;
    fn set_flags(&self, folder: &str, ids: Vec<&str>, flags: &Flags) -> Result<()>;
    fn remove_flags(&self, folder: &str, ids: Vec<&str>, flags: &Flags) -> Result<()>;

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
