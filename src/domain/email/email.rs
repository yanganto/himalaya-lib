use imap::types::{Fetch, ZeroCopy};
use lettre::{
    address::AddressError,
    message::{Mailbox, Mailboxes},
};
use log::{trace, warn};
use mailparse::{DispositionType, MailHeaderMap, MailParseError, ParsedMail};
use std::{collections::HashSet, fmt::Debug, io, path::PathBuf};
use thiserror::Error;
use tree_magic;

use crate::{
    account, AccountConfig, Attachment, Parts, PartsIterator, Tpl, TplBuilder, TplBuilderOpts,
    DEFAULT_SIGNATURE_DELIM,
};

use super::tpl::ShowHeaders;

#[derive(Error, Debug)]
pub enum EmailError {
    #[error("cannot parse email")]
    ParseEmailError(#[source] MailParseError),
    #[error("cannot parse email: raw email is empty")]
    ParseEmailEmptyRawError,
    #[error("cannot parse message or address")]
    ParseEmailAddressError(#[from] AddressError),
    #[error("cannot delete local draft at {1}")]
    DeleteLocalDraftError(#[source] io::Error, PathBuf),

    #[cfg(feature = "imap-backend")]
    #[error("cannot parse email from imap fetches: empty fetches")]
    ParseEmailFromImapFetchesEmptyError,

    #[error(transparent)]
    ConfigError(#[from] account::config::Error),

    // TODO: sort me
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
}

#[derive(Debug)]
pub enum RawEmail<'a> {
    Vec(Vec<u8>),
    Bytes(&'a [u8]),
    #[cfg(feature = "imap-backend")]
    ImapFetches(ZeroCopy<Vec<Fetch>>),
}

#[derive(Debug)]
pub struct Email<'a> {
    raw: RawEmail<'a>,
    parsed: Option<ParsedMail<'a>>,
}

impl<'a> Email<'a> {
    pub fn parsed(&'a mut self) -> Result<&ParsedMail<'a>, EmailError> {
        if self.parsed.is_none() {
            self.parsed = Some(match &self.raw {
                RawEmail::Vec(vec) => {
                    mailparse::parse_mail(vec).map_err(EmailError::ParseEmailError)
                }
                RawEmail::Bytes(bytes) => {
                    mailparse::parse_mail(*bytes).map_err(EmailError::ParseEmailError)
                }
                #[cfg(feature = "imap-backend")]
                RawEmail::ImapFetches(fetches) => {
                    let body = fetches
                        .first()
                        .and_then(|fetch| fetch.body())
                        .ok_or(EmailError::ParseEmailFromImapFetchesEmptyError)?;
                    mailparse::parse_mail(body).map_err(EmailError::ParseEmailError)
                }
            }?)
        }

        self.parsed
            .as_ref()
            .ok_or_else(|| EmailError::ParseEmailEmptyRawError)
    }

    pub fn attachments(&'a mut self) -> Result<Vec<Attachment>, EmailError> {
        let attachments = PartsIterator::new(self.parsed()?).filter_map(|part| {
            let cdisp = part.get_content_disposition();
            if let DispositionType::Attachment = cdisp.disposition {
                let filename = cdisp.params.get("filename");
                let body = part
                    .get_body_raw()
                    .map_err(|err| {
                        let filename = filename
                            .map(|f| format!("attachment {}", f))
                            .unwrap_or_else(|| "unknown attachment".into());
                        warn!("skipping {}: {}", filename, err);
                        trace!("skipping part: {:#?}", part);
                        err
                    })
                    .ok()?;

                Some(Attachment {
                    filename: filename.map(String::from),
                    mime: tree_magic::from_u8(&body),
                    body,
                })
            } else {
                None
            }
        });

        Ok(attachments.collect())
    }

    pub fn text_parts(
        &'a self,
        parsed: &'a ParsedMail,
    ) -> Result<Vec<&ParsedMail<'a>>, EmailError> {
        let text_parts = PartsIterator::new(parsed).filter_map(|part| {
            if part.ctype.mimetype.starts_with("text") {
                Some(part)
            } else {
                None
            }
        });

        Ok(text_parts.collect())
    }

    pub fn as_raw(&self) -> Result<&[u8], EmailError> {
        match self.raw {
            RawEmail::Vec(ref vec) => Ok(vec),
            RawEmail::Bytes(bytes) => Ok(bytes),
            #[cfg(feature = "imap-backend")]
            RawEmail::ImapFetches(ref fetches) => fetches
                .first()
                .and_then(|fetch| fetch.body())
                .ok_or_else(|| EmailError::ParseEmailFromImapFetchesEmptyError),
        }
    }

    pub fn to_read_tpl(
        &'a mut self,
        config: &'a AccountConfig,
        opts: TplBuilderOpts,
    ) -> Result<Tpl, EmailError> {
        let mut tpl = TplBuilder::default();

        let parsed = self.parsed()?;
        let parsed_headers = parsed.get_headers();

        let mut headers_to_show = config.email_reading_headers().clone();

        if let ShowHeaders::Only(headers) = opts.show_headers_or_default() {
            headers_to_show.extend(headers.to_owned());
        }

        let opts = TplBuilderOpts {
            show_headers: Some(ShowHeaders::Only(HashSet::from_iter(
                headers_to_show.clone(),
            ))),
            ..opts
        };

        for ref header in headers_to_show {
            if let Some(header) = parsed_headers.get_first_header(header) {
                tpl = tpl.header(header.get_key(), header.get_value())
            }
        }

        for part in PartsIterator::new(parsed) {
            match part.ctype.mimetype.as_str() {
                "text/plain" => {
                    tpl =
                        tpl.text_plain_part(part.get_body().map_err(EmailError::ParseEmailError)?);
                }
                // TODO: manage other mime types
                _ => (),
            }
        }

        Ok(tpl.build(opts))
    }

    pub fn to_reply_tpl(
        &'a mut self,
        config: &AccountConfig,
        all: bool,
    ) -> Result<Tpl, EmailError> {
        let mut tpl = Tpl::default();
        let parsed = self.parsed()?;
        let headers = parsed.get_headers();
        let sender = config.addr()?;

        println!("sender: {:?}", sender.to_string());

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

        let text_bodies = Parts::concat_text_plain_bodies(&parsed)?;
        tpl.push_str("\n");

        for line in text_bodies.lines() {
            // removes existing signature from the original body
            if line[..] == DEFAULT_SIGNATURE_DELIM[0..3] {
                break;
            }

            tpl.push('>');
            if !line.starts_with('>') {
                tpl.push_str(" ")
            }
            tpl.push_str(line);
            tpl.push_str("\n");
        }

        // Signature

        if let Some(ref sig) = config.signature()? {
            tpl.push_str("\n");
            tpl.push_str(sig);
        }

        Ok(tpl)
    }

    pub fn to_forward_tpl(&'a mut self, config: &AccountConfig) -> Result<Tpl, EmailError> {
        let mut tpl = Tpl::default();
        let parsed = self.parsed()?;
        let headers = parsed.get_headers();
        let sender = config.addr()?;

        // From

        tpl.push_header("From", &sender.to_string());

        // To

        tpl.push_header("To", "");

        // Subject

        let subject = headers.get_first_value("Subject").unwrap_or_default();
        tpl.push_header("Subject", format!("Fwd: {}", subject));

        // Signature

        if let Some(ref sig) = config.signature()? {
            tpl.push_str("\n");
            tpl.push_str(sig);
            tpl.push_str("\n");
        }

        // Body

        tpl.push_str("\n-------- Forwarded Message --------\n");
        tpl.push_header("Subject", subject);
        if let Some(date) = headers.get_first_value("date") {
            tpl.push_header("Date: ", date);
        }
        tpl.push_header("From: ", headers.get_all_values("from").join(", "));
        tpl.push_header("To: ", headers.get_all_values("to").join(", "));
        tpl.push_str("\n");
        tpl.push_str(&Parts::concat_text_plain_bodies(&parsed)?);

        Ok(tpl)
    }
}

impl<'a> From<Vec<u8>> for Email<'a> {
    fn from(vec: Vec<u8>) -> Self {
        Self {
            raw: RawEmail::Vec(vec),
            parsed: None,
        }
    }
}

impl<'a> From<&'a [u8]> for Email<'a> {
    fn from(bytes: &'a [u8]) -> Self {
        Self {
            raw: RawEmail::Bytes(bytes),
            parsed: None,
        }
    }
}

impl<'a> From<&'a str> for Email<'a> {
    fn from(str: &'a str) -> Self {
        str.as_bytes().into()
    }
}

impl<'a> From<ParsedMail<'a>> for Email<'a> {
    fn from(parsed: ParsedMail<'a>) -> Self {
        Self {
            raw: RawEmail::Bytes(parsed.raw_bytes),
            parsed: Some(parsed),
        }
    }
}

#[cfg(feature = "imap-backend")]
impl TryFrom<ZeroCopy<Vec<Fetch>>> for Email<'_> {
    type Error = EmailError;

    fn try_from(fetches: ZeroCopy<Vec<Fetch>>) -> Result<Self, Self::Error> {
        if fetches.is_empty() {
            Err(EmailError::ParseEmailFromImapFetchesEmptyError)
        } else {
            Ok(Self {
                raw: RawEmail::ImapFetches(fetches),
                parsed: None,
            })
        }
    }
}

#[cfg(test)]
mod test_to_read_tpl {
    use concat_with::concat_line;

    use crate::{AccountConfig, Email, TplBuilderOpts};

    #[test]
    fn test_default() {
        let config = AccountConfig::default();
        let opts = TplBuilderOpts::default();

        let mut email = Email::from(concat_line!(
            "From: from@localhost",
            "To: to@localhost",
            "Subject: subject",
            "",
            "Hello!",
            "",
            "-- ",
            "Regards,"
        ));

        let tpl = email.to_read_tpl(&config, opts).unwrap();

        let expected_tpl = concat_line!("Hello!", "", "-- ", "Regards,");

        assert_eq!(expected_tpl, *tpl);
    }

    #[test]
    fn test_with_email_reading_headers_only() {
        let config = AccountConfig {
            email_reading_headers: Some(vec![
                // existing headers
                "From".into(),
                "Subject".into(),
                // nonexisting headers
                "Cc".into(),
                "Bcc".into(),
            ]),
            ..AccountConfig::default()
        };

        let opts = TplBuilderOpts::default();

        let mut email = Email::from(concat_line!(
            "From: from@localhost",
            "To: to@localhost",
            "Subject: subject",
            "",
            "Hello!",
            "",
            "-- ",
            "Regards,"
        ));

        let tpl = email.to_read_tpl(&config, opts).unwrap();

        let expected_tpl = concat_line!(
            "From: from@localhost",
            "Subject: subject",
            "",
            "Hello!",
            "",
            "-- ",
            "Regards,"
        );

        assert_eq!(expected_tpl, *tpl);
    }

    #[test]
    fn test_with_email_reading_headers_and_show_headers() {
        let config = AccountConfig {
            email_reading_headers: Some(vec![
                // existing headers
                "From".into(),
                "Subject".into(),
                // nonexisting headers
                "Cc".into(),
                "Bcc".into(),
            ]),
            ..AccountConfig::default()
        };

        let opts = TplBuilderOpts::default().show_headers(
            [
                // existing headers
                "To",
                // nonexisting header
                "Content-Type",
            ]
            .iter(),
        );

        let mut email = Email::from(concat_line!(
            "From: from@localhost",
            "To: to@localhost",
            "Subject: subject",
            "",
            "Hello!",
            "",
            "-- ",
            "Regards,"
        ));

        let tpl = email.to_read_tpl(&config, opts).unwrap();

        let expected_tpl = concat_line!(
            "From: from@localhost",
            "Subject: subject",
            "To: to@localhost",
            "",
            "Hello!",
            "",
            "-- ",
            "Regards,"
        );

        assert_eq!(expected_tpl, *tpl);
    }
}

#[cfg(test)]
mod test_to_reply_tpl {
    use crate::{AccountConfig, Email};

    #[test]
    fn test_default_config() {
        let config = AccountConfig {
            email: "to@localhost".into(),
            ..AccountConfig::default()
        };

        let tpl = r#"From: from@localhost
To: to@localhost
Subject: subject

Hello!

-- 
Regards,
"#;

        let expected_tpl = r#"From: to@localhost
To: from@localhost
Subject: Re: subject

> Hello!
> 
"#;

        assert_eq!(
            expected_tpl,
            Email::from(tpl)
                .to_reply_tpl(&config, false)
                .unwrap()
                .to_string()
        );
    }

    #[test]
    fn test_with_display_name_and_signature() {
        let config = AccountConfig {
            email: "to@localhost".into(),
            display_name: Some("To".into()),
            signature: Some("Cordialement,".into()),
            ..AccountConfig::default()
        };

        let tpl = r#"From: from@localhost
To: to@localhost
Subject: subject

Hello!

-- 
Regards,
"#;

        let expected_tpl = r#"From: To <to@localhost>
To: from@localhost
Subject: Re: subject

> Hello!
> 

-- 
Cordialement,"#;

        assert_eq!(
            expected_tpl,
            Email::from(tpl.trim_start())
                .to_reply_tpl(&config, false)
                .unwrap()
                .to_string()
        );
    }
}
