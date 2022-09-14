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

//! Sender module.
//!
//! This module contains the sender interface.

use std::result;

use thiserror::Error;

use crate::{config, email, smtp, Config, Email};

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    EmailError(#[from] email::Error),
    #[error(transparent)]
    ConfigError(#[from] config::Error),
    #[error(transparent)]
    SmtpError(#[from] smtp::Error),
}

pub type Result<T> = result::Result<T, Error>;

pub trait Sender {
    fn send(&mut self, config: &Config, msg: &Email) -> Result<Vec<u8>>;
}
