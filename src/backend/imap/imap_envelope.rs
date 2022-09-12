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

//! IMAP envelope module.
//!
//! This module provides IMAP types and conversion utilities related
//! to the envelope.

use rfc2047_decoder;

use crate::{
    backend::{
        from_imap_flags,
        imap::{Error, Result},
    },
    email::Envelope,
};

/// Represents the raw envelope returned by the `imap` crate.
pub type ImapFetch = imap::types::Fetch;

pub fn from_imap_fetch(fetch: &ImapFetch) -> Result<Envelope> {
    let envelope = fetch
        .envelope()
        .ok_or_else(|| Error::GetEnvelopeError(fetch.message))?;

    let id = fetch.message.to_string();

    let flags = from_imap_flags(fetch.flags());

    let subject = envelope
        .subject
        .as_ref()
        .map(|subj| {
            rfc2047_decoder::decode(subj)
                .map_err(|err| Error::DecodeSubjectError(err, fetch.message))
        })
        .unwrap_or_else(|| Ok(String::default()))?;

    let sender = envelope
        .sender
        .as_ref()
        .and_then(|addrs| addrs.get(0))
        .or_else(|| envelope.from.as_ref().and_then(|addrs| addrs.get(0)))
        .ok_or_else(|| Error::GetSenderError(fetch.message))?;
    let sender = if let Some(ref name) = sender.name {
        rfc2047_decoder::decode(&name.to_vec())
            .map_err(|err| Error::DecodeSenderNameError(err, fetch.message))?
    } else {
        let mbox = sender
            .mailbox
            .as_ref()
            .ok_or_else(|| Error::GetSenderError(fetch.message))
            .and_then(|mbox| {
                rfc2047_decoder::decode(&mbox.to_vec())
                    .map_err(|err| Error::DecodeSenderNameError(err, fetch.message))
            })?;
        let host = sender
            .host
            .as_ref()
            .ok_or_else(|| Error::GetSenderError(fetch.message))
            .and_then(|host| {
                rfc2047_decoder::decode(&host.to_vec())
                    .map_err(|err| Error::DecodeSenderNameError(err, fetch.message))
            })?;
        format!("{}@{}", mbox, host)
    };

    let date = fetch
        .internal_date()
        .map(|date| date.naive_local().to_string());

    Ok(Envelope {
        id: id.clone(),
        internal_id: id,
        flags,
        subject,
        sender,
        date,
    })
}
