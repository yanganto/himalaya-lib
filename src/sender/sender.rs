use crate::{config::Config, msg::Msg};

pub trait Sender {
    type Error;
    fn send(&mut self, config: &Config, msg: &Msg) -> Result<Vec<u8>, Self::Error>;
}
