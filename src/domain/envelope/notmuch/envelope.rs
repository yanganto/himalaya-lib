//! Notmuch folder module.
//!
//! This module provides Notmuch types and conversion utilities
//! related to the envelope

use chrono::{Local, NaiveDateTime};
use log::{info, trace};
use notmuch;

use crate::{
    backend::notmuch::{Error, Result},
    envelope::Mailbox,
    Envelope, Flag,
};

/// Represents the raw envelope returned by the `notmuch` crate.
pub type RawEnvelope = notmuch::Message;

pub fn from_raw(raw: RawEnvelope) -> Result<Envelope> {
    info!("begin: try building envelope from notmuch parsed mail");

    let internal_id = raw.id().to_string();
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
    let from = {
        let from = raw
            .header("from")
            .map_err(|err| Error::ParseMsgHeaderError(err, String::from("from")))?
            .ok_or_else(|| Error::FindMsgHeaderError(String::from("from")))?
            .to_string();
        let addrs =
            mailparse::addrparse(&from).map_err(|err| Error::ParseSenderError(err, from))?;
        match addrs.first() {
            Some(mailparse::MailAddr::Single(single)) => Ok(Mailbox::new(
                single.display_name.clone(),
                single.addr.clone(),
            )),
            // TODO
            Some(mailparse::MailAddr::Group(_)) => Err(Error::FindSenderError),
            None => Err(Error::FindSenderError),
        }?
    };
    let date = {
        let date = raw
            .header("date")
            .map_err(|err| Error::ParseMsgHeaderError(err, String::from("date")))?
            .ok_or_else(|| Error::FindMsgHeaderError(String::from("from")))?
            .to_string();
        let timestamp = mailparse::dateparse(&date)
            .map_err(|err| Error::ParseTimestampFromEnvelopeError(err, date))?;
        let date = NaiveDateTime::from_timestamp_opt(timestamp, 0)
            .and_then(|date| date.and_local_timezone(Local).earliest());
        date.unwrap_or_default()
    };

    let envelope = Envelope {
        id: String::new(),
        internal_id,
        flags: raw.tags().map(Flag::from).collect(),
        message_id,
        subject,
        from,
        date,
    };
    trace!("envelope: {:?}", envelope);

    info!("end: try building envelope from notmuch parsed mail");
    Ok(envelope)
}
