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

use serde::Serialize;

/// Represents the flag variants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Flag {
    Seen,
    Answered,
    Flagged,
    Deleted,
    Draft,
    Recent,
    Custom(String),
}

impl From<&str> for Flag {
    fn from(flag_str: &str) -> Self {
        match flag_str {
            "seen" => Flag::Seen,
            "answered" | "replied" => Flag::Answered,
            "flagged" => Flag::Flagged,
            "deleted" | "trashed" => Flag::Deleted,
            "draft" => Flag::Draft,
            "recent" => Flag::Recent,
            flag => Flag::Custom(flag.into()),
        }
    }
}
