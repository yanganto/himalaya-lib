//! Email config module.
//!
//! This module contains structures related to email configuration.

use crate::SendmailConfig;
#[cfg(feature = "smtp-sender")]
use crate::SmtpConfig;

/// Represents the email sender provider.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum EmailSender {
    None,
    #[cfg(feature = "smtp-sender")]
    /// Represents the internal SMTP mailer library.
    Smtp(SmtpConfig),
    /// Represents the sendmail command.
    Sendmail(SendmailConfig),
}

impl Default for EmailSender {
    fn default() -> Self {
        Self::None
    }
}

/// Represents the text/plain format as defined in the [RFC2646].
///
/// [RFC2646]: https://www.ietf.org/rfc/rfc2646.txt
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum EmailTextPlainFormat {
    /// Makes the content fit its container.
    Auto,
    /// Does not restrict the content.
    Flowed,
    /// Forces the content width with a fixed amount of pixels.
    Fixed(usize),
}

impl Default for EmailTextPlainFormat {
    fn default() -> Self {
        Self::Auto
    }
}

/// Represents the email hooks. Useful for doing extra email
/// processing before or after sending it.
#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct EmailHooks {
    /// Represents the hook called just before sending an email.
    pub pre_send: Option<String>,
}
