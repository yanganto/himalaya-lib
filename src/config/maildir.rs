use serde::Deserialize;
use std::path::PathBuf;

/// Represents the Maildir backend config.
#[cfg(feature = "maildir-backend")]
#[derive(Debug, Default, Clone, Eq, PartialEq, Deserialize)]
pub struct MaildirConfig {
    /// Represents the Maildir root directory.
    pub root_dir: PathBuf,
}
