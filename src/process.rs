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

//! Process module.
//!
//! This module contains cross platform helpers around the
//! `std::process` crate.

use log::{debug, trace};
use std::{
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
pub fn run(cmd: &str) -> Result<String> {
    debug!("running command: {}", cmd);

    let output = if cfg!(target_os = "windows") {
        Command::new("cmd").args(&["/C", cmd]).output()
    } else {
        Command::new("sh").arg("-c").arg(cmd).output()
    };
    let output = output.map_err(|err| Error::RunCmdError(err, cmd.to_string()))?;
    let output = String::from_utf8(output.stdout).map_err(Error::ParseCmdOutputError)?;
    trace!("command output: {}", output);

    Ok(output)
}

/// Runs the given command in a pipeline and returns the raw output.
pub fn pipe(cmd: &str, input: &[u8]) -> Result<Vec<u8>> {
    let mut output = Vec::new();

    let pipeline = Command::new(cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
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
