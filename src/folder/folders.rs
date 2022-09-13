// himalaya-lib, a Rust library for email management.
// Copyright (C) 2022  soywod <clement.douin@posteo.net>
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Folders module.
//!
//! This module contains the representation of the email folders.

use std::ops;

use super::Folder;

/// Represents the list of folders.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Folders {
    pub folders: Vec<Folder>,
}

impl ops::Deref for Folders {
    type Target = Vec<Folder>;

    fn deref(&self) -> &Self::Target {
        &self.folders
    }
}

impl ops::DerefMut for Folders {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.folders
    }
}
