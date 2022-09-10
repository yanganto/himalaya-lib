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

//! Config module.
//!
//! This module contains the representation of the user configuration.

use mailparse::MailAddr;
use serde::Deserialize;
use shellexpand;
use std::{collections::HashMap, env, ffi::OsStr, fs, path::PathBuf};
use thiserror::Error;

use crate::process::{self, ProcessError};

use super::{BackendConfig, SmtpConfig};

pub const DEFAULT_PAGE_SIZE: usize = 10;
pub const DEFAULT_SIG_DELIM: &str = "-- \n";

pub const DEFAULT_INBOX_FOLDER: &str = "INBOX";
pub const DEFAULT_SENT_FOLDER: &str = "Sent";
pub const DEFAULT_DRAFT_FOLDER: &str = "Drafts";

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("cannot encrypt file using pgp")]
    EncryptFileError(#[source] ProcessError),
    #[error("cannot find encrypt file command from config file")]
    EncryptFileMissingCmdError,
    #[error("cannot decrypt file using pgp")]
    DecryptFileError(#[source] ProcessError),
    #[error("cannot find decrypt file command from config file")]
    DecryptFileMissingCmdError,
    #[error("cannot parse account address {0}")]
    ParseAccountAddrError(#[source] mailparse::MailParseError, String),
    #[error("cannot find account address in {0}")]
    ParseAccountAddrNotFoundError(String),
    #[error("cannot expand mailbox alias {1}")]
    ExpandFolderAliasError(#[source] shellexpand::LookupError<env::VarError>, String),
    #[error("cannot parse download file name from {0}")]
    ParseDownloadFileNameError(PathBuf),
    #[error("cannot find a default account")]
    FindDefaultAccountError,
    #[error("cannot find account {0}")]
    FindAccountError(String),
}

/// Represents the user global config.
#[derive(Debug, Default, Clone, Eq, PartialEq, Deserialize)]
pub struct Config {
    pub global: GlobalConfig,
    pub account: AccountConfig,
    pub accounts: HashMap<String, AccountConfig>,
}

impl Config {
    pub fn from_config_and_opt_account_name(
        global: GlobalConfig,
        accounts: HashMap<String, AccountConfig>,
        account_name: Option<&str>,
    ) -> Result<Self, ConfigError> {
        let account = match account_name.map(|name| name.trim()) {
            Some("default") | Some("") | None => accounts
                .iter()
                .find_map(|(_, account)| {
                    if account.base.default.unwrap_or_default() {
                        Some(account)
                    } else {
                        None
                    }
                })
                .ok_or_else(|| ConfigError::FindDefaultAccountError),
            Some(name) => accounts
                .get(name)
                .ok_or_else(|| ConfigError::FindAccountError(name.to_owned())),
        }?;

        Ok(Self {
            global,
            account: account.to_owned(),
            accounts,
        })
    }

    /// Builds the full RFC822 compliant user address email.
    pub fn address(&self) -> Result<MailAddr, ConfigError> {
        let display_name = &self
            .account
            .base
            .display_name
            .as_ref()
            .or_else(|| self.global.display_name.as_ref())
            .map(ToOwned::to_owned)
            .unwrap_or_default();
        let has_special_chars = "()<>[]:;@.,".contains(|c| display_name.contains(c));
        let addr = if display_name.is_empty() {
            self.account.base.email.clone()
        } else if has_special_chars {
            format!("\"{}\" <{}>", display_name, &self.account.base.email)
        } else {
            format!("{} <{}>", display_name, &self.account.base.email)
        };
        let addr = mailparse::addrparse(&addr)
            .map_err(|err| ConfigError::ParseAccountAddrError(err, addr.to_owned()))?
            .first()
            .ok_or_else(|| ConfigError::ParseAccountAddrNotFoundError(addr.to_owned()))?
            .to_owned();

        Ok(addr)
    }

    // TODO: move this function to a better location.
    /// Encrypts a file.
    pub fn pgp_encrypt_file(&self, addr: &str, path: PathBuf) -> Result<String, ConfigError> {
        let cmd = self
            .account
            .base
            .email_writing_encrypt_cmd
            .as_ref()
            .or_else(|| self.global.email_writing_encrypt_cmd.as_ref())
            .ok_or_else(|| ConfigError::EncryptFileMissingCmdError)?;
        let cmd = &format!("{} {} {:?}", cmd, addr, path);
        process::run(cmd).map_err(ConfigError::EncryptFileError)
    }

    // TODO: move this function to a better location.
    /// Decrypts a file.
    pub fn pgp_decrypt_file(&self, path: PathBuf) -> Result<String, ConfigError> {
        let cmd = self
            .account
            .base
            .email_reading_decrypt_cmd
            .as_ref()
            .or_else(|| self.global.email_reading_decrypt_cmd.as_ref())
            .ok_or_else(|| ConfigError::DecryptFileMissingCmdError)?;
        let cmd = &format!("{} {:?}", cmd, path);
        process::run(cmd).map_err(ConfigError::DecryptFileError)
    }

    /// Gets the downloads directory path.
    pub fn downloads_dir(&self) -> PathBuf {
        self.account
            .base
            .downloads_dir
            .as_ref()
            .and_then(|dir| dir.to_str())
            .and_then(|dir| shellexpand::full(dir).ok())
            .or_else(|| {
                self.global
                    .downloads_dir
                    .as_ref()
                    .and_then(|dir| dir.to_str())
                    .and_then(|dir| shellexpand::full(dir).ok())
            })
            .map(|dir| PathBuf::from(dir.to_string()))
            .unwrap_or_else(env::temp_dir)
    }

    /// Gets the download path from a file name.
    pub fn get_download_file_path<S: AsRef<str>>(
        &self,
        file_name: S,
    ) -> Result<PathBuf, ConfigError> {
        let file_path = self.downloads_dir().join(file_name.as_ref());
        self.get_unique_download_file_path(&file_path, |path, _count| path.is_file())
    }

    /// Gets the unique download path from a file name by adding
    /// suffixes in case of name conflicts.
    pub(crate) fn get_unique_download_file_path(
        &self,
        original_file_path: &PathBuf,
        is_file: impl Fn(&PathBuf, u8) -> bool,
    ) -> Result<PathBuf, ConfigError> {
        let mut count = 0;
        let file_ext = original_file_path
            .extension()
            .and_then(OsStr::to_str)
            .map(|fext| String::from(".") + fext)
            .unwrap_or_default();
        let mut file_path = original_file_path.clone();

        while is_file(&file_path, count) {
            count += 1;
            file_path.set_file_name(OsStr::new(
                &original_file_path
                    .file_stem()
                    .and_then(OsStr::to_str)
                    .map(|fstem| format!("{}_{}{}", fstem, count, file_ext))
                    .ok_or_else(|| ConfigError::ParseDownloadFileNameError(file_path.to_owned()))?,
            ));
        }

        Ok(file_path)
    }

    /// Gets the alias of the given folder if exists, otherwise
    /// returns the folder itself. Also tries to expand shell
    /// variables.
    pub fn folder_alias(&self, folder: &str) -> Result<String, ConfigError> {
        let aliases = self.folder_aliases();
        let folder = folder.trim().to_lowercase();
        let alias =
            aliases
                .get(&folder)
                .map(|s| s.as_str())
                .unwrap_or_else(|| match folder.as_str() {
                    "inbox" => DEFAULT_INBOX_FOLDER,
                    "draft" => DEFAULT_DRAFT_FOLDER,
                    "sent" => DEFAULT_SENT_FOLDER,
                    folder => folder,
                });
        let alias = shellexpand::full(alias)
            .map(String::from)
            .map_err(|err| ConfigError::ExpandFolderAliasError(err, alias.to_owned()))?;
        Ok(alias)
    }

    pub fn folder_aliases(&self) -> HashMap<String, String> {
        let mut folder_aliases = self.global.folder_aliases.clone().unwrap_or_default();
        folder_aliases.extend(self.account.base.folder_aliases.clone().unwrap_or_default());
        folder_aliases
    }

    pub fn email_hooks(&self) -> Hooks {
        Hooks {
            pre_send: self
                .account
                .base
                .email_hooks
                .as_ref()
                .and_then(|hooks| hooks.pre_send.as_ref())
                .or_else(|| {
                    self.global
                        .email_hooks
                        .as_ref()
                        .and_then(|hooks| hooks.pre_send.as_ref())
                })
                .map(ToOwned::to_owned),
        }
    }

    pub fn email_listing_page_size(&self) -> usize {
        self.account
            .base
            .email_listing_page_size
            .or_else(|| self.global.email_listing_page_size)
            .unwrap_or(DEFAULT_PAGE_SIZE)
    }

    pub fn email_reading_format(&self) -> TextPlainFormat {
        self.account
            .base
            .email_reading_format
            .as_ref()
            .or_else(|| self.global.email_reading_format.as_ref())
            .map(ToOwned::to_owned)
            .unwrap_or_default()
    }

    pub fn email_reading_headers(&self) -> Vec<String> {
        self.account
            .base
            .email_reading_headers
            .as_ref()
            .or_else(|| self.global.email_reading_headers.as_ref())
            .map(ToOwned::to_owned)
            .unwrap_or_default()
    }

    pub fn signature(&self) -> Option<String> {
        let delim = self
            .account
            .base
            .signature_delim
            .as_ref()
            .or_else(|| self.global.signature_delim.as_ref())
            .map(|s| s.as_str())
            .unwrap_or(DEFAULT_SIG_DELIM);
        let signature = self
            .account
            .base
            .signature
            .as_ref()
            .or_else(|| self.global.signature.as_ref());
        signature
            .and_then(|sig| shellexpand::full(sig).ok())
            .map(String::from)
            .and_then(|sig| fs::read_to_string(sig).ok())
            .or_else(|| signature.map(ToOwned::to_owned))
            .map(|sig| format!("{}{}", delim, sig.trim_end()))
    }

    pub fn email_sender(&self) -> Option<&EmailSender> {
        self.global
            .email_sender
            .as_ref()
            .or_else(|| self.account.base.email_sender.as_ref())
    }
}

/// Represents the user global config.
#[derive(Debug, Default, Clone, Eq, PartialEq, Deserialize)]
pub struct GlobalConfig {
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
    pub email_reading_format: Option<TextPlainFormat>,
    /// Represents the command used to decrypt an email.
    pub email_reading_decrypt_cmd: Option<String>,
    /// Represents the command used to encrypt an email.
    pub email_writing_encrypt_cmd: Option<String>,
    /// Represents the email sender provider.
    pub email_sender: Option<EmailSender>,
    /// Represents the email hooks.
    pub email_hooks: Option<Hooks>,
}

/// Represents the base user account config.
#[derive(Debug, Default, Clone, Eq, PartialEq, Deserialize)]
pub struct BaseAccountConfig {
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
    pub email_reading_format: Option<TextPlainFormat>,
    /// Represents the command used to decrypt an email.
    pub email_reading_decrypt_cmd: Option<String>,
    /// Represents the command used to encrypt an email.
    pub email_writing_encrypt_cmd: Option<String>,
    /// Represents the email sender provider.
    pub email_sender: Option<EmailSender>,
    /// Represents the email hooks.
    pub email_hooks: Option<Hooks>,
}

/// Represents the base user account config.
#[derive(Debug, Default, Clone, Eq, PartialEq, Deserialize)]
pub struct AccountConfig {
    /// Represents the base account configuration.
    pub base: BaseAccountConfig,
    /// Represents the backend configuration of the account.
    pub backend: BackendConfig,
}

/// Represents the email sender provider.
#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
pub enum EmailSender {
    // Uses the internal mailer library.
    Internal(SmtpConfig),
    // Uses the given system command.
    Cmd(String),
}

/// Represents the text/plain format as defined in the [RFC2646].
///
/// [RFC2646]: https://www.ietf.org/rfc/rfc2646.txt
#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
pub enum TextPlainFormat {
    // Forces the content width with a fixed amount of pixels.
    Fixed(usize),
    // Makes the content fit its container.
    Auto,
    // Does not restrict the content.
    Flowed,
}

impl Default for TextPlainFormat {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Default, Clone, Eq, PartialEq, Deserialize)]
pub struct Hooks {
    pub pre_send: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_unique_download_file_path() {
        let config = Config::default();
        let path = PathBuf::from("downloads/file.ext");

        // When file path is unique
        assert!(matches!(
            config.get_unique_download_file_path(&path, |_, _| false),
            Ok(path) if path == PathBuf::from("downloads/file.ext")
        ));

        // When 1 file path already exist
        assert!(matches!(
            config.get_unique_download_file_path(&path, |_, count| count <  1),
            Ok(path) if path == PathBuf::from("downloads/file_1.ext")
        ));

        // When 5 file paths already exist
        assert!(matches!(
            config.get_unique_download_file_path(&path, |_, count| count < 5),
            Ok(path) if path == PathBuf::from("downloads/file_5.ext")
        ));

        // When file path has no extension
        let path = PathBuf::from("downloads/file");
        assert!(matches!(
            config.get_unique_download_file_path(&path, |_, count| count < 5),
            Ok(path) if path == PathBuf::from("downloads/file_5")
        ));

        // When file path has 2 extensions
        let path = PathBuf::from("downloads/file.ext.ext2");
        assert!(matches!(
            config.get_unique_download_file_path(&path, |_, count| count < 5),
            Ok(path) if path == PathBuf::from("downloads/file.ext_5.ext2")
        ));
    }
}
