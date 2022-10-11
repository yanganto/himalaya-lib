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
    config: &'a SmtpConfig,
    transport: Option<SmtpTransport>,
}

impl<'a> Smtp<'a> {
    pub fn new(config: &'a SmtpConfig) -> Self {
        Self {
            config,
            transport: None,
        }
    }

    fn transport(&mut self) -> Result<&SmtpTransport> {
        if let Some(ref transport) = self.transport {
            Ok(transport)
        } else {
            let builder = if self.config.ssl() {
                let tls = TlsParameters::builder(self.config.host.to_owned())
                    .dangerous_accept_invalid_hostnames(self.config.insecure())
                    .dangerous_accept_invalid_certs(self.config.insecure())
                    .build()
                    .map_err(Error::BuildTlsParamsError)?;

                if self.config.starttls() {
                    SmtpTransport::starttls_relay(&self.config.host)
                        .map_err(Error::BuildTransportRelayError)?
                        .tls(Tls::Required(tls))
                } else {
                    SmtpTransport::relay(&self.config.host)
                        .map_err(Error::BuildTransportRelayError)?
                        .tls(Tls::Wrapper(tls))
                }
            } else {
                SmtpTransport::relay(&self.config.host)
                    .map_err(Error::BuildTransportRelayError)?
                    .tls(Tls::None)
            };

            self.transport = Some(
                builder
                    .port(self.config.port)
                    .credentials(self.config.credentials()?)
                    .build(),
            );

            Ok(self.transport.as_ref().unwrap())
        }
    }
}

impl<'a> Sender for Smtp<'a> {
    fn send(&mut self, config: &AccountConfig, msg: &Email) -> sender::Result<Vec<u8>> {
        let raw_msg = msg.into_sendable_msg(config)?.formatted();

        let envelope: lettre::address::Envelope = if let Some(cmd) =
            config.email_hooks.pre_send.as_deref()
        {
            let raw_msg = process::run(cmd, &raw_msg).map_err(Error::ExecutePreSendHookError)?;
            let parsed_mail = mailparse::parse_mail(&raw_msg).map_err(Error::ParseEmailError)?;
            Email::from_parsed_mail(parsed_mail, config)?.try_into()
        } else {
            msg.try_into()
        }?;

        self.transport()?
            .send_raw(&envelope, &raw_msg)
            .map_err(Error::SendError)?;
        Ok(raw_msg)
    }
}
