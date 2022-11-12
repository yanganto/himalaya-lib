//! Module related to message template CLI.
//!
//! This module provides subcommands, arguments and a command matcher related to message template.

use ammonia;
use lettre::{
    error::Error as LettreError,
    message::{Message, SinglePart},
};
use log::warn;
use mailparse::MailParseError;
use regex::Regex;
use std::{
    collections::{HashMap, HashSet},
    ops::{Deref, DerefMut},
    string,
};
use thiserror::Error;

use crate::{account, AccountConfig};

type HeaderKey = String;
type PartMime = String;
type PartBody = String;

#[derive(Debug, Clone)]
pub enum HeaderVal {
    String(String),
    Addrs(Vec<String>),
}

impl Default for HeaderVal {
    fn default() -> Self {
        Self::String(String::default())
    }
}

#[derive(Error, Debug)]
pub enum TplError {
    #[error("cannot parse encrypted part of multipart")]
    ParseTplError(#[source] MailParseError),
    #[error("cannot compile template")]
    CompileTplError(#[source] LettreError),
    #[error("cannot decode compiled template using utf-8")]
    DecodeCompiledTplError(#[source] string::FromUtf8Error),

    #[error(transparent)]
    ConfigError(#[from] account::config::Error),
}

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
    pub fn new(config: &AccountConfig) -> Result<Self, TplError> {
        let tpl = TplBuilder::default()
            .from(config.addr()?)
            .to("")
            .subject("")
            .text_plain_part(
                config
                    .signature()?
                    .map(|ref signature| String::from("\n\n") + signature)
                    .unwrap_or_default(),
            );

        Ok(tpl.build(TplBuilderOpts::default()))
    }

    pub fn push_header<V: AsRef<str>>(&mut self, header: &str, value: V) -> &mut Self {
        self.push_str(header);
        self.push_str(": ");
        self.push_str(value.as_ref());
        self.push_str("\n");
        self
    }

    pub fn compile(&self) -> Result<Message, TplError> {
        let input = mailparse::parse_mail(self.as_bytes()).map_err(TplError::ParseTplError)?;
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
            .map_err(TplError::CompileTplError)
    }
}

#[cfg(test)]
mod test_tpl {
    use concat_with::concat_line;

    use crate::{AccountConfig, Tpl};

    #[test]
    fn test_new_tpl() {
        let config = AccountConfig {
            email: "from@localhost".into(),
            ..AccountConfig::default()
        };

        let tpl = Tpl::new(&config).unwrap();

        let expected_tpl = concat_line!("From: from@localhost", "To: ", "Subject: ", "", "");

        assert_eq!(expected_tpl, *tpl);
    }

    #[test]
    fn test_new_tpl_with_signature() {
        let config = AccountConfig {
            email: "from@localhost".into(),
            signature: Some("Regards,".into()),
            ..AccountConfig::default()
        };

        let tpl = Tpl::new(&config).unwrap();

        let expected_tpl = concat_line!(
            "From: from@localhost",
            "To: ",
            "Subject: ",
            "",
            "",
            "",
            "-- ",
            "Regards,"
        );

        assert_eq!(expected_tpl, *tpl);
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
    PlainOtherwiseHtml,
    HtmlOtherwisePlain,
}

impl Default for ShowTextPartStrategy {
    fn default() -> Self {
        Self::PlainOtherwiseHtml
    }
}

#[derive(Debug, Clone)]
pub enum ShowHeaders {
    All,
    Only(Vec<HeaderKey>),
}

impl ShowHeaders {
    pub fn contains(&self, key: &String) -> bool {
        match self {
            Self::All => true,
            Self::Only(headers) => headers.contains(key),
        }
    }
}

// TODO: find how to use this struct. Maybe rename to
// ReadEmailOpts. For now it should be used only for reading. Get
// email => build Tpl based on those opts.
#[derive(Debug, Default, Clone)]
pub struct TplBuilderOpts {
    pub show_headers: Option<ShowHeaders>,
    pub show_text_part_strategy: Option<ShowTextPartStrategy>,
    pub show_text_parts_only: Option<bool>,
    pub sanitize_text_plain_parts: Option<bool>,
    pub sanitize_text_html_parts: Option<bool>,
}

impl TplBuilderOpts {
    pub const DEFAULT_SHOW_HEADERS: ShowHeaders = ShowHeaders::All;
    pub const DEFAULT_SHOW_TEXT_PART_STRATEGY: ShowTextPartStrategy =
        ShowTextPartStrategy::PlainOtherwiseHtml;
    pub const DEFAULT_SHOW_TEXT_PARTS_ONLY: bool = true;
    pub const DEFAULT_SANITIZE_TEXT_PLAIN_PARTS: bool = true;
    pub const DEFAULT_SANITIZE_TEXT_HTML_PARTS: bool = true;

    pub fn show_headers_or_default(&self) -> &ShowHeaders {
        self.show_headers
            .as_ref()
            .unwrap_or(&Self::DEFAULT_SHOW_HEADERS)
    }

    pub fn show_text_part_strategy_or_default(&self) -> &ShowTextPartStrategy {
        self.show_text_part_strategy
            .as_ref()
            .unwrap_or(&Self::DEFAULT_SHOW_TEXT_PART_STRATEGY)
    }

    pub fn show_text_parts_only_or_default(&self) -> bool {
        self.show_text_parts_only
            .unwrap_or(Self::DEFAULT_SHOW_TEXT_PARTS_ONLY)
    }

    pub fn sanitize_text_plain_parts_or_default(&self) -> bool {
        self.sanitize_text_plain_parts
            .unwrap_or(Self::DEFAULT_SANITIZE_TEXT_PLAIN_PARTS)
    }

    pub fn sanitize_text_html_parts_or_default(&self) -> bool {
        self.sanitize_text_html_parts
            .unwrap_or(Self::DEFAULT_SANITIZE_TEXT_HTML_PARTS)
    }

    pub fn show_header<H: ToString>(mut self, header: H) -> Self {
        match self.show_headers_or_default() {
            ShowHeaders::All => {
                self.show_headers = Some(ShowHeaders::Only(vec![header.to_string()]));
            }
            ShowHeaders::Only(prev_headers) => {
                let mut set = prev_headers.clone();
                set.push(header.to_string());
                self.show_headers = Some(ShowHeaders::Only(set));
            }
        };

        self
    }

    pub fn show_headers<S: ToString, B: Iterator<Item = S>>(mut self, headers: B) -> Self {
        let headers = headers
            .into_iter()
            .map(|header| header.to_string())
            .collect();

        match self.show_headers_or_default() {
            ShowHeaders::All => {
                self.show_headers = Some(ShowHeaders::Only(headers));
            }
            ShowHeaders::Only(prev_headers) => {
                let mut set = prev_headers.clone();
                set.extend(headers);
                self.show_headers = Some(ShowHeaders::Only(set));
            }
        };

        self
    }

    pub fn show_all_headers(mut self) -> Self {
        self.show_headers = Some(ShowHeaders::All);
        self
    }

    pub fn show_text_parts_only(mut self) -> Self {
        self.show_text_parts_only = Some(true);
        self
    }

    pub fn use_show_text_part_strategy(mut self, strategy: ShowTextPartStrategy) -> Self {
        self.show_text_part_strategy = Some(strategy);
        self
    }

    pub fn sanitize_text_plain_parts(mut self) -> Self {
        self.sanitize_text_plain_parts = Some(true);
        self
    }

    pub fn sanitize_text_html_parts(mut self) -> Self {
        self.sanitize_text_html_parts = Some(true);
        self
    }

    pub fn sanitize_text_parts(self) -> Self {
        self.sanitize_text_plain_parts().sanitize_text_html_parts()
    }
}

#[derive(Debug, Default, Clone)]
pub struct TplBuilder {
    pub headers: HashMap<HeaderKey, HeaderVal>,
    pub headers_order: Vec<HeaderKey>,
    pub parts: HashMap<PartMime, PartBody>,
    pub parts_order: Vec<PartMime>,
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

    pub fn build(&self, opts: TplBuilderOpts) -> Tpl {
        let mut tpl = Tpl::default();

        for key in &self.headers_order {
            if !opts.show_headers_or_default().contains(key) {
                continue;
            }

            if let Some(val) = self.headers.get(key) {
                match val {
                    HeaderVal::String(string) => tpl.push_header(key, string),
                    HeaderVal::Addrs(addrs) => tpl.push_header(key, addrs.join(", ")),
                };
            }
        }

        if !tpl.is_empty() {
            tpl.push_str("\n");
        }

        let plain_part = self.parts.get("text/plain").map(|plain| {
            if opts.sanitize_text_plain_parts_or_default() {
                return {
                    // merges new line chars
                    let sanitized_plain = Regex::new(r"(\r?\n\s*){2,}")
                        .unwrap()
                        .replace_all(&plain, "\n\n")
                        .to_string();
                    // replaces tabulations by spaces
                    let sanitized_plain = Regex::new(r"\t")
                        .unwrap()
                        .replace_all(&sanitized_plain, " ")
                        .to_string();
                    // merges spaces
                    let sanitized_plain = Regex::new(r" {2,}")
                        .unwrap()
                        .replace_all(&sanitized_plain, "  ")
                        .to_string();

                    sanitized_plain
                };
            };
            plain.to_owned()
        });

        let html_part = self.parts.get("text/html").map(|html| {
            if opts.sanitize_text_html_parts_or_default() {
                return {
                    // removes html markup
                    let sanitized_html = ammonia::Builder::new()
                        .tags(HashSet::default())
                        .clean(&html)
                        .to_string();
                    // merges new line chars
                    let sanitized_html = Regex::new(r"(\r?\n\s*){2,}")
                        .unwrap()
                        .replace_all(&sanitized_html, "\n\n")
                        .to_string();
                    // replaces tabulations and &npsp; by spaces
                    let sanitized_html = Regex::new(r"(\t|&nbsp;)")
                        .unwrap()
                        .replace_all(&sanitized_html, " ")
                        .to_string();
                    // merges spaces
                    let sanitized_html = Regex::new(r" {2,}")
                        .unwrap()
                        .replace_all(&sanitized_html, "  ")
                        .to_string();
                    // decodes html entities
                    let sanitized_html =
                        html_escape::decode_html_entities(&sanitized_html).to_string();

                    sanitized_html
                };
            };
            html.to_owned()
        });

        match opts.show_text_part_strategy_or_default() {
            ShowTextPartStrategy::PlainOtherwiseHtml => {
                if let Some(ref part) = plain_part.or(html_part) {
                    tpl.push_str(part)
                }
            }
            ShowTextPartStrategy::HtmlOtherwisePlain => {
                if let Some(ref part) = html_part.or(plain_part) {
                    tpl.push_str(part)
                }
            }
        }

        if !opts.show_text_parts_only_or_default() {
            // TODO: manage other mime parts
        }

        tpl
    }
}

#[cfg(test)]
mod test_tpl_builder {
    use concat_with::concat_line;

    use crate::{TplBuilder, TplBuilderOpts};

    #[test]
    fn test_build() {
        let tpl = TplBuilder::default()
            .from("from")
            .to("")
            .cc("cc")
            .subject("subject")
            .cc("cc2")
            .text_plain_part("body")
            .bcc("bcc")
            .build(TplBuilderOpts::default());

        let expected_tpl = concat_line!(
            "From: from",
            "To: ",
            "Cc: cc, cc2",
            "Subject: subject",
            "Bcc: bcc",
            "",
            "body",
        );

        assert_eq!(expected_tpl, *tpl);
    }
}
