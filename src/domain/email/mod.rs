//! Message module.
//!
//! This module contains everything related to emails.

pub mod addr;
pub mod attachment;
pub mod config;
pub mod email;
pub mod parts;
pub mod tpl;
pub mod utils;

pub use addr::*;
pub use attachment::Attachment;
pub use config::{EmailHooks, EmailSender, EmailTextPlainFormat};
pub use email::*;
pub use parts::{sanitize_text_plain_part, Parts, PartsIterator};
pub use tpl::{
    HeaderVal, ShowHeaders, ShowTextPartStrategy, Tpl, TplBuilder, TplBuilderOpts, TplOverride,
};
pub use utils::*;
