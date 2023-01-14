use chrono::{DateTime, Local};
use serde::{Serialize, Serializer};

use crate::Flags;

/// Represents the message envelope. The envelope is just a message
/// subset, and is mostly used for listings.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Envelope {
    /// Represents the identifier.
    pub id: String,
    /// Represents the internal identifier.
    pub internal_id: String,
    /// Represents the flags.
    pub flags: Flags,
    /// Represents the Message-ID header.
    pub message_id: String,
    /// Represents the first sender.
    pub sender: String,
    /// Represents the Subject header.
    pub subject: String,
    #[serde(serialize_with = "date")]
    /// Represents the Date header.
    pub date: Option<DateTime<Local>>,
}

fn date<S: Serializer>(date: &Option<DateTime<Local>>, s: S) -> Result<S::Ok, S::Error> {
    match date {
        Some(date) => s.serialize_str(&date.to_rfc3339()),
        None => s.serialize_none(),
    }
}

impl Envelope {
    /// Builds the envelope hash using the given folder name, the
    /// Message-ID, the Subject, the sender and the internal date.
    pub fn hash<F: AsRef<str>>(&self, folder: F) -> String {
        let hash = md5::compute(&format!(
            "{}{}{}{}{:?}",
            folder.as_ref(),
            self.message_id,
            self.subject,
            self.sender,
            self.date.map(|date| date.to_rfc3339()),
        ));
        format!("{:x}", hash)
    }
}
