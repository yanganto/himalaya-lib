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

use crate::{Email, Envelopes, Folders};

pub trait Backend {
    type Error;

    fn connect(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
    fn disconnect(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn folder_add(&mut self, folder: &str) -> Result<(), Self::Error>;
    fn folder_list(&mut self) -> Result<Folders, Self::Error>;
    fn folder_delete(&mut self, folder: &str) -> Result<(), Self::Error>;

    fn envelope_list(
        &mut self,
        folder: &str,
        page_size: usize,
        page: usize,
    ) -> Result<Envelopes, Self::Error>;
    fn envelope_search(
        &mut self,
        folder: &str,
        query: &str,
        sort: &str,
        page_size: usize,
        page: usize,
    ) -> Result<Envelopes, Self::Error>;

    fn email_add(&mut self, folder: &str, msg: &[u8], flags: &str) -> Result<String, Self::Error>;
    fn email_list(&mut self, folder: &str, id: &str) -> Result<Email, Self::Error>;
    fn email_copy(
        &mut self,
        folder_src: &str,
        folder_dst: &str,
        ids: &str,
    ) -> Result<(), Self::Error>;
    fn email_move(
        &mut self,
        folder_src: &str,
        folder_dst: &str,
        ids: &str,
    ) -> Result<(), Self::Error>;
    fn email_delete(&mut self, folder: &str, ids: &str) -> Result<(), Self::Error>;

    fn flags_add(&mut self, folder: &str, ids: &str, flags: &str) -> Result<(), Self::Error>;
    fn flags_set(&mut self, folder: &str, ids: &str, flags: &str) -> Result<(), Self::Error>;
    fn flags_delete(&mut self, folder: &str, ids: &str, flags: &str) -> Result<(), Self::Error>;
}
