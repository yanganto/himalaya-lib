use sqlite;
use std::result;
use thiserror::Error;

use crate::{backend, email};

#[derive(Debug, Error)]
pub enum Error {
    #[error("cannot find email by internal id {0}")]
    FindEmailError(String),

    #[error(transparent)]
    BackendError(#[from] backend::Error),
    #[error(transparent)]
    CacheError(#[from] sqlite::Error),
    #[error(transparent)]
    EmailError(#[from] email::Error),
}

pub type Result<T> = result::Result<T, Error>;
