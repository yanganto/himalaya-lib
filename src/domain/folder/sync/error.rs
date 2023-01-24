use rusqlite;
use std::result;
use thiserror::Error;

use crate::backend;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    BackendError(#[from] backend::Error),
    #[error(transparent)]
    CacheError(#[from] rusqlite::Error),
}

pub type Result<T> = result::Result<T, Error>;
