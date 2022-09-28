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

//! Module related to message template CLI.
//!
//! This module provides subcommands, arguments and a command matcher related to message template.

#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub struct TplOverride<'a> {
    pub subject: Option<&'a str>,
    pub from: Option<Vec<&'a str>>,
    pub to: Option<Vec<&'a str>>,
    pub cc: Option<Vec<&'a str>>,
    pub bcc: Option<Vec<&'a str>>,
    pub headers: Option<Vec<&'a str>>,
    pub body: Option<&'a str>,
    pub signature: Option<&'a str>,
}
