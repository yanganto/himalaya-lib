use serde::Serialize;

/// Represents the flag variants.
#[derive(Debug, Clone, Eq, Hash, PartialEq, Serialize)]
pub enum Flag {
    Seen,
    Answered,
    Flagged,
    Deleted,
    Draft,
    Recent,
    Custom(String),
}

impl Flag {
    pub fn custom<F: ToString>(flag: F) -> Self {
        Self::Custom(flag.to_string())
    }
}

impl From<&str> for Flag {
    fn from(s: &str) -> Self {
        match s {
            "seen" => Flag::Seen,
            "answered" | "replied" => Flag::Answered,
            "flagged" => Flag::Flagged,
            "deleted" | "trashed" => Flag::Deleted,
            "draft" => Flag::Draft,
            "recent" => Flag::Recent,
            flag => Flag::Custom(flag.into()),
        }
    }
}

impl From<String> for Flag {
    fn from(s: String) -> Self {
        s.as_str().into()
    }
}

impl ToString for Flag {
    fn to_string(&self) -> String {
        match self {
            Flag::Seen => "seen".into(),
            Flag::Answered => "answered".into(),
            Flag::Flagged => "flagged".into(),
            Flag::Deleted => "deleted".into(),
            Flag::Draft => "draft".into(),
            Flag::Recent => "recent".into(),
            Flag::Custom(flag) => flag.clone(),
        }
    }
}
