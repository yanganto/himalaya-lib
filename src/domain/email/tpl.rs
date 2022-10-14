//! Module related to message template CLI.
//!
//! This module provides subcommands, arguments and a command matcher related to message template.

use lettre::{
    error::Error as LettreError,
    message::{Message, SinglePart},
};
use log::warn;
use mailparse::MailParseError;
use std::{result, string};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("cannot parse encrypted part of multipart")]
    ParseTplError(#[source] MailParseError),
    #[error("cannot compile template")]
    CompileTplError(#[source] LettreError),
    #[error("cannot decode compiled template using utf-8")]
    DecodeCompiledTplError(#[source] string::FromUtf8Error),
}

pub type Result<T> = result::Result<T, Error>;

pub fn compile(tpl: &[u8]) -> Result<Message> {
    let input = mailparse::parse_mail(tpl).map_err(Error::ParseTplError)?;
    let mut output = Message::builder();

    for header in input.get_headers() {
        output = match header.get_key().to_lowercase().as_str() {
            "message-id" => output.message_id(Some(header.get_value())),
            "in-reply-to" => output.in_reply_to(header.get_value()),
            "subject" => output.subject(header.get_value()),
            "from" => {
                if let Ok(header) = header.get_value().parse() {
                    output.from(header)
                } else {
                    warn!("cannot parse header From: {}", header.get_value());
                    output
                }
            }
            "to" => {
                if let Ok(header) = header.get_value().parse() {
                    output.to(header)
                } else {
                    warn!("cannot parse header To: {}", header.get_value());
                    output
                }
            }
            "reply-to" => {
                if let Ok(header) = header.get_value().parse() {
                    output.reply_to(header)
                } else {
                    warn!("cannot parse header Reply-To: {}", header.get_value());
                    output
                }
            }
            "cc" => {
                if let Ok(header) = header.get_value().parse() {
                    output.cc(header)
                } else {
                    warn!("cannot parse header Cc: {}", header.get_value());
                    output
                }
            }
            "bcc" => {
                if let Ok(header) = header.get_value().parse() {
                    output.bcc(header)
                } else {
                    warn!("cannot parse header Bcc: {}", header.get_value());
                    output
                }
            }
            _ => output,
        };
    }

    output
        .singlepart(SinglePart::plain(input.get_body().unwrap_or_default()))
        .map_err(Error::CompileTplError)
}

#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub struct TplOverride<'a> {
    pub subject: Option<&'a str>,
    pub from: Option<Vec<&'a str>>,
    pub to: Option<Vec<&'a str>>,
    pub cc: Option<Vec<&'a str>>,
    pub bcc: Option<Vec<&'a str>>,
    pub headers: Option<Vec<&'a str>>,
    pub body: Option<&'a str>,
    pub signature: Option<&'a str>,
}
