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

use crate::{backend, config, email, id_mapper, Email, Envelopes, Folders};

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    EmailError(#[from] email::Error),
    #[error(transparent)]
    IdMapper(#[from] id_mapper::Error),
    #[error(transparent)]
    ConfigError(#[from] config::Error),

    #[cfg(feature = "imap-backend")]
    #[error(transparent)]
    ImapBackendError(#[from] backend::imap::Error),
    #[cfg(feature = "maildir-backend")]
    #[error(transparent)]
    MaildirBackendError(#[from] backend::maildir::Error),
    #[cfg(feature = "notmuch-backend")]
    #[error(transparent)]
    NotmuchBackendError(#[from] backend::notmuch::Error),
}

pub type Result<T> = result::Result<T, Error>;

pub trait Backend {
    fn connect(&mut self) -> Result<()> {
        Ok(())
    }
    fn disconnect(&mut self) -> Result<()> {
        Ok(())
    }

    fn folder_add(&mut self, folder: &str) -> Result<()>;
    fn folder_list(&mut self) -> Result<Folders>;
    fn folder_delete(&mut self, folder: &str) -> Result<()>;

    fn envelope_list(&mut self, folder: &str, page_size: usize, page: usize) -> Result<Envelopes>;
    fn envelope_search(
        &mut self,
        folder: &str,
        query: &str,
        sort: &str,
        page_size: usize,
        page: usize,
    ) -> Result<Envelopes>;

    fn email_add(&mut self, folder: &str, msg: &[u8], flags: &str) -> Result<String>;
    fn email_get(&mut self, folder: &str, id: &str) -> Result<Email>;
    fn email_list(&mut self, folder: &str, id: &str) -> Result<Email>;
    fn email_copy(&mut self, folder_src: &str, folder_dst: &str, ids: &str) -> Result<()>;
    fn email_move(&mut self, folder_src: &str, folder_dst: &str, ids: &str) -> Result<()>;
    fn email_delete(&mut self, folder: &str, ids: &str) -> Result<()>;

    fn flags_add(&mut self, folder: &str, ids: &str, flags: &str) -> Result<()>;
    fn flags_set(&mut self, folder: &str, ids: &str, flags: &str) -> Result<()>;
    fn flags_delete(&mut self, folder: &str, ids: &str, flags: &str) -> Result<()>;
}
