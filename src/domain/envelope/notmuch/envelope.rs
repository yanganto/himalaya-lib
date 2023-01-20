//! Notmuch folder module.
//!
//! This module provides Notmuch types and conversion utilities
//! related to the envelope

use chrono::{Local, NaiveDateTime};
use lettre::message::Mailboxes;
use log::{info, trace, warn};
use notmuch;

use crate::{
    backend::notmuch::{Error, Result},
    Envelope, Flag,
};

/// Represents the raw envelope returned by the `notmuch` crate.
pub type RawEnvelope = notmuch::Message;

pub fn from_raw(raw: RawEnvelope) -> Result<Envelope> {
    info!("begin: try building envelope from notmuch parsed mail");

    let internal_id = raw.id().to_string();
    let id = format!("{:x}", md5::compute(&internal_id));
    let subject = raw
        .header("subject")
        .map_err(|err| Error::ParseMsgHeaderError(err, String::from("subject")))?
        .unwrap_or_default()
        .to_string();
    let message_id = raw
        .header("message-id")
        .map_err(|err| Error::ParseMsgHeaderError(err, String::from("message-id")))?
        .unwrap_or_default()
        .to_string();
    let sender = raw
        .header("from")
        .map_err(|err| Error::ParseMsgHeaderError(err, String::from("from")))?
        .ok_or_else(|| Error::FindMsgHeaderError(String::from("from")))?
        .to_string();
    let sender: Mailboxes = sender
        .parse()
        .map_err(|err| Error::ParseSendersError(err, sender.to_owned()))?;
    let sender = sender
        .into_single()
        .ok_or_else(|| Error::FindSenderError)?
        .to_string();
    let date = {
        let date_string = raw
            .header("date")
            .map_err(|err| Error::ParseMsgHeaderError(err, String::from("date")))?
            .ok_or_else(|| Error::FindMsgHeaderError(String::from("date")))?
            .to_string();
        let date_str = date_string.as_str();

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
    };

    let envelope = Envelope {
        id,
        internal_id,
        flags: raw.tags().map(Flag::from).collect(),
        message_id,
        subject,
        sender,
        date,
    };
    trace!("envelope: {:?}", envelope);

    info!("end: try building envelope from notmuch parsed mail");
    Ok(envelope)
}
