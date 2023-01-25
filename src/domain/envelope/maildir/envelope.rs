use chrono::{Local, NaiveDateTime};
use log::trace;
use mailparse::MailAddr;

use crate::{
    backend::maildir::{Error, Result},
    domain::flag::maildir::flags,
    envelope::Mailbox,
    Envelope,
};

/// Represents the raw envelope returned by the `maildir` crate.
pub type RawEnvelope = maildir::MailEntry;

pub fn from_raw(mut entry: RawEnvelope) -> Result<Envelope> {
    let mut envelope = Envelope::default();

    envelope.internal_id = entry.id().to_owned();
    envelope.flags = flags::from_raw(&entry);

    let parsed_mail = entry.parsed().map_err(Error::ParseMsgError)?;

    for header in parsed_mail.get_headers() {
        let key = header.get_key();
        trace!("header key: {}", key);

        let val = header.get_value();
        trace!("header value: {}", val);

        match key.to_lowercase().as_str() {
            "message-id" => {
                envelope.message_id = val.trim().into();
            }
            "subject" => {
                envelope.subject = val.into();
            }
            "from" => {
                envelope.from = {
                    let addrs = mailparse::addrparse_header(header)
                        .map_err(|err| Error::ParseHeaderError(err, key.to_owned()))?;
                    match addrs.first() {
                        Some(MailAddr::Single(single)) => Ok(Mailbox::new(
                            single.display_name.clone(),
                            single.addr.clone(),
                        )),
                        // TODO
                        Some(MailAddr::Group(_)) => Err(Error::FindSenderError),
                        None => Err(Error::FindSenderError),
                    }?
                }
            }
            "date" => {
                let timestamp = mailparse::dateparse(&val)
                    .map_err(|err| Error::ParseTimestampFromMaildirEnvelopeError(err, val))?;
                let date = NaiveDateTime::from_timestamp_opt(timestamp, 0)
                    .and_then(|date| date.and_local_timezone(Local).earliest());
                envelope.date = date.unwrap_or_default()
            }
            _ => (),
        }
    }

    trace!("maildir envelope: {:?}", envelope);

    Ok(envelope)
}
