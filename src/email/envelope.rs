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

use super::Flags;

/// Represents the message envelope. The envelope is just a message
/// subset, and is mostly used for listings.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Envelope {
    /// Represents the message identifier.
    pub id: String,
    /// Represents the internal message identifier.
    pub internal_id: String,
    /// Represents the message flags.
    pub flags: Flags,
    /// Represents the subject of the message.
    pub subject: String,
    /// Represents the first sender of the message.
    pub sender: String,
    /// Represents the internal date of the message.
    pub date: Option<String>,
}
