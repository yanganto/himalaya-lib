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

use shellexpand;
use std::{env, path::PathBuf};
use thiserror::Error;

use crate::process::ProcessError;

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
