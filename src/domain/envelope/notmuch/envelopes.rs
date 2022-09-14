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

use crate::{backend::notmuch::Result, Envelopes};

use super::envelope;

/// Represents a list of raw envelopees returned by the `notmuch`
/// crate.
pub type RawEnvelopes = notmuch::Messages;

pub fn from_raws(raws: RawEnvelopes) -> Result<Envelopes> {
    let mut envelopes = Envelopes::default();
    for msg in raws {
        let envelope = envelope::from_raw(msg)?;
        envelopes.push(envelope);
    }
    Ok(envelopes)
}
