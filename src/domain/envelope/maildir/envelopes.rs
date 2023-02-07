//! Maildir mailbox module.
//!
//! This module provides Maildir types and conversion utilities
//! related to the envelope.
use rayon::prelude::*;

use crate::{
    backend::maildir::{Error, Result},
    Envelopes,
};

use super::envelope;

/// Represents a list of raw envelopees returned by the `maildir`
/// crate.
pub type RawEnvelopes = maildir::MailEntries;

pub fn from_raws(entries: RawEnvelopes) -> Result<Envelopes> {
    Ok(Envelopes::from_iter(
        // TODO: clean me please
        entries
            .collect::<Vec<_>>()
            .into_par_iter()
            .map(|entry| entry.map_err(Error::DecodeEntryError))
            .collect::<Result<Vec<_>>>()?
            .into_par_iter()
            .map(|entry| envelope::from_raw(entry))
            .collect::<Result<Vec<_>>>()?,
    ))
}
