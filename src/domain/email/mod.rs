//! Message module.
//!
//! This module contains everything related to emails.

pub mod attachment;
pub mod config;
pub mod email;
pub mod parts;
pub mod tpl;
pub mod utils;

pub use attachment::Attachment;
pub use config::{EmailHooks, EmailSender, EmailTextPlainFormat};
pub use email::*;
pub use parts::*;
pub use tpl::{HeaderVal, ShowHeaders, ShowTextPartStrategy, Tpl, TplBuilder, TplOverride};
pub use utils::*;
