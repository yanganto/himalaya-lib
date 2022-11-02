//! Module related to message template CLI.
//!
//! This module provides subcommands, arguments and a command matcher related to message template.

use lettre::{
    error::Error as LettreError,
    message::{Message, SinglePart},
};
use log::warn;
use mailparse::MailParseError;
use std::{
    collections::{HashMap, HashSet},
    ops::{Deref, DerefMut},
    result, string,
};
use thiserror::Error;

type HeaderKey = String;
type PartMime = String;
type PartBody = String;

#[derive(Debug, Clone)]
enum HeaderVal {
    String(String),
    Addrs(Vec<String>),
}

impl Default for HeaderVal {
    fn default() -> Self {
        Self::String(String::default())
    }
}

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

#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct Tpl(String);

impl Deref for Tpl {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Tpl {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Tpl {
    pub fn push_header<V: AsRef<str>>(&mut self, header: &str, value: V) -> &mut Self {
        self.push_str(header);
        self.push_str(": ");
        self.push_str(value.as_ref());
        self.push_str("\n");
        self
    }

    pub fn compile(&self) -> Result<Message> {
        let input = mailparse::parse_mail(self.as_bytes()).map_err(Error::ParseTplError)?;
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

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum ShowTextPartStrategy {
    Raw,
    PlainOtherwiseHtml,
    HtmlOtherwisePlain,
}

impl Default for ShowTextPartStrategy {
    fn default() -> Self {
        Self::Raw
    }
}

// TODO: find how to use this struct. Maybe rename to
// ReadEmailOpts. For now it should be used only for reading. Get
// email => build Tpl based on those opts.
#[derive(Debug, Default, Clone)]
pub struct TplFromEmailOpts {
    show_headers: HashSet<HeaderKey>,
    show_text_part_strategy: ShowTextPartStrategy,
    show_text_parts_only: bool,
    sanitize_text_plain_parts: bool,
    sanitize_text_html_parts: bool,
}

impl TplFromEmailOpts {
    pub fn show_header<H: ToString>(mut self, header: H) -> Self {
        self.show_headers.insert(header.to_string());
        self
    }

    pub fn show_headers<S: ToString, B: Iterator<Item = S>>(mut self, headers: B) -> Self {
        let headers = headers
            .into_iter()
            .map(|header| header.to_string())
            .collect::<Vec<_>>();
        self.show_headers.extend(headers);
        self
    }

    pub fn hide_header<H: AsRef<str>>(mut self, header: H) -> Self {
        self.show_headers.remove(header.as_ref());
        self
    }

    pub fn show_text_parts_only(mut self) -> Self {
        self.show_text_parts_only = true;
        self
    }

    pub fn use_show_text_part_strategy(mut self, strategy: ShowTextPartStrategy) -> Self {
        self.show_text_part_strategy = strategy;
        self
    }

    pub fn sanitize_text_plain_parts(mut self) -> Self {
        self.sanitize_text_plain_parts = true;
        self
    }

    pub fn sanitize_text_html_parts(mut self) -> Self {
        self.sanitize_text_html_parts = true;
        self
    }

    pub fn sanitize_text_parts(self) -> Self {
        self.sanitize_text_plain_parts().sanitize_text_html_parts()
    }
}

#[derive(Debug, Default, Clone)]
pub struct TplBuilder {
    headers: HashMap<HeaderKey, HeaderVal>,
    headers_order: Vec<HeaderKey>,
    parts: HashMap<PartMime, PartBody>,
    parts_order: Vec<PartMime>,
}

impl TplBuilder {
    pub fn header<K: AsRef<str> + ToString, V: ToString>(mut self, key: K, val: V) -> Self {
        if let Some(prev_val) = self.headers.get_mut(key.as_ref()) {
            if let HeaderVal::String(ref mut prev_val) = prev_val {
                *prev_val = val.to_string();
            }
        } else {
            self.headers
                .insert(key.to_string(), HeaderVal::String(val.to_string()));
            self.headers_order.push(key.to_string());
        }
        self
    }

    pub fn header_addr<K: AsRef<str> + ToString, A: ToString>(mut self, key: K, addr: A) -> Self {
        if let Some(addrs) = self.headers.get_mut(key.as_ref()) {
            if let HeaderVal::Addrs(addrs) = addrs {
                addrs.push(addr.to_string())
            }
        } else {
            self.headers
                .insert(key.to_string(), HeaderVal::Addrs(vec![addr.to_string()]));
            self.headers_order.push(key.to_string());
        }
        self
    }

    pub fn subject<S: ToString>(self, subject: S) -> Self {
        self.header("Subject", subject)
    }

    pub fn from<A: ToString>(self, addr: A) -> Self {
        self.header_addr("From", addr)
    }

    pub fn to<A: ToString>(self, addr: A) -> Self {
        self.header_addr("To", addr)
    }

    pub fn cc<A: ToString>(self, addr: A) -> Self {
        self.header_addr("Cc", addr)
    }

    pub fn bcc<A: ToString>(self, addr: A) -> Self {
        self.header_addr("Bcc", addr)
    }

    pub fn part<M: AsRef<str> + ToString, P: ToString>(mut self, mime: M, part: P) -> Self {
        if let Some(prev_part) = self.parts.get_mut(mime.as_ref()) {
            *prev_part = part.to_string();
        } else {
            self.parts.insert(mime.to_string(), part.to_string());
            self.parts_order.push(mime.to_string());
        }
        self
    }

    pub fn text_plain_part<P: ToString>(self, part: P) -> Self {
        self.part("text/plain", part)
    }

    pub fn build(&self) -> Tpl {
        let mut tpl = Tpl::default();

        for key in &self.headers_order {
            if let Some(val) = self.headers.get(key) {
                match val {
                    HeaderVal::String(string) => tpl.push_header(key, string),
                    HeaderVal::Addrs(addrs) => tpl.push_header(key, addrs.join(", ")),
                };
            }
        }

        tpl.push_str("\n");

        if let Some(part) = self.parts.get("text/plain") {
            tpl.push_str(part)
        }

        // TODO: manage other mime parts

        tpl
    }
}

#[cfg(test)]
mod test_tpl_builder {
    use super::*;

    #[test]
    fn test_build() {
        let tpl = TplBuilder::default()
            .from("from")
            .to("")
            .cc("cc")
            .subject("subject")
            .cc("cc2")
            .text_plain_part("body\n")
            .bcc("bcc")
            .build();

        let expected_tpl = r#"
From: from
To: 
Cc: cc, cc2
Subject: subject
Bcc: bcc

body
"#;

        assert_eq!(expected_tpl.trim_start(), *tpl);
    }
}
