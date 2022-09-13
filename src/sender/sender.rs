use crate::{config::Config, email::Email};

pub trait Sender {
    type Error;
    fn send(&mut self, config: &Config, msg: &Email) -> Result<Vec<u8>, Self::Error>;
}
