//! Folders module.
//!
//! This module contains the representation of the email folders.

use std::ops;

use serde::Serialize;

use crate::Folder;

/// Represents the list of folders.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct Folders(pub Vec<Folder>);

impl ops::Deref for Folders {
    type Target = Vec<Folder>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ops::DerefMut for Folders {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
