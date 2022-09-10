use serde::Deserialize;
use thiserror::Error;

use crate::process::{self, ProcessError};

#[cfg(feature = "imap-backend")]
#[derive(Debug, Error)]
pub enum ImapConfigError {
    #[error("cannot get imap password")]
    GetPasswdError(#[source] ProcessError),
    #[error("cannot get imap password: password is empty")]
    GetPasswdEmptyError,
    #[error("cannot start the notify mode")]
    StartNotifyModeError(#[source] ProcessError),
}

/// Represents the IMAP backend config.
#[cfg(feature = "imap-backend")]
#[derive(Debug, Default, Clone, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ImapConfig {
    /// Represents the IMAP server host.
    pub host: String,
    /// Represents the IMAP server port.
    pub port: u16,
    /// Enables StartTLS.
    pub starttls: Option<bool>,
    /// Trusts any certificate.
    pub insecure: Option<bool>,
    /// Represents the IMAP server login.
    pub login: String,
    /// Represents the IMAP server password command.
    pub passwd_cmd: String,

    /// Represents the IMAP notify command.
    pub notify_cmd: Option<String>,
    /// Overrides the default IMAP query "NEW" used to fetch new
    /// messages.
    pub notify_query: Option<String>,
    /// Represents the watch commands.
    pub watch_cmds: Option<Vec<String>>,
}

#[cfg(feature = "imap-backend")]
impl ImapConfig {
    /// Executes the IMAP password command in order to retrieve the
    /// IMAP server password.
    pub fn passwd(&self) -> Result<String, ImapConfigError> {
        let passwd = process::run(&self.passwd_cmd).map_err(ImapConfigError::GetPasswdError)?;
        let passwd = passwd
            .lines()
            .next()
            .ok_or_else(|| ImapConfigError::GetPasswdEmptyError)?;
        Ok(passwd.to_owned())
    }

    /// Gets the StartTLS IMAP option.
    pub fn starttls(&self) -> bool {
        self.starttls.unwrap_or_default()
    }

    /// Gets the StartTLS IMAP option.
    pub fn insecure(&self) -> bool {
        self.insecure.unwrap_or_default()
    }

    /// Runs the IMAP notify command.
    pub fn run_notify_cmd<S: AsRef<str>>(
        &self,
        subject: S,
        sender: S,
    ) -> Result<(), ImapConfigError> {
        let subject = subject.as_ref();
        let sender = sender.as_ref();

        let default_cmd = format!(r#"notify-send "New message from {}" "{}""#, sender, subject);
        let cmd = self
            .notify_cmd
            .as_ref()
            .map(|cmd| format!(r#"{} {:?} {:?}"#, cmd, subject, sender))
            .unwrap_or(default_cmd);

        process::run(&cmd).map_err(ImapConfigError::StartNotifyModeError)?;
        Ok(())
    }

    pub fn notify_query(&self) -> String {
        self.notify_query
            .as_ref()
            .unwrap_or(&String::from("NEW"))
            .to_owned()
    }

    pub fn watch_cmds(&self) -> Vec<String> {
        self.watch_cmds.as_ref().unwrap_or(&vec![]).to_owned()
    }
}
