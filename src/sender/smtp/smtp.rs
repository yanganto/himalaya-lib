//! SMTP module.
//!
//! This module contains the representation of the SMTP email sender.

use lettre::{
    self,
    transport::smtp::{
        client::{Tls, TlsParameters},
        SmtpTransport,
    },
    Transport,
};
use std::{convert::TryInto, result};
use thiserror::Error;

use crate::{account, email, process, sender, AccountConfig, Email, Sender, SmtpConfig};

#[derive(Debug, Error)]
pub enum Error {
    #[error("cannot build smtp transport relay")]
    BuildTransportRelayError(#[source] lettre::transport::smtp::Error),
    #[error("cannot build smtp tls parameters")]
    BuildTlsParamsError(#[source] lettre::transport::smtp::Error),
    #[error("cannot parse email before sending")]
    ParseEmailError(#[source] mailparse::MailParseError),
    #[error("cannot send email")]
    SendError(#[source] lettre::transport::smtp::Error),
    #[error("cannot execute pre-send hook")]
    ExecutePreSendHookError(#[source] process::Error),

    #[error(transparent)]
    SmtpConfigError(#[from] sender::smtp::config::Error),
    #[error(transparent)]
    ConfigError(#[from] account::config::Error),
    #[error(transparent)]
    MsgError(#[from] email::Error),
}

pub type Result<T> = result::Result<T, Error>;

pub struct Smtp<'a> {
    account_config: &'a AccountConfig,
    smtp_config: &'a SmtpConfig,
    transport: Option<SmtpTransport>,
}

impl<'a> Smtp<'a> {
    pub fn new(account_config: &'a AccountConfig, smtp_config: &'a SmtpConfig) -> Self {
        Self {
            account_config,
            smtp_config,
            transport: None,
        }
    }

    fn transport(&mut self) -> Result<&SmtpTransport> {
        if let Some(ref transport) = self.transport {
            Ok(transport)
        } else {
            let builder = if self.smtp_config.ssl() {
                let tls = TlsParameters::builder(self.smtp_config.host.to_owned())
                    .dangerous_accept_invalid_hostnames(self.smtp_config.insecure())
                    .dangerous_accept_invalid_certs(self.smtp_config.insecure())
                    .build()
                    .map_err(Error::BuildTlsParamsError)?;

                if self.smtp_config.starttls() {
                    SmtpTransport::starttls_relay(&self.smtp_config.host)
                        .map_err(Error::BuildTransportRelayError)?
                        .tls(Tls::Required(tls))
                } else {
                    SmtpTransport::relay(&self.smtp_config.host)
                        .map_err(Error::BuildTransportRelayError)?
                        .tls(Tls::Wrapper(tls))
                }
            } else {
                SmtpTransport::relay(&self.smtp_config.host)
                    .map_err(Error::BuildTransportRelayError)?
                    .tls(Tls::None)
            };

            self.transport = Some(
                builder
                    .port(self.smtp_config.port)
                    .credentials(self.smtp_config.credentials()?)
                    .build(),
            );

            Ok(self.transport.as_ref().unwrap())
        }
    }
}

impl<'a> Sender for Smtp<'a> {
    fn send(&mut self, email: &Email) -> sender::Result<Vec<u8>> {
        let raw_email = email.into_sendable_msg(self.account_config)?.formatted();

        let envelope: lettre::address::Envelope = if let Some(cmd) =
            self.account_config.email_hooks.pre_send.as_deref()
        {
            let raw_email =
                process::run(cmd, &raw_email).map_err(Error::ExecutePreSendHookError)?;
            let parsed_mail = mailparse::parse_mail(&raw_email).map_err(Error::ParseEmailError)?;
            Email::from_parsed_mail(parsed_mail, self.account_config)?.try_into()
        } else {
            email.try_into()
        }?;

        self.transport()?
            .send_raw(&envelope, &raw_email)
            .map_err(Error::SendError)?;
        Ok(raw_email)
    }
}
