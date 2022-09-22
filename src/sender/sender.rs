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

use crate::{config, email, AccountConfig, Email, EmailSender};

#[cfg(feature = "internal-sender")]
use crate::{smtp, Smtp};

#[derive(Debug, Error)]
pub enum Error {
    #[error("cannot build email sender: external email sender not implemented yet")]
    BuildExternalEmailSenderUnimplementedError,
    #[error("cannot build email sender: sender is not defined")]
    BuildEmailSenderMissingError,

    #[error(transparent)]
    EmailError(#[from] email::Error),
    #[error(transparent)]
    ConfigError(#[from] config::Error),
    #[cfg(feature = "internal-sender")]
    #[error(transparent)]
    SmtpError(#[from] smtp::Error),
}

pub type Result<T> = result::Result<T, Error>;

pub trait Sender {
    fn send(&mut self, config: &AccountConfig, msg: &Email) -> Result<Vec<u8>>;
}

#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct SenderBuilder;

impl<'a> SenderBuilder {
    pub fn build(account_config: &'a AccountConfig) -> Result<Box<dyn Sender + 'a>> {
        match &account_config.email_sender {
            EmailSender::Internal(config) => Ok(Box::new(Smtp::new(config))),
            EmailSender::External(_cmd) => Err(Error::BuildExternalEmailSenderUnimplementedError),
            EmailSender::None => return Err(Error::BuildEmailSenderMissingError),
        }
    }
}
