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
//! This module contains everything related to the user's
//! configuration.

use mailparse::MailAddr;
use shellexpand;
use std::{collections::HashMap, env, ffi::OsStr, fs, path::PathBuf, result};
use thiserror::Error;

use crate::{process, EmailHooks, EmailSender, EmailTextPlainFormat};

pub const DEFAULT_PAGE_SIZE: usize = 10;
pub const DEFAULT_SIGNATURE_DELIM: &str = "-- \n";

pub const DEFAULT_INBOX_FOLDER: &str = "INBOX";
pub const DEFAULT_SENT_FOLDER: &str = "Sent";
pub const DEFAULT_DRAFT_FOLDER: &str = "Drafts";

#[derive(Debug, Error)]
pub enum Error {
    #[error("cannot encrypt file using pgp")]
    EncryptFileError(#[source] process::Error),
    #[error("cannot find encrypt file command from config file")]
    EncryptFileMissingCmdError,
    #[error("cannot decrypt file using pgp")]
    DecryptFileError(#[source] process::Error),
    #[error("cannot find decrypt file command from config file")]
    DecryptFileMissingCmdError,
    #[error("cannot parse account address {0}")]
    ParseAccountAddrError(#[source] mailparse::MailParseError, String),
    #[error("cannot find account address in {0}")]
    ParseAccountAddrNotFoundError(String),
    #[error("cannot expand folder alias {1}")]
    ExpandFolderAliasError(#[source] shellexpand::LookupError<env::VarError>, String),
    #[error("cannot parse download file name from {0}")]
    ParseDownloadFileNameError(PathBuf),
    #[error("cannot find a default account")]
    FindDefaultAccountError,
    #[error("cannot find account {0}")]
    FindAccountError(String),
}

pub type Result<T> = result::Result<T, Error>;

/// Represents the user configuration.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct Config {
    /// Represents the global configuration.
    pub global: GlobalConfig,
    /// Represents the accounts configuration.
    pub accounts: AccountsConfig,
    /// Represents the optional selected account name.
    pub account_name: Option<String>,
}

impl Config {
    // Gets the account configuration matching the account name. If
    // the account name is not defined, gets the first default
    // account from the accounts map.
    // pub fn account(&self) -> Result<&AccountConfig> {
    //     match self.account_name.as_ref().map(|s| s.as_str()) {
    //         Some("default") | Some("") | None => self
    //             .accounts
    //             .iter()
    //             .find_map(|(_, account)| {
    //                 if account.is_default() {
    //                     Some(account)
    //                 } else {
    //                     None
    //                 }
    //             })
    //             .ok_or_else(|| Error::FindDefaultAccountError),
    //         Some(name) => self
    //             .accounts
    //             .get(name)
    //             .ok_or_else(|| Error::FindAccountError(name.to_owned())),
    //     }
    // }
}

/// Represents the use top level configuration.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
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
    pub email_reading_format: Option<EmailTextPlainFormat>,
    /// Represents the command used to decrypt an email.
    pub email_reading_decrypt_cmd: Option<String>,
    /// Represents the command used to encrypt an email.
    pub email_writing_encrypt_cmd: Option<String>,
    /// Represents the email hooks.
    pub email_hooks: Option<EmailHooks>,
}

/// Represents the configuration of all the user accounts.
pub type AccountsConfig = HashMap<String, AccountConfig>;

/// Represents the configuration of the user account.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct AccountConfig {
    /// Represents the email address of the user.
    pub email: String,
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
    pub folder_aliases: HashMap<String, String>,

    /// Represents the page size when listing emails.
    pub email_listing_page_size: Option<usize>,
    /// Represents the user downloads directory (mostly for
    /// attachments).
    pub email_reading_headers: Option<Vec<String>>,
    /// Represents the text/plain format as defined in the
    /// [RFC2646](https://www.ietf.org/rfc/rfc2646.txt)
    pub email_reading_format: EmailTextPlainFormat,
    /// Represents the command used to decrypt an email.
    pub email_reading_decrypt_cmd: Option<String>,
    /// Represents the command used to encrypt an email.
    pub email_writing_encrypt_cmd: Option<String>,
    /// Represents the email sender provider.
    pub email_sender: EmailSender,
    /// Represents the email hooks.
    pub email_hooks: EmailHooks,
}

impl AccountConfig {
    /// Builds the full RFC822 compliant user address email.
    pub fn address(&self) -> Result<MailAddr> {
        let display_name = self
            .display_name
            .as_ref()
            .map(ToOwned::to_owned)
            .unwrap_or_default();

        let has_special_chars = "()<>[]:;@.,".contains(|c| display_name.contains(c));

        let addr = if display_name.is_empty() {
            self.email.clone()
        } else if has_special_chars {
            format!("\"{}\" <{}>", display_name, &self.email)
        } else {
            format!("{} <{}>", display_name, &self.email)
        };

        let addr = mailparse::addrparse(&addr)
            .map_err(|err| Error::ParseAccountAddrError(err, addr.to_owned()))?
            .first()
            .ok_or_else(|| Error::ParseAccountAddrNotFoundError(addr.to_owned()))?
            .to_owned();

        Ok(addr)
    }

    pub fn pgp_encrypt_file(&self, addr: &str, path: PathBuf) -> Result<String> {
        let cmd = self
            .email_writing_encrypt_cmd
            .as_ref()
            .ok_or_else(|| Error::EncryptFileMissingCmdError)?;
        let cmd = &format!("{} {} {:?}", cmd, addr, path);
        process::run(cmd).map_err(Error::EncryptFileError)
    }

    pub fn pgp_decrypt_file(&self, path: PathBuf) -> Result<String> {
        let cmd = self
            .email_reading_decrypt_cmd
            .as_ref()
            .ok_or_else(|| Error::DecryptFileMissingCmdError)?;
        let cmd = &format!("{} {:?}", cmd, path);
        process::run(cmd).map_err(Error::DecryptFileError)
    }

    /// Gets the downloads directory path.
    pub fn downloads_dir(&self) -> PathBuf {
        self.downloads_dir
            .as_ref()
            .and_then(|dir| dir.to_str())
            .and_then(|dir| shellexpand::full(dir).ok())
            .map(|dir| PathBuf::from(dir.to_string()))
            .unwrap_or_else(env::temp_dir)
    }

    /// Gets the download path from a file name.
    pub fn get_download_file_path<S: AsRef<str>>(&self, file_name: S) -> Result<PathBuf> {
        let file_path = self.downloads_dir().join(file_name.as_ref());
        self.get_unique_download_file_path(&file_path, |path, _count| path.is_file())
    }

    /// Gets the unique download path from a file name by adding
    /// suffixes in case of name conflicts.
    pub(crate) fn get_unique_download_file_path(
        &self,
        original_file_path: &PathBuf,
        is_file: impl Fn(&PathBuf, u8) -> bool,
    ) -> Result<PathBuf> {
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
                    .ok_or_else(|| Error::ParseDownloadFileNameError(file_path.to_owned()))?,
            ));
        }

        Ok(file_path)
    }

    /// Gets the alias of the given folder if exists, otherwise
    /// returns the folder itself. Also tries to expand shell
    /// variables.
    pub fn folder_alias(&self, folder: &str) -> Result<String> {
        let folder = folder.trim().to_lowercase();
        let alias = self
            .folder_aliases
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
            .map_err(|err| Error::ExpandFolderAliasError(err, alias.to_owned()))?;
        Ok(alias)
    }

    pub fn email_listing_page_size(&self) -> usize {
        self.email_listing_page_size.unwrap_or(DEFAULT_PAGE_SIZE)
    }

    pub fn email_reading_headers(&self) -> Vec<String> {
        self.email_reading_headers
            .as_ref()
            .map(ToOwned::to_owned)
            .unwrap_or_default()
    }

    pub fn signature(&self) -> Result<Option<String>> {
        let delim = self
            .signature_delim
            .as_ref()
            .map(|s| s.as_str())
            .unwrap_or(DEFAULT_SIGNATURE_DELIM);
        let signature = self.signature.as_ref();

        Ok(signature
            .and_then(|sig| shellexpand::full(sig).ok())
            .map(String::from)
            .and_then(|sig| fs::read_to_string(sig).ok())
            .or_else(|| signature.map(ToOwned::to_owned))
            .map(|sig| format!("{}{}", delim, sig.trim_end())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_unique_download_file_path() {
        let config = AccountConfig::default();
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
