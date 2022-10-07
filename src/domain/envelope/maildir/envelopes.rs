//! Maildir mailbox module.
//!
//! This module provides Maildir types and conversion utilities
//! related to the envelope.

use crate::{
    backend::maildir::{Error, Result},
    Envelopes,
};

use super::envelope;

/// Represents a list of raw envelopees returned by the `maildir`
/// crate.
pub type RawEnvelopes = maildir::MailEntries;

pub fn from_raws(entries: RawEnvelopes) -> Result<Envelopes> {
    let mut envelopes = Envelopes::default();
    for entry in entries {
        let entry = entry.map_err(Error::DecodeEntryError)?;
        envelopes.push(envelope::from_raw(entry)?);
    }
    Ok(envelopes)
}
