use imap;

use crate::Flag;

pub type RawFlag<'a> = imap::types::Flag<'a>;

pub fn from_raw(imap_flag: &RawFlag<'_>) -> Flag {
    match imap_flag {
        imap::types::Flag::Seen => Flag::Seen,
        imap::types::Flag::Answered => Flag::Answered,
        imap::types::Flag::Flagged => Flag::Flagged,
        imap::types::Flag::Deleted => Flag::Deleted,
        imap::types::Flag::Draft => Flag::Draft,
        imap::types::Flag::Recent => Flag::Recent,
        imap::types::Flag::MayCreate => Flag::Custom(String::from("MayCreate")),
        imap::types::Flag::Custom(flag) => Flag::Custom(flag.to_string()),
        flag => Flag::Custom(flag.to_string()),
    }
}
