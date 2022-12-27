use imap;
use std::borrow::Cow;

use crate::Flag;

pub type ImapFlag<'a> = imap::types::Flag<'a>;

impl Flag {
    pub fn to_imap_query(&self) -> String {
        match self {
            Flag::Seen => String::from("\\Seen"),
            Flag::Answered => String::from("\\Answered"),
            Flag::Flagged => String::from("\\Flagged"),
            Flag::Deleted => String::from("\\Deleted"),
            Flag::Draft => String::from("\\Draft"),
            Flag::Recent => String::from("\\Recent"),
            Flag::Custom(flag) => flag.clone(),
        }
    }
}

impl From<&ImapFlag<'_>> for Flag {
    fn from(imap_flag: &ImapFlag<'_>) -> Self {
        match imap_flag {
            ImapFlag::Seen => Flag::Seen,
            ImapFlag::Answered => Flag::Answered,
            ImapFlag::Flagged => Flag::Flagged,
            ImapFlag::Deleted => Flag::Deleted,
            ImapFlag::Draft => Flag::Draft,
            ImapFlag::Recent => Flag::Recent,
            ImapFlag::MayCreate => Flag::Custom(String::from("MayCreate")),
            ImapFlag::Custom(flag) => Flag::Custom(flag.to_string()),
            flag => Flag::Custom(flag.to_string()),
        }
    }
}

impl From<ImapFlag<'_>> for Flag {
    fn from(imap_flag: ImapFlag<'_>) -> Self {
        Flag::from(&imap_flag)
    }
}

impl Into<ImapFlag<'static>> for Flag {
    fn into(self) -> ImapFlag<'static> {
        match self {
            Flag::Seen => ImapFlag::Seen,
            Flag::Answered => ImapFlag::Answered,
            Flag::Flagged => ImapFlag::Flagged,
            Flag::Deleted => ImapFlag::Deleted,
            Flag::Draft => ImapFlag::Draft,
            Flag::Recent => ImapFlag::Recent,
            Flag::Custom(flag) => ImapFlag::Custom(Cow::Owned(flag.clone())),
        }
    }
}
