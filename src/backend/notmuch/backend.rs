use lettre::address::AddressError;
use log::{debug, trace};
use std::{any::Any, fs, io, result};
use thiserror::Error;

use crate::{
    account, backend, email, envelope::notmuch::envelopes, id_mapper, AccountConfig, Backend,
    Emails, Envelopes, Flag, Flags, Folder, Folders, IdMapper, MaildirBackend, MaildirConfig,
    NotmuchConfig,
};

#[derive(Debug, Error)]
pub enum Error {
    #[error("cannot get notmuch backend from config")]
    GetBackendFromConfigError,
    #[error("cannot get notmuch inner maildir backend")]
    GetMaildirBackendError,
    #[error("cannot parse notmuch message header {1}")]
    ParseMsgHeaderError(#[source] notmuch::Error, String),
    #[error("cannot parse notmuch message date {1}")]
    ParseMsgDateError(#[source] chrono::ParseError, String),
    #[error("cannot find notmuch message header {0}")]
    FindMsgHeaderError(String),
    #[error("cannot find notmuch message sender")]
    FindSenderError,
    #[error("cannot parse notmuch message senders {1}")]
    ParseSendersError(#[source] AddressError, String),
    #[error("cannot open notmuch database")]
    OpenDbError(#[source] notmuch::Error),
    #[error("cannot build notmuch query")]
    BuildQueryError(#[source] notmuch::Error),
    #[error("cannot search notmuch envelopes")]
    SearchEnvelopesError(#[source] notmuch::Error),
    #[error("cannot get notmuch envelopes at page {0}")]
    GetEnvelopesOutOfBoundsError(usize),
    #[error("cannot add notmuch mailbox: feature not implemented")]
    AddMboxUnimplementedError,
    #[error("cannot purge notmuch folder: feature not implemented")]
    PurgeFolderUnimplementedError,
    #[error("cannot delete notmuch mailbox: feature not implemented")]
    DelMboxUnimplementedError,
    #[error("cannot copy notmuch message: feature not implemented")]
    CopyMsgUnimplementedError,
    #[error("cannot move notmuch message: feature not implemented")]
    MoveMsgUnimplementedError,
    #[error("cannot index notmuch message")]
    IndexFileError(#[source] notmuch::Error),
    #[error("cannot find notmuch message")]
    FindMsgError(#[source] notmuch::Error),
    #[error("cannot find notmuch message")]
    FindMsgEmptyError,
    #[error("cannot read notmuch raw message from file")]
    ReadMsgError(#[source] io::Error),
    #[error("cannot parse notmuch raw message")]
    ParseMsgError(#[source] mailparse::MailParseError),
    #[error("cannot delete notmuch message")]
    DelMsgError(#[source] notmuch::Error),
    #[error("cannot add notmuch tag")]
    AddTagError(#[source] notmuch::Error),
    #[error("cannot delete notmuch tag")]
    DelTagError(#[source] notmuch::Error),

    #[error(transparent)]
    ConfigError(#[from] account::config::Error),
    #[error(transparent)]
    IdMapperError(#[from] id_mapper::Error),
    #[error(transparent)]
    EmailError(#[from] email::Error),
    #[error(transparent)]
    MaildirError(#[from] backend::maildir::Error),
}

pub type Result<T> = result::Result<T, Error>;

/// Represents the Notmuch backend.
pub struct NotmuchBackend<'a> {
    account_config: &'a AccountConfig,
    notmuch_config: &'a NotmuchConfig,
    db: notmuch::Database,
}

impl<'a> NotmuchBackend<'a> {
    pub fn new(
        account_config: &'a AccountConfig,
        notmuch_config: &'a NotmuchConfig,
    ) -> Result<Self> {
        let db = notmuch::Database::open(
            notmuch_config.db_path.clone(),
            notmuch::DatabaseMode::ReadWrite,
        )
        .map_err(Error::OpenDbError)?;

        Ok(Self {
            account_config,
            notmuch_config,
            db,
        })
    }

    fn _search_envelopes(&self, query: &str, page_size: usize, page: usize) -> Result<Envelopes> {
        // Gets envelopes matching the given Notmuch query.
        let query_builder = self
            .db
            .create_query(query)
            .map_err(Error::BuildQueryError)?;
        let mut envelopes = envelopes::from_raws(
            query_builder
                .search_messages()
                .map_err(Error::SearchEnvelopesError)?,
        )?;
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
            let mut mapper = IdMapper::new(&self.notmuch_config.db_path)?;
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
}

impl<'a> Backend for NotmuchBackend<'a> {
    fn add_folder(&self, _folder: &str) -> backend::Result<()> {
        Err(Error::AddMboxUnimplementedError)?
    }

    fn list_folder(&self) -> backend::Result<Folders> {
        trace!(">> get notmuch virtual folders");

        let mut mboxes = Folders::default();
        for (name, desc) in &self.account_config.folder_aliases {
            mboxes.push(Folder {
                name: name.into(),
                desc: desc.into(),
                ..Folder::default()
            })
        }
        mboxes.sort_by(|a, b| b.name.partial_cmp(&a.name).unwrap());

        trace!("notmuch virtual folders: {:?}", mboxes);
        Ok(mboxes)
    }

    fn purge_folder(&self, _folder: &str) -> backend::Result<()> {
        Err(Error::PurgeFolderUnimplementedError)?
    }

    fn delete_folder(&self, _folder: &str) -> backend::Result<()> {
        Err(Error::DelMboxUnimplementedError)?
    }

    fn list_envelopes(
        &self,
        virtual_folder: &str,
        page_size: usize,
        page: usize,
    ) -> backend::Result<Envelopes> {
        let query = self
            .account_config
            .folder_alias(virtual_folder)
            .unwrap_or_else(|_| String::from("all"));
        let envelopes = self._search_envelopes(&query, page_size, page)?;

        Ok(envelopes)
    }

    fn search_envelopes(
        &self,
        virtual_folder: &str,
        query: &str,
        _sort: &str,
        page_size: usize,
        page: usize,
    ) -> backend::Result<Envelopes> {
        let query = if query.is_empty() {
            self.account_config
                .folder_alias(virtual_folder)
                .unwrap_or_else(|_| String::from("all"))
        } else {
            query.to_owned()
        };
        debug!("final query: {:?}", query);
        let envelopes = self._search_envelopes(&query, page_size, page)?;

        Ok(envelopes)
    }

    fn add_email(&self, _: &str, email: &[u8], flags: &Flags) -> backend::Result<String> {
        let mut flags = flags.clone();
        flags.insert(Flag::Seen);

        // Adds the message to the maildir folder and gets its hash.
        let mdir_config = MaildirConfig {
            root_dir: self.notmuch_config.db_path.clone(),
        };
        // TODO: find a way to move this to the Backend::connect method.
        let mdir = MaildirBackend::new(self.account_config, &mdir_config);
        let hash = mdir.add_email("", email, &flags)?;
        debug!("hash: {:?}", hash);

        // Retrieves the file path of the added message by its maildir
        // identifier.
        let mut id_mapper = IdMapper::new(&self.notmuch_config.db_path)?;
        let id = id_mapper.find(&hash)?;
        debug!("id: {:?}", id);
        let file_path = self
            .notmuch_config
            .db_path
            .join("cur")
            .join(format!("{}:2,S", id));
        debug!("file path: {:?}", file_path);

        // Adds the message to the notmuch database by indexing it.
        let email = self
            .db
            .index_file(file_path, None)
            .map_err(Error::IndexFileError)?;
        let id = email.id().to_string();
        let hash = format!("{:x}", md5::compute(&id));

        // Appends hash entry to the id mapper cache file.
        id_mapper.append(vec![(hash.clone(), id.clone())])?;

        // Attaches tags to the notmuch message.
        self.add_flags("", vec![&hash], &flags)?;

        Ok(hash)
    }

    fn get_emails(&self, _: &str, short_hashes: Vec<&str>) -> backend::Result<Emails> {
        debug!("short hashes: {:?}", short_hashes);

        let id_mapper = IdMapper::new(&self.notmuch_config.db_path)?;
        let ids: Vec<String> = short_hashes
            .into_iter()
            .map(|short_hash| id_mapper.find(short_hash))
            .collect::<id_mapper::Result<Vec<String>>>()?;
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        debug!("ids: {:?}", ids);

        let emails: Emails = ids
            .iter()
            .map(|id| {
                let email_filepath = self
                    .db
                    .find_message(&id)
                    .map_err(Error::FindMsgError)?
                    .ok_or_else(|| Error::FindMsgEmptyError)?
                    .filename()
                    .to_owned();
                fs::read(&email_filepath).map_err(Error::ReadMsgError)
            })
            .collect::<Result<Vec<_>>>()?
            .into();

        Ok(emails)
    }

    fn copy_emails(
        &self,
        _from_dir: &str,
        _to_dir: &str,
        _short_hashes: Vec<&str>,
    ) -> backend::Result<()> {
        Err(Error::CopyMsgUnimplementedError)?
    }

    fn move_emails(
        &self,
        _from_dir: &str,
        _to_dir: &str,
        _short_hashes: Vec<&str>,
    ) -> backend::Result<()> {
        Err(Error::MoveMsgUnimplementedError)?
    }

    fn delete_emails(&self, _virtual_folder: &str, short_hashes: Vec<&str>) -> backend::Result<()> {
        debug!("short hashes: {:?}", short_hashes);

        let id_mapper = IdMapper::new(&self.notmuch_config.db_path)?;
        let ids: Vec<String> = short_hashes
            .into_iter()
            .map(|short_hash| id_mapper.find(short_hash))
            .collect::<id_mapper::Result<Vec<String>>>()?;
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        debug!("ids: {:?}", ids);

        ids.iter().try_for_each(|id| {
            let msg_file_path = self
                .db
                .find_message(&id)
                .map_err(Error::FindMsgError)?
                .ok_or_else(|| Error::FindMsgEmptyError)?
                .filename()
                .to_owned();
            self.db
                .remove_message(msg_file_path)
                .map_err(Error::DelMsgError)
        })?;

        Ok(())
    }

    fn add_flags(
        &self,
        _virtual_folder: &str,
        short_hashes: Vec<&str>,
        flags: &Flags,
    ) -> backend::Result<()> {
        debug!("short hashes: {:?}", short_hashes);

        let id_mapper = IdMapper::new(&self.notmuch_config.db_path)?;
        let ids: Vec<String> = short_hashes
            .into_iter()
            .map(|short_hash| id_mapper.find(short_hash))
            .collect::<id_mapper::Result<Vec<String>>>()?;
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        debug!("ids: {:?}", ids);

        let query = format!("mid:\"/^({})$/\"", ids.join("|"));
        debug!("query: {:?}", query);

        let query_builder = self
            .db
            .create_query(&query)
            .map_err(Error::BuildQueryError)?;
        let msgs = query_builder
            .search_messages()
            .map_err(Error::SearchEnvelopesError)?;

        for msg in msgs {
            for flag in flags.iter() {
                msg.add_tag(&flag.to_string()).map_err(Error::AddTagError)?;
            }
        }

        Ok(())
    }

    fn set_flags(
        &self,
        _virtual_folder: &str,
        short_hashes: Vec<&str>,
        flags: &Flags,
    ) -> backend::Result<()> {
        debug!("short hashes: {:?}", short_hashes);

        let id_mapper = IdMapper::new(&self.notmuch_config.db_path)?;
        let ids: Vec<String> = short_hashes
            .into_iter()
            .map(|short_hash| id_mapper.find(short_hash))
            .collect::<id_mapper::Result<Vec<String>>>()?;
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        debug!("ids: {:?}", ids);

        let query = format!("mid:\"/^({})$/\"", ids.join("|"));
        debug!("query: {:?}", query);

        let query_builder = self
            .db
            .create_query(&query)
            .map_err(Error::BuildQueryError)?;
        let msgs = query_builder
            .search_messages()
            .map_err(Error::SearchEnvelopesError)?;

        for msg in msgs {
            msg.remove_all_tags().map_err(Error::DelTagError)?;
            for flag in flags.iter() {
                msg.add_tag(&flag.to_string()).map_err(Error::AddTagError)?;
            }
        }

        Ok(())
    }

    fn remove_flags(
        &self,
        _virtual_folder: &str,
        short_hashes: Vec<&str>,
        flags: &Flags,
    ) -> backend::Result<()> {
        debug!("short hashes: {:?}", short_hashes);

        let id_mapper = IdMapper::new(&self.notmuch_config.db_path)?;
        let ids: Vec<String> = short_hashes
            .into_iter()
            .map(|short_hash| id_mapper.find(short_hash))
            .collect::<id_mapper::Result<Vec<String>>>()?;
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        debug!("ids: {:?}", ids);

        let query = format!("mid:\"/^({})$/\"", ids.join("|"));
        debug!("query: {:?}", query);

        let query_builder = self
            .db
            .create_query(&query)
            .map_err(Error::BuildQueryError)?;
        let msgs = query_builder
            .search_messages()
            .map_err(Error::SearchEnvelopesError)?;

        for msg in msgs {
            for flag in flags.iter() {
                msg.remove_tag(&flag.to_string())
                    .map_err(Error::AddTagError)?;
            }
        }

        Ok(())
    }

    fn as_any(&self) -> &(dyn Any + 'a) {
        self
    }
}
