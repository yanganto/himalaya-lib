//! IMAP envelope module.
//!
//! This module provides IMAP types and conversion utilities related
//! to the envelope.

use std::borrow::Cow;

use chrono::{DateTime, Local, NaiveDateTime};
use imap::{self, types::Fetch};
use rfc2047_decoder;

use crate::{
    backend::imap::{Error, Result},
    envelope::Mailbox,
    Envelope, Flags,
};

pub fn from_raw(fetch: &Fetch) -> Result<Envelope> {
    let decode = |input: &Cow<[u8]>| {
        rfc2047_decoder::Decoder::new()
            .skip_encoded_word_length(true)
            .decode(input)
    };

    let envelope = fetch
        .envelope()
        .ok_or_else(|| Error::GetEnvelopeError(fetch.message.to_string()))?;

    let id = fetch.message.to_string();

    let internal_id = fetch
        .uid
        .ok_or_else(|| Error::GetUidError(fetch.message))?
        .to_string();

    let message_id = String::from_utf8(envelope.message_id.clone().unwrap_or_default().to_vec())
        .map_err(|err| Error::ParseMessageIdError(err, fetch.message))?;

    let flags = Flags::from(fetch.flags());

    let subject = envelope
        .subject
        .as_ref()
        .map(|subject| decode(subject).map_err(|err| Error::DecodeSubjectError(err, fetch.message)))
        .unwrap_or_else(|| Ok(String::default()))?;

    let from = envelope
        .from
        .as_ref()
        .and_then(|addrs| addrs.get(0))
        .map(|addr| {
            match (
                addr.name.as_ref(),
                addr.mailbox.as_ref(),
                addr.host.as_ref(),
            ) {
                (name, Some(mbox), Some(host)) => {
                    let mbox =
                        decode(mbox).map_err(Error::DecodeSenderMailboxFromImapEnvelopeError)?;
                    let host =
                        decode(host).map_err(Error::DecodeSenderHostFromImapEnvelopeError)?;

                    match name {
                        None => Ok(Mailbox::new_nameless([mbox, host].join("@"))),
                        Some(name) => {
                            let name = decode(name)
                                .map_err(Error::DecodeSenderNameFromImapEnvelopeError)?;
                            Ok(Mailbox::new(Some(name), [mbox, host].join("@")))
                        }
                    }
                }
                _ => Err(Error::ParseSenderFromImapEnvelopeError),
            }
        })
        .ok_or_else(|| Error::GetSenderError(fetch.message))??;

    let date = envelope.date.as_ref().map(|date| {
        let date = decode(date).map_err(Error::DecodeDateFromImapEnvelopeError)?;
        let timestamp = mailparse::dateparse(&date)
            .map_err(|err| Error::ParseTimestampFromImapEnvelopeError(err, date.to_string()))?;
        let date = NaiveDateTime::from_timestamp_opt(timestamp, 0)
            .and_then(|date| date.and_local_timezone(Local).earliest());
        Result::Ok(date)
    });
    let date = match date {
        Some(date) => date?.unwrap_or_default(),
        None => DateTime::default(),
    };

    Ok(Envelope {
        id,
        internal_id,
        flags,
        message_id,
        subject,
        from,
        date,
    })
}
