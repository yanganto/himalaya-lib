use imap::types::{Fetch, ZeroCopy};
use lettre::{address::AddressError, message::Mailboxes};
use log::{trace, warn};
use mailparse::{DispositionType, MailHeaderMap, MailParseError, ParsedMail};
use std::{fmt::Debug, io, path::PathBuf, result};
use thiserror::Error;
use tree_magic;

use crate::{
    account, sanitize_text_plain_part, tpl, AccountConfig, Attachment, PartsIterator, ShowHeaders,
    Tpl, TplBuilder, TplBuilderOpts, DEFAULT_SIGNATURE_DELIM,
};

#[derive(Error, Debug)]
pub enum Error {
    #[error("cannot parse email")]
    ParseEmailError(#[source] MailParseError),
    #[error("cannot parse email body")]
    ParseEmailBodyError(#[source] MailParseError),
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
    #[error(transparent)]
    TplError(#[from] tpl::Error),

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

pub type Result<T> = result::Result<T, Error>;

#[derive(Debug)]
enum RawEmail<'a> {
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
    pub fn parsed(&'a mut self) -> Result<&ParsedMail<'a>> {
        if self.parsed.is_none() {
            self.parsed = Some(match &self.raw {
                RawEmail::Vec(vec) => mailparse::parse_mail(vec).map_err(Error::ParseEmailError),
                RawEmail::Bytes(bytes) => {
                    mailparse::parse_mail(*bytes).map_err(Error::ParseEmailError)
                }
                #[cfg(feature = "imap-backend")]
                RawEmail::ImapFetches(fetches) => {
                    let body = fetches
                        .first()
                        .and_then(|fetch| fetch.body())
                        .ok_or(Error::ParseEmailFromImapFetchesEmptyError)?;
                    mailparse::parse_mail(body).map_err(Error::ParseEmailError)
                }
            }?)
        }

        self.parsed
            .as_ref()
            .ok_or_else(|| Error::ParseEmailEmptyRawError)
    }

    pub fn attachments(&'a mut self) -> Result<Vec<Attachment>> {
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

    pub fn as_raw(&self) -> Result<&[u8]> {
        match self.raw {
            RawEmail::Vec(ref vec) => Ok(vec),
            RawEmail::Bytes(bytes) => Ok(bytes),
            #[cfg(feature = "imap-backend")]
            RawEmail::ImapFetches(ref fetches) => fetches
                .first()
                .and_then(|fetch| fetch.body())
                .ok_or_else(|| Error::ParseEmailFromImapFetchesEmptyError),
        }
    }

    pub fn to_read_tpl(&'a mut self, opts: TplBuilderOpts) -> Result<Tpl> {
        let mut tpl = TplBuilder::default();

        let parsed = self.parsed()?;
        let parsed_headers = parsed.get_headers();

        if let Some(show_headers) = opts.show_headers {
            match show_headers {
                ShowHeaders::All => {
                    for header in parsed_headers {
                        tpl = tpl.header(header.get_key(), header.get_value())
                    }
                }
                ShowHeaders::Only(ref headers) => {
                    for header in headers {
                        if let Some(header) = parsed_headers.get_first_header(header) {
                            tpl = tpl.header(header.get_key(), header.get_value())
                        }
                    }
                }
                ShowHeaders::None => (),
            }
        };

        let opts = TplBuilderOpts {
            show_headers: Some(ShowHeaders::Only(tpl.headers_order.clone())),
            ..opts
        };

        for part in PartsIterator::new(parsed) {
            match part.ctype.mimetype.as_str() {
                "text/plain" => {
                    tpl = tpl.text_plain_part(part.get_body().map_err(Error::ParseEmailError)?);
                }
                // TODO: manage other mime types
                _ => (),
            }
        }

        Ok(tpl.opts(opts).build())
    }

    pub fn to_reply_tpl_builder(
        &'a mut self,
        config: &AccountConfig,
        all: bool,
    ) -> Result<TplBuilder> {
        let mut tpl = TplBuilder::default().opts(TplBuilderOpts::default().show_headers(
            config.email_writing_headers(["From", "To", "In-Reply-To", "Cc", "Subject"]),
        ));

        let parsed = self.parsed()?;
        let parsed_headers = parsed.get_headers();
        let sender = config.addr()?;

        // From

        tpl = tpl.from(&sender);

        // To

        tpl = tpl.to({
            let mut all_mboxes = Mailboxes::new();

            let from = parsed_headers.get_all_values("From");
            let to = parsed_headers.get_all_values("To");
            let reply_to = parsed_headers.get_all_values("Reply-To");

            let reply_to_iter = if reply_to.is_empty() {
                from.into_iter()
            } else {
                reply_to.into_iter()
            };

            for reply_to in reply_to_iter {
                let mboxes: Mailboxes = reply_to.parse()?;
                all_mboxes.extend(mboxes.into_iter().filter(|mbox| mbox.email != sender.email));
            }

            for reply_to in to.into_iter() {
                let mboxes: Mailboxes = reply_to.parse()?;
                all_mboxes.extend(mboxes.into_iter().filter(|mbox| mbox.email != sender.email));
            }

            if all {
                all_mboxes
            } else {
                all_mboxes
                    .into_single()
                    .map(|mbox| Mailboxes::from_iter([mbox]))
                    .unwrap_or_default()
            }
        });

        // In-Reply-To

        if let Some(ref message_id) = parsed_headers.get_first_value("Message-Id") {
            tpl = tpl.in_reply_to(message_id);
        }

        // Cc

        if all {
            tpl = tpl.cc({
                let mut cc = Mailboxes::new();

                for mboxes in parsed_headers.get_all_values("Cc") {
                    let mboxes: Mailboxes = mboxes.parse()?;
                    cc.extend(mboxes.into_iter().filter(|mbox| mbox.email != sender.email))
                }

                cc
            });
        }

        // Subject

        if let Some(ref subject) = parsed_headers.get_first_value("Subject") {
            tpl = tpl.subject(String::from("Re: ") + subject);
        }

        // Body

        tpl = tpl.text_plain_part({
            let mut lines = String::default();

            for part in PartsIterator::new(&parsed) {
                if part.ctype.mimetype != "text/plain" {
                    continue;
                }

                let body =
                    sanitize_text_plain_part(part.get_body().map_err(Error::ParseEmailBodyError)?);

                lines.push_str("\n\n");

                for line in body.lines() {
                    // removes existing signature from the original body
                    if line[..] == DEFAULT_SIGNATURE_DELIM[0..3] {
                        break;
                    }

                    lines.push('>');
                    if !line.starts_with('>') {
                        lines.push_str(" ")
                    }
                    lines.push_str(line);
                    lines.push_str("\n");
                }
            }

            if let Some(ref signature) = config.signature()? {
                lines.push_str("\n");
                lines.push_str(signature);
            }

            lines
        });

        Ok(tpl)
    }

    pub fn to_forward_tpl_builder(&'a mut self, config: &AccountConfig) -> Result<TplBuilder> {
        let mut tpl = TplBuilder::default().opts(
            TplBuilderOpts::default()
                .show_headers(config.email_writing_headers(["From", "To", "Subject"])),
        );

        let parsed = self.parsed()?;
        let parsed_headers = parsed.get_headers();
        let sender = config.addr()?;

        // From

        tpl = tpl.from(&sender);

        // To

        tpl = tpl.to("");

        // Subject

        let subject = parsed_headers
            .get_first_value("Subject")
            .unwrap_or_default();

        tpl = tpl.subject(format!("Fwd: {}", subject));

        // Body

        tpl = tpl.text_plain_part({
            let mut lines = String::from("\n");

            if let Some(ref signature) = config.signature()? {
                lines.push_str("\n");
                lines.push_str(signature);
            }

            lines.push_str("\n-------- Forwarded Message --------\n");

            if let Some(date) = parsed_headers.get_first_value("date") {
                lines.push_str(&format!("Date: {}\n", date));
            }

            lines.push_str(&format!("From: {}\n", {
                let mut from = Mailboxes::new();
                for mboxes in parsed_headers.get_all_values("From") {
                    let mboxes: Mailboxes = mboxes.parse()?;
                    from.extend(mboxes)
                }
                from
            }));

            lines.push_str(&format!("To: {}\n", {
                let mut to = Mailboxes::new();
                for mboxes in parsed_headers.get_all_values("To") {
                    let mboxes: Mailboxes = mboxes.parse()?;
                    to.extend(mboxes)
                }
                to
            }));

            lines.push_str(&format!("Subject: {}\n", subject));

            lines.push_str("\n");

            for part in PartsIterator::new(&parsed) {
                if part.ctype.mimetype != "text/plain" {
                    continue;
                }

                lines.push_str(&sanitize_text_plain_part(
                    part.get_body().map_err(Error::ParseEmailBodyError)?,
                ));
            }

            lines
        });

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
    type Error = Error;

    fn try_from(fetches: ZeroCopy<Vec<Fetch>>) -> Result<Self> {
        if fetches.is_empty() {
            Err(Error::ParseEmailFromImapFetchesEmptyError)
        } else {
            Ok(Self {
                raw: RawEmail::ImapFetches(fetches),
                parsed: None,
            })
        }
    }
}

#[cfg(test)]
mod test_email {
    use concat_with::concat_line;

    use crate::{AccountConfig, Email, TplBuilderOpts};

    #[test]
    fn test_to_read_tpl_builder() {
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

        let tpl = email.to_read_tpl(opts).unwrap();

        let expected_tpl = concat_line!("Hello!", "", "-- ", "Regards,");

        assert_eq!(expected_tpl, *tpl);
    }

    #[test]
    fn test_to_read_tpl_builder_with_email_reading_headers_config() {
        let opts = TplBuilderOpts::default().show_headers([
            "From", "Subject", // existing headers
            "Cc", "Bcc", // nonexisting headers
        ]);

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

        let tpl = email.to_read_tpl(opts).unwrap();

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
    fn test_to_read_tpl_builder_with_show_all_headers_option() {
        let opts = TplBuilderOpts::default().show_all_headers();

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

        let tpl = email.to_read_tpl(opts).unwrap();

        let expected_tpl = concat_line!(
            "From: from@localhost",
            "To: to@localhost",
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
    fn test_to_read_tpl_builder_with_show_only_headers_option() {
        let opts = TplBuilderOpts::default().show_headers([
            // existing headers
            "Subject",
            "To",
            // nonexisting header
            "Content-Type",
        ]);

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

        let tpl = email.to_read_tpl(opts).unwrap();

        let expected_tpl = concat_line!(
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

    #[test]
    fn test_to_reply_tpl_builder() {
        let config = AccountConfig {
            email: "to@localhost".into(),
            ..AccountConfig::default()
        };

        let mut email = Email::from(concat_line!(
            "From: from@localhost",
            "To: to@localhost, to2@localhost",
            "Cc: cc@localhost, cc2@localhost",
            "Bcc: bcc@localhost",
            "Subject: subject",
            "",
            "Hello!",
            "",
            "-- ",
            "Regards,"
        ));

        let tpl = email.to_reply_tpl_builder(&config, false).unwrap().build();

        let expected_tpl = concat_line!(
            "From: to@localhost",
            "To: from@localhost",
            "Subject: Re: subject",
            "",
            "",
            "",
            "> Hello!",
            "> ",
            ""
        );

        assert_eq!(expected_tpl, *tpl);
    }

    #[test]
    fn test_to_reply_all_tpl_builder() {
        let config = AccountConfig {
            email: "to@localhost".into(),
            ..AccountConfig::default()
        };

        let mut email = Email::from(concat_line!(
            "From: from@localhost",
            "To: to@localhost, to2@localhost",
            "Cc: to@localhost, cc@localhost, cc2@localhost",
            "Bcc: bcc@localhost",
            "Subject: subject",
            "",
            "Hello!",
            "",
            "-- ",
            "Regards,"
        ));

        let tpl = email.to_reply_tpl_builder(&config, true).unwrap().build();

        let expected_tpl = concat_line!(
            "From: to@localhost",
            "To: from@localhost, to2@localhost",
            "Cc: cc@localhost, cc2@localhost",
            "Subject: Re: subject",
            "",
            "",
            "",
            "> Hello!",
            "> ",
            ""
        );

        assert_eq!(expected_tpl, *tpl);
    }

    #[test]
    fn test_to_reply_tpl_builder_with_signature() {
        let config = AccountConfig {
            email: "to@localhost".into(),
            signature: Some("Cordialement,".into()),
            ..AccountConfig::default()
        };

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

        let tpl = email.to_reply_tpl_builder(&config, false).unwrap().build();

        let expected_tpl = concat_line!(
            "From: to@localhost",
            "To: from@localhost",
            "Subject: Re: subject",
            "",
            "",
            "",
            "> Hello!",
            "> ",
            "",
            "-- ",
            "Cordialement,"
        );

        assert_eq!(expected_tpl, *tpl);
    }

    #[test]
    fn test_to_forward_tpl_builder() {
        let config = AccountConfig {
            email: "to@localhost".into(),
            ..AccountConfig::default()
        };

        let mut email = Email::from(concat_line!(
            "From: from@localhost",
            "To: to@localhost, to2@localhost",
            "Cc: cc@localhost, cc2@localhost",
            "Bcc: bcc@localhost",
            "Subject: subject",
            "",
            "Hello!",
            "",
            "-- ",
            "Regards,"
        ));

        let tpl = email.to_forward_tpl_builder(&config).unwrap().build();

        let expected_tpl = concat_line!(
            "From: to@localhost",
            "To: ",
            "Subject: Fwd: subject",
            "",
            "",
            "",
            "-------- Forwarded Message --------",
            "From: from@localhost",
            "To: to@localhost, to2@localhost",
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
    fn test_to_forward_tpl_builder_with_date_and_signature() {
        let config = AccountConfig {
            email: "to@localhost".into(),
            signature: Some("Cordialement,".into()),
            ..AccountConfig::default()
        };

        let mut email = Email::from(concat_line!(
            "Date: Thu, 10 Nov 2022 14:26:33 +0000",
            "From: from@localhost",
            "To: to@localhost, to2@localhost",
            "Cc: cc@localhost, cc2@localhost",
            "Bcc: bcc@localhost",
            "Subject: subject",
            "",
            "Hello!",
            "",
            "-- ",
            "Regards,"
        ));

        let tpl = email.to_forward_tpl_builder(&config).unwrap().build();

        let expected_tpl = concat_line!(
            "From: to@localhost",
            "To: ",
            "Subject: Fwd: subject",
            "",
            "",
            "",
            "-- ",
            "Cordialement,",
            "-------- Forwarded Message --------",
            "Date: Thu, 10 Nov 2022 14:26:33 +0000",
            "From: from@localhost",
            "To: to@localhost, to2@localhost",
            "Subject: subject",
            "",
            "Hello!",
            "",
            "-- ",
            "Regards,"
        );

        assert_eq!(expected_tpl, *tpl);
    }
}
