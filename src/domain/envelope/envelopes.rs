use serde::Serialize;
use std::ops::{Deref, DerefMut};

use crate::Envelope;

/// Represents the list of envelopes.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Envelopes(Vec<Envelope>);

impl Deref for Envelopes {
    type Target = Vec<Envelope>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Envelopes {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl FromIterator<Envelope> for Envelopes {
    fn from_iter<T: IntoIterator<Item = Envelope>>(iter: T) -> Self {
        let mut envelopes = Envelopes::default();
        envelopes.extend(iter);
        envelopes
    }
}
