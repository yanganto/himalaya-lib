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
//! This module contains the representation of the user account.

use lettre::transport::smtp::authentication::Credentials as SmtpCredentials;
use mailparse::MailAddr;
use serde::Deserialize;
use shellexpand;
use std::{collections::HashMap, env, ffi::OsStr, path::PathBuf};
use thiserror::Error;

use crate::process::{self, ProcessError};

pub const DEFAULT_PAGE_SIZE: usize = 10;
pub const DEFAULT_SIG_DELIM: &str = "-- \n";

pub const DEFAULT_INBOX_FOLDER: &str = "INBOX";
pub const DEFAULT_SENT_FOLDER: &str = "Sent";
pub const DEFAULT_DRAFT_FOLDER: &str = "Drafts";

#[derive(Debug, Error)]
pub enum AccountError {
    #[error("cannot encrypt file using pgp")]
    EncryptFileError(#[source] ProcessError),
    #[error("cannot find encrypt file command from config file")]
    EncryptFileMissingCmdError,

    #[error("cannot decrypt file using pgp")]
    DecryptFileError(#[source] ProcessError),
    #[error("cannot find decrypt file command from config file")]
    DecryptFileMissingCmdError,

    #[error("cannot get smtp password")]
    GetSmtpPasswdError(#[source] ProcessError),
    #[error("cannot get smtp password: password is empty")]
    GetSmtpPasswdEmptyError,

    #[cfg(feature = "imap-backend")]
    #[error("cannot get imap password")]
    GetImapPasswdError(#[source] ProcessError),
    #[cfg(feature = "imap-backend")]
    #[error("cannot get imap password: password is empty")]
    GetImapPasswdEmptyError,

    #[error("cannot find default account")]
    FindDefaultAccountError,
    #[error("cannot find account {0}")]
    FindAccountError(String),
    #[error("cannot parse account address {0}")]
    ParseAccountAddrError(#[source] mailparse::MailParseError, String),
    #[error("cannot find account address in {0}")]
    ParseAccountAddrNotFoundError(String),

    #[cfg(feature = "maildir-backend")]
    #[error("cannot expand maildir path")]
    ExpandMaildirPathError(#[source] shellexpand::LookupError<env::VarError>),
    #[cfg(feature = "notmuch-backend")]
    #[error("cannot expand notmuch path")]
    ExpandNotmuchDatabasePathError(#[source] shellexpand::LookupError<env::VarError>),
    #[error("cannot expand mailbox alias {1}")]
    ExpandMboxAliasError(#[source] shellexpand::LookupError<env::VarError>, String),

    #[error("cannot parse download file name from {0}")]
    ParseDownloadFileNameError(PathBuf),

    #[error("cannot start the notify mode")]
    StartNotifyModeError(#[source] ProcessError),
}

/// Represents the user account.
#[derive(Debug, Default, Clone)]
pub struct Account {
    /// Represents the name of the user account.
    pub name: String,
    /// Makes this account the default one.
    pub default: bool,
    /// Represents the display name of the user account.
    pub display_name: String,
    /// Represents the email address of the user account.
    pub email: String,
    /// Represents the downloads directory (mostly for attachments).
    pub downloads_dir: PathBuf,
    /// Represents the signature of the user.
    pub sig: Option<String>,
    /// Represents the default page size for listings.
    pub default_page_size: usize,
    /// Represents the notify command.
    pub notify_cmd: Option<String>,
    /// Overrides the default IMAP query "NEW" used to fetch new messages
    pub notify_query: String,
    /// Represents the watch commands.
    pub watch_cmds: Vec<String>,
    /// Represents the text/plain format as defined in the
    /// [RFC2646](https://www.ietf.org/rfc/rfc2646.txt)
    pub format: TextPlainFormat,
    /// Overrides the default headers displayed at the top of
    /// the read message.
    pub read_headers: Vec<String>,

    /// Represents mailbox aliases.
    pub mailboxes: HashMap<String, String>,

    /// Represents hooks.
    pub hooks: Hooks,

    /// Represents the SMTP host.
    pub smtp_host: String,
    /// Represents the SMTP port.
    pub smtp_port: u16,
    /// Enables StartTLS.
    pub smtp_starttls: bool,
    /// Trusts any certificate.
    pub smtp_insecure: bool,
    /// Represents the SMTP login.
    pub smtp_login: String,
    /// Represents the SMTP password command.
    pub smtp_passwd_cmd: String,

    /// Represents the command used to encrypt a message.
    pub pgp_encrypt_cmd: Option<String>,
    /// Represents the command used to decrypt a message.
    pub pgp_decrypt_cmd: Option<String>,
}

impl<'a> Account {
    /// Builds the full RFC822 compliant address of the user account.
    pub fn address(&self) -> Result<MailAddr, AccountError> {
        let has_special_chars = "()<>[]:;@.,".contains(|c| self.display_name.contains(c));
        let addr = if self.display_name.is_empty() {
            self.email.clone()
        } else if has_special_chars {
            // Wraps the name with double quotes if it contains any special character.
            format!("\"{}\" <{}>", self.display_name, self.email)
        } else {
            format!("{} <{}>", self.display_name, self.email)
        };

        Ok(mailparse::addrparse(&addr)
            .map_err(|err| AccountError::ParseAccountAddrError(err, addr.to_owned()))?
            .first()
            .ok_or_else(|| AccountError::ParseAccountAddrNotFoundError(addr.to_owned()))?
            .clone())
    }

    /// Builds the user account SMTP credentials.
    pub fn smtp_creds(&self) -> Result<SmtpCredentials, AccountError> {
        let passwd =
            process::run(&self.smtp_passwd_cmd).map_err(AccountError::GetSmtpPasswdError)?;
        let passwd = passwd
            .lines()
            .next()
            .ok_or_else(|| AccountError::GetSmtpPasswdEmptyError)?;

        Ok(SmtpCredentials::new(
            self.smtp_login.to_owned(),
            passwd.to_owned(),
        ))
    }

    /// Encrypts a file.
    pub fn pgp_encrypt_file(&self, addr: &str, path: PathBuf) -> Result<String, AccountError> {
        if let Some(cmd) = self.pgp_encrypt_cmd.as_ref() {
            let encrypt_file_cmd = format!("{} {} {:?}", cmd, addr, path);
            Ok(process::run(&encrypt_file_cmd).map_err(AccountError::EncryptFileError)?)
        } else {
            Err(AccountError::EncryptFileMissingCmdError)
        }
    }

    /// Decrypts a file.
    pub fn pgp_decrypt_file(&self, path: PathBuf) -> Result<String, AccountError> {
        if let Some(cmd) = self.pgp_decrypt_cmd.as_ref() {
            let decrypt_file_cmd = format!("{} {:?}", cmd, path);
            Ok(process::run(&decrypt_file_cmd).map_err(AccountError::DecryptFileError)?)
        } else {
            Err(AccountError::DecryptFileMissingCmdError)
        }
    }

    /// Gets the download path from a file name.
    pub fn get_download_file_path<S: AsRef<str>>(
        &self,
        file_name: S,
    ) -> Result<PathBuf, AccountError> {
        let file_path = self.downloads_dir.join(file_name.as_ref());
        self.get_unique_download_file_path(&file_path, |path, _count| path.is_file())
    }

    /// Gets the unique download path from a file name by adding
    /// suffixes in case of name conflicts.
    pub fn get_unique_download_file_path(
        &self,
        original_file_path: &PathBuf,
        is_file: impl Fn(&PathBuf, u8) -> bool,
    ) -> Result<PathBuf, AccountError> {
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
                    .ok_or_else(|| {
                        AccountError::ParseDownloadFileNameError(file_path.to_owned())
                    })?,
            ));
        }

        Ok(file_path)
    }

    /// Runs the notify command.
    pub fn run_notify_cmd<S: AsRef<str>>(&self, subject: S, sender: S) -> Result<(), AccountError> {
        let subject = subject.as_ref();
        let sender = sender.as_ref();

        let default_cmd = format!(r#"notify-send "New message from {}" "{}""#, sender, subject);
        let cmd = self
            .notify_cmd
            .as_ref()
            .map(|cmd| format!(r#"{} {:?} {:?}"#, cmd, subject, sender))
            .unwrap_or(default_cmd);

        process::run(&cmd).map_err(AccountError::StartNotifyModeError)?;
        Ok(())
    }

    /// Gets the mailbox alias if exists, otherwise returns the
    /// mailbox. Also tries to expand shell variables.
    pub fn get_mbox_alias(&self, mbox: &str) -> Result<String, AccountError> {
        let mbox = self
            .mailboxes
            .get(&mbox.trim().to_lowercase())
            .map(|s| s.as_str())
            .unwrap_or(mbox);
        let mbox = shellexpand::full(mbox)
            .map(String::from)
            .map_err(|err| AccountError::ExpandMboxAliasError(err, mbox.to_owned()))?;
        Ok(mbox)
    }
}

/// Represents all existing kind of account (backend).
#[derive(Debug, Clone)]
pub enum BackendConfig {
    #[cfg(feature = "imap-backend")]
    Imap(ImapBackendConfig),
    #[cfg(feature = "maildir-backend")]
    Maildir(MaildirBackendConfig),
    #[cfg(feature = "notmuch-backend")]
    Notmuch(NotmuchBackendConfig),
}

/// Represents the IMAP backend.
#[cfg(feature = "imap-backend")]
#[derive(Debug, Default, Clone)]
pub struct ImapBackendConfig {
    /// Represents the IMAP host.
    pub imap_host: String,
    /// Represents the IMAP port.
    pub imap_port: u16,
    /// Enables StartTLS.
    pub imap_starttls: bool,
    /// Trusts any certificate.
    pub imap_insecure: bool,
    /// Represents the IMAP login.
    pub imap_login: String,
    /// Represents the IMAP password command.
    pub imap_passwd_cmd: String,
}

#[cfg(feature = "imap-backend")]
impl ImapBackendConfig {
    /// Gets the IMAP password of the user account.
    pub fn imap_passwd(&self) -> Result<String, AccountError> {
        let passwd =
            process::run(&self.imap_passwd_cmd).map_err(AccountError::GetImapPasswdError)?;
        let passwd = passwd
            .lines()
            .next()
            .ok_or_else(|| AccountError::GetImapPasswdEmptyError)?;
        Ok(passwd.to_string())
    }
}

/// Represents the Maildir backend.
#[cfg(feature = "maildir-backend")]
#[derive(Debug, Default, Clone)]
pub struct MaildirBackendConfig {
    /// Represents the Maildir directory path.
    pub maildir_dir: PathBuf,
}

/// Represents the Notmuch backend.
#[cfg(feature = "notmuch-backend")]
#[derive(Debug, Default, Clone)]
pub struct NotmuchBackendConfig {
    /// Represents the Notmuch database path.
    pub notmuch_database_dir: PathBuf,
}

/// Represents the text/plain format as defined in the [RFC2646].
///
/// [RFC2646]: https://www.ietf.org/rfc/rfc2646.txt
#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
#[serde(tag = "type", content = "width", rename_all = "lowercase")]
pub enum TextPlainFormat {
    // Forces the content width with a fixed amount of pixels.
    Fixed(usize),
    // Makes the content fit the terminal.
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
#[serde(rename_all = "kebab-case")]
pub struct Hooks {
    pub pre_send: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_should_get_unique_download_file_path() {
        let account = Account::default();
        let path = PathBuf::from("downloads/file.ext");

        // When file path is unique
        assert!(matches!(
            account.get_unique_download_file_path(&path, |_, _| false),
            Ok(path) if path == PathBuf::from("downloads/file.ext")
        ));

        // When 1 file path already exist
        assert!(matches!(
            account.get_unique_download_file_path(&path, |_, count| count <  1),
            Ok(path) if path == PathBuf::from("downloads/file_1.ext")
        ));

        // When 5 file paths already exist
        assert!(matches!(
            account.get_unique_download_file_path(&path, |_, count| count < 5),
            Ok(path) if path == PathBuf::from("downloads/file_5.ext")
        ));

        // When file path has no extension
        let path = PathBuf::from("downloads/file");
        assert!(matches!(
            account.get_unique_download_file_path(&path, |_, count| count < 5),
            Ok(path) if path == PathBuf::from("downloads/file_5")
        ));

        // When file path has 2 extensions
        let path = PathBuf::from("downloads/file.ext.ext2");
        assert!(matches!(
            account.get_unique_download_file_path(&path, |_, count| count < 5),
            Ok(path) if path == PathBuf::from("downloads/file.ext_5.ext2")
        ));
    }
}
