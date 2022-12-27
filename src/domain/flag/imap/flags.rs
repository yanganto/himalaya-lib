use crate::{Flag, Flags};

use super::ImapFlag;

impl Flags {
    pub fn to_imap_query(&self) -> String {
        let mut flags = String::default();
        let mut glue = "";

        for flag in &self.0 {
            flags.push_str(glue);
            flags.push_str(&flag.to_imap_query());
            glue = " ";
        }

        flags
    }

    pub fn into_imap_flags_vec(&self) -> Vec<ImapFlag<'static>> {
        self.iter().map(|flag| flag.clone().into()).collect()
    }
}

impl From<&[ImapFlag<'_>]> for Flags {
    fn from(imap_flags: &[ImapFlag<'_>]) -> Self {
        imap_flags.iter().map(Flag::from).collect()
    }
}

impl From<Vec<ImapFlag<'_>>> for Flags {
    fn from(imap_flags: Vec<ImapFlag<'_>>) -> Self {
        Flags::from(imap_flags.as_slice())
    }
}

impl Into<Vec<ImapFlag<'_>>> for Flags {
    fn into(self) -> Vec<ImapFlag<'static>> {
        self.iter()
            .map(ToOwned::to_owned)
            .map(<Flag as Into<ImapFlag>>::into)
            .collect()
    }
}
