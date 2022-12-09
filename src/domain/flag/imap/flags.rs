use crate::{Flag, Flags};

use super::ImapFlag;

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
