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

//! Email config module.
//!
//! This module contains structures related to email configuration.

use super::SmtpConfig;

/// Represents the email sender provider.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum EmailSender {
    None,
    #[cfg(feature = "internal-sender")]
    /// Represents the internal SMTP mailer library.
    Internal(SmtpConfig),
    /// Represents the system command.
    External(EmailSendCmd),
}

impl Default for EmailSender {
    fn default() -> Self {
        Self::None
    }
}

/// Represents the external sender config.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct EmailSendCmd {
    /// Represents the send command.
    pub cmd: String,
}

/// Represents the text/plain format as defined in the [RFC2646].
///
/// [RFC2646]: https://www.ietf.org/rfc/rfc2646.txt
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum EmailTextPlainFormat {
    /// Makes the content fit its container.
    Auto,
    /// Does not restrict the content.
    Flowed,
    /// Forces the content width with a fixed amount of pixels.
    Fixed(usize),
}

impl Default for EmailTextPlainFormat {
    fn default() -> Self {
        Self::Auto
    }
}

/// Represents the email hooks. Useful for doing extra email
/// processing before or after sending it.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct EmailHooks {
    /// Represents the hook called just before sending an email.
    pub pre_send: Option<String>,
}
