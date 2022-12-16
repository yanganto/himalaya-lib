use chrono::{DateTime, NaiveDateTime, Utc};
use log::{trace, warn};
use mailparse::MailAddr;

use crate::{
    backend::maildir::{Error, Result},
    domain::flag::maildir::flags,
    Envelope,
};

/// Represents the raw envelope returned by the `maildir` crate.
pub type RawEnvelope = maildir::MailEntry;

pub fn from_raw(mut entry: RawEnvelope) -> Result<Envelope> {
    let mut envelope = Envelope::default();

    envelope.internal_id = entry.id().to_owned();
    envelope.id = format!("{:x}", md5::compute(&envelope.internal_id));
    envelope.flags = flags::from_raw(&entry);

    let parsed_mail = entry.parsed().map_err(Error::ParseMsgError)?;

    for header in parsed_mail.get_headers() {
        let key = header.get_key();
        trace!("header key: {}", key);

        let val = header.get_value();
        trace!("header value: {}", val);

        match key.to_lowercase().as_str() {
            "date" => {
                envelope.date = mailparse::dateparse(&val)
                    .map_err(|err| {
                        warn!("skipping invalid date {}", val);
                        err
                    })
                    .ok()
                    .and_then(|secs| NaiveDateTime::from_timestamp_opt(secs, 0))
                    .map(|naive_date_time| {
                        DateTime::<Utc>::from_utc(naive_date_time, Utc).to_rfc2822()
                    })
            }
            "subject" => {
                envelope.subject = val.into();
            }
            "from" => {
                envelope.sender = {
                    let addrs = mailparse::addrparse_header(header)
                        .map_err(|err| Error::ParseHeaderError(err, key.to_owned()))?;
                    match addrs.first() {
                        Some(MailAddr::Group(group)) => Ok(group.to_string()),
                        Some(MailAddr::Single(single)) => Ok(single.to_string()),
                        None => Err(Error::FindSenderError),
                    }?
                }
            }
            _ => (),
        }
    }
    trace!("<< parse headers");

    trace!("envelope: {:?}", envelope);
    trace!("<< build envelope from maildir parsed mail");
    Ok(envelope)
}
