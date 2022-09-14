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

use crate::{Flag, Flags};

use super::flag;

pub fn into_imap_flags<'a>(flags: &'a Flags) -> Vec<flag::ImapFlag<'a>> {
    flags
        .iter()
        .map(|flag| match flag {
            Flag::Seen => flag::ImapFlag::Seen,
            Flag::Answered => flag::ImapFlag::Answered,
            Flag::Flagged => flag::ImapFlag::Flagged,
            Flag::Deleted => flag::ImapFlag::Deleted,
            Flag::Draft => flag::ImapFlag::Draft,
            Flag::Recent => flag::ImapFlag::Recent,
            Flag::Custom(flag) => flag::ImapFlag::Custom(flag.into()),
        })
        .collect()
}

pub fn from_imap_flags(imap_flags: &[flag::ImapFlag<'_>]) -> Flags {
    imap_flags.iter().map(flag::from_imap_flag).collect()
}
