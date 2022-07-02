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

//! Module related to email addresses.
//!
//! This module regroups email address entities and converters.

use mailparse;
use std::{fmt, result};

use crate::msg::Result;

/// Defines a single email address.
pub type Addr = mailparse::MailAddr;

/// Defines a list of email addresses.
pub type Addrs = mailparse::MailAddrList;

/// Converts a slice into an optional list of addresses.
pub fn from_slice_to_addrs<S: AsRef<str> + fmt::Debug>(
    addrs: S,
) -> result::Result<Option<Addrs>, mailparse::MailParseError> {
    let addrs = mailparse::addrparse(addrs.as_ref())?;
    Ok(if addrs.is_empty() { None } else { Some(addrs) })
}

/// Converts a list of addresses into a list of [`lettre::message::Mailbox`].
pub fn from_addrs_to_sendable_mbox(addrs: &Addrs) -> Result<Vec<lettre::message::Mailbox>> {
    let mut sendable_addrs: Vec<lettre::message::Mailbox> = vec![];
    for addr in addrs.iter() {
        match addr {
            Addr::Single(mailparse::SingleInfo { display_name, addr }) => sendable_addrs.push(
                lettre::message::Mailbox::new(display_name.clone(), addr.parse()?),
            ),
            Addr::Group(mailparse::GroupInfo { group_name, addrs }) => {
                for addr in addrs {
                    sendable_addrs.push(lettre::message::Mailbox::new(
                        addr.display_name.clone().or(Some(group_name.clone())),
                        addr.to_string().parse()?,
                    ))
                }
            }
        }
    }
    Ok(sendable_addrs)
}

/// Converts a list of addresses into a list of [`lettre::Address`].
pub fn from_addrs_to_sendable_addrs(addrs: &Addrs) -> Result<Vec<lettre::Address>> {
    let mut sendable_addrs = vec![];
    for addr in addrs.iter() {
        match addr {
            mailparse::MailAddr::Single(mailparse::SingleInfo {
                display_name: _,
                addr,
            }) => {
                sendable_addrs.push(addr.parse()?);
            }
            mailparse::MailAddr::Group(mailparse::GroupInfo {
                group_name: _,
                addrs,
            }) => {
                for addr in addrs {
                    sendable_addrs.push(addr.addr.parse()?);
                }
            }
        };
    }
    Ok(sendable_addrs)
}
