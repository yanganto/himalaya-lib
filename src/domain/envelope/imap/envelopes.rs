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

use crate::{backend::imap::Result, Envelopes};

use super::envelope;

/// Represents the list of raw envelopes returned by the `imap` crate.
pub type RawEnvelopes = imap::types::ZeroCopy<Vec<envelope::RawEnvelope>>;

pub fn from_raws(raws: RawEnvelopes) -> Result<Envelopes> {
    let mut envelopes = Envelopes::default();
    for fetch in raws.iter().rev() {
        envelopes.push(envelope::from_raw(fetch)?);
    }
    Ok(envelopes)
}
