//! Folder module.
//!
//! This module contains the representation of the email folder.

use serde::Serialize;
use std::fmt;

/// Represents the folder.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct Folder {
    /// Represents the folder hierarchie delimiter.
    pub delim: String,
    /// Represents the folder name.
    pub name: String,
    /// Represents the folder description.
    pub desc: String,
}

impl fmt::Display for Folder {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}
