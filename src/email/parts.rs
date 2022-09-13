// himalaya-lib, a Rust library for email management.
// Copyright (C) 2022  soywod <clement.douin@posteo.net>
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use mailparse::MailHeaderMap;
use serde::Serialize;
use std::{
    env, fs,
    ops::{Deref, DerefMut},
};
use uuid::Uuid;

use crate::{config::Config, email};

#[derive(Debug, Clone, Default, Serialize)]
pub struct TextPlainPart {
    pub content: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct TextHtmlPart {
    pub content: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct BinaryPart {
    pub filename: String,
    pub mime: String,
    pub content: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
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

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Parts(pub Vec<Part>);

impl Parts {
    pub fn replace_text_plain_parts_with(&mut self, part: TextPlainPart) {
        self.retain(|part| !matches!(part, Part::TextPlain(_)));
        self.push(Part::TextPlain(part));
    }

    pub fn from_parsed_mail<'a>(
        config: &'a Config,
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
    config: &Config,
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

fn decrypt_part(config: &Config, email: &mailparse::ParsedMail) -> email::Result<String> {
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