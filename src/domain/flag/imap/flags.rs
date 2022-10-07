use crate::{Flag, Flags};

use super::flag;

pub fn into_raws<'a>(flags: &'a Flags) -> Vec<flag::RawFlag<'a>> {
    flags
        .iter()
        .map(|flag| match flag {
            Flag::Seen => flag::RawFlag::Seen,
            Flag::Answered => flag::RawFlag::Answered,
            Flag::Flagged => flag::RawFlag::Flagged,
            Flag::Deleted => flag::RawFlag::Deleted,
            Flag::Draft => flag::RawFlag::Draft,
            Flag::Recent => flag::RawFlag::Recent,
            Flag::Custom(flag) => flag::RawFlag::Custom(flag.into()),
        })
        .collect()
}

pub fn from_raws(imap_flags: &[flag::RawFlag<'_>]) -> Flags {
    imap_flags.iter().map(flag::from_raw).collect()
}
