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

pub trait Backend<'a> {
    fn add_folder(&'a self, folder: &'a str) -> Result<()>;
    fn list_folder(&'a self) -> Result<Folders>;
    fn delete_folder(&'a self, folder: &'a str) -> Result<()>;

    fn list_envelope(&'a self, folder: &'a str, page_size: usize, page: usize)
        -> Result<Envelopes>;
    fn search_envelope(
        &'a self,
        folder: &'a str,
        query: &'a str,
        sort: &'a str,
        page_size: usize,
        page: usize,
    ) -> Result<Envelopes>;

    fn add_email(&'a self, folder: &'a str, email: &'a [u8], flags: &'a str) -> Result<String>;
    fn get_email(&'a self, folder: &'a str, id: &'a str) -> Result<Email<'a>>;
    fn copy_email(&'a self, folder: &'a str, folder_target: &'a str, ids: &'a str) -> Result<()>;
    fn move_email(&'a self, folder: &'a str, folder_target: &'a str, ids: &'a str) -> Result<()>;
    fn delete_email(&'a self, folder: &'a str, ids: &'a str) -> Result<()>;

    fn add_flags(&'a self, folder: &'a str, ids: &'a str, flags: &'a str) -> Result<()>;
    fn set_flags(&'a self, folder: &'a str, ids: &'a str, flags: &'a str) -> Result<()>;
    fn delete_flags(&'a self, folder: &'a str, ids: &'a str, flags: &'a str) -> Result<()>;

    // only for downcasting
    fn as_any(&self) -> &(dyn Any + 'a);
}

#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct BackendBuilder;

impl<'a> BackendBuilder {
    pub fn build(
        account_config: &'a AccountConfig,
        backend_config: &'a BackendConfig<'a>,
    ) -> Result<Box<dyn Backend<'a> + 'a>> {
        match backend_config {
            #[cfg(feature = "imap-backend")]
            BackendConfig::Imap(config) => Ok(Box::new(ImapBackend::new(config)?)),
            #[cfg(feature = "maildir-backend")]
            BackendConfig::Maildir(config) => {
                Ok(Box::new(MaildirBackend::new(account_config, config)))
            }
            #[cfg(feature = "notmuch-backend")]
            BackendConfig::Notmuch(config) => {
                Ok(Box::new(NotmuchBackend::new(account_config, config)?))
            }
            BackendConfig::None => Err(Error::BuildBackendError),
        }
    }
}
