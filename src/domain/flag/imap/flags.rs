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

pub fn into_raws<'a>(flags: &'a Flags) -> Vec<flag::RawFlag<'a>> {
    flags
        .iter()
        .map(|flag| match flag {
            Flag::Seen => flag::RawFlag::Seen,
            Flag::Answered => flag::RawFlag::Answered,
            Flag::Flagged => flag::RawFlag::Flagged,
            Flag::Deleted => flag::RawFlag::Deleted,
            Flag::Draft => flag::RawFlag::Draft,
            Flag::Recent => flag::RawFlag::Recent,
            Flag::Custom(flag) => flag::RawFlag::Custom(flag.into()),
        })
        .collect()
}

pub fn from_raws(imap_flags: &[flag::RawFlag<'_>]) -> Flags {
    imap_flags.iter().map(flag::from_raw).collect()
}
