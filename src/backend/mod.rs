mod backend;
mod config;
pub mod id_mapper;

#[cfg(feature = "imap-backend")]
pub mod imap;
#[cfg(feature = "maildir-backend")]
pub mod maildir;
#[cfg(feature = "notmuch-backend")]
pub mod notmuch;

pub use self::backend::{Backend, BackendBuilder, Error, Result};
pub use self::config::BackendConfig;
pub use self::id_mapper::IdMapper;
#[cfg(feature = "imap-backend")]
pub use self::imap::{ImapBackend, ImapBackendBuilder, ImapConfig};
#[cfg(feature = "maildir-backend")]
pub use self::maildir::{MaildirBackend, MaildirBackendBuilder, MaildirConfig};
#[cfg(feature = "notmuch-backend")]
pub use self::notmuch::{NotmuchBackend, NotmuchConfig};
