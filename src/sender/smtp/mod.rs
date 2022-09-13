#[cfg(feature = "internal-sender")]
pub mod config;
#[cfg(feature = "internal-sender")]
pub use config::SmtpConfig;

#[cfg(feature = "internal-sender")]
pub mod smtp;
#[cfg(feature = "internal-sender")]
pub use smtp::Smtp;
