use ammonia;
use mailparse::{parse_header, MailParseError};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    ops::{Deref, DerefMut},
    result, string,
};
use thiserror::Error;

use crate::{account, sanitize_text_plain_part, AccountConfig};

type HeaderKey = String;
type PartMime = String;
type PartBody = Vec<u8>;

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
pub enum Error {
    #[error("cannot parse encrypted part of multipart")]
    ParseTplError(#[source] MailParseError),
    #[error("cannot parse template header {1}")]
    ParseHeaderError(#[source] MailParseError, String),
    #[error("cannot decode compiled template using utf-8")]
    DecodeCompiledTplError(#[source] string::FromUtf8Error),

    #[error(transparent)]
    ConfigError(#[from] account::config::Error),

    #[error(transparent)]
    CompileTplError(#[from] mml::Error),
}

pub type Result<T> = result::Result<T, Error>;

#[derive(Debug, Default, Clone, Eq, PartialEq, Serialize, Deserialize)]
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

impl From<&str> for Tpl {
    fn from(tpl: &str) -> Self {
        Self(tpl.to_owned())
    }
}

impl From<String> for Tpl {
    fn from(tpl: String) -> Self {
        Self(tpl)
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

    pub fn compile(&self) -> Result<Vec<u8>> {
        Ok(mml::compile(&self.0)?)
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
    PlainOnly,
    HtmlOtherwisePlain,
    HtmlOnly,
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

impl Default for ShowHeaders {
    fn default() -> Self {
        Self::All
    }
}

impl ShowHeaders {
    pub fn contains(&self, key: &String) -> bool {
        match self {
            Self::All => true,
            Self::Only(headers) => headers.contains(key),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct TplBuilder {
    pub headers: HashMap<HeaderKey, HeaderVal>,
    pub headers_order: Vec<HeaderKey>,
    pub parts: HashMap<PartMime, PartBody>,
    pub parts_order: Vec<PartMime>,
    pub show_headers: ShowHeaders,
    pub show_text_part_strategy: ShowTextPartStrategy,
    pub show_text_parts_only: bool,
    pub sanitize_text_plain_parts: bool,
    pub sanitize_text_html_parts: bool,
}

impl TplBuilder {
    pub fn write(config: &AccountConfig) -> Result<Self> {
        let tpl = Self::default()
            .from(config.addr()?)
            .to("")
            .subject("")
            .text_plain_part(if let Some(ref signature) = config.signature()? {
                String::from("\n\n") + signature
            } else {
                String::new()
            });

        Ok(tpl)
    }

    pub fn some_headers<'a, H: IntoIterator<Item = &'a str>>(
        mut self,
        headers: Option<H>,
    ) -> Result<Self> {
        if let Some(headers) = headers {
            self = self.headers(headers)?;
        }

        Ok(self)
    }

    pub fn headers<'a, S: AsRef<str>, H: IntoIterator<Item = S>>(
        mut self,
        headers: H,
    ) -> Result<Self> {
        for header in headers {
            let (header, _) = parse_header(header.as_ref().as_bytes())
                .map_err(|err| Error::ParseHeaderError(err, header.as_ref().to_owned()))?;
            self = self.header(header.get_key(), header.get_value());
        }

        Ok(self)
    }

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

    pub fn in_reply_to<H: ToString>(self, header: H) -> Self {
        self.header("In-Reply-To", header)
    }

    pub fn subject<H: ToString>(self, header: H) -> Self {
        self.header("Subject", header)
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

    pub fn part<M: AsRef<str> + ToString, P: AsRef<[u8]>>(mut self, mime: M, part: P) -> Self {
        if let Some(prev_part) = self.parts.get_mut(mime.as_ref()) {
            *prev_part = part.as_ref().to_owned()
        } else {
            self.parts
                .insert(mime.to_string(), part.as_ref().to_owned());
            self.parts_order.push(mime.to_string());
        }
        self
    }

    pub fn text_plain_part<P: AsRef<[u8]>>(self, part: P) -> Self {
        self.part("text/plain", part)
    }

    pub fn text_html_part<P: AsRef<[u8]>>(self, part: P) -> Self {
        self.part("text/html", part)
    }

    pub fn some_text_plain_part<P: AsRef<[u8]>>(self, part: Option<P>) -> Self {
        if let Some(part) = part {
            self.text_plain_part(part)
        } else {
            self
        }
    }

    pub fn show_header<H: ToString>(mut self, header: H) -> Self {
        match self.show_headers {
            ShowHeaders::All => {
                self.show_headers = ShowHeaders::Only(vec![header.to_string()]);
            }
            ShowHeaders::Only(prev_headers) => {
                let mut set = prev_headers.clone();
                set.push(header.to_string());
                self.show_headers = ShowHeaders::Only(set);
            }
        };

        self
    }

    pub fn show_headers<S: ToString, B: IntoIterator<Item = S>>(mut self, headers: B) -> Self {
        let headers = headers
            .into_iter()
            .map(|header| header.to_string())
            .collect();

        match self.show_headers {
            ShowHeaders::All => {
                self.show_headers = ShowHeaders::Only(headers);
            }
            ShowHeaders::Only(prev_headers) => {
                let mut set = prev_headers.clone();
                set.extend(headers);
                self.show_headers = ShowHeaders::Only(set);
            }
        };

        self
    }

    pub fn show_all_headers(mut self) -> Self {
        self.show_headers = ShowHeaders::All;
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

    pub fn sanitize_text_plain_parts(mut self, sanitize: bool) -> Self {
        self.sanitize_text_plain_parts = sanitize;
        self
    }

    pub fn sanitize_text_html_parts(mut self, sanitize: bool) -> Self {
        self.sanitize_text_html_parts = sanitize;
        self
    }

    pub fn sanitize_text_parts(self, sanitize: bool) -> Self {
        self.sanitize_text_plain_parts(sanitize)
            .sanitize_text_html_parts(sanitize)
    }

    pub fn build(&self) -> Tpl {
        let mut tpl = Tpl::default();

        let headers_order = if let ShowHeaders::Only(headers) = &self.show_headers {
            headers
        } else {
            &self.headers_order
        };

        for key in headers_order {
            if !self.show_headers.contains(key) {
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
            let plain = String::from_utf8_lossy(plain).to_string();

            if self.sanitize_text_plain_parts {
                sanitize_text_plain_part(plain)
            } else {
                plain.to_owned()
            }
        });

        let html_part = self.parts.get("text/html").map(|html| {
            let html = String::from_utf8_lossy(html).to_string();

            if self.sanitize_text_html_parts {
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

        match self.show_text_part_strategy {
            ShowTextPartStrategy::PlainOtherwiseHtml => {
                if let Some(ref part) = plain_part.or(html_part) {
                    tpl.push_str(part)
                }
            }
            ShowTextPartStrategy::PlainOnly => {
                if let Some(ref part) = plain_part {
                    tpl.push_str(part)
                }
            }
            ShowTextPartStrategy::HtmlOtherwisePlain => {
                if let Some(ref part) = html_part.or(plain_part) {
                    tpl.push_str(part)
                }
            }
            ShowTextPartStrategy::HtmlOnly => {
                if let Some(ref part) = html_part {
                    tpl.push_str(part)
                }
            }
        }

        if !self.show_text_parts_only {
            // TODO: manage other mime parts, maybe to do in rust-mml
        }

        tpl
    }
}

#[cfg(test)]
mod tpl_builder {
    use concat_with::concat_line;

    use crate::{AccountConfig, TplBuilder};

    #[test]
    fn build() {
        let tpl = TplBuilder::default()
            .from("from")
            .to("")
            .cc("cc")
            .subject("subject")
            .cc("cc2")
            .text_plain_part("body")
            .bcc("bcc")
            .build();

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

    #[test]
    fn write() {
        let config = AccountConfig {
            email: "from@localhost".into(),
            ..AccountConfig::default()
        };

        let tpl = TplBuilder::write(&config).unwrap().build();

        let expected_tpl = concat_line!("From: from@localhost", "To: ", "Subject: ", "", "");

        assert_eq!(expected_tpl, *tpl);
    }

    #[test]
    fn write_with_signature() {
        let config = AccountConfig {
            email: "from@localhost".into(),
            signature: Some("Regards,".into()),
            ..AccountConfig::default()
        };

        let tpl = TplBuilder::write(&config).unwrap().build();

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

    #[test]
    fn to_write_tpl_builder_with_signature_delim() {
        let config = AccountConfig {
            email: "from@localhost".into(),
            signature_delim: Some("~~\n".into()),
            signature: Some("Regards,".into()),
            ..AccountConfig::default()
        };

        let tpl = TplBuilder::write(&config).unwrap().build();

        let expected_tpl = concat_line!(
            "From: from@localhost",
            "To: ",
            "Subject: ",
            "",
            "",
            "",
            "~~",
            "Regards,"
        );

        assert_eq!(expected_tpl, *tpl);
    }
}
