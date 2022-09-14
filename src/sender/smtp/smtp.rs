// himalaya-lib, a Rust library for email management.
// Copyright (C) 2022  soywod <clement.douin@posteo.net>
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

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

use crate::{config, email, process, sender, Config, Email, Sender, SmtpConfig};

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
    ConfigError(#[from] config::Error),
    #[error(transparent)]
    MsgError(#[from] email::Error),
}

pub type Result<T> = result::Result<T, Error>;

pub struct Smtp<'a> {
    config: &'a SmtpConfig,
    transport: Option<SmtpTransport>,
}

impl Smtp<'_> {
    fn transport(&mut self) -> Result<&SmtpTransport> {
        if let Some(ref transport) = self.transport {
            Ok(transport)
        } else {
            let builder = if self.config.starttls() {
                SmtpTransport::starttls_relay(&self.config.host)
            } else {
                SmtpTransport::relay(&self.config.host)
            }
            .map_err(Error::BuildTransportRelayError)?;

            let tls = TlsParameters::builder(self.config.host.to_owned())
                .dangerous_accept_invalid_hostnames(self.config.insecure())
                .dangerous_accept_invalid_certs(self.config.insecure())
                .build()
                .map_err(Error::BuildTlsParamsError)?;

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
    fn send(&mut self, config: &Config, msg: &Email) -> sender::Result<Vec<u8>> {
        let mut raw_msg = msg.into_sendable_msg(config)?.formatted();

        let envelope: lettre::address::Envelope = if let Some(cmd) =
            config.email_hooks()?.pre_send.as_deref()
        {
            for cmd in cmd.split('|') {
                raw_msg =
                    process::pipe(cmd.trim(), &raw_msg).map_err(Error::ExecutePreSendHookError)?;
            }
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

impl<'a> From<&'a SmtpConfig> for Smtp<'a> {
    fn from(config: &'a SmtpConfig) -> Self {
        Self {
            config,
            transport: None,
        }
    }
}
