pub mod backend;
pub mod config;
pub mod id_mapper;
#[cfg(feature = "imap-backend")]
pub mod imap;
#[cfg(feature = "maildir-backend")]
pub mod maildir;
#[cfg(feature = "notmuch-backend")]
pub mod notmuch;

pub use self::backend::*;
pub use self::config::BackendConfig;
pub use self::id_mapper::*;
#[cfg(feature = "imap-backend")]
pub use self::imap::{ImapBackend, ImapConfig};
#[cfg(feature = "maildir-backend")]
pub use self::maildir::{MaildirBackend, MaildirConfig};
#[cfg(feature = "notmuch-backend")]
pub use self::notmuch::{NotmuchBackend, NotmuchConfig};
