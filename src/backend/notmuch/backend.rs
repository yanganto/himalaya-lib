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

use log::{debug, info, trace};
use std::{fs, io, result};
use thiserror::Error;

use crate::{
    backend, config, email, envelope::notmuch::envelopes, id_mapper, Backend, Config, Email,
    Envelopes, Folder, Folders, IdMapper, MaildirBackend, NotmuchConfig,
};

#[derive(Debug, Error)]
pub enum Error {
    #[error("cannot parse notmuch message header {1}")]
    ParseMsgHeaderError(#[source] notmuch::Error, String),
    #[error("cannot parse notmuch message date {1}")]
    ParseMsgDateError(#[source] chrono::ParseError, String),
    #[error("cannot find notmuch message header {0}")]
    FindMsgHeaderError(String),
    #[error("cannot find notmuch message sender")]
    FindSenderError,
    #[error("cannot parse notmuch message senders {1}")]
    ParseSendersError(#[source] mailparse::MailParseError, String),
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
    ConfigError(#[from] config::Error),
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
    config: &'a Config,
    notmuch_config: &'a NotmuchConfig,
    pub mdir: &'a mut MaildirBackend<'a>,
    db: notmuch::Database,
}

impl<'a> NotmuchBackend<'a> {
    pub fn new(
        config: &'a Config,
        notmuch_config: &'a NotmuchConfig,
        mdir: &'a mut MaildirBackend<'a>,
    ) -> Result<NotmuchBackend<'a>> {
        info!(">> create new notmuch backend");

        let backend = Self {
            config,
            notmuch_config,
            mdir,
            db: notmuch::Database::open(
                notmuch_config.db_path.clone(),
                notmuch::DatabaseMode::ReadWrite,
            )
            .map_err(Error::OpenDbError)?,
        };

        info!("<< create new notmuch backend");
        Ok(backend)
    }

    fn _search_envelopes(
        &mut self,
        query: &str,
        page_size: usize,
        page: usize,
    ) -> Result<Envelopes> {
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
        envelopes.envelopes = envelopes[page_begin..page_end].to_owned();

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
    fn folder_add(&mut self, _mbox: &str) -> backend::Result<()> {
        Err(Error::AddMboxUnimplementedError)?
    }

    fn folder_list(&mut self) -> backend::Result<Folders> {
        trace!(">> get notmuch virtual folders");

        let mut mboxes = Folders::default();
        for (name, desc) in &self.config.folder_aliases()? {
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

    fn folder_delete(&mut self, _mbox: &str) -> backend::Result<()> {
        Err(Error::DelMboxUnimplementedError)?
    }

    fn envelope_list(
        &mut self,
        virt_mbox: &str,
        page_size: usize,
        page: usize,
    ) -> backend::Result<Envelopes> {
        info!(">> get notmuch envelopes");
        debug!("virtual folder: {:?}", virt_mbox);
        debug!("page size: {:?}", page_size);
        debug!("page: {:?}", page);

        let query = self
            .config
            .folder_alias(virt_mbox)
            .unwrap_or_else(|_| String::from("all"));
        debug!("query: {:?}", query);
        let envelopes = self._search_envelopes(&query, page_size, page)?;

        info!("<< get notmuch envelopes");
        Ok(envelopes)
    }

    fn envelope_search(
        &mut self,
        virt_mbox: &str,
        query: &str,
        _sort: &str,
        page_size: usize,
        page: usize,
    ) -> backend::Result<Envelopes> {
        info!(">> search notmuch envelopes");
        debug!("virtual folder: {:?}", virt_mbox);
        debug!("query: {:?}", query);
        debug!("page size: {:?}", page_size);
        debug!("page: {:?}", page);

        let query = if query.is_empty() {
            self.config
                .folder_alias(virt_mbox)
                .unwrap_or_else(|_| String::from("all"))
        } else {
            query.to_owned()
        };
        debug!("final query: {:?}", query);
        let envelopes = self._search_envelopes(&query, page_size, page)?;

        info!("<< search notmuch envelopes");
        Ok(envelopes)
    }

    fn email_add(&mut self, _: &str, msg: &[u8], tags: &str) -> backend::Result<String> {
        info!(">> add notmuch envelopes");
        debug!("tags: {:?}", tags);

        let dir = &self.notmuch_config.db_path;

        // Adds the message to the maildir folder and gets its hash.
        let hash = self.mdir.email_add("", msg, "seen")?;
        debug!("hash: {:?}", hash);

        // Retrieves the file path of the added message by its maildir
        // identifier.
        let mut mapper = IdMapper::new(dir)?;
        let id = mapper.find(&hash)?;
        debug!("id: {:?}", id);
        let file_path = dir.join("cur").join(format!("{}:2,S", id));
        debug!("file path: {:?}", file_path);

        // Adds the message to the notmuch database by indexing it.
        let id = self
            .db
            .index_file(file_path, None)
            .map_err(Error::IndexFileError)?
            .id()
            .to_string();
        let hash = format!("{:x}", md5::compute(&id));

        // Appends hash entry to the id mapper cache file.
        mapper.append(vec![(hash.clone(), id.clone())])?;

        // Attaches tags to the notmuch message.
        self.flags_add("", &hash, tags)?;

        info!("<< add notmuch envelopes");
        Ok(hash)
    }

    fn email_get(&mut self, _: &str, short_hash: &str) -> backend::Result<Email> {
        info!(">> add notmuch envelopes");
        debug!("short hash: {:?}", short_hash);

        let dir = &self.notmuch_config.db_path;
        let id = IdMapper::new(dir)?.find(short_hash)?;
        debug!("id: {:?}", id);
        let msg_file_path = self
            .db
            .find_message(&id)
            .map_err(Error::FindMsgError)?
            .ok_or_else(|| Error::FindMsgEmptyError)?
            .filename()
            .to_owned();
        debug!("message file path: {:?}", msg_file_path);
        let raw_msg = fs::read(&msg_file_path).map_err(Error::ReadMsgError)?;
        let msg = mailparse::parse_mail(&raw_msg).map_err(Error::ParseMsgError)?;
        let msg = Email::from_parsed_mail(msg, &self.config)?;
        trace!("message: {:?}", msg);

        info!("<< get notmuch message");
        Ok(msg)
    }

    fn email_list(&mut self, _: &str, short_hash: &str) -> backend::Result<Email> {
        info!(">> add notmuch envelopes");
        debug!("short hash: {:?}", short_hash);

        let dir = &self.notmuch_config.db_path;
        let id = IdMapper::new(dir)?.find(short_hash)?;
        debug!("id: {:?}", id);
        let msg_file_path = self
            .db
            .find_message(&id)
            .map_err(Error::FindMsgError)?
            .ok_or_else(|| Error::FindMsgEmptyError)?
            .filename()
            .to_owned();
        debug!("message file path: {:?}", msg_file_path);
        let raw_msg = fs::read(&msg_file_path).map_err(Error::ReadMsgError)?;
        let msg = mailparse::parse_mail(&raw_msg).map_err(Error::ParseMsgError)?;
        let msg = Email::from_parsed_mail(msg, &self.config)?;
        trace!("message: {:?}", msg);

        info!("<< get notmuch message");
        Ok(msg)
    }

    fn email_copy(
        &mut self,
        _dir_src: &str,
        _dir_dst: &str,
        _short_hash: &str,
    ) -> backend::Result<()> {
        info!(">> copy notmuch message");
        info!("<< copy notmuch message");
        Err(Error::CopyMsgUnimplementedError)?
    }

    fn email_move(
        &mut self,
        _dir_src: &str,
        _dir_dst: &str,
        _short_hash: &str,
    ) -> backend::Result<()> {
        info!(">> move notmuch message");
        info!("<< move notmuch message");
        Err(Error::MoveMsgUnimplementedError)?
    }

    fn email_delete(&mut self, _virt_mbox: &str, short_hash: &str) -> backend::Result<()> {
        info!(">> delete notmuch message");
        debug!("short hash: {:?}", short_hash);

        let dir = &self.notmuch_config.db_path;
        let id = IdMapper::new(dir)?.find(short_hash)?;
        debug!("id: {:?}", id);
        let msg_file_path = self
            .db
            .find_message(&id)
            .map_err(Error::FindMsgError)?
            .ok_or_else(|| Error::FindMsgEmptyError)?
            .filename()
            .to_owned();
        debug!("message file path: {:?}", msg_file_path);
        self.db
            .remove_message(msg_file_path)
            .map_err(Error::DelMsgError)?;

        info!("<< delete notmuch message");
        Ok(())
    }

    fn flags_add(&mut self, _virt_mbox: &str, short_hash: &str, tags: &str) -> backend::Result<()> {
        info!(">> add notmuch message flags");
        debug!("tags: {:?}", tags);

        let dir = &self.notmuch_config.db_path;
        let id = IdMapper::new(dir)?.find(short_hash)?;
        debug!("id: {:?}", id);
        let query = format!("id:{}", id);
        debug!("query: {:?}", query);
        let tags: Vec<_> = tags.split_whitespace().collect();
        let query_builder = self
            .db
            .create_query(&query)
            .map_err(Error::BuildQueryError)?;
        let msgs = query_builder
            .search_messages()
            .map_err(Error::SearchEnvelopesError)?;

        for msg in msgs {
            for tag in tags.iter() {
                msg.add_tag(*tag).map_err(Error::AddTagError)?;
            }
        }

        info!("<< add notmuch message flags");
        Ok(())
    }

    fn flags_set(&mut self, _virt_mbox: &str, short_hash: &str, tags: &str) -> backend::Result<()> {
        info!(">> set notmuch message flags");
        debug!("tags: {:?}", tags);

        let dir = &self.notmuch_config.db_path;
        let id = IdMapper::new(dir)?.find(short_hash)?;
        debug!("id: {:?}", id);
        let query = format!("id:{}", id);
        debug!("query: {:?}", query);
        let tags: Vec<_> = tags.split_whitespace().collect();
        let query_builder = self
            .db
            .create_query(&query)
            .map_err(Error::BuildQueryError)?;
        let msgs = query_builder
            .search_messages()
            .map_err(Error::SearchEnvelopesError)?;
        for msg in msgs {
            msg.remove_all_tags().map_err(Error::DelTagError)?;

            for tag in tags.iter() {
                msg.add_tag(*tag).map_err(Error::AddTagError)?;
            }
        }

        info!("<< set notmuch message flags");
        Ok(())
    }

    fn flags_delete(
        &mut self,
        _virt_mbox: &str,
        short_hash: &str,
        tags: &str,
    ) -> backend::Result<()> {
        info!(">> delete notmuch message flags");
        debug!("tags: {:?}", tags);

        let dir = &self.notmuch_config.db_path;
        let id = IdMapper::new(dir)?.find(short_hash)?;
        debug!("id: {:?}", id);
        let query = format!("id:{}", id);
        debug!("query: {:?}", query);
        let tags: Vec<_> = tags.split_whitespace().collect();
        let query_builder = self
            .db
            .create_query(&query)
            .map_err(Error::BuildQueryError)?;
        let msgs = query_builder
            .search_messages()
            .map_err(Error::SearchEnvelopesError)?;
        for msg in msgs {
            for tag in tags.iter() {
                msg.remove_tag(*tag).map_err(Error::DelTagError)?;
            }
        }

        info!("<< delete notmuch message flags");
        Ok(())
    }
}
