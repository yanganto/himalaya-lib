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

use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum NotmuchError {
    #[error("cannot parse notmuch message header {1}")]
    ParseMsgHeaderError(#[source] notmuch::Error, String),
    #[error("cannot parse notmuch message date {1}")]
    ParseMsgDateError(#[source] chrono::ParseError, String),
    #[error("cannot find notmuch message header {0}")]
    FindMsgHeaderError(String),
    #[error("cannot find notmuch message sender")]
    FindSenderError,
    #[error("cannot parse notmuch message senders {1}")]
    ParseSendersError(#[source] mailparse::MailParseError, String),
    #[error("cannot open notmuch database")]
    OpenDbError(#[source] notmuch::Error),
    #[error("cannot build notmuch query")]
    BuildQueryError(#[source] notmuch::Error),
    #[error("cannot search notmuch envelopes")]
    SearchEnvelopesError(#[source] notmuch::Error),
    #[error("cannot get notmuch envelopes at page {0}")]
    GetEnvelopesOutOfBoundsError(usize),
    #[error("cannot add notmuch mailbox: feature not implemented")]
    AddMboxUnimplementedError,
    #[error("cannot delete notmuch mailbox: feature not implemented")]
    DelMboxUnimplementedError,
    #[error("cannot copy notmuch message: feature not implemented")]
    CopyMsgUnimplementedError,
    #[error("cannot move notmuch message: feature not implemented")]
    MoveMsgUnimplementedError,
    #[error("cannot index notmuch message")]
    IndexFileError(#[source] notmuch::Error),
    #[error("cannot find notmuch message")]
    FindMsgError(#[source] notmuch::Error),
    #[error("cannot find notmuch message")]
    FindMsgEmptyError,
    #[error("cannot read notmuch raw message from file")]
    ReadMsgError(#[source] io::Error),
    #[error("cannot parse notmuch raw message")]
    ParseMsgError(#[source] mailparse::MailParseError),
    #[error("cannot delete notmuch message")]
    DelMsgError(#[source] notmuch::Error),
    #[error("cannot add notmuch tag")]
    AddTagError(#[source] notmuch::Error),
    #[error("cannot delete notmuch tag")]
    DelTagError(#[source] notmuch::Error),
}
