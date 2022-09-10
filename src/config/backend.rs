use serde::Deserialize;

use super::{ImapConfig, MaildirConfig, NotmuchConfig};

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
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
