//! Notmuch folder module.
//!
//! This module provides Notmuch types and conversion utilities
//! related to the envelope

use chrono::DateTime;
use log::{info, trace};
use notmuch;

use crate::{
    backend::notmuch::{Error, Result},
    from_slice_to_addrs, Addr, Envelope, Flag,
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
    let sender = from_slice_to_addrs(&sender)
        .map_err(|err| Error::ParseSendersError(err, sender.to_owned()))?
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
            Addr::Group(mailparse::GroupInfo { group_name, .. }) => group_name.to_owned(),
        })
        .ok_or_else(|| Error::FindSenderError)?;
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
