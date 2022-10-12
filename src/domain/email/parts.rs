use ammonia::Builder as AmmoniaBuilder;
use mailparse::MailHeaderMap;
use regex::Regex;
use std::{
    collections::HashSet,
    env, fs,
    ops::{Deref, DerefMut},
};
use uuid::Uuid;

use crate::{email, AccountConfig};

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
