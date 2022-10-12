//! Sendmail module.
//!
//! This module contains the representation of the sendmail email
//! sender.

use std::result;
use thiserror::Error;

use crate::{process, sender, AccountConfig, Email, Sender, SendmailConfig};

#[derive(Debug, Error)]
pub enum Error {
    #[error("cannot run sendmail command")]
    RunCmdError(#[source] process::Error),
}

pub type Result<T> = result::Result<T, Error>;

pub struct Sendmail<'a> {
    account_config: &'a AccountConfig,
    sendmail_config: &'a SendmailConfig,
}

impl<'a> Sendmail<'a> {
    pub fn new(account_config: &'a AccountConfig, sendmail_config: &'a SendmailConfig) -> Self {
        Self {
            account_config,
            sendmail_config,
        }
    }
}

impl<'a> Sender for Sendmail<'a> {
    fn send(&mut self, email: &Email) -> sender::Result<Vec<u8>> {
        let input = email.into_sendable_msg(self.account_config)?.formatted();
        let output = process::run(&self.sendmail_config.cmd, &input).map_err(Error::RunCmdError)?;
        Ok(if output.is_empty() { input } else { output })
    }
}
