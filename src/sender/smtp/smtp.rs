use lettre::{
    self,
    transport::smtp::{
        client::{Tls, TlsParameters},
        SmtpTransport,
    },
    Transport,
};
use std::convert::TryInto;
use thiserror::Error;

use crate::{
    config::{Config, ConfigError},
    email::{self, Email},
    process::{self, ProcessError},
    Sender, SenderError,
};

use super::{SmtpConfig, SmtpConfigError};

#[derive(Debug, Error)]
pub enum SmtpError {
    #[error("cannot build smtp transport relay")]
    BuildTransportRelayError(#[source] lettre::transport::smtp::Error),
    #[error("cannot build smtp tls parameters")]
    BuildTlsParamsError(#[source] lettre::transport::smtp::Error),
    #[error("cannot parse email before sending")]
    ParseEmailError(#[source] mailparse::MailParseError),
    #[error("cannot send email")]
    SendError(#[source] lettre::transport::smtp::Error),
    #[error("cannot execute pre-send hook")]
    ExecutePreSendHookError(#[source] ProcessError),

    #[error(transparent)]
    SmtpConfigError(#[from] SmtpConfigError),
    #[error(transparent)]
    ConfigError(#[from] ConfigError),
    #[error(transparent)]
    MsgError(#[from] email::error::Error),
}

pub struct Smtp<'a> {
    config: &'a SmtpConfig,
    transport: Option<SmtpTransport>,
}

impl Smtp<'_> {
    fn transport(&mut self) -> Result<&SmtpTransport, SmtpError> {
        if let Some(ref transport) = self.transport {
            Ok(transport)
        } else {
            let builder = if self.config.starttls() {
                SmtpTransport::starttls_relay(&self.config.host)
            } else {
                SmtpTransport::relay(&self.config.host)
            }
            .map_err(SmtpError::BuildTransportRelayError)?;

            let tls = TlsParameters::builder(self.config.host.to_owned())
                .dangerous_accept_invalid_hostnames(self.config.insecure())
                .dangerous_accept_invalid_certs(self.config.insecure())
                .build()
                .map_err(SmtpError::BuildTlsParamsError)?;

            let tls = if self.config.starttls() {
                Tls::Required(tls)
            } else {
                Tls::Wrapper(tls)
            };

            self.transport = Some(
                builder
                    .tls(tls)
                    .port(self.config.port)
                    .credentials(self.config.credentials()?)
                    .build(),
            );

            Ok(self.transport.as_ref().unwrap())
        }
    }
}

impl Sender for Smtp<'_> {
    fn send(&mut self, config: &Config, msg: &Email) -> Result<Vec<u8>, SenderError> {
        let mut raw_msg = msg.into_sendable_msg(config)?.formatted();

        let envelope: lettre::address::Envelope =
            if let Some(cmd) = config.email_hooks()?.pre_send.as_deref() {
                for cmd in cmd.split('|') {
                    raw_msg = process::pipe(cmd.trim(), &raw_msg)
                        .map_err(SmtpError::ExecutePreSendHookError)?;
                }
                let parsed_mail =
                    mailparse::parse_mail(&raw_msg).map_err(SmtpError::ParseEmailError)?;
                Email::from_parsed_mail(parsed_mail, config)?.try_into()
            } else {
                msg.try_into()
            }?;

        self.transport()?
            .send_raw(&envelope, &raw_msg)
            .map_err(SmtpError::SendError)?;
        Ok(raw_msg)
    }
}

impl<'a> From<&'a SmtpConfig> for Smtp<'a> {
    fn from(config: &'a SmtpConfig) -> Self {
        Self {
            config,
            transport: None,
        }
    }
}
