use lettre::transport::smtp::authentication::Credentials as SmtpCredentials;
use serde::Deserialize;
use thiserror::Error;

use crate::process::{self, ProcessError};

#[cfg(feature = "internal-sender")]
#[derive(Debug, Error)]
pub enum SmtpConfigError {
    #[error("cannot get smtp password")]
    GetPasswdError(#[source] ProcessError),
    #[error("cannot get smtp password: password is empty")]
    GetPasswdEmptyError,
}

/// Represents the internal sender config.
#[cfg(feature = "internal-sender")]
#[derive(Debug, Default, Clone, Eq, PartialEq, Deserialize)]
pub struct SmtpConfig {
    /// Represents the SMTP server host.
    pub host: String,
    /// Represents the SMTP server port.
    pub port: u16,
    /// Enables StartTLS.
    pub starttls: Option<bool>,
    /// Trusts any certificate.
    pub insecure: Option<bool>,
    /// Represents the SMTP server login.
    pub login: String,
    /// Represents the SMTP password command.
    pub passwd_cmd: String,
}

#[cfg(feature = "internal-sender")]
impl SmtpConfig {
    /// Builds the internal SMTP sender credentials.
    pub fn credentials(&self) -> Result<SmtpCredentials, SmtpConfigError> {
        let passwd = process::run(&self.passwd_cmd).map_err(SmtpConfigError::GetPasswdError)?;
        let passwd = passwd
            .lines()
            .next()
            .ok_or_else(|| SmtpConfigError::GetPasswdEmptyError)?;
        Ok(SmtpCredentials::new(
            self.login.to_owned(),
            passwd.to_owned(),
        ))
    }

    pub fn starttls(&self) -> bool {
        self.starttls.unwrap_or_default()
    }

    pub fn insecure(&self) -> bool {
        self.insecure.unwrap_or_default()
    }
}
