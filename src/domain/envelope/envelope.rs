use chrono::{DateTime, Local};
use serde::{Serialize, Serializer};

use crate::Flags;

fn date<S: Serializer>(date: &DateTime<Local>, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&date.to_rfc3339())
}

#[derive(Clone, Debug, Default, Eq, Serialize)]
pub struct Mailbox {
    pub name: Option<String>,
    pub addr: String,
}

impl PartialEq for Mailbox {
    fn eq(&self, other: &Self) -> bool {
        self.addr == other.addr
    }
}

impl Mailbox {
    pub fn new<N, A>(name: Option<N>, address: A) -> Self
    where
        N: ToString,
        A: ToString,
    {
        Self {
            name: name.map(|name| name.to_string()),
            addr: address.to_string(),
        }
    }

    pub fn new_nameless<A>(address: A) -> Self
    where
        A: ToString,
    {
        Self {
            name: None,
            addr: address.to_string(),
        }
    }
}

/// Represents the message envelope. The envelope is just a message
/// subset, and is mostly used for listings.
#[derive(Clone, Debug, Default, Eq, Serialize)]
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
    pub from: Mailbox,
    /// Represents the Subject header.
    pub subject: String,
    #[serde(serialize_with = "date")]
    /// Represents the Date header.
    pub date: DateTime<Local>,
}

impl PartialEq for Envelope {
    fn eq(&self, other: &Self) -> bool {
        self.flags == other.flags
            && self.message_id == other.message_id
            && self.from == other.from
            && self.subject == other.subject
            && self.date == other.date
    }
}

impl Envelope {
    /// Builds the envelope hash using the given folder name, the
    /// Message-ID, the Subject, the sender address and the internal
    /// date.
    pub fn hash<F: ToString>(&self, folder: F) -> String {
        let hash = md5::compute(
            folder.to_string()
                + &self.message_id
                + &self.subject
                + &self.from.addr
                + &self.date.to_rfc3339(),
        );
        format!("{:x}", hash)
    }
}
