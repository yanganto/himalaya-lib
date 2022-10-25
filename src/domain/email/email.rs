use imap::types::{Fetch, ZeroCopy};
use lettre::message::{Mailbox, Mailboxes};
use mailparse::{MailHeaderMap, MailParseError, ParsedMail};
use std::{borrow::Cow, cell::RefCell, env, fmt::Debug, io, path::PathBuf, result};
use thiserror::Error;

use crate::{account, AccountConfig, Tpl, DEFAULT_SIGNATURE_DELIM};

use super::parts::PartsWrapper;

#[derive(Error, Debug)]
pub enum Error {
    #[error("cannot parse imap body of email")]
    ParseImapBodyError,

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

#[derive(Debug)]
pub struct Email<'a> {
    pub bytes: Cow<'a, [u8]>,
    pub parsed: RefCell<Option<ParsedMail<'a>>>,
}

impl<'a> Email<'a> {
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

impl<'a> From<ParsedMail<'a>> for Email<'a> {
    fn from(parsed: ParsedMail<'a>) -> Self {
        Self {
            bytes: Cow::Borrowed(parsed.raw_bytes),
            parsed: RefCell::new(Some(parsed)),
        }
    }
}

impl<'a> From<Vec<u8>> for Email<'a> {
    fn from(vec: Vec<u8>) -> Self {
        Self {
            bytes: Cow::Owned(vec),
            parsed: RefCell::new(None),
        }
    }
}

impl<'a> From<&'a [u8]> for Email<'a> {
    fn from(bytes: &'a [u8]) -> Self {
        Self {
            bytes: Cow::Borrowed(bytes),
            parsed: RefCell::new(None),
        }
    }
}

impl<'a> From<&'a str> for Email<'a> {
    fn from(str: &'a str) -> Self {
        str.as_bytes().into()
    }
}

impl<'a> TryFrom<&'a Option<ZeroCopy<Vec<Fetch>>>> for Email<'a> {
    type Error = Error;

    fn try_from(fetches: &'a Option<ZeroCopy<Vec<Fetch>>>) -> Result<Self> {
        Ok(fetches
            .as_ref()
            .and_then(|fetches| fetches.first())
            .and_then(|fetch| fetch.body())
            .ok_or_else(|| Error::ParseImapBodyError)?
            .into())
    }
}

#[cfg(test)]
mod test_email_to_reply_tpl {
    use crate::{AccountConfig, Email};

    #[test]
    fn test_empty_config() {
        let config = AccountConfig {
            email: "to@localhost".into(),
            ..AccountConfig::default()
        };

        let email = Email::from(concat!(
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

        let email = Email::from(concat!(
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
