//! Maildir backend module.
//!
//! This module contains the definition of the maildir backend and its
//! traits implementation.

use log::{debug, trace, warn};
use maildir::Maildir;
use std::{
    any::Any,
    borrow::Cow,
    env,
    ffi::OsStr,
    fs, io,
    path::{self, PathBuf},
    result,
};
use thiserror::Error;

use crate::{
    account, backend, email,
    envelope::maildir::{envelope, envelopes},
    flag::maildir::flags,
    id_mapper, AccountConfig, Backend, Emails, Envelope, Envelopes, Flag, Flags, Folder, Folders,
    IdMapper, MaildirConfig, ThreadSafeBackend, DEFAULT_INBOX_FOLDER,
};

#[derive(Debug, Error)]
pub enum Error {
    #[error("cannot parse header date as timestamp")]
    ParseDateHeaderError,
    #[error("cannot get envelope by short hash {0}")]
    GetEnvelopeError(String),
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
    #[error("cannot init maildir directories")]
    InitDirsError(#[source] io::Error),
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
    account_config: Cow<'a, AccountConfig>,
    mdir: maildir::Maildir,
}

impl Clone for MaildirBackend<'_> {
    fn clone(&self) -> Self {
        Self {
            account_config: self.account_config.clone(),
            mdir: self.mdir.path().to_owned().into(),
        }
    }
}

impl<'a> MaildirBackend<'a> {
    pub fn new(
        account_config: Cow<'a, AccountConfig>,
        backend_config: Cow<'a, MaildirConfig>,
    ) -> Result<Self> {
        let mdir = Maildir::from(backend_config.root_dir.clone());
        mdir.create_dirs().map_err(Error::InitDirsError)?;

        Ok(Self {
            account_config,
            mdir,
        })
    }

    fn validate_mdir_path(&self, mdir_path: PathBuf) -> Result<PathBuf> {
        if mdir_path.is_dir() {
            Ok(mdir_path)
        } else {
            Err(Error::ReadDirError(mdir_path.to_owned()))
        }
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

impl<'a> Backend for MaildirBackend<'a> {
    fn name(&self) -> String {
        self.account_config.name.clone()
    }

    fn add_folder(&self, subdir: &str) -> backend::Result<()> {
        debug!("subdir: {:?}", subdir);

        let path = match self.account_config.folder_alias(subdir)?.as_str() {
            DEFAULT_INBOX_FOLDER => self.mdir.path().join("cur"),
            dir => self.mdir.path().join(format!(".{}", dir)),
        };
        debug!("subdir path: {:?}", path);

        fs::create_dir_all(&path)
            .map_err(|err| Error::CreateSubdirError(err, subdir.to_owned()))?;

        Ok(())
    }

    fn list_folders(&self) -> backend::Result<Folders> {
        let mut folders = Folders::default();

        folders.push(Folder {
            delim: String::from("/"),
            name: self.account_config.folder_alias(DEFAULT_INBOX_FOLDER)?,
            desc: DEFAULT_INBOX_FOLDER.into(),
        });

        for (name, alias) in &self.account_config.folder_aliases {
            match name.to_lowercase().as_str() {
                "inbox" => (),
                name => folders.push(Folder {
                    delim: String::from("/"),
                    name: alias.into(),
                    desc: name.into(),
                }),
            }
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

    fn purge_folder(&self, dir: &str) -> backend::Result<()> {
        debug!("dir: {}", dir);

        let mdir = self.get_mdir_from_dir(dir)?;
        let ids: Vec<maildir::MailEntry> = mdir.list_cur().filter_map(io::Result::ok).collect();
        let ids: Vec<&str> = ids.iter().map(|entry| entry.id()).collect();

        self.delete_emails(dir, ids)?;

        Ok(())
    }

    fn delete_folder(&self, dir: &str) -> backend::Result<()> {
        debug!("dir: {:?}", dir);

        let path = self.mdir.path().join(format!(".{}", dir));
        debug!("dir path: {:?}", path);

        fs::remove_dir_all(&path).map_err(|err| Error::DeleteAllDirError(err, path.to_owned()))?;
        Ok(())
    }

    fn get_envelope(&self, dir: &str, short_hash: &str) -> backend::Result<Envelope> {
        debug!("dir: {}", dir);
        debug!("short hash: {}", short_hash);

        let mdir = self.get_mdir_from_dir(dir)?;
        let id_mapper = IdMapper::new(mdir.path())?;
        let id = id_mapper.find(short_hash)?;
        debug!("id: {}", id);

        let envelope = envelope::from_raw(
            mdir.find(&id)
                .ok_or_else(|| Error::GetEnvelopeError(short_hash.to_owned()))?,
        )?;

        Ok(envelope)
    }

    fn get_envelope_internal(&self, dir: &str, internal_id: &str) -> backend::Result<Envelope> {
        debug!("dir: {}", dir);
        debug!("internal id: {}", internal_id);

        let mdir = self.get_mdir_from_dir(dir)?;

        let envelope = envelope::from_raw(
            mdir.find(internal_id)
                .ok_or_else(|| Error::GetEnvelopeError(internal_id.to_owned()))?,
        )?;

        Ok(envelope)
    }

    fn list_envelopes(
        &self,
        dir: &str,
        page_size: usize,
        page: usize,
    ) -> backend::Result<Envelopes> {
        debug!("dir: {}", dir);
        debug!("page size: {}", page_size);
        debug!("page: {}", page);

        let mdir = self.get_mdir_from_dir(dir)?;

        // Reads envelopes from the "cur" folder of the selected
        // maildir.
        let mut envelopes = envelopes::from_raws(mdir.list_cur())?;
        debug!("envelopes len: {:?}", envelopes.len());
        trace!("envelopes: {:?}", envelopes);

        // Calculates pagination boundaries.
        let page_begin = page * page_size;
        debug!("page begin: {}", page_begin);
        if page_begin > envelopes.len() {
            return Err(Error::GetEnvelopesOutOfBoundsError(page_begin + 1))?;
        }
        let page_end = envelopes.len().min(if page_size == 0 {
            envelopes.len()
        } else {
            page_begin + page_size
        });
        debug!("page end: {}", page_end);

        // Sorts envelopes by most recent date.
        envelopes.sort_by(|a, b| b.date.partial_cmp(&a.date).unwrap());

        // Applies pagination boundaries.
        *envelopes = envelopes[page_begin..page_end].to_owned();

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

    fn search_envelopes(
        &self,
        _dir: &str,
        _query: &str,
        _sort: &str,
        _page_size: usize,
        _page: usize,
    ) -> backend::Result<Envelopes> {
        Err(Error::SearchEnvelopesUnimplementedError)?
    }

    fn add_email(&self, dir: &str, email: &[u8], flags: &Flags) -> backend::Result<String> {
        debug!("dir: {}", dir);
        debug!("flags: {:?}", flags);

        let mut flags = flags.clone();
        flags.insert(Flag::Seen);

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

    fn add_email_internal(
        &self,
        dir: &str,
        email: &[u8],
        flags: &Flags,
    ) -> backend::Result<String> {
        debug!("dir: {}", dir);
        debug!("flags: {:?}", flags);

        let mut flags = flags.clone();
        flags.insert(Flag::Seen);

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

        Ok(id.clone())
    }

    fn get_emails(&self, dir: &str, short_hashes: Vec<&str>) -> backend::Result<Emails> {
        debug!("dir: {:?}", dir);
        debug!("short hashes: {:?}", short_hashes);

        let mdir = self.get_mdir_from_dir(dir)?;
        let id_mapper = IdMapper::new(mdir.path())?;
        let ids = short_hashes
            .iter()
            .map(|short_hash| id_mapper.find(short_hash))
            .collect::<id_mapper::Result<Vec<_>>>()?;
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        debug!("ids: {:?}", ids);

        let matching_ids = |entry: io::Result<maildir::MailEntry>| match entry {
            Ok(entry) => {
                if let Some(pos) = ids.iter().position(|id| *id == entry.id()) {
                    Some((pos, entry))
                } else {
                    None
                }
            }
            Err(err) => {
                warn!("skipping invalid maildir entry: {}", err);
                None
            }
        };

        let mut emails = mdir
            .list_cur()
            .filter_map(&matching_ids)
            .chain(mdir.list_new().filter_map(&matching_ids))
            .collect::<Vec<(usize, maildir::MailEntry)>>();

        emails.sort_by_key(|(pos, _)| *pos);

        let emails: Emails = emails
            .into_iter()
            .map(|(_, entry)| entry)
            .collect::<Vec<_>>()
            .try_into()?;

        Ok(emails)
    }

    fn get_emails_internal(&self, dir: &str, internal_ids: Vec<&str>) -> backend::Result<Emails> {
        debug!("dir: {:?}", dir);
        debug!("internal ids: {:?}", internal_ids);

        let mdir = self.get_mdir_from_dir(dir)?;

        let matching_internal_ids = |entry: io::Result<maildir::MailEntry>| match entry {
            Ok(entry) if internal_ids.contains(&entry.id()) => Some(entry),
            Ok(_) => None,
            Err(err) => {
                warn!("skipping invalid maildir entry: {}", err);
                None
            }
        };

        let emails: Emails = mdir
            .list_cur()
            .filter_map(&matching_internal_ids)
            .chain(mdir.list_new().filter_map(&matching_internal_ids))
            .collect::<Vec<maildir::MailEntry>>()
            .try_into()?;

        Ok(emails)
    }

    fn copy_emails(
        &self,
        from_dir: &str,
        to_dir: &str,
        short_hashes: Vec<&str>,
    ) -> backend::Result<()> {
        debug!("from dir: {:?}", from_dir);
        debug!("to dir: {:?}", to_dir);
        debug!("short hashes: {:?}", short_hashes);

        let from_mdir = self.get_mdir_from_dir(from_dir)?;
        let to_mdir = self.get_mdir_from_dir(to_dir)?;
        let id_mapper = IdMapper::new(from_mdir.path())?;
        let ids = short_hashes
            .iter()
            .map(|short_hash| id_mapper.find(short_hash))
            .collect::<id_mapper::Result<Vec<_>>>()?;
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        debug!("ids: {:?}", ids);

        let mut id_mapper = IdMapper::new(to_mdir.path())?;
        for id in ids {
            from_mdir
                .copy_to(&id, &to_mdir)
                .map_err(Error::CopyMsgError)?;

            // Appends hash entry to the id mapper cache file.
            let hash = format!("{:x}", md5::compute(&id));
            id_mapper.append(vec![(hash.clone(), id.to_owned())])?;
        }

        Ok(())
    }

    fn copy_emails_internal(
        &self,
        from_dir: &str,
        to_dir: &str,
        internal_ids: Vec<&str>,
    ) -> backend::Result<()> {
        debug!("from dir: {:?}", from_dir);
        debug!("to dir: {:?}", to_dir);
        debug!("internal ids: {:?}", internal_ids);

        let from_mdir = self.get_mdir_from_dir(from_dir)?;
        let to_mdir = self.get_mdir_from_dir(to_dir)?;
        let mut id_mapper = IdMapper::new(to_mdir.path())?;

        for internal_id in internal_ids {
            from_mdir
                .copy_to(&internal_id, &to_mdir)
                .map_err(Error::CopyMsgError)?;

            // Appends hash entry to the id mapper cache file.
            let hash = format!("{:x}", md5::compute(&internal_id));
            id_mapper.append(vec![(hash.clone(), internal_id.to_owned())])?;
        }

        Ok(())
    }

    fn move_emails(
        &self,
        from_dir: &str,
        to_dir: &str,
        short_hashes: Vec<&str>,
    ) -> backend::Result<()> {
        debug!("from dir: {:?}", from_dir);
        debug!("to dir: {:?}", to_dir);
        debug!("short hashes: {:?}", short_hashes);

        let from_mdir = self.get_mdir_from_dir(from_dir)?;
        let to_mdir = self.get_mdir_from_dir(to_dir)?;
        let id_mapper = IdMapper::new(from_mdir.path())?;
        let ids = short_hashes
            .iter()
            .map(|short_hash| id_mapper.find(short_hash))
            .collect::<id_mapper::Result<Vec<_>>>()?;
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        debug!("ids: {:?}", ids);

        let mut id_mapper = IdMapper::new(to_mdir.path())?;
        for id in ids {
            from_mdir
                .move_to(&id, &to_mdir)
                .map_err(Error::MoveMsgError)?;

            // Appends hash entry to the id mapper cache file.
            let hash = format!("{:x}", md5::compute(&id));
            id_mapper.append(vec![(hash.clone(), id.to_owned())])?;
        }

        Ok(())
    }

    fn move_emails_internal(
        &self,
        from_dir: &str,
        to_dir: &str,
        internal_ids: Vec<&str>,
    ) -> backend::Result<()> {
        debug!("from dir: {:?}", from_dir);
        debug!("to dir: {:?}", to_dir);
        debug!("internal ids: {:?}", internal_ids);

        let from_mdir = self.get_mdir_from_dir(from_dir)?;
        let to_mdir = self.get_mdir_from_dir(to_dir)?;
        let mut id_mapper = IdMapper::new(to_mdir.path())?;

        for internal_id in internal_ids {
            from_mdir
                .move_to(&internal_id, &to_mdir)
                .map_err(Error::MoveMsgError)?;

            // Appends hash entry to the id mapper cache file.
            let hash = format!("{:x}", md5::compute(&internal_id));
            id_mapper.append(vec![(hash.clone(), internal_id.to_owned())])?;
        }

        Ok(())
    }

    fn delete_emails(&self, dir: &str, short_hashes: Vec<&str>) -> backend::Result<()> {
        debug!("dir: {:?}", dir);
        debug!("short hashes: {:?}", short_hashes);

        let mdir = self.get_mdir_from_dir(dir)?;
        let id_mapper = IdMapper::new(mdir.path())?;
        let ids = short_hashes
            .iter()
            .map(|short_hash| id_mapper.find(short_hash))
            .collect::<id_mapper::Result<Vec<_>>>()?;
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        debug!("ids: {:?}", ids);

        for id in ids {
            mdir.delete(&id).map_err(Error::DelMsgError)?;
        }

        Ok(())
    }

    fn delete_emails_internal(&self, dir: &str, internal_ids: Vec<&str>) -> backend::Result<()> {
        debug!("dir: {:?}", dir);
        debug!("internal ids: {:?}", internal_ids);

        let mdir = self.get_mdir_from_dir(dir)?;

        for internal_id in internal_ids {
            mdir.delete(&internal_id).map_err(Error::DelMsgError)?;
        }

        Ok(())
    }

    fn add_flags(&self, dir: &str, short_hashes: Vec<&str>, flags: &Flags) -> backend::Result<()> {
        debug!("dir: {}", dir);
        debug!("short hashes: {:?}", short_hashes);
        debug!("flags: {:?}", flags);

        let mdir = self.get_mdir_from_dir(dir)?;
        let id_mapper = IdMapper::new(mdir.path())?;
        let ids = short_hashes
            .iter()
            .map(|short_hash| id_mapper.find(short_hash))
            .collect::<id_mapper::Result<Vec<_>>>()?;
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        debug!("ids: {:?}", ids);

        for id in ids {
            mdir.add_flags(&id, &flags::to_normalized_string(&flags))
                .map_err(Error::AddFlagsError)?;
        }

        Ok(())
    }

    fn add_flags_internal(
        &self,
        dir: &str,
        internal_ids: Vec<&str>,
        flags: &Flags,
    ) -> backend::Result<()> {
        debug!("dir: {}", dir);
        debug!("internal ids: {:?}", internal_ids);
        debug!("flags: {:?}", flags);

        let mdir = self.get_mdir_from_dir(dir)?;

        for internal_id in internal_ids {
            mdir.add_flags(&internal_id, &flags::to_normalized_string(&flags))
                .map_err(Error::AddFlagsError)?;
        }

        Ok(())
    }

    fn set_flags(&self, dir: &str, short_hashes: Vec<&str>, flags: &Flags) -> backend::Result<()> {
        debug!("dir: {}", dir);
        debug!("short hashes: {:?}", short_hashes);
        debug!("flags: {:?}", flags);

        let mdir = self.get_mdir_from_dir(dir)?;
        let id_mapper = IdMapper::new(mdir.path())?;
        let ids = short_hashes
            .iter()
            .map(|short_hash| id_mapper.find(short_hash))
            .collect::<id_mapper::Result<Vec<_>>>()?;
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        debug!("ids: {:?}", ids);

        for id in ids {
            mdir.set_flags(&id, &flags::to_normalized_string(&flags))
                .map_err(Error::SetFlagsError)?;
        }

        Ok(())
    }

    fn set_flags_internal(
        &self,
        dir: &str,
        internal_ids: Vec<&str>,
        flags: &Flags,
    ) -> backend::Result<()> {
        debug!("dir: {}", dir);
        debug!("internal ids: {:?}", internal_ids);
        debug!("flags: {:?}", flags);

        let mdir = self.get_mdir_from_dir(dir)?;

        for internal_id in internal_ids {
            mdir.set_flags(internal_id, &flags::to_normalized_string(&flags))
                .map_err(Error::SetFlagsError)?;
        }

        Ok(())
    }

    fn remove_flags(
        &self,
        dir: &str,
        short_hashes: Vec<&str>,
        flags: &Flags,
    ) -> backend::Result<()> {
        debug!("dir: {}", dir);
        debug!("short hashes: {:?}", short_hashes);
        debug!("flags: {:?}", flags);

        let mdir = self.get_mdir_from_dir(dir)?;
        let id_mapper = IdMapper::new(mdir.path())?;
        let ids = short_hashes
            .iter()
            .map(|short_hash| id_mapper.find(short_hash))
            .collect::<id_mapper::Result<Vec<_>>>()?;
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        debug!("ids: {:?}", ids);

        for id in ids {
            mdir.remove_flags(&id, &flags::to_normalized_string(&flags))
                .map_err(Error::DelFlagsError)?;
        }

        Ok(())
    }

    fn remove_flags_internal(
        &self,
        dir: &str,
        internal_ids: Vec<&str>,
        flags: &Flags,
    ) -> backend::Result<()> {
        debug!("dir: {}", dir);
        debug!("internal ids: {:?}", internal_ids);
        debug!("flags: {:?}", flags);

        let mdir = self.get_mdir_from_dir(dir)?;

        for internal_id in internal_ids {
            mdir.remove_flags(&internal_id, &flags::to_normalized_string(&flags))
                .map_err(Error::DelFlagsError)?;
        }

        Ok(())
    }

    fn as_any(&self) -> &(dyn Any + 'a) {
        self
    }
}

impl ThreadSafeBackend for MaildirBackend<'_> {}
