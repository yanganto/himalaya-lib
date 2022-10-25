use chrono::{DateTime, Local, TimeZone, Utc};
use convert_case::{Case, Casing};
use lettre::message::{header::ContentType, Attachment, Mailbox, Mailboxes, MultiPart, SinglePart};
use log::{info, trace, warn};
use mailparse::{MailHeaderMap, MailParseError, ParsedMail};
use std::{
    borrow::Cow,
    cell::RefCell,
    collections::HashMap,
    convert::TryInto,
    env::{self, temp_dir},
    fmt::Debug,
    fs, io,
    path::PathBuf,
    result,
};
use thiserror::Error;
use tree_magic;
use uuid::Uuid;

use crate::{
    account, from_addrs_to_sendable_addrs, from_addrs_to_sendable_mbox, from_slice_to_addrs,
    AccountConfig, Addr, Addrs, BinaryPart, Part, Parts, PartsReaderOptions, TextPlainPart, Tpl,
    TplOverride, DEFAULT_SIGNATURE_DELIM,
};

use super::parts::PartsWrapper;

#[derive(Error, Debug)]
pub enum Error {
    #[error("cannot expand attachment path {1}")]
    ExpandAttachmentPathError(#[source] shellexpand::LookupError<env::VarError>, String),
    #[error("cannot read attachment at {1}")]
    ReadAttachmentError(#[source] io::Error, PathBuf),
    #[error("cannot parse template")]
    ParseTplError(#[source] mailparse::MailParseError),
    #[error("cannot parse content type of attachment {1}")]
    ParseAttachmentContentTypeError(#[source] lettre::message::header::ContentTypeErr, String),
    #[error("cannot write temporary multipart on the disk")]
    WriteTmpMultipartError(#[source] io::Error),
    #[error("cannot write temporary multipart on the disk")]
    BuildSendableMsgError(#[source] lettre::error::Error),
    #[error("cannot parse {1} value: {2}")]
    ParseHeaderError(#[source] mailparse::MailParseError, String, String),
    #[error("cannot build envelope")]
    BuildEnvelopeError(#[source] lettre::error::Error),
    #[error("cannot get file name of attachment {0}")]
    GetAttachmentFilenameError(PathBuf),
    #[error("cannot parse recipient")]
    ParseRecipientError,
    #[error("cannot parse email from raw data")]
    ParseRawEmailError(#[source] mailparse::MailParseError),
    #[error("cannot parse email body")]
    ParseBodyError(#[source] MailParseError),
    #[error("cannot parse email from raw data")]
    ParseRawEmailEmptyError,

    #[error("cannot parse message or address")]
    ParseAddressError(#[from] lettre::address::AddressError),

    #[error(transparent)]
    ConfigError(#[from] account::config::Error),

    #[error("cannot get content type of multipart")]
    GetMultipartContentTypeError,
    #[error("cannot find encrypted part of multipart")]
    GetEncryptedPartMultipartError,
    #[error("cannot parse encrypted part of multipart")]
    ParseEncryptedPartError(#[source] mailparse::MailParseError),
    #[error("cannot get body from encrypted part")]
    GetEncryptedPartBodyError(#[source] mailparse::MailParseError),
    #[error("cannot write encrypted part to temporary file")]
    WriteEncryptedPartBodyError(#[source] io::Error),
    #[error("cannot write encrypted part to temporary file")]
    DecryptPartError(#[source] account::config::Error),

    #[error("cannot delete local draft: {1}")]
    DeleteLocalDraftError(#[source] io::Error, PathBuf),
}

pub type Result<T> = result::Result<T, Error>;

/// Representation of a message.
#[derive(Debug, Clone, Default)]
pub struct Email {
    pub id: u32,
    pub subject: String,
    pub from: Option<Addrs>,
    pub reply_to: Option<Addrs>,
    pub to: Option<Addrs>,
    pub cc: Option<Addrs>,
    pub bcc: Option<Addrs>,
    pub in_reply_to: Option<String>,
    pub message_id: Option<String>,
    pub headers: HashMap<String, String>,
    pub date: Option<DateTime<Local>>,
    pub parts: Parts,
    pub encrypt: bool,
    pub raw: Vec<u8>,
}

impl Email {
    pub fn attachments(&self) -> Vec<BinaryPart> {
        self.parts
            .iter()
            .filter_map(|part| match part {
                Part::Binary(part) => Some(part.to_owned()),
                _ => None,
            })
            .collect()
    }

    pub fn into_reply(mut self, all: bool, config: &AccountConfig) -> Result<Self> {
        let account_addr = config.address()?;

        // In-Reply-To
        self.in_reply_to = self.message_id.to_owned();

        // Message-Id
        self.message_id = None;

        // To
        let addrs = self
            .reply_to
            .as_deref()
            .or_else(|| self.from.as_deref())
            .map(|addrs| {
                addrs.iter().cloned().filter(|addr| match addr {
                    Addr::Group(_) => false,
                    Addr::Single(a) => match &account_addr {
                        Addr::Group(_) => false,
                        Addr::Single(b) => a.addr != b.addr,
                    },
                })
            });
        if all {
            self.to = addrs.map(|addrs| addrs.collect::<Vec<_>>().into());
        } else {
            self.to = addrs
                .and_then(|mut addrs| addrs.next())
                .map(|addr| vec![addr].into());
        }

        // Cc
        self.cc = if all {
            self.cc.as_deref().map(|addrs| {
                addrs
                    .iter()
                    .cloned()
                    .filter(|addr| match addr {
                        Addr::Group(_) => false,
                        Addr::Single(a) => match &account_addr {
                            Addr::Group(_) => false,
                            Addr::Single(b) => a.addr != b.addr,
                        },
                    })
                    .collect::<Vec<_>>()
                    .into()
            })
        } else {
            None
        };

        // Bcc
        self.bcc = None;

        // Body
        let plain_content = {
            let date = self
                .date
                .as_ref()
                .map(|date| date.format("%d %b %Y, at %H:%M (%z)").to_string())
                .unwrap_or_else(|| "unknown date".into());
            let sender = self
                .reply_to
                .as_ref()
                .or_else(|| self.from.as_ref())
                .and_then(|addrs| addrs.clone().extract_single_info())
                .map(|addr| addr.display_name.clone().unwrap_or_else(|| addr.addr))
                .unwrap_or_else(|| "unknown sender".into());
            let mut content = format!("\n\nOn {}, {} wrote:\n", date, sender);

            let mut glue = "";
            for line in self
                .parts
                .to_readable(PartsReaderOptions::default())
                .trim()
                .lines()
            {
                if line == DEFAULT_SIGNATURE_DELIM {
                    break;
                }
                content.push_str(glue);
                content.push('>');
                content.push_str(if line.starts_with('>') { "" } else { " " });
                content.push_str(line);
                glue = "\n";
            }

            content
        };

        self.parts = Parts(vec![Part::new_text_plain(plain_content)]);

        // Subject
        if !self.subject.starts_with("Re:") {
            self.subject = format!("Re: {}", self.subject);
        }

        // From
        self.from = Some(vec![account_addr.clone()].into());

        Ok(self)
    }

    pub fn into_forward(mut self, config: &AccountConfig) -> Result<Self> {
        let account_addr = config.address()?;

        let prev_subject = self.subject.to_owned();
        let prev_date = self.date.to_owned();
        let prev_from = self.reply_to.to_owned().or_else(|| self.from.to_owned());
        let prev_to = self.to.to_owned();

        // Message-Id
        self.message_id = None;

        // In-Reply-To
        self.in_reply_to = None;

        // From
        self.from = Some(vec![account_addr].into());

        // To
        self.to = Some(vec![].into());

        // Cc
        self.cc = None;

        // Bcc
        self.bcc = None;

        // Subject
        if !self.subject.starts_with("Fwd:") {
            self.subject = format!("Fwd: {}", self.subject);
        }

        // Body
        let mut content = String::default();
        content.push_str("\n\n-------- Forwarded Message --------\n");
        content.push_str(&format!("Subject: {}\n", prev_subject));
        if let Some(date) = prev_date {
            content.push_str(&format!("Date: {}\n", date.to_rfc2822()));
        }
        if let Some(addrs) = prev_from.as_ref() {
            content.push_str("From: ");
            content.push_str(&addrs.to_string());
            content.push('\n');
        }
        if let Some(addrs) = prev_to.as_ref() {
            content.push_str("To: ");
            content.push_str(&addrs.to_string());
            content.push('\n');
        }
        content.push('\n');
        content.push_str(&self.parts.to_readable(PartsReaderOptions::default()));
        self.parts
            .replace_text_plain_parts_with(TextPlainPart { content });

        Ok(self)
    }

    pub fn encrypt(mut self, encrypt: bool) -> Self {
        self.encrypt = encrypt;
        self
    }

    pub fn add_attachments(mut self, attachments_paths: Vec<&str>) -> Result<Self> {
        for path in attachments_paths {
            let path = shellexpand::full(path)
                .map_err(|err| Error::ExpandAttachmentPathError(err, path.to_owned()))?;
            let path = PathBuf::from(path.to_string());
            let filename: String = path
                .file_name()
                .ok_or_else(|| Error::GetAttachmentFilenameError(path.to_owned()))?
                .to_string_lossy()
                .into();
            let content =
                fs::read(&path).map_err(|err| Error::ReadAttachmentError(err, path.to_owned()))?;
            let mime = tree_magic::from_u8(&content);

            self.parts.push(Part::Binary(BinaryPart {
                filename,
                mime,
                content,
            }))
        }

        Ok(self)
    }

    pub fn merge_with(&mut self, email: Email) {
        self.from = email.from;
        self.reply_to = email.reply_to;
        self.to = email.to;
        self.cc = email.cc;
        self.bcc = email.bcc;
        self.subject = email.subject;

        if email.message_id.is_some() {
            self.message_id = email.message_id;
        }

        if email.in_reply_to.is_some() {
            self.in_reply_to = email.in_reply_to;
        }

        for part in email.parts.0.into_iter() {
            match part {
                Part::Binary(_) => self.parts.push(part),
                Part::TextPlain(_) => {
                    self.parts.retain(|p| !matches!(p, Part::TextPlain(_)));
                    self.parts.push(part);
                }
                Part::TextHtml(_) => {
                    self.parts.retain(|p| !matches!(p, Part::TextHtml(_)));
                    self.parts.push(part);
                }
            }
        }
    }

    pub fn to_tpl(&self, opts: TplOverride, config: &AccountConfig) -> Result<String> {
        let account_addr: Addrs = vec![config.address()?].into();
        let mut tpl = String::default();

        tpl.push_str("Content-Type: text/plain; charset=utf-8\n");

        if let Some(in_reply_to) = self.in_reply_to.as_ref() {
            tpl.push_str(&format!("In-Reply-To: {}\n", in_reply_to))
        }

        // From
        tpl.push_str(&format!(
            "From: {}\n",
            opts.from
                .map(|addrs| addrs.join(", "))
                .unwrap_or_else(|| account_addr.to_string())
        ));

        // To
        tpl.push_str(&format!(
            "To: {}\n",
            opts.to
                .map(|addrs| addrs.join(", "))
                .or_else(|| self.to.clone().map(|addrs| addrs.to_string()))
                .unwrap_or_default()
        ));

        // Cc
        if let Some(addrs) = opts
            .cc
            .map(|addrs| addrs.join(", "))
            .or_else(|| self.cc.clone().map(|addrs| addrs.to_string()))
        {
            tpl.push_str(&format!("Cc: {}\n", addrs));
        }

        // Bcc
        if let Some(addrs) = opts
            .bcc
            .map(|addrs| addrs.join(", "))
            .or_else(|| self.bcc.clone().map(|addrs| addrs.to_string()))
        {
            tpl.push_str(&format!("Bcc: {}\n", addrs));
        }

        // Subject
        tpl.push_str(&format!(
            "Subject: {}\n",
            opts.subject.unwrap_or(&self.subject)
        ));

        // Headers <=> body separator
        tpl.push('\n');

        // Body
        if let Some(body) = opts.body {
            tpl.push_str(body);
        } else {
            tpl.push_str(&self.parts.to_readable(PartsReaderOptions::default()))
        }

        // Signature
        if let Some(sig) = opts.signature {
            tpl.push_str("\n\n");
            tpl.push_str(sig);
        } else if let Some(ref sig) = config.signature()? {
            tpl.push_str("\n\n");
            tpl.push_str(sig);
        }

        tpl.push('\n');

        trace!("template: {:?}", tpl);
        Ok(tpl)
    }

    pub fn from_tpl(tpl: &str) -> Result<Self> {
        info!("begin: building message from template");
        trace!("template: {:?}", tpl);

        let parsed_mail = mailparse::parse_mail(tpl.as_bytes()).map_err(Error::ParseTplError)?;

        info!("end: building message from template");
        Self::from_parsed_mail(parsed_mail, &AccountConfig::default())
    }

    pub fn into_sendable(&self, config: &AccountConfig) -> Result<lettre::Message> {
        let mut msg_builder = lettre::Message::builder()
            .message_id(self.message_id.to_owned())
            .subject(self.subject.to_owned());

        if let Some(id) = self.in_reply_to.as_ref() {
            msg_builder = msg_builder.in_reply_to(id.to_owned());
        };

        if let Some(addrs) = self.from.as_ref() {
            for addr in from_addrs_to_sendable_mbox(addrs)? {
                msg_builder = msg_builder.from(addr)
            }
        };

        if let Some(addrs) = self.to.as_ref() {
            for addr in from_addrs_to_sendable_mbox(addrs)? {
                msg_builder = msg_builder.to(addr)
            }
        };

        if let Some(addrs) = self.reply_to.as_ref() {
            for addr in from_addrs_to_sendable_mbox(addrs)? {
                msg_builder = msg_builder.reply_to(addr)
            }
        };

        if let Some(addrs) = self.cc.as_ref() {
            for addr in from_addrs_to_sendable_mbox(addrs)? {
                msg_builder = msg_builder.cc(addr)
            }
        };

        if let Some(addrs) = self.bcc.as_ref() {
            for addr in from_addrs_to_sendable_mbox(addrs)? {
                msg_builder = msg_builder.bcc(addr)
            }
        };

        let mut multipart = {
            let mut multipart = MultiPart::mixed().singlepart(SinglePart::plain(
                self.parts.to_readable(PartsReaderOptions::default()),
            ));
            for part in self.attachments() {
                multipart = multipart.singlepart(Attachment::new(part.filename.clone()).body(
                    part.content,
                    part.mime.parse().map_err(|err| {
                        Error::ParseAttachmentContentTypeError(err, part.filename)
                    })?,
                ))
            }
            multipart
        };

        if self.encrypt {
            let multipart_buffer = temp_dir().join(Uuid::new_v4().to_string());
            fs::write(multipart_buffer.clone(), multipart.formatted())
                .map_err(Error::WriteTmpMultipartError)?;
            let addr = self
                .to
                .as_ref()
                .and_then(|addrs| addrs.clone().extract_single_info())
                .map(|addr| addr.addr)
                .ok_or_else(|| Error::ParseRecipientError)?;
            let encrypted_multipart = config.pgp_encrypt_file(&addr, multipart_buffer.clone())?;
            trace!("encrypted multipart: {:#?}", encrypted_multipart);
            multipart = MultiPart::encrypted(String::from("application/pgp-encrypted"))
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::parse("application/pgp-encrypted").unwrap())
                        .body(String::from("Version: 1")),
                )
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::parse("application/octet-stream").unwrap())
                        .body(encrypted_multipart),
                )
        }

        msg_builder
            .multipart(multipart)
            .map_err(Error::BuildSendableMsgError)
    }

    pub fn from_parsed_mail(
        parsed_mail: mailparse::ParsedMail<'_>,
        config: &AccountConfig,
    ) -> Result<Self> {
        trace!(">> build message from parsed mail");
        trace!("parsed mail: {:?}", parsed_mail);

        let mut email = Email::default();
        for header in parsed_mail.get_headers() {
            trace!(">> parse header {:?}", header);

            let key = header.get_key();
            trace!("header key: {:?}", key);

            let val = header.get_value();
            trace!("header value: {:?}", val);

            match key.to_lowercase().as_str() {
                "message-id" => email.message_id = Some(val),
                "in-reply-to" => email.in_reply_to = Some(val),
                "subject" => {
                    email.subject = val;
                }
                "date" => match mailparse::dateparse(&val) {
                    Ok(timestamp) => {
                        email.date = Some(Utc.timestamp(timestamp, 0).with_timezone(&Local))
                    }
                    Err(err) => {
                        warn!("cannot parse message date {:?}, skipping it", val);
                        warn!("{}", err);
                    }
                },
                "from" => {
                    email.from = from_slice_to_addrs(&val)
                        .map_err(|err| Error::ParseHeaderError(err, key, val.to_owned()))?
                }
                "to" => {
                    email.to = from_slice_to_addrs(&val)
                        .map_err(|err| Error::ParseHeaderError(err, key, val.to_owned()))?
                }
                "reply-to" => {
                    email.reply_to = from_slice_to_addrs(&val)
                        .map_err(|err| Error::ParseHeaderError(err, key, val.to_owned()))?
                }
                "cc" => {
                    email.cc = from_slice_to_addrs(&val)
                        .map_err(|err| Error::ParseHeaderError(err, key, val.to_owned()))?
                }
                "bcc" => {
                    email.bcc = from_slice_to_addrs(&val)
                        .map_err(|err| Error::ParseHeaderError(err, key, val.to_owned()))?
                }
                key => {
                    email.headers.insert(key.to_lowercase(), val);
                }
            }
            trace!("<< parse header");
        }

        email.parts = Parts::from_parsed_mail(config, &parsed_mail)?;
        trace!("message: {:?}", email);

        info!("<< build message from parsed mail");
        Ok(email)
    }

    /// Transforms a message into a readable string. A readable
    /// message is like a template, except that:
    ///  - headers part is customizable (can be omitted if empty filter given in argument)
    ///  - body type is customizable (plain or html)
    pub fn to_readable(
        &self,
        config: &AccountConfig,
        opts: PartsReaderOptions,
        headers: Vec<&str>,
    ) -> Result<String> {
        let mut all_headers = vec![];
        for h in config.email_reading_headers().iter() {
            let h = h.to_lowercase();
            if !all_headers.contains(&h) {
                all_headers.push(h)
            }
        }
        for h in headers.iter() {
            let h = h.to_lowercase();
            if !all_headers.contains(&h) {
                all_headers.push(h)
            }
        }

        let mut readable_email = String::new();
        for h in all_headers {
            match h.as_str() {
                "message-id" => match self.message_id {
                    Some(ref message_id) if !message_id.is_empty() => {
                        readable_email.push_str(&format!("Message-Id: {}\n", message_id));
                    }
                    _ => (),
                },
                "in-reply-to" => match self.in_reply_to {
                    Some(ref in_reply_to) if !in_reply_to.is_empty() => {
                        readable_email.push_str(&format!("In-Reply-To: {}\n", in_reply_to));
                    }
                    _ => (),
                },
                "subject" => {
                    readable_email.push_str(&format!("Subject: {}\n", self.subject));
                }
                "date" => {
                    if let Some(ref date) = self.date {
                        readable_email.push_str(&format!("Date: {}\n", date.to_rfc2822()));
                    }
                }
                "from" => match self.from {
                    Some(ref addrs) if !addrs.is_empty() => {
                        readable_email.push_str(&format!("From: {}\n", addrs));
                    }
                    _ => (),
                },
                "to" => match self.to {
                    Some(ref addrs) if !addrs.is_empty() => {
                        readable_email.push_str(&format!("To: {}\n", addrs));
                    }
                    _ => (),
                },
                "reply-to" => match self.reply_to {
                    Some(ref addrs) if !addrs.is_empty() => {
                        readable_email.push_str(&format!("Reply-To: {}\n", addrs));
                    }
                    _ => (),
                },
                "cc" => match self.cc {
                    Some(ref addrs) if !addrs.is_empty() => {
                        readable_email.push_str(&format!("Cc: {}\n", addrs));
                    }
                    _ => (),
                },
                "bcc" => match self.bcc {
                    Some(ref addrs) if !addrs.is_empty() => {
                        readable_email.push_str(&format!("Bcc: {}\n", addrs));
                    }
                    _ => (),
                },
                key => match self.headers.get(key) {
                    Some(ref val) if !val.is_empty() => {
                        readable_email.push_str(&format!(
                            "{}: {}\n",
                            key.to_case(Case::Train),
                            val
                        ));
                    }
                    _ => (),
                },
            };
        }

        if !readable_email.is_empty() {
            readable_email.push_str("\n");
        }

        readable_email.push_str(&self.parts.to_readable(opts));

        Ok(readable_email)
    }
}

#[cfg(feature = "smtp-sender")]
impl TryInto<lettre::address::Envelope> for Email {
    type Error = Error;

    fn try_into(self) -> Result<lettre::address::Envelope> {
        (&self).try_into()
    }
}

#[cfg(feature = "smtp-sender")]
impl TryInto<lettre::address::Envelope> for &Email {
    type Error = Error;

    fn try_into(self) -> Result<lettre::address::Envelope> {
        let from = match self
            .from
            .as_ref()
            .and_then(|addrs| addrs.clone().extract_single_info())
        {
            Some(addr) => addr.addr.parse().map(Some),
            None => Ok(None),
        }?;
        let to = self
            .to
            .as_ref()
            .map(from_addrs_to_sendable_addrs)
            .unwrap_or(Ok(vec![]))?;
        Ok(lettre::address::Envelope::new(from, to).map_err(Error::BuildEnvelopeError)?)
    }
}

#[derive(Debug)]
pub struct EmailParsed<'a> {
    bytes: Cow<'a, [u8]>,
    parsed: RefCell<Option<ParsedMail<'a>>>,
}

impl<'a> EmailParsed<'a> {
    pub fn parsed(&'a self) -> Result<ParsedMail<'a>> {
        let mut parsed = self.parsed.borrow_mut();
        if parsed.is_none() {
            *parsed = Some(mailparse::parse_mail(&self.bytes).map_err(Error::ParseRawEmailError)?);
        }
        Ok(parsed.take().unwrap())
    }

    pub fn to_reply_tpl(&'a self, config: &AccountConfig, all: bool) -> Result<Tpl> {
        let parsed = self.parsed()?;
        let headers = parsed.get_headers();
        let sender = config.addr()?;
        let mut tpl = Tpl::default();

        // From

        tpl.push_header("From", &sender.to_string());

        // To

        let mut to = Mailboxes::new();
        let reply_to = headers.get_all_values("Reply-To");
        let from = headers.get_all_values("From");

        let mut to_iter = if !reply_to.is_empty() {
            reply_to.iter()
        } else {
            from.iter()
        };

        if let Some(addr) = to_iter.next() {
            to.push((*addr).parse()?)
        }

        if all {
            for addr in to_iter {
                to.push((*addr).parse()?);
            }
        }

        tpl.push_header("To", &to.to_string());

        // In-Reply-To

        if let Some(ref message_id) = headers.get_first_value("Message-Id") {
            tpl.push_header("In-Reply-To", message_id);
        }

        // Cc

        if all {
            let mut cc = Mailboxes::new();

            for addr in headers.get_all_values("Cc") {
                let addr: Mailbox = addr.parse()?;
                if addr.email != sender.email {
                    cc.push(addr);
                }
            }
            tpl.push_header("Cc", cc.to_string());
        }

        // Subject

        if let Some(ref subject) = headers.get_first_value("Subject") {
            tpl.push_header("Subject", String::from("Re: ") + subject);
        }

        // Body

        tpl.push_str("\n");
        let text_bodies = PartsWrapper::new(self).concat_text_plain_bodies().unwrap();

        let mut glue = "";
        for line in text_bodies.lines() {
            // removes existing signature from the original body
            if line[..] == DEFAULT_SIGNATURE_DELIM[0..3] {
                break;
            }

            tpl.push_str(glue);
            tpl.push('>');
            if !line.starts_with('>') {
                tpl.push_str(" ")
            }
            tpl.push_str(line);

            glue = "\n";
        }

        // Signature

        if let Some(ref sig) = config.signature()? {
            tpl.push_str("\n\n");
            tpl.push_str(sig);
        }

        Ok(tpl)
    }
}

impl<'a> From<ParsedMail<'a>> for EmailParsed<'a> {
    fn from(parsed: ParsedMail<'a>) -> Self {
        Self {
            bytes: Cow::Borrowed(parsed.raw_bytes),
            parsed: RefCell::new(Some(parsed)),
        }
    }
}

impl<'a> From<Vec<u8>> for EmailParsed<'a> {
    fn from(vec: Vec<u8>) -> Self {
        Self {
            bytes: Cow::Owned(vec),
            parsed: RefCell::new(None),
        }
    }
}

impl<'a> From<&'a [u8]> for EmailParsed<'a> {
    fn from(bytes: &'a [u8]) -> Self {
        Self {
            bytes: Cow::Borrowed(bytes),
            parsed: RefCell::new(None),
        }
    }
}

impl<'a> From<&'a str> for EmailParsed<'a> {
    fn from(str: &'a str) -> Self {
        str.as_bytes().into()
    }
}

#[cfg(test)]
mod test_email_to_reply_tpl {
    use crate::{AccountConfig, EmailParsed};

    #[test]
    fn test_empty_config() {
        let config = AccountConfig {
            email: "to@localhost".into(),
            ..AccountConfig::default()
        };

        let email = EmailParsed::from(concat!(
            "From: from@localhost\n",
            "To: to@localhost\n",
            "Subject: subject\n",
            "\n",
            "Hello!\n",
            "\n",
            "-- \n",
            "From regards,"
        ));

        let expected_tpl = concat!(
            "From: to@localhost\n",
            "To: from@localhost\n",
            "Subject: Re: subject\n",
            "\n",
            "> Hello!\n",
            "> "
        );

        assert_eq!(expected_tpl, email.to_reply_tpl(&config, false).unwrap().0);
    }

    #[test]
    fn test_with_display_name_and_signature() {
        let config = AccountConfig {
            email: "to@localhost".into(),
            display_name: Some("Tȯ".into()),
            signature: Some("To regards,".into()),
            ..AccountConfig::default()
        };

        let email = EmailParsed::from(concat!(
            "From: from@localhost\n",
            "To: to@localhost\n",
            "Subject: subject\n",
            "\n",
            "Hello!\n",
            "\n",
            "-- \n",
            "From Regards,"
        ));

        let expected_tpl = concat!(
            "From: Tȯ <to@localhost>\n",
            "To: from@localhost\n",
            "Subject: Re: subject\n",
            "\n",
            "> Hello!\n",
            "> \n",
            "\n",
            "-- \n",
            "To regards,"
        );

        assert_eq!(expected_tpl, email.to_reply_tpl(&config, false).unwrap().0);
    }
}

#[cfg(test)]
mod tests {
    use mailparse::SingleInfo;
    use std::iter::FromIterator;

    use crate::{email::Addr, AccountConfig};

    use super::*;

    #[test]
    fn test_into_reply() {
        let config = AccountConfig {
            display_name: Some("Test".into()),
            email: "test-account@local".into(),
            ..AccountConfig::default()
        };

        // Checks that:
        //  - "message_id" moves to "in_reply_to"
        //  - "subject" starts by "Re: "
        //  - "to" is replaced by "from"
        //  - "from" is replaced by the address from the account config

        let email = Email {
            message_id: Some("email-id".into()),
            subject: "subject".into(),
            from: Some(
                vec![Addr::Single(SingleInfo {
                    addr: "test-sender@local".into(),
                    display_name: None,
                })]
                .into(),
            ),
            ..Email::default()
        }
        .into_reply(false, &config)
        .unwrap();

        assert_eq!(email.message_id, None);
        assert_eq!(email.in_reply_to.unwrap(), "email-id");
        assert_eq!(email.subject, "Re: subject");
        assert_eq!(
            email.from.unwrap().to_string(),
            "\"Test\" <test-account@local>"
        );
        assert_eq!(email.to.unwrap().to_string(), "test-sender@local");

        // Checks that:
        //  - "subject" does not contains additional "Re: "
        //  - "to" is replaced by reply_to
        //  - "to" contains one address when "all" is false
        //  - "cc" are empty when "all" is false

        let email = Email {
            subject: "Re: subject".into(),
            from: Some(
                vec![Addr::Single(SingleInfo {
                    addr: "test-sender@local".into(),
                    display_name: None,
                })]
                .into(),
            ),
            reply_to: Some(
                vec![
                    Addr::Single(SingleInfo {
                        addr: "test-sender-to-reply@local".into(),
                        display_name: Some("Sender".into()),
                    }),
                    Addr::Single(SingleInfo {
                        addr: "test-sender-to-reply-2@local".into(),
                        display_name: Some("Sender 2".into()),
                    }),
                ]
                .into(),
            ),
            cc: Some(
                vec![Addr::Single(SingleInfo {
                    addr: "test-cc@local".into(),
                    display_name: None,
                })]
                .into(),
            ),
            ..Email::default()
        }
        .into_reply(false, &config)
        .unwrap();

        assert_eq!(email.subject, "Re: subject");
        assert_eq!(
            email.to.unwrap().to_string(),
            "\"Sender\" <test-sender-to-reply@local>"
        );
        assert_eq!(email.cc, None);

        // Checks that:
        //  - "to" contains all addresses except for the sender when "all" is true
        //  - "cc" contains all addresses except for the sender when "all" is true

        let email = Email {
            from: Some(
                vec![
                    Addr::Single(SingleInfo {
                        addr: "test-sender-1@local".into(),
                        display_name: Some("Sender 1".into()),
                    }),
                    Addr::Single(SingleInfo {
                        addr: "test-sender-2@local".into(),
                        display_name: Some("Sender 2".into()),
                    }),
                    Addr::Single(SingleInfo {
                        addr: "test-account@local".into(),
                        display_name: Some("Test".into()),
                    }),
                ]
                .into(),
            ),
            cc: Some(
                vec![
                    Addr::Single(SingleInfo {
                        addr: "test-sender-1@local".into(),
                        display_name: Some("Sender 1".into()),
                    }),
                    Addr::Single(SingleInfo {
                        addr: "test-sender-2@local".into(),
                        display_name: Some("Sender 2".into()),
                    }),
                    Addr::Single(SingleInfo {
                        addr: "test-account@local".into(),
                        display_name: None,
                    }),
                ]
                .into(),
            ),
            ..Email::default()
        }
        .into_reply(true, &config)
        .unwrap();

        assert_eq!(
            email.to.unwrap().to_string(),
            "\"Sender 1\" <test-sender-1@local>, \"Sender 2\" <test-sender-2@local>"
        );
        assert_eq!(
            email.cc.unwrap().to_string(),
            "\"Sender 1\" <test-sender-1@local>, \"Sender 2\" <test-sender-2@local>"
        );
    }

    #[test]
    fn test_to_readable() {
        let config = AccountConfig::default();
        let email = Email {
            parts: Parts(vec![Part::TextPlain(TextPlainPart {
                content: String::from("hello, world!"),
            })]),
            ..Email::default()
        };
        let opts = PartsReaderOptions::default();

        // empty email headers, empty headers, empty config
        assert_eq!(
            "hello, world!",
            email.to_readable(&config, opts.clone(), vec![]).unwrap()
        );
        // empty email headers, basic headers
        assert_eq!(
            "hello, world!",
            email
                .to_readable(&config, opts.clone(), vec!["From", "DATE", "custom-hEader"])
                .unwrap()
        );
        // empty email headers, multiple subject headers
        assert_eq!(
            "Subject: \n\nhello, world!",
            email
                .to_readable(&config, opts.clone(), vec!["subject", "Subject", "SUBJECT"])
                .unwrap()
        );

        let email = Email {
            headers: HashMap::from_iter([("custom-header".into(), "custom value".into())]),
            message_id: Some("<message-id>".into()),
            from: Some(
                vec![Addr::Single(SingleInfo {
                    addr: "test@local".into(),
                    display_name: Some("Test".into()),
                })]
                .into(),
            ),
            cc: Some(vec![].into()),
            parts: Parts(vec![Part::TextPlain(TextPlainPart {
                content: String::from("hello, world!"),
            })]),
            ..Email::default()
        };

        // header present in email headers, empty config
        assert_eq!(
            "From: \"Test\" <test@local>\n\nhello, world!",
            email
                .to_readable(&config, opts.clone(), vec!["from"])
                .unwrap()
        );
        // header present but empty in email headers, empty config
        assert_eq!(
            "hello, world!",
            email
                .to_readable(&config, opts.clone(), vec!["cc"])
                .unwrap()
        );
        // multiple same custom headers present in email headers, empty
        // config
        assert_eq!(
            "Custom-Header: custom value\n\nhello, world!",
            email
                .to_readable(
                    &config,
                    opts.clone(),
                    vec!["custom-header", "cuSTom-HeaDer"]
                )
                .unwrap()
        );

        let config = AccountConfig {
            email_reading_headers: Some(vec![
                "CusTOM-heaDER".into(),
                "Subject".into(),
                "from".into(),
                "cc".into(),
            ]),
            ..AccountConfig::default()
        };
        // header present but empty in email headers, empty config
        assert_eq!(
            "Custom-Header: custom value\nSubject: \nFrom: \"Test\" <test@local>\nMessage-Id: <message-id>\n\nhello, world!",
            email.to_readable(&config, opts, vec!["cc", "message-ID"])
                .unwrap()
        );
    }
}
