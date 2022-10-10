use crate::Flags;

use super::flag;

pub fn from_raw(entry: &maildir::MailEntry) -> Flags {
    entry.flags().chars().map(flag::from_char).collect()
}

pub fn to_normalized_string(flags: &Flags) -> String {
    String::from_iter(flags.iter().filter_map(flag::to_normalized_char))
}
