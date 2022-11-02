use mailparse::ParsedMail;
use regex::Regex;
use thiserror::Error;

use crate::EmailError;

#[derive(Error, Debug)]
pub enum PartsError {
    #[error(transparent)]
    EmailError(#[from] EmailError),
}

#[derive(Debug)]
pub struct Parts;

impl Parts {
    pub fn concat_text_plain_bodies<'a>(parsed: &ParsedMail<'a>) -> Result<String, EmailError> {
        let mut text_bodies = String::new();

        for part in PartsIterator::new(parsed) {
            if part.ctype.mimetype == "text/plain" {
                if !text_bodies.is_empty() {
                    text_bodies.push_str("\n\n")
                }
                text_bodies.push_str(&part.get_body().unwrap_or_default())
            }
        }

        // trims more than two consecutive new lines
        let text_bodies = Regex::new(r"(\r?\n\s*){2,}")
            .unwrap()
            .replace_all(&text_bodies, "\n\n")
            .to_string();

        Ok(text_bodies)
    }
}

#[cfg(test)]
mod test_parts_concat_text_plain_bodies {
    use crate::{Email, Parts};

    #[test]
    fn test_no_part() {
        let mut email = Email::from(
            r#"MIME-Version: 1.0
From: from@localhost
To: to@localhost
Subject: subject

Hello!"#,
        );
        let parsed = email.parsed().unwrap();

        assert_eq!("Hello!", Parts::concat_text_plain_bodies(&parsed).unwrap());
    }

    #[test]
    fn test_multipart() {
        let mut email = Email::from(
            r#"MIME-Version: 1.0
From: from@localhost
To: to@localhost
Subject: subject
Content-Type: multipart/mixed; boundary=boundary

--boundary
Content-Type: text/plain

Hello!
--boundary
Content-Type: application/octet-stream
Content-Transfer-Encoding: base64

PGh0bWw+CiAgPGhlYWQ+CiAgPC9oZWFkPgogIDxib2R5PgogICAgPHA+VGhpcyBpcyB0aGUgYm9keSBvZiB0aGUgbWVzc2FnZS48L3A+CiAgPC9ib2R5Pgo8L2h0bWw+Cg==

--boundary
Content-Type: text/html

<h1>Hello!</h1>

--boundary
Content-Type: text/plain

How are you?
--boundary--"#,
        );
        let parsed = email.parsed().unwrap();

        assert_eq!(
            "Hello!\n\nHow are you?\n",
            Parts::concat_text_plain_bodies(&parsed).unwrap()
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
mod test_parts_iterator {
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
