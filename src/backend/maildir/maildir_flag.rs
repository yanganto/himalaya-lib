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

use crate::msg::Flag;

pub fn from_char(c: char) -> Flag {
    match c {
        'r' | 'R' => Flag::Answered,
        's' | 'S' => Flag::Seen,
        't' | 'T' => Flag::Deleted,
        'd' | 'D' => Flag::Draft,
        'f' | 'F' => Flag::Flagged,
        'p' | 'P' => Flag::Custom(String::from("Passed")),
        flag => Flag::Custom(flag.to_string()),
    }
}

pub fn to_normalized_char(flag: &Flag) -> Option<char> {
    match flag {
        Flag::Answered => Some('R'),
        Flag::Seen => Some('S'),
        Flag::Deleted => Some('T'),
        Flag::Draft => Some('D'),
        Flag::Flagged => Some('F'),
        _ => None,
    }
}
