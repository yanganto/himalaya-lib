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

use std::{
    env, io,
    path::{self, PathBuf},
    result,
};
use thiserror::Error;

use crate::account;

#[derive(Error, Debug)]
pub enum Error {
    #[error("cannot expand attachment path {1}")]
    ExpandAttachmentPathError(#[source] shellexpand::LookupError<env::VarError>, String),
    #[error("cannot read attachment at {1}")]
    ReadAttachmentError(#[source] io::Error, PathBuf),
    #[error("cannot parse template")]
    ParseTplError(#[source] mailparse::MailParseError),
    #[error("cannot parse content type of attachment {1}")]
    ParseAttachmentContentTypeError(#[source] lettre::message::header::ContentTypeErr, String),
    #[error("cannot write temporary multipart on the disk")]
    WriteTmpMultipartError(#[source] io::Error),
    #[error("cannot write temporary multipart on the disk")]
    BuildSendableMsgError(#[source] lettre::error::Error),
    #[error("cannot parse {1} value: {2}")]
    ParseHeaderError(#[source] mailparse::MailParseError, String, String),
    #[error("cannot build envelope")]
    BuildEnvelopeError(#[source] lettre::error::Error),
    #[error("cannot get file name of attachment {0}")]
    GetAttachmentFilenameError(PathBuf),
    #[error("cannot parse recipient")]
    ParseRecipientError,

    #[error("cannot parse message or address")]
    ParseAddressError(#[from] lettre::address::AddressError),

    #[error(transparent)]
    AccountError(#[from] account::AccountError),

    #[error("cannot get content type of multipart")]
    GetMultipartContentTypeError,
    #[error("cannot find encrypted part of multipart")]
    GetEncryptedPartMultipartError,
    #[error("cannot parse encrypted part of multipart")]
    ParseEncryptedPartError(#[source] mailparse::MailParseError),
    #[error("cannot get body from encrypted part")]
    GetEncryptedPartBodyError(#[source] mailparse::MailParseError),
    #[error("cannot write encrypted part to temporary file")]
    WriteEncryptedPartBodyError(#[source] io::Error),
    #[error("cannot write encrypted part to temporary file")]
    DecryptPartError(#[source] account::AccountError),

    #[error("cannot delete local draft: {1}")]
    DeleteLocalDraftError(#[source] io::Error, path::PathBuf),
}

pub type Result<T> = result::Result<T, Error>;
