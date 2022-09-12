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

//! Account config module.
//!
//! This module contains the representation of the configuration of
//! the user account(s).

use std::{collections::HashMap, path::PathBuf};

use super::{BackendConfig, EmailHooks, EmailSender, EmailTextPlainFormat};

/// Represents the configuration of all the user accounts.
pub type AccountsConfig = HashMap<String, AccountConfig>;

/// Represents the configuration of the user account.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct AccountConfig {
    /// Represents the name of the account.
    pub name: String,
    /// Represents the email address of the user.
    pub email: String,
    /// Represents the defaultness of the account.
    pub default: Option<bool>,
    /// Represents the display name of the user.
    pub display_name: Option<String>,
    /// Represents the email signature delimiter of the user.
    pub signature_delim: Option<String>,
    /// Represents the email signature of the user.
    pub signature: Option<String>,
    /// Represents the downloads directory (mostly for attachments).
    pub downloads_dir: Option<PathBuf>,

    /// Represents the page size when listing folders.
    pub folder_listing_page_size: Option<usize>,
    /// Represents the folder aliases map.
    pub folder_aliases: Option<HashMap<String, String>>,

    /// Represents the page size when listing emails.
    pub email_listing_page_size: Option<usize>,
    /// Represents the user downloads directory (mostly for
    /// attachments).
    pub email_reading_headers: Option<Vec<String>>,
    /// Represents the text/plain format as defined in the
    /// [RFC2646](https://www.ietf.org/rfc/rfc2646.txt)
    pub email_reading_format: Option<EmailTextPlainFormat>,
    /// Represents the command used to decrypt an email.
    pub email_reading_decrypt_cmd: Option<String>,
    /// Represents the command used to encrypt an email.
    pub email_writing_encrypt_cmd: Option<String>,
    /// Represents the email sender provider.
    pub email_sender: EmailSender,
    /// Represents the email hooks.
    pub email_hooks: Option<EmailHooks>,

    /// Represents the backend configuration of the account.
    pub backend: BackendConfig,
}

impl AccountConfig {
    pub fn is_default(&self) -> bool {
        self.default.unwrap_or_default()
    }
}
