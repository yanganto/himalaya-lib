use thiserror::Error;

use crate::{
    config::Config,
    email::{self, Email},
    ConfigError, SmtpError,
};

#[derive(Error, Debug)]
pub enum SenderError {
    #[error(transparent)]
    ConfigError(#[from] ConfigError),
    #[error(transparent)]
    SmtpError(#[from] SmtpError),
    #[error(transparent)]
    EmailError(#[from] email::error::Error),
}

pub trait Sender {
    fn send(&mut self, config: &Config, msg: &Email) -> Result<Vec<u8>, SenderError>;
}
