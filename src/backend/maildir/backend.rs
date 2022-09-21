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

//! Maildir backend module.
//!
//! This module contains the definition of the maildir backend and its
//! traits implementation.

use log::{debug, info, trace};
use std::{
    env,
    ffi::OsStr,
    fs, io,
    path::{self, PathBuf},
    result,
};
use thiserror::Error;

use crate::{
    backend, config, email, envelope::maildir::envelopes, flag::maildir::flags, id_mapper,
    AccountConfig, Backend, Email, Envelopes, Flags, Folder, Folders, IdMapper, MaildirConfig,
    DEFAULT_INBOX_FOLDER,
};

#[derive(Debug, Error)]
pub enum Error {
    #[error("cannot get maildir backend from config")]
    GetBackendFromConfigError,
    #[error("cannot find maildir sender")]
    FindSenderError,
    #[error("cannot read maildir directory {0}")]
    ReadDirError(path::PathBuf),
    #[error("cannot parse maildir subdirectory {0}")]
    ParseSubdirError(path::PathBuf),
    #[error("cannot get maildir envelopes at page {0}")]
    GetEnvelopesOutOfBoundsError(usize),
    #[error("cannot search maildir envelopes: feature not implemented")]
    SearchEnvelopesUnimplementedError,
    #[error("cannot get maildir message {0}")]
    GetMsgError(String),
    #[error("cannot decode maildir entry")]
    DecodeEntryError(#[source] io::Error),
    #[error("cannot parse maildir message")]
    ParseMsgError(#[source] maildir::MailEntryError),
    #[error("cannot decode header {0}")]
    DecodeHeaderError(#[source] rfc2047_decoder::Error, String),
    #[error("cannot parse maildir message header {0}")]
    ParseHeaderError(#[source] mailparse::MailParseError, String),
    #[error("cannot create maildir subdirectory {1}")]
    CreateSubdirError(#[source] io::Error, String),
    #[error("cannot decode maildir subdirectory")]
    DecodeSubdirError(#[source] io::Error),
    #[error("cannot delete subdirectories at {1}")]
    DeleteAllDirError(#[source] io::Error, path::PathBuf),
    #[error("cannot get current directory")]
    GetCurrentDirError(#[source] io::Error),
    #[error("cannot store maildir message with flags")]
    StoreWithFlagsError(#[source] maildir::MaildirError),
    #[error("cannot copy maildir message")]
    CopyMsgError(#[source] io::Error),
    #[error("cannot move maildir message")]
    MoveMsgError(#[source] io::Error),
    #[error("cannot delete maildir message")]
    DelMsgError(#[source] io::Error),
    #[error("cannot add maildir flags")]
    AddFlagsError(#[source] io::Error),
    #[error("cannot set maildir flags")]
    SetFlagsError(#[source] io::Error),
    #[error("cannot remove maildir flags")]
    DelFlagsError(#[source] io::Error),

    #[error(transparent)]
    ConfigError(#[from] config::Error),
    #[error(transparent)]
    IdMapperError(#[from] id_mapper::Error),
    #[error(transparent)]
    EmailError(#[from] email::Error),
}

pub type Result<T> = result::Result<T, Error>;

/// Represents the maildir backend.
pub struct MaildirBackend {
    account: AccountConfig,
    mdir: maildir::Maildir,
}

impl MaildirBackend {
    pub fn new(account: AccountConfig, backend: MaildirConfig) -> Self {
        Self {
            account,
            mdir: backend.root_dir.to_owned().into(),
        }
    }

    fn validate_mdir_path(&self, mdir_path: PathBuf) -> Result<PathBuf> {
        let path = if mdir_path.is_dir() {
            Ok(mdir_path)
        } else {
            Err(Error::ReadDirError(mdir_path.to_owned()))
        }?;
        Ok(path)
    }

    /// Creates a maildir instance from a string slice.
    pub fn get_mdir_from_dir(&self, dir: &str) -> Result<maildir::Maildir> {
        let dir = self.account.folder_alias(dir)?;

        // If the dir points to the inbox folder, creates a maildir
        // instance from the root folder.
        if dir == DEFAULT_INBOX_FOLDER {
            return self
                .validate_mdir_path(self.mdir.path().to_owned())
                .map(maildir::Maildir::from);
        }

        // If the dir is a valid maildir path, creates a maildir
        // instance from it. First checks for absolute path,
        self.validate_mdir_path((&dir).into())
            // then for relative path to `maildir-dir`,
            .or_else(|_| self.validate_mdir_path(self.mdir.path().join(&dir)))
            // and finally for relative path to the current directory.
            .or_else(|_| {
                self.validate_mdir_path(
                    env::current_dir()
                        .map_err(Error::GetCurrentDirError)?
                        .join(&dir),
                )
            })
            .or_else(|_| {
                // Otherwise creates a maildir instance from a maildir
                // subdirectory by adding a "." in front of the name
                // as described in the [spec].
                //
                // [spec]: http://www.courier-mta.org/imap/README.maildirquota.html
                self.validate_mdir_path(self.mdir.path().join(format!(".{}", dir)))
            })
            .map(maildir::Maildir::from)
    }
}

impl Backend for MaildirBackend {
    fn folder_add(&mut self, subdir: &str) -> backend::Result<()> {
        info!(">> add maildir subdir");
        debug!("subdir: {:?}", subdir);

        let path = self.mdir.path().join(format!(".{}", subdir));
        trace!("subdir path: {:?}", path);

        fs::create_dir(&path).map_err(|err| Error::CreateSubdirError(err, subdir.to_owned()))?;

        info!("<< add maildir subdir");
        Ok(())
    }

    fn folder_list(&mut self) -> backend::Result<Folders> {
        trace!(">> get maildir mailboxes");

        let mut mboxes = Folders::default();
        for (name, desc) in &self.account.folder_aliases {
            mboxes.push(Folder {
                delim: String::from("/"),
                name: name.into(),
                desc: desc.into(),
            })
        }
        for entry in self.mdir.list_subdirs() {
            let dir = entry.map_err(Error::DecodeSubdirError)?;
            let dirname = dir.path().file_name();
            mboxes.push(Folder {
                delim: String::from("/"),
                name: dirname
                    .and_then(OsStr::to_str)
                    .and_then(|s| if s.len() < 2 { None } else { Some(&s[1..]) })
                    .ok_or_else(|| Error::ParseSubdirError(dir.path().to_owned()))?
                    .into(),
                ..Folder::default()
            });
        }

        trace!("maildir mailboxes: {:?}", mboxes);
        trace!("<< get maildir mailboxes");
        Ok(mboxes)
    }

    fn folder_delete(&mut self, dir: &str) -> backend::Result<()> {
        info!(">> delete maildir dir");
        debug!("dir: {:?}", dir);

        let path = self.mdir.path().join(format!(".{}", dir));
        trace!("dir path: {:?}", path);

        fs::remove_dir_all(&path).map_err(|err| Error::DeleteAllDirError(err, path.to_owned()))?;

        info!("<< delete maildir dir");
        Ok(())
    }

    fn envelope_list(
        &mut self,
        dir: &str,
        page_size: usize,
        page: usize,
    ) -> backend::Result<Envelopes> {
        info!(">> get maildir envelopes");
        debug!("dir: {:?}", dir);
        debug!("page size: {:?}", page_size);
        debug!("page: {:?}", page);

        let mdir = self.get_mdir_from_dir(dir)?;

        // Reads envelopes from the "cur" folder of the selected
        // maildir.
        let mut envelopes = envelopes::from_raws(mdir.list_cur())?;
        debug!("envelopes len: {:?}", envelopes.len());
        trace!("envelopes: {:?}", envelopes);

        // Calculates pagination boundaries.
        let page_begin = page * page_size;
        debug!("page begin: {:?}", page_begin);
        if page_begin > envelopes.len() {
            return Err(Error::GetEnvelopesOutOfBoundsError(page_begin + 1))?;
        }
        let page_end = envelopes.len().min(page_begin + page_size);
        debug!("page end: {:?}", page_end);

        // Sorts envelopes by most recent date.
        envelopes.sort_by(|a, b| b.date.partial_cmp(&a.date).unwrap());

        // Applies pagination boundaries.
        envelopes.envelopes = envelopes[page_begin..page_end].to_owned();

        // Appends envelopes hash to the id mapper cache file and
        // calculates the new short hash length. The short hash length
        // represents the minimum hash length possible to avoid
        // conflicts.
        let short_hash_len = {
            let mut mapper = IdMapper::new(mdir.path())?;
            let entries = envelopes
                .iter()
                .map(|env| (env.id.to_owned(), env.internal_id.to_owned()))
                .collect();
            mapper.append(entries)?
        };
        debug!("short hash length: {:?}", short_hash_len);

        // Shorten envelopes hash.
        envelopes
            .iter_mut()
            .for_each(|env| env.id = env.id[0..short_hash_len].to_owned());

        info!("<< get maildir envelopes");
        Ok(envelopes)
    }

    fn envelope_search(
        &mut self,
        _dir: &str,
        _query: &str,
        _sort: &str,
        _page_size: usize,
        _page: usize,
    ) -> backend::Result<Envelopes> {
        info!(">> search maildir envelopes");
        info!("<< search maildir envelopes");
        Err(Error::SearchEnvelopesUnimplementedError)?
    }

    fn email_add(&mut self, dir: &str, msg: &[u8], flags: &str) -> backend::Result<String> {
        info!(">> add maildir message");
        debug!("dir: {:?}", dir);
        debug!("flags: {:?}", flags);

        let flags = Flags::from(flags);
        debug!("flags: {:?}", flags);

        let mdir = self.get_mdir_from_dir(dir)?;
        let id = mdir
            .store_cur_with_flags(msg, &flags::to_normalized_string(&flags))
            .map_err(Error::StoreWithFlagsError)?;
        debug!("id: {:?}", id);
        let hash = format!("{:x}", md5::compute(&id));
        debug!("hash: {:?}", hash);

        // Appends hash entry to the id mapper cache file.
        let mut mapper = IdMapper::new(mdir.path())?;
        mapper.append(vec![(hash.clone(), id.clone())])?;

        info!("<< add maildir message");
        Ok(hash)
    }

    fn email_get(&mut self, dir: &str, short_hash: &str) -> backend::Result<Email> {
        info!(">> get maildir message");
        debug!("dir: {:?}", dir);
        debug!("short hash: {:?}", short_hash);

        let mdir = self.get_mdir_from_dir(dir)?;
        let id = IdMapper::new(mdir.path())?.find(short_hash)?;
        debug!("id: {:?}", id);
        let mut mail_entry = mdir
            .find(&id)
            .ok_or_else(|| Error::GetMsgError(id.to_owned()))?;
        let parsed_mail = mail_entry.parsed().map_err(Error::ParseMsgError)?;
        let msg = Email::from_parsed_mail(parsed_mail, &self.account)?;
        trace!("message: {:?}", msg);

        info!("<< get maildir message");
        Ok(msg)
    }

    fn email_list(&mut self, dir: &str, short_hash: &str) -> backend::Result<Email> {
        info!(">> get maildir message");
        debug!("dir: {:?}", dir);
        debug!("short hash: {:?}", short_hash);

        let mdir = self.get_mdir_from_dir(dir)?;
        let id = IdMapper::new(mdir.path())?.find(short_hash)?;
        debug!("id: {:?}", id);
        let mut mail_entry = mdir
            .find(&id)
            .ok_or_else(|| Error::GetMsgError(id.to_owned()))?;
        let parsed_mail = mail_entry.parsed().map_err(Error::ParseMsgError)?;
        let msg = Email::from_parsed_mail(parsed_mail, &self.account)?;
        trace!("message: {:?}", msg);

        info!("<< get maildir message");
        Ok(msg)
    }

    fn email_copy(
        &mut self,
        dir_src: &str,
        dir_dst: &str,
        short_hash: &str,
    ) -> backend::Result<()> {
        info!(">> copy maildir message");
        debug!("source dir: {:?}", dir_src);
        debug!("destination dir: {:?}", dir_dst);

        let mdir_src = self.get_mdir_from_dir(dir_src)?;
        let mdir_dst = self.get_mdir_from_dir(dir_dst)?;
        let id = IdMapper::new(mdir_src.path())?.find(short_hash)?;
        debug!("id: {:?}", id);

        mdir_src
            .copy_to(&id, &mdir_dst)
            .map_err(Error::CopyMsgError)?;

        // Appends hash entry to the id mapper cache file.
        let mut mapper = IdMapper::new(mdir_dst.path())?;
        let hash = format!("{:x}", md5::compute(&id));
        mapper.append(vec![(hash.clone(), id.clone())])?;

        info!("<< copy maildir message");
        Ok(())
    }

    fn email_move(
        &mut self,
        dir_src: &str,
        dir_dst: &str,
        short_hash: &str,
    ) -> backend::Result<()> {
        info!(">> move maildir message");
        debug!("source dir: {:?}", dir_src);
        debug!("destination dir: {:?}", dir_dst);

        let mdir_src = self.get_mdir_from_dir(dir_src)?;
        let mdir_dst = self.get_mdir_from_dir(dir_dst)?;
        let id = IdMapper::new(mdir_src.path())?.find(short_hash)?;
        debug!("id: {:?}", id);

        mdir_src
            .move_to(&id, &mdir_dst)
            .map_err(Error::MoveMsgError)?;

        // Appends hash entry to the id mapper cache file.
        let mut mapper = IdMapper::new(mdir_dst.path())?;
        let hash = format!("{:x}", md5::compute(&id));
        mapper.append(vec![(hash.clone(), id.clone())])?;

        info!("<< move maildir message");
        Ok(())
    }

    fn email_delete(&mut self, dir: &str, short_hash: &str) -> backend::Result<()> {
        info!(">> delete maildir message");
        debug!("dir: {:?}", dir);
        debug!("short hash: {:?}", short_hash);

        let mdir = self.get_mdir_from_dir(dir)?;
        let id = IdMapper::new(mdir.path())?.find(short_hash)?;
        debug!("id: {:?}", id);
        mdir.delete(&id).map_err(Error::DelMsgError)?;

        info!("<< delete maildir message");
        Ok(())
    }

    fn flags_add(&mut self, dir: &str, short_hash: &str, flags: &str) -> backend::Result<()> {
        info!(">> add maildir message flags");
        debug!("dir: {:?}", dir);
        debug!("short hash: {:?}", short_hash);
        let flags = Flags::from(flags);
        debug!("flags: {:?}", flags);

        let mdir = self.get_mdir_from_dir(dir)?;
        let id = IdMapper::new(mdir.path())?.find(short_hash)?;
        debug!("id: {:?}", id);

        mdir.add_flags(&id, &flags::to_normalized_string(&flags))
            .map_err(Error::AddFlagsError)?;

        info!("<< add maildir message flags");
        Ok(())
    }

    fn flags_set(&mut self, dir: &str, short_hash: &str, flags: &str) -> backend::Result<()> {
        info!(">> set maildir message flags");
        debug!("dir: {:?}", dir);
        debug!("short hash: {:?}", short_hash);
        let flags = Flags::from(flags);
        debug!("flags: {:?}", flags);

        let mdir = self.get_mdir_from_dir(dir)?;
        let id = IdMapper::new(mdir.path())?.find(short_hash)?;
        debug!("id: {:?}", id);
        mdir.set_flags(&id, &flags::to_normalized_string(&flags))
            .map_err(Error::SetFlagsError)?;

        info!("<< set maildir message flags");
        Ok(())
    }

    fn flags_delete(&mut self, dir: &str, short_hash: &str, flags: &str) -> backend::Result<()> {
        info!(">> delete maildir message flags");
        debug!("dir: {:?}", dir);
        debug!("short hash: {:?}", short_hash);
        let flags = Flags::from(flags);
        debug!("flags: {:?}", flags);

        let mdir = self.get_mdir_from_dir(dir)?;
        let id = IdMapper::new(mdir.path())?.find(short_hash)?;
        debug!("id: {:?}", id);
        mdir.remove_flags(&id, &flags::to_normalized_string(&flags))
            .map_err(Error::DelFlagsError)?;

        info!("<< delete maildir message flags");
        Ok(())
    }
}
