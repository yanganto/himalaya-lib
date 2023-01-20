//! Backend config module.
//!
//! This module contains the representation of the backend
//! configuration of the user account.

#[cfg(feature = "imap-backend")]
use crate::ImapConfig;

#[cfg(feature = "maildir-backend")]
use crate::MaildirConfig;

#[cfg(feature = "notmuch-backend")]
use crate::NotmuchConfig;

/// Represents the backend configuration of the user account.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum BackendConfig {
    None,
    #[cfg(feature = "imap-backend")]
    Imap(ImapConfig),
    #[cfg(feature = "maildir-backend")]
    Maildir(MaildirConfig),
    #[cfg(feature = "notmuch-backend")]
    Notmuch(NotmuchConfig),
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self::None
    }
}
