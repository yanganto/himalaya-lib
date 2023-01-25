use serde::Serialize;
use std::{collections::HashSet, ops};

use crate::Flag;

/// Represents the list of flags.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Flags(pub HashSet<Flag>);

impl Flags {
    pub fn clone_without_customs(&self) -> Self {
        Self::from_iter(
            self.iter()
                .filter(|f| match f {
                    Flag::Custom(_) => false,
                    _ => true,
                })
                .cloned(),
        )
    }

    /// Builds a symbols string.
    pub fn to_symbols_string(&self) -> String {
        let mut flags = String::new();
        flags.push_str(if self.contains(&Flag::Seen) {
            " "
        } else {
            "✷"
        });
        flags.push_str(if self.contains(&Flag::Answered) {
            "↵"
        } else {
            " "
        });
        flags.push_str(if self.contains(&Flag::Flagged) {
            "⚑"
        } else {
            " "
        });
        flags
    }
}

impl ToString for Flags {
    fn to_string(&self) -> String {
        let mut flags = String::default();
        let mut glue = "";

        for flag in &self.0 {
            flags.push_str(glue);
            flags.push_str(&flag.to_string());
            glue = " ";
        }

        flags
    }
}

impl ops::Deref for Flags {
    type Target = HashSet<Flag>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ops::DerefMut for Flags {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<&str> for Flags {
    fn from(flags: &str) -> Self {
        Flags(
            flags
                .split_whitespace()
                .map(|flag| flag.trim().into())
                .collect(),
        )
    }
}

impl FromIterator<Flag> for Flags {
    fn from_iter<T: IntoIterator<Item = Flag>>(iter: T) -> Self {
        let mut flags = Flags::default();
        flags.extend(iter);
        flags
    }
}
