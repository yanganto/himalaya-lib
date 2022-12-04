use mailparse::ParsedMail;

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
mod parts_iterator {
    use lettre::{
        message::{MultiPart, SinglePart},
        Message,
    };
    use mailparse::MailHeaderMap;

    use crate::PartsIterator;

    #[test]
    fn one_part_no_subpart() {
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
    fn one_part_one_subpart() {
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
