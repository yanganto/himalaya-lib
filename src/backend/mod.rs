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

pub mod config;
pub use config::BackendConfig;

pub mod backend;
pub use backend::*;

pub mod id_mapper;
pub use id_mapper::*;

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
