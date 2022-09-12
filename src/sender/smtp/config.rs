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

//! SMTP config module.
//!
//! This module contains the representation of the SMTP email sender
//! configuration of the user account.

use lettre::transport::smtp::authentication::Credentials as SmtpCredentials;
use thiserror::Error;

use crate::process::{self, ProcessError};

#[derive(Debug, Error)]
pub enum SmtpConfigError {
    #[error("cannot get smtp password")]
    GetPasswdError(#[source] ProcessError),
    #[error("cannot get smtp password: password is empty")]
    GetPasswdEmptyError,
}

/// Represents the internal sender config.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct SmtpConfig {
    /// Represents the SMTP server host.
    pub host: String,
    /// Represents the SMTP server port.
    pub port: u16,
    /// Enables StartTLS.
    pub starttls: Option<bool>,
    /// Trusts any certificate.
    pub insecure: Option<bool>,
    /// Represents the SMTP server login.
    pub login: String,
    /// Represents the SMTP password command.
    pub passwd_cmd: String,
}

impl SmtpConfig {
    /// Builds the internal SMTP sender credentials.
    pub fn credentials(&self) -> Result<SmtpCredentials, SmtpConfigError> {
        let passwd = process::run(&self.passwd_cmd).map_err(SmtpConfigError::GetPasswdError)?;
        let passwd = passwd
            .lines()
            .next()
            .ok_or_else(|| SmtpConfigError::GetPasswdEmptyError)?;
        Ok(SmtpCredentials::new(
            self.login.to_owned(),
            passwd.to_owned(),
        ))
    }

    pub fn starttls(&self) -> bool {
        self.starttls.unwrap_or_default()
    }

    pub fn insecure(&self) -> bool {
        self.insecure.unwrap_or_default()
    }
}
