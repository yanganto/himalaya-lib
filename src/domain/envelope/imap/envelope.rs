//! IMAP envelope module.
//!
//! This module provides IMAP types and conversion utilities related
//! to the envelope.

use chrono::{Local, NaiveDateTime};
use imap;
use log::warn;
use rfc2047_decoder;

use crate::{
    backend::imap::{Error, Result},
    Envelope, Flags,
};

/// Represents the raw envelope returned by the `imap` crate.
pub type RawEnvelope<'a> = imap::types::Fetch<'a>;

pub fn from_raw(raw: &RawEnvelope) -> Result<Envelope> {
    let envelope = raw
        .envelope()
        .ok_or_else(|| Error::GetEnvelopeError(raw.message.to_string()))?;

    let id = raw.message.to_string();

    let internal_id = raw
        .uid
        .ok_or_else(|| Error::GetUidError(raw.message))?
        .to_string();

    let message_id = String::from_utf8(envelope.message_id.clone().unwrap_or_default().to_vec())
        .map_err(|err| Error::ParseMessageIdError(err, raw.message))?;

    let flags = Flags::from(raw.flags());

    let subject = envelope
        .subject
        .as_ref()
        .map(|subj| {
            rfc2047_decoder::Decoder::new()
                .skip_encoded_word_length(true)
                .decode(subj)
                .map_err(|err| Error::DecodeSubjectError(err, raw.message))
        })
        .unwrap_or_else(|| Ok(String::default()))?;

    let sender = envelope
        .sender
        .as_ref()
        .and_then(|addrs| addrs.get(0))
        .or_else(|| envelope.from.as_ref().and_then(|addrs| addrs.get(0)))
        .ok_or_else(|| Error::GetSenderError(raw.message))?;
    let sender = if let Some(ref name) = sender.name {
        rfc2047_decoder::Decoder::new()
            .skip_encoded_word_length(true)
            .decode(&name.to_vec())
            .map_err(|err| Error::DecodeSenderNameError(err, raw.message))?
    } else {
        let mbox = sender
            .mailbox
            .as_ref()
            .ok_or_else(|| Error::GetSenderError(raw.message))
            .and_then(|mbox| {
                rfc2047_decoder::Decoder::new()
                    .skip_encoded_word_length(true)
                    .decode(&mbox.to_vec())
                    .map_err(|err| Error::DecodeSenderNameError(err, raw.message))
            })?;
        let host = sender
            .host
            .as_ref()
            .ok_or_else(|| Error::GetSenderError(raw.message))
            .and_then(|host| {
                rfc2047_decoder::Decoder::new()
                    .skip_encoded_word_length(true)
                    .decode(&host.to_vec())
                    .map_err(|err| Error::DecodeSenderNameError(err, raw.message))
            })?;
        format!("{}@{}", mbox, host)
    };

    let date = envelope.date.as_ref().and_then(|date_cow| {
        let date_str = String::from_utf8_lossy(date_cow.to_vec().as_slice()).to_string();
        let date_str = date_str.as_str();

        let timestamp = match mailparse::dateparse(date_str) {
            Ok(timestamp) => Some(timestamp),
            Err(err) => {
                warn!("invalid date {}, skipping it: {}", date_str, err);
                None
            }
        };

        let date = timestamp
            .and_then(|timestamp| NaiveDateTime::from_timestamp_opt(timestamp, 0))
            .and_then(|date| date.and_local_timezone(Local).earliest());

        if let None = date {
            warn!("invalid date {}, skipping it", date_str);
        }

        date
    });

    Ok(Envelope {
        id,
        internal_id,
        flags,
        message_id,
        subject,
        sender,
        date,
    })
}
