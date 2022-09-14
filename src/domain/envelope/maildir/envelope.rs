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

use chrono::DateTime;
use log::trace;

use crate::{
    backend::maildir::{Error, Result},
    domain::flag::maildir::flags,
    from_slice_to_addrs, Addr, Envelope,
};

/// Represents the raw envelope returned by the `maildir` crate.
pub type RawEnvelope = maildir::MailEntry;

pub fn from_raw(mut entry: RawEnvelope) -> Result<Envelope> {
    trace!(">> build envelope from maildir parsed mail");

    let mut envelope = Envelope::default();

    envelope.internal_id = entry.id().to_owned();
    envelope.id = format!("{:x}", md5::compute(&envelope.internal_id));
    envelope.flags = flags::from_raw(&entry);

    let parsed_mail = entry.parsed().map_err(Error::ParseMsgError)?;

    trace!(">> parse headers");
    for h in parsed_mail.get_headers() {
        let k = h.get_key();
        trace!("header key: {:?}", k);

        let v = rfc2047_decoder::decode(h.get_value_raw())
            .map_err(|err| Error::DecodeHeaderError(err, k.to_owned()))?;
        trace!("header value: {:?}", v);

        match k.to_lowercase().as_str() {
            "date" => {
                envelope.date =
                    DateTime::parse_from_rfc2822(v.split_at(v.find(" (").unwrap_or(v.len())).0)
                        .map(|date| date.naive_local().to_string())
                        .ok()
            }
            "subject" => {
                envelope.subject = v.into();
            }
            "from" => {
                envelope.sender = from_slice_to_addrs(v)
                    .map_err(|err| Error::ParseHeaderError(err, k.to_owned()))?
                    .and_then(|senders| {
                        if senders.is_empty() {
                            None
                        } else {
                            Some(senders)
                        }
                    })
                    .map(|senders| match &senders[0] {
                        Addr::Single(mailparse::SingleInfo { display_name, addr }) => {
                            display_name.as_ref().unwrap_or_else(|| addr).to_owned()
                        }
                        Addr::Group(mailparse::GroupInfo { group_name, .. }) => {
                            group_name.to_owned()
                        }
                    })
                    .ok_or_else(|| Error::FindSenderError)?;
            }
            _ => (),
        }
    }
    trace!("<< parse headers");

    trace!("envelope: {:?}", envelope);
    trace!("<< build envelope from maildir parsed mail");
    Ok(envelope)
}
