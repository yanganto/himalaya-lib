use serde::Deserialize;
use std::path::PathBuf;

/// Represents the Notmuch backend config.
#[cfg(feature = "notmuch-backend")]
#[derive(Debug, Default, Clone, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct NotmuchConfig {
    /// Represents the notmuch database path.
    pub db_path: PathBuf,
}
