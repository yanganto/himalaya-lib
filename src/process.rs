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
    string,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProcessError {
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

pub fn run(cmd: &str) -> Result<String, ProcessError> {
    debug!("running command: {}", cmd);

    let output = if cfg!(target_os = "windows") {
        Command::new("cmd").args(&["/C", cmd]).output()
    } else {
        Command::new("sh").arg("-c").arg(cmd).output()
    };
    let output = output.map_err(|err| ProcessError::RunCmdError(err, cmd.to_string()))?;
    let output = String::from_utf8(output.stdout).map_err(ProcessError::ParseCmdOutputError)?;
    trace!("command output: {}", output);

    Ok(output)
}

pub fn pipe(cmd: &str, data: &[u8]) -> Result<Vec<u8>, ProcessError> {
    let mut res = Vec::new();

    let process = Command::new(cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|err| ProcessError::SpawnProcessError(err, cmd.to_string()))?;
    process
        .stdin
        .ok_or_else(|| ProcessError::GetStdinError)?
        .write_all(data)
        .map_err(ProcessError::WriteStdinError)?;
    process
        .stdout
        .ok_or_else(|| ProcessError::GetStdoutError)?
        .read_to_end(&mut res)
        .map_err(ProcessError::ReadStdoutError)?;

    Ok(res)
}
