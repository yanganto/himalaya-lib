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

use crate::{
    backend::from_imap_flag,
    msg::{Flag, Flags},
};

pub fn into_imap_flags<'a>(flags: &'a Flags) -> Vec<imap::types::Flag<'a>> {
    flags
        .iter()
        .map(|flag| match flag {
            Flag::Seen => imap::types::Flag::Seen,
            Flag::Answered => imap::types::Flag::Answered,
            Flag::Flagged => imap::types::Flag::Flagged,
            Flag::Deleted => imap::types::Flag::Deleted,
            Flag::Draft => imap::types::Flag::Draft,
            Flag::Recent => imap::types::Flag::Recent,
            Flag::Custom(flag) => imap::types::Flag::Custom(flag.into()),
        })
        .collect()
}

pub fn from_imap_flags(imap_flags: &[imap::types::Flag<'_>]) -> Flags {
    imap_flags.iter().map(from_imap_flag).collect()
}
