pub mod sender;
pub use sender::Sender;

pub mod smtp;
pub use smtp::{Smtp, SmtpConfig, SmtpConfigError, SmtpError};
