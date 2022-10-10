use crate::{backend::notmuch::Result, Envelopes};

use super::envelope;

/// Represents a list of raw envelopees returned by the `notmuch`
/// crate.
pub type RawEnvelopes = notmuch::Messages;

pub fn from_raws(raws: RawEnvelopes) -> Result<Envelopes> {
    let mut envelopes = Envelopes::default();
    for msg in raws {
        let envelope = envelope::from_raw(msg)?;
        envelopes.push(envelope);
    }
    Ok(envelopes)
}
