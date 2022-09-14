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

use imap;

use crate::Flag;

pub type ImapFlag<'a> = imap::types::Flag<'a>;

pub fn from_imap_flag(imap_flag: &ImapFlag<'_>) -> Flag {
    match imap_flag {
        imap::types::Flag::Seen => Flag::Seen,
        imap::types::Flag::Answered => Flag::Answered,
        imap::types::Flag::Flagged => Flag::Flagged,
        imap::types::Flag::Deleted => Flag::Deleted,
        imap::types::Flag::Draft => Flag::Draft,
        imap::types::Flag::Recent => Flag::Recent,
        imap::types::Flag::MayCreate => Flag::Custom(String::from("MayCreate")),
        imap::types::Flag::Custom(flag) => Flag::Custom(flag.to_string()),
        flag => Flag::Custom(flag.to_string()),
    }
}
