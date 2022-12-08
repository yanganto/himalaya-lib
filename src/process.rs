//! Process module.
//!
//! This module contains cross platform helpers around the
//! `std::process` crate.

use log::debug;
use std::{
    env,
    io::{self, prelude::*},
    process::{Command, Stdio},
    result, string,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("cannot run command {1:?}")]
    RunCmdError(#[source] io::Error, String),
    #[error("cannot parse command output")]
    ParseCmdOutputError(#[source] string::FromUtf8Error),
    #[error("cannot spawn process for command {1:?}")]
    SpawnProcessError(#[source] io::Error, String),
    #[error("cannot get standard input")]
    GetStdinError,
    #[error("cannot write data to standard input")]
    WriteStdinError(#[source] io::Error),
    #[error("cannot get standard output")]
    GetStdoutError,
    #[error("cannot read data from standard output")]
    ReadStdoutError(#[source] io::Error),
}

pub type Result<T> = result::Result<T, Error>;

/// Runs the given command and returns the output as UTF8 string.
pub fn run(cmd: &str, input: &[u8]) -> Result<Vec<u8>> {
    let mut output = input.to_owned();

    for cmd in cmd.split('|') {
        debug!("running command: {}", cmd);
        output = pipe(cmd.trim(), &output)?;
    }

    Ok(output)
}

/// Runs the given command in a pipeline and returns the raw output.
pub fn pipe(cmd: &str, input: &[u8]) -> Result<Vec<u8>> {
    let mut output = Vec::new();

    let windows = cfg!(target_os = "windows")
        && env::var("MSYSTEM")
            .map(|env| !env.starts_with("MINGW"))
            .unwrap_or_default();

    let pipeline = if windows {
        Command::new("cmd")
            .args(&["/C", cmd])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    } else {
        Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    }
    .map_err(|err| Error::SpawnProcessError(err, cmd.to_string()))?;

    pipeline
        .stdin
        .ok_or_else(|| Error::GetStdinError)?
        .write_all(input)
        .map_err(Error::WriteStdinError)?;

    pipeline
        .stdout
        .ok_or_else(|| Error::GetStdoutError)?
        .read_to_end(&mut output)
        .map_err(Error::ReadStdoutError)?;

    Ok(output)
}
