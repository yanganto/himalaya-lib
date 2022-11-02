//! Message module.
//!
//! This module contains everything related to messages.

pub mod config;
pub use config::{EmailHooks, EmailSender, EmailTextPlainFormat};

mod attachment;
pub use attachment::Attachment;

mod parts;
pub use parts::{Parts, PartsIterator};

mod addr;
pub use addr::*;

mod tpl;
pub use tpl::{ShowTextPartStrategy, Tpl, TplBuilder, TplOverride};

mod email;
pub use email::*;

mod utils;
pub use utils::*;
