pub mod global;
pub use global::*;

pub mod backend;
pub use backend::*;

#[cfg(feature = "imap-backend")]
pub mod imap;
#[cfg(feature = "imap-backend")]
pub use self::imap::*;

#[cfg(feature = "maildir-backend")]
pub mod maildir;
#[cfg(feature = "maildir-backend")]
pub use self::maildir::*;

#[cfg(feature = "notmuch-backend")]
pub mod notmuch;
#[cfg(feature = "notmuch-backend")]
pub use self::notmuch::*;

#[cfg(feature = "internal-sender")]
pub mod smtp;
#[cfg(feature = "internal-sender")]
pub use smtp::*;
