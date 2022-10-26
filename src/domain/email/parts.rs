use ammonia::Builder as AmmoniaBuilder;
use mailparse::{MailHeaderMap, ParsedMail};
use regex::Regex;
use std::{
    collections::HashSet,
    env, fs,
    ops::{Deref, DerefMut},
    result,
};
use thiserror::Error;
use uuid::Uuid;

use crate::{email, AccountConfig, Email};

#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct TextPlainPart {
    pub content: String,
}

#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct TextHtmlPart {
    pub content: String,
}

#[derive(Debug, Default, Clone, Eq, PartialEq)]
pub struct BinaryPart {
    pub filename: String,
    pub mime: String,
    pub content: Vec<u8>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Part {
    TextPlain(TextPlainPart),
    TextHtml(TextHtmlPart),
    Binary(BinaryPart),
}

impl Part {
    pub fn new_text_plain(content: String) -> Self {
        Self::TextPlain(TextPlainPart { content })
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PartsReaderOptions {
    pub plain_first: bool,
    pub sanitize: bool,
}

impl Default for PartsReaderOptions {
    fn default() -> Self {
        Self {
            plain_first: true,
            sanitize: true,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Parts(pub Vec<Part>);

impl Parts {
    pub fn replace_text_plain_parts_with(&mut self, part: TextPlainPart) {
        self.retain(|part| !matches!(part, Part::TextPlain(_)));
        self.push(Part::TextPlain(part));
    }

    pub fn from_parsed_mail<'a>(
        config: &'a AccountConfig,
        part: &'a mailparse::ParsedMail<'a>,
    ) -> email::Result<Self> {
        let mut parts = vec![];
        if part.subparts.is_empty() && part.get_headers().get_first_value("content-type").is_none()
        {
            let content = part.get_body().unwrap_or_default();
            parts.push(Part::TextPlain(TextPlainPart { content }))
        } else {
            build_parts_map_rec(config, part, &mut parts)?;
        }
        Ok(Self(parts))
    }

    /// Folds string body from all plain text parts into a single
    /// string body. If no plain text parts are found, HTML parts are
    /// used instead. The result is sanitized (all HTML markup is
    /// removed).
    pub fn to_readable(&self, opts: PartsReaderOptions) -> String {
        let (mut plain, mut html) = self.iter().fold(
            (String::default(), String::default()),
            |(mut plain, mut html), part| {
                match part {
                    Part::TextPlain(part) => {
                        let glue = if plain.is_empty() { "" } else { "\n\n" };
                        plain.push_str(glue);
                        plain.push_str(&part.content);
                    }
                    Part::TextHtml(part) => {
                        let glue = if html.is_empty() { "" } else { "\n\n" };
                        html.push_str(glue);
                        html.push_str(&part.content);
                    }
                    _ => (),
                };
                (plain, html)
            },
        );

        if opts.sanitize {
            html = {
                // removes html markup
                let sanitized_html = AmmoniaBuilder::new()
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
                let sanitized_html = html_escape::decode_html_entities(&sanitized_html).to_string();

                sanitized_html
            };

            plain = {
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

        if opts.plain_first {
            if plain.is_empty() {
                html
            } else {
                plain
            }
        } else {
            if html.is_empty() {
                plain
            } else {
                html
            }
        }
    }
}

impl Deref for Parts {
    type Target = Vec<Part>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Parts {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

fn build_parts_map_rec(
    config: &AccountConfig,
    parsed_mail: &mailparse::ParsedMail,
    parts: &mut Vec<Part>,
) -> email::Result<()> {
    if parsed_mail.subparts.is_empty() {
        let cdisp = parsed_mail.get_content_disposition();
        match cdisp.disposition {
            mailparse::DispositionType::Attachment => {
                let filename = cdisp
                    .params
                    .get("filename")
                    .map(String::from)
                    .unwrap_or_else(|| String::from("noname"));
                let content = parsed_mail.get_body_raw().unwrap_or_default();
                let mime = tree_magic::from_u8(&content);
                parts.push(Part::Binary(BinaryPart {
                    filename,
                    mime,
                    content,
                }));
            }
            // TODO: manage other use cases
            _ => {
                if let Some(ctype) = parsed_mail.get_headers().get_first_value("content-type") {
                    let content = parsed_mail.get_body().unwrap_or_default();
                    if ctype.starts_with("text/plain") {
                        parts.push(Part::TextPlain(TextPlainPart { content }))
                    } else if ctype.starts_with("text/html") {
                        parts.push(Part::TextHtml(TextHtmlPart { content }))
                    }
                }
            }
        };
    } else {
        let ctype = parsed_mail
            .get_headers()
            .get_first_value("content-type")
            .ok_or_else(|| email::Error::GetMultipartContentTypeError)?;
        if ctype.starts_with("multipart/encrypted") {
            let decrypted_part = parsed_mail
                .subparts
                .get(1)
                .ok_or_else(|| email::Error::GetEncryptedPartMultipartError)
                .and_then(|part| decrypt_part(config, part))?;
            let parsed_mail = mailparse::parse_mail(decrypted_part.as_bytes())
                .map_err(email::Error::ParseEncryptedPartError)?;
            build_parts_map_rec(config, &parsed_mail, parts)?;
        } else {
            for part in parsed_mail.subparts.iter() {
                build_parts_map_rec(config, part, parts)?;
            }
        }
    }

    Ok(())
}

fn decrypt_part(config: &AccountConfig, email: &mailparse::ParsedMail) -> email::Result<String> {
    let email_path = env::temp_dir().join(Uuid::new_v4().to_string());
    let email_body = email
        .get_body()
        .map_err(email::Error::GetEncryptedPartBodyError)?;
    fs::write(email_path.clone(), &email_body)
        .map_err(email::Error::WriteEncryptedPartBodyError)?;
    let content = config
        .pgp_decrypt_file(email_path.clone())
        .map_err(email::Error::DecryptPartError)?;
    Ok(content)
}

// New API

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    EmailError(#[from] email::Error),
}

pub type Result<T> = result::Result<T, Error>;

#[derive(Debug)]
pub struct PartsWrapper<'a>(&'a Email<'a>);

impl<'a> PartsWrapper<'a> {
    pub fn new(email: &'a Email<'a>) -> Self {
        Self(email)
    }

    pub fn concat_text_plain_bodies(&'a self) -> Result<String> {
        let text_bodies = self.0.with_parsed(|parsed| {
            let mut text_bodies = String::new();
            for part in PartsIterator::new(&parsed) {
                if part.ctype.mimetype == "text/plain" {
                    if !text_bodies.is_empty() {
                        text_bodies.push_str("\n\n")
                    }
                    println!("part: {:?}", &part.get_body());
                    text_bodies.push_str(&part.get_body().unwrap_or_default())
                }
            }

            // trims consecutive new lines bigger than two
            let text_bodies = Regex::new(r"(\r?\n\s*){2,}")
                .unwrap()
                .replace_all(&text_bodies, "\n\n")
                .to_string();

            email::Result::Ok(text_bodies)
        })?;
        Ok(text_bodies)
    }
}

impl<'a> Deref for PartsWrapper<'a> {
    type Target = Email<'a>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(test)]
mod parts_tests {
    use super::*;

    #[test]
    fn test_concat_text_plain_bodies_no_part() {
        let email = Email::try_from(concat!(
            "From: from@localhost",
            "To: to@localhost",
            "Subject: subject",
            "",
            "Hello!"
        ))
        .unwrap();
        let parts = PartsWrapper::new(&email);

        assert_eq!("Hello!", parts.concat_text_plain_bodies().unwrap());
    }

    #[test]
    fn test_concat_text_plain_bodies_multipart() {
        let email = Email::try_from(concat!(
            "From: from@localhost",
            "To: to@localhost",
            "Subject: subject",
            "MIME-Version: 1.0",
            "Content-Type: multipart/mixed; boundary=boundary",
            "",
            "--boundary",
            "Content-Type: text/plain",
            "",
            "Hello!",
            "--boundary",
            "Content-Type: application/octet-stream",
            "Content-Transfer-Encoding: base64",
            "",
            "PGh0bWw+CiAgPGhlYWQ+CiAgPC9oZWFkPgogIDxib2R5PgogICAgPHA+VGhpcyBpcyB0aGUgYm9keSBvZiB0aGUgbWVzc2FnZS48L3A+CiAgPC9ib2R5Pgo8L2h0bWw+Cg==",
            "",
            "--boundary",
            "Content-Type: text/html",
            "",
            "<h1>Hello!</h1>",
            "",
            "--boundary",
            "Content-Type: text/plain",
            "",
            "How are you?",
            "--boundary--",
	))
        .unwrap();
        let parts = PartsWrapper::new(&email);

        assert_eq!(
            "Hello!\n\nHow are you?\n",
            parts.concat_text_plain_bodies().unwrap()
        );
    }
}

#[derive(Debug)]
pub struct PartsIterator<'a> {
    pub pos: usize,
    pub parts: Vec<&'a ParsedMail<'a>>,
}

impl<'a> PartsIterator<'a> {
    pub fn new(part: &'a ParsedMail<'a>) -> Self {
        Self {
            pos: 0,
            parts: vec![part],
        }
    }
}

impl<'a> Iterator for PartsIterator<'a> {
    type Item = &'a ParsedMail<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.parts.len() {
            return None;
        }

        for part in &self.parts[self.pos].subparts {
            self.parts.push(part)
        }

        let item = self.parts[self.pos];
        self.pos += 1;
        Some(item)
    }
}

#[cfg(test)]
mod parts_iterator_tests {
    use lettre::{
        message::{MultiPart, SinglePart},
        Message,
    };
    use mailparse::MailHeaderMap;

    use crate::PartsIterator;

    #[test]
    fn test_one_part_no_subpart() {
        let email = Message::builder()
            .from("from@localhost".parse().unwrap())
            .to("to@localhost".parse().unwrap())
            .singlepart(SinglePart::plain(String::new()))
            .unwrap()
            .formatted();
        let email = mailparse::parse_mail(&email).unwrap();

        let parts = PartsIterator::new(&email).into_iter().collect::<Vec<_>>();

        assert_eq!(1, parts.len());
        assert!(parts[0]
            .get_headers()
            .get_first_value("Content-Type")
            .unwrap()
            .starts_with("text/plain"));
    }

    #[test]
    fn test_one_part_one_subpart() {
        let email = Message::builder()
            .from("from@localhost".parse().unwrap())
            .to("to@localhost".parse().unwrap())
            .multipart(MultiPart::mixed().singlepart(SinglePart::plain(String::new())))
            .unwrap()
            .formatted();
        let email = mailparse::parse_mail(&email).unwrap();

        let parts = PartsIterator::new(&email).into_iter().collect::<Vec<_>>();

        assert_eq!(2, parts.len());
        assert!(parts[0]
            .get_headers()
            .get_first_value("Content-Type")
            .unwrap()
            .starts_with("multipart/mixed"));
        assert!(parts[1]
            .get_headers()
            .get_first_value("Content-Type")
            .unwrap()
            .starts_with("text/plain"));
    }
}
