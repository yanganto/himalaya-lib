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

//! Message module.
//!
//! This module contains everything related to messages.

pub mod config;
pub use config::{EmailHooks, EmailSendCmd, EmailSender, EmailTextPlainFormat};

mod parts;
pub use parts::*;

mod addr;
pub use addr::*;

mod tpl;
pub use tpl::*;

mod email;
pub use email::*;

mod utils;
pub use utils::*;
