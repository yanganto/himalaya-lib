//! Maildir backend module.
//!
//! This module contains the definition of the maildir backend and its
//! traits implementation.

use log::{debug, trace};
use std::{
    any::Any,
    env,
    ffi::OsStr,
    fs, io,
    path::{self, PathBuf},
    result,
};
use thiserror::Error;

use crate::{
    account, backend, email, envelope::maildir::envelopes, flag::maildir::flags, id_mapper,
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
    ConfigError(#[from] account::config::Error),
    #[error(transparent)]
    IdMapperError(#[from] id_mapper::Error),
    #[error(transparent)]
    EmailError(#[from] email::Error),
}

pub type Result<T> = result::Result<T, Error>;

/// Represents the maildir backend.
pub struct MaildirBackend<'a> {
    account_config: &'a AccountConfig,
    mdir: maildir::Maildir,
}

impl<'a> MaildirBackend<'a> {
    pub fn new(account_config: &'a AccountConfig, backend_config: &'a MaildirConfig) -> Self {
        Self {
            account_config,
            mdir: backend_config.root_dir.to_owned().into(),
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
        let dir = self.account_config.folder_alias(dir)?;

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

impl<'a> Backend<'a> for MaildirBackend<'a> {
    fn add_folder(&'a self, subdir: &'a str) -> backend::Result<()> {
        debug!("subdir: {:?}", subdir);

        let path = self.mdir.path().join(format!(".{}", subdir));
        debug!("subdir path: {:?}", path);

        fs::create_dir(&path).map_err(|err| Error::CreateSubdirError(err, subdir.to_owned()))?;
        Ok(())
    }

    fn list_folder(&'a self) -> backend::Result<Folders> {
        let mut folders = Folders::default();

        for (name, desc) in &self.account_config.folder_aliases {
            folders.push(Folder {
                delim: String::from("/"),
                name: name.into(),
                desc: desc.into(),
            })
        }

        for entry in self.mdir.list_subdirs() {
            let dir = entry.map_err(Error::DecodeSubdirError)?;
            let dirname = dir.path().file_name();
            folders.push(Folder {
                delim: String::from("/"),
                name: dirname
                    .and_then(OsStr::to_str)
                    .and_then(|s| if s.len() < 2 { None } else { Some(&s[1..]) })
                    .ok_or_else(|| Error::ParseSubdirError(dir.path().to_owned()))?
                    .into(),
                ..Folder::default()
            });
        }

        trace!("folders: {:?}", folders);
        Ok(folders)
    }

    fn delete_folder(&'a self, dir: &'a str) -> backend::Result<()> {
        debug!("dir: {:?}", dir);

        let path = self.mdir.path().join(format!(".{}", dir));
        debug!("dir path: {:?}", path);

        fs::remove_dir_all(&path).map_err(|err| Error::DeleteAllDirError(err, path.to_owned()))?;
        Ok(())
    }

    fn list_envelope(
        &'a self,
        dir: &'a str,
        page_size: usize,
        page: usize,
    ) -> backend::Result<Envelopes> {
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
        envelopes.0 = envelopes[page_begin..page_end].to_owned();

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

        Ok(envelopes)
    }

    fn search_envelope(
        &'a self,
        _dir: &'a str,
        _query: &'a str,
        _sort: &'a str,
        _page_size: usize,
        _page: usize,
    ) -> backend::Result<Envelopes> {
        Err(Error::SearchEnvelopesUnimplementedError)?
    }

    fn add_email(
        &'a self,
        dir: &'a str,
        email: &'a [u8],
        flags: &'a str,
    ) -> backend::Result<String> {
        debug!("dir: {:?}", dir);
        debug!("flags: {:?}", flags);

        let flags = Flags::from(flags);
        debug!("flags: {:?}", flags);

        let mdir = self.get_mdir_from_dir(dir)?;
        let id = mdir
            .store_cur_with_flags(email, &flags::to_normalized_string(&flags))
            .map_err(Error::StoreWithFlagsError)?;
        debug!("id: {:?}", id);
        let hash = format!("{:x}", md5::compute(&id));
        debug!("hash: {:?}", hash);

        // Appends hash entry to the id mapper cache file.
        let mut mapper = IdMapper::new(mdir.path())?;
        mapper.append(vec![(hash.clone(), id.clone())])?;

        Ok(hash)
    }

    fn get_email(&'a self, dir: &'a str, short_hash: &'a str) -> backend::Result<Email<'a>> {
        debug!("dir: {:?}", dir);
        debug!("short hash: {:?}", short_hash);

        let mdir = self.get_mdir_from_dir(dir)?;
        let id = IdMapper::new(mdir.path())?.find(short_hash)?;
        debug!("id: {:?}", id);

        let mut mail_entry = mdir
            .find(&id)
            .ok_or_else(|| Error::GetMsgError(id.to_owned()))?;

        // FIXME: find why borrowing does not work here
        let email = Email::from(
            mail_entry
                .parsed()
                .map_err(Error::ParseMsgError)?
                .raw_bytes
                .to_vec(),
        );
        trace!("email: {:?}", email);

        Ok(email)
    }

    fn copy_email(
        &'a self,
        dir_src: &'a str,
        dir_dst: &'a str,
        short_hash: &'a str,
    ) -> backend::Result<()> {
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

        Ok(())
    }

    fn move_email(
        &'a self,
        dir_src: &'a str,
        dir_dst: &'a str,
        short_hash: &'a str,
    ) -> backend::Result<()> {
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

        Ok(())
    }

    fn delete_email(&'a self, dir: &'a str, short_hash: &'a str) -> backend::Result<()> {
        debug!("dir: {:?}", dir);
        debug!("short hash: {:?}", short_hash);

        let mdir = self.get_mdir_from_dir(dir)?;
        let id = IdMapper::new(mdir.path())?.find(short_hash)?;
        debug!("id: {:?}", id);
        mdir.delete(&id).map_err(Error::DelMsgError)?;

        Ok(())
    }

    fn add_flags(
        &'a self,
        dir: &'a str,
        short_hash: &'a str,
        flags: &'a str,
    ) -> backend::Result<()> {
        debug!("dir: {:?}", dir);
        debug!("short hash: {:?}", short_hash);
        let flags = Flags::from(flags);
        debug!("flags: {:?}", flags);

        let mdir = self.get_mdir_from_dir(dir)?;
        let id = IdMapper::new(mdir.path())?.find(short_hash)?;
        debug!("id: {:?}", id);

        mdir.add_flags(&id, &flags::to_normalized_string(&flags))
            .map_err(Error::AddFlagsError)?;

        Ok(())
    }

    fn set_flags(
        &'a self,
        dir: &'a str,
        short_hash: &'a str,
        flags: &'a str,
    ) -> backend::Result<()> {
        debug!("dir: {:?}", dir);
        debug!("short hash: {:?}", short_hash);
        let flags = Flags::from(flags);
        debug!("flags: {:?}", flags);

        let mdir = self.get_mdir_from_dir(dir)?;
        let id = IdMapper::new(mdir.path())?.find(short_hash)?;
        debug!("id: {:?}", id);
        mdir.set_flags(&id, &flags::to_normalized_string(&flags))
            .map_err(Error::SetFlagsError)?;

        Ok(())
    }

    fn delete_flags(
        &'a self,
        dir: &'a str,
        short_hash: &'a str,
        flags: &'a str,
    ) -> backend::Result<()> {
        debug!("dir: {:?}", dir);
        debug!("short hash: {:?}", short_hash);
        let flags = Flags::from(flags);
        debug!("flags: {:?}", flags);

        let mdir = self.get_mdir_from_dir(dir)?;
        let id = IdMapper::new(mdir.path())?.find(short_hash)?;
        debug!("id: {:?}", id);
        mdir.remove_flags(&id, &flags::to_normalized_string(&flags))
            .map_err(Error::DelFlagsError)?;

        Ok(())
    }

    fn as_any(&self) -> &(dyn Any + 'a) {
        self
    }
}
