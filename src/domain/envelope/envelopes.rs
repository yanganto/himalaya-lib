use serde::Serialize;
use std::ops;

use crate::Envelope;

/// Represents the list of envelopes.
#[derive(Debug, Default, Serialize)]
pub struct Envelopes(pub Vec<Envelope>);

impl ops::Deref for Envelopes {
    type Target = Vec<Envelope>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ops::DerefMut for Envelopes {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
