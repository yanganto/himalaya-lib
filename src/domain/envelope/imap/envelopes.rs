use crate::{backend::imap::Result, Envelopes};

use super::envelope;

/// Represents the list of raw envelopes returned by the `imap` crate.
pub type RawEnvelopes = imap::types::Fetches;

pub fn from_raws(raws: RawEnvelopes) -> Result<Envelopes> {
    let mut envelopes = Envelopes::default();
    for fetch in raws.iter().rev() {
        envelopes.push(envelope::from_raw(fetch)?);
    }
    Ok(envelopes)
}
