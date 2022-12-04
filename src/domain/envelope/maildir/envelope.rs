use chrono::DateTime;
use log::trace;
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

    for h in parsed_mail.get_headers() {
        let k = h.get_key();
        trace!("header key: {:?}", k);

        // let v = rfc2047_decoder::Decoder::new()
        //     .skip_encoded_word_length(true)
        //     .decode(h.get_value_raw())
        //     .map_err(|err| Error::DecodeHeaderError(err, k.to_owned()))?;
        let v = h.get_value();
        trace!("header value: {:?}", v);

        match k.to_lowercase().as_str() {
            "date" => {
                envelope.date =
                    DateTime::parse_from_rfc2822(v.split_at(v.find(" (").unwrap_or(v.len())).0)
                        .map(|date| date.naive_local().to_string())
                        .ok()
            }
            "subject" => {
                envelope.subject = v.into();
            }
            "from" => {
                envelope.sender = {
                    let addrs = mailparse::addrparse_header(h)
                        .map_err(|err| Error::ParseHeaderError(err, k.to_owned()))?;
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
