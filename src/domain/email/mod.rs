//! Message module.
//!
//! This module contains everything related to messages.

pub mod config;
pub use config::{EmailHooks, EmailSender, EmailTextPlainFormat};

mod parts;
pub use parts::*;

mod addr;
pub use addr::*;

mod tpl;
pub use tpl::{compile, TplOverride};

mod email;
pub use email::*;

mod utils;
pub use utils::*;
