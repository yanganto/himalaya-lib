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

//! Backend module.
//!
//! This module exposes the backend trait, which can be used to create
//! custom backend implementations.

use std::result;
use thiserror::Error;

use crate::{
    account,
    config::{ConfigError, ImapConfigError},
    mbox::Mboxes,
    msg::{self, Envelopes, Msg},
};

use super::id_mapper;

#[cfg(feature = "maildir-backend")]
use super::MaildirError;

#[cfg(feature = "notmuch-backend")]
use super::NotmuchError;

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    ImapError(#[from] super::imap::Error),

    #[error(transparent)]
    AccountError(#[from] account::AccountError),
    #[error(transparent)]
    ConfigError(#[from] ConfigError),
    #[cfg(feature = "imap-backend")]
    #[error(transparent)]
    ImapConfigError(#[from] ImapConfigError),

    #[error(transparent)]
    MsgError(#[from] msg::Error),

    #[error(transparent)]
    IdMapperError(#[from] id_mapper::Error),

    #[cfg(feature = "maildir-backend")]
    #[error(transparent)]
    MaildirError(#[from] MaildirError),

    #[cfg(feature = "notmuch-backend")]
    #[error(transparent)]
    NotmuchError(#[from] NotmuchError),
}

pub type Result<T> = result::Result<T, Error>;

pub trait Backend<'a> {
    fn connect(&mut self) -> Result<()> {
        Ok(())
    }

    fn add_mbox(&mut self, mbox: &str) -> Result<()>;
    fn get_mboxes(&mut self) -> Result<Mboxes>;
    fn del_mbox(&mut self, mbox: &str) -> Result<()>;
    fn get_envelopes(&mut self, mbox: &str, page_size: usize, page: usize) -> Result<Envelopes>;
    fn search_envelopes(
        &mut self,
        mbox: &str,
        query: &str,
        sort: &str,
        page_size: usize,
        page: usize,
    ) -> Result<Envelopes>;
    fn add_msg(&mut self, mbox: &str, msg: &[u8], flags: &str) -> Result<String>;
    fn get_msg(&mut self, mbox: &str, id: &str) -> Result<Msg>;
    fn copy_msg(&mut self, mbox_src: &str, mbox_dst: &str, ids: &str) -> Result<()>;
    fn move_msg(&mut self, mbox_src: &str, mbox_dst: &str, ids: &str) -> Result<()>;
    fn del_msg(&mut self, mbox: &str, ids: &str) -> Result<()>;
    fn add_flags(&mut self, mbox: &str, ids: &str, flags: &str) -> Result<()>;
    fn set_flags(&mut self, mbox: &str, ids: &str, flags: &str) -> Result<()>;
    fn del_flags(&mut self, mbox: &str, ids: &str, flags: &str) -> Result<()>;

    fn disconnect(&mut self) -> Result<()> {
        Ok(())
    }
}
