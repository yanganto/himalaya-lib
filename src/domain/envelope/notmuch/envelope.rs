//! Notmuch folder module.
//!
//! This module provides Notmuch types and conversion utilities
//! related to the envelope

use chrono::DateTime;
use lettre::message::Mailboxes;
use log::{info, trace};
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
    let date = raw
        .header("date")
        .map_err(|err| Error::ParseMsgHeaderError(err, String::from("date")))?
        .ok_or_else(|| Error::FindMsgHeaderError(String::from("date")))?
        .to_string();
    let date = DateTime::parse_from_rfc2822(date.split_at(date.find(" (").unwrap_or(date.len())).0)
        .map_err(|err| Error::ParseMsgDateError(err, date.to_owned()))
        .map(|date| date.naive_local().to_string())
        .ok();

    let envelope = Envelope {
        id,
        internal_id,
        flags: raw
            .tags()
            .map(|tag| Flag::Custom(tag.to_string()))
            .collect(),
        subject,
        sender,
        date,
    };
    trace!("envelope: {:?}", envelope);

    info!("end: try building envelope from notmuch parsed mail");
    Ok(envelope)
}
