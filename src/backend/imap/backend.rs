//! IMAP backend module.
//!
//! This module contains the definition of the IMAP backend.

use imap::extensions::idle::{stop_on_any, SetReadTimeout};
use log::{debug, log_enabled, trace, Level};
use native_tls::{TlsConnector, TlsStream};
use std::{
    any::Any,
    borrow::Cow,
    collections::HashSet,
    convert::TryInto,
    io::{self, Read, Write},
    net::TcpStream,
    result,
    string::FromUtf8Error,
    sync::{Mutex, MutexGuard},
    thread,
    time::Duration,
};
use thiserror::Error;
use utf7_imap::{decode_utf7_imap as decode_utf7, encode_utf7_imap as encode_utf7};

use crate::{
    account, backend, email, envelope, process, AccountConfig, Backend, Emails, Envelope,
    Envelopes, Flag, Flags, Folder, Folders, ImapConfig, ThreadSafeBackend,
};

#[derive(Error, Debug)]
pub enum Error {
    #[error("cannot find session from pool at cursor {0}")]
    FindSessionByCursorError(usize),
    #[error("cannot parse Message-ID of email {0}")]
    ParseMessageIdError(#[source] FromUtf8Error, u32),
    #[error("cannot lock imap session: {0}")]
    LockSessionError(String),
    #[error("cannot lock imap sessions pool cursor: {0}")]
    LockSessionsPoolCursorError(String),
    #[error("cannot get imap session: session not initialized")]
    GetSessionNotInitializedError,
    #[error("cannot get last imap message uid")]
    GetLastUidError,
    #[error("cannot get imap fetches: fetches not initialized")]
    GetFetchesNotInitializedError,

    #[error("cannot get imap backend from config")]
    GetBackendFromConfigError,
    #[error("cannot get envelope of message {0}")]
    GetEnvelopeError(String),
    #[error("cannot get sender of message {0}")]
    GetSenderError(u32),
    #[error("cannot get internal date of email {0}")]
    GetInternalDateError(u32),
    #[error("cannot get imap session")]
    GetSessionError,
    #[error("cannot retrieve message {0}'s uid")]
    GetMsgUidError(u32),
    #[error("cannot find message {0}")]
    FindMsgError(String),
    #[error("cannot parse sort criterion {0}")]
    ParseSortCriterionError(String),

    #[error("cannot decode subject of message {1}")]
    DecodeSubjectError(#[source] rfc2047_decoder::Error, u32),
    #[error("cannot decode sender name of message {1}")]
    DecodeSenderNameError(#[source] rfc2047_decoder::Error, u32),
    #[error("cannot decode sender mailbox of message {1}")]
    DecodeSenderMboxError(#[source] rfc2047_decoder::Error, u32),
    #[error("cannot decode sender host of message {1}")]
    DecodeSenderHostError(#[source] rfc2047_decoder::Error, u32),

    #[error("cannot copy email(s) {1} from {2} to {3}")]
    CopyEmailError(#[source] imap::Error, String, String, String),
    #[error("cannot move email(s) {1} from {2} to {3}")]
    MoveEmailError(#[source] imap::Error, String, String, String),
    #[error("cannot create tls connector")]
    CreateTlsConnectorError(#[source] native_tls::Error),
    #[error("cannot connect to imap server")]
    ConnectImapServerError(#[source] imap::Error),
    #[error("cannot login to imap server")]
    LoginImapServerError(#[source] imap::Error),
    #[error("cannot search new messages")]
    SearchNewMsgsError(#[source] imap::Error),
    #[error("cannot examine mailbox {1}")]
    ExamineMboxError(#[source] imap::Error, String),
    #[error("cannot start the idle mode")]
    StartIdleModeError(#[source] imap::Error),
    #[error("cannot parse message {1}")]
    ParseMsgError(#[source] mailparse::MailParseError, String),
    #[error("cannot fetch new messages envelope")]
    FetchNewMsgsEnvelopeError(#[source] imap::Error),
    #[error("cannot get uid of message {0}")]
    GetUidError(u32),
    #[error("cannot create mailbox {1}")]
    CreateMboxError(#[source] imap::Error, String),
    #[error("cannot list mailboxes")]
    ListMboxesError(#[source] imap::Error),
    #[error("cannot delete mailbox {1}")]
    DeleteMboxError(#[source] imap::Error, String),
    #[error("cannot select mailbox {1}")]
    SelectFolderError(#[source] imap::Error, String),
    #[error("cannot fetch messages within range {1}")]
    FetchMsgsByRangeError(#[source] imap::Error, String),
    #[error("cannot fetch messages by sequence {1}")]
    GetEmailsBySeqError(#[source] imap::Error, String),
    #[error("cannot append message to mailbox {1}")]
    AppendMsgError(#[source] imap::Error, String),
    #[error("cannot sort messages in mailbox {1} with query: {2}")]
    SortMsgsError(#[source] imap::Error, String, String),
    #[error("cannot search messages in mailbox {1} with query: {2}")]
    SearchMsgsError(#[source] imap::Error, String, String),
    #[error("cannot expunge mailbox {1}")]
    ExpungeError(#[source] imap::Error, String),
    #[error("cannot add flags {1} to message(s) {2}")]
    AddFlagsError(#[source] imap::Error, String, String),
    #[error("cannot set flags {1} to message(s) {2}")]
    SetFlagsError(#[source] imap::Error, String, String),
    #[error("cannot delete flags {1} to message(s) {2}")]
    DelFlagsError(#[source] imap::Error, String, String),
    #[error("cannot close imap session")]
    CloseImapSessionError(#[source] imap::Error),

    #[error(transparent)]
    ConfigError(#[from] account::config::Error),
    #[error(transparent)]
    ImapConfigError(#[from] backend::imap::config::Error),
    #[error(transparent)]
    MsgError(#[from] email::Error),
}

pub type Result<T> = result::Result<T, Error>;

pub enum ImapSessionStream {
    Tls(TlsStream<TcpStream>),
    Tcp(TcpStream),
}

impl SetReadTimeout for ImapSessionStream {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> imap::Result<()> {
        match self {
            Self::Tls(stream) => stream.set_read_timeout(timeout),
            Self::Tcp(stream) => stream.set_read_timeout(timeout),
        }
    }
}

impl Read for ImapSessionStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Tls(stream) => stream.read(buf),
            Self::Tcp(stream) => stream.read(buf),
        }
    }
}

impl Write for ImapSessionStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Tls(stream) => stream.write(buf),
            Self::Tcp(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Tls(stream) => stream.flush(),
            Self::Tcp(stream) => stream.flush(),
        }
    }
}

pub type ImapSession = imap::Session<ImapSessionStream>;

pub struct ImapBackendBuilder {
    sessions_pool_size: usize,
}

impl Default for ImapBackendBuilder {
    fn default() -> Self {
        Self {
            sessions_pool_size: 1,
        }
    }
}

impl<'a> ImapBackendBuilder {
    pub fn pool_size(mut self, pool_size: usize) -> Self {
        self.sessions_pool_size = pool_size;
        self
    }

    pub fn build(
        &self,
        account_config: Cow<'a, AccountConfig>,
        imap_config: Cow<'a, ImapConfig>,
    ) -> Result<ImapBackend<'a>> {
        Ok(ImapBackend {
            account_config,
            imap_config: imap_config.clone(),
            sessions_pool_size: self.sessions_pool_size,
            sessions_pool_cursor: Mutex::new(0),
            sessions_pool: (0..self.sessions_pool_size).try_fold(vec![], |mut pool, _| {
                ImapBackend::create_session(Cow::Borrowed(&imap_config)).map(|session| {
                    pool.push(Mutex::new(session));
                    pool
                })
            })?,
        })
    }
}

pub struct ImapBackend<'a> {
    account_config: Cow<'a, AccountConfig>,
    imap_config: Cow<'a, ImapConfig>,
    sessions_pool_size: usize,
    sessions_pool_cursor: Mutex<usize>,
    sessions_pool: Vec<Mutex<ImapSession>>,
}

impl<'a> ImapBackend<'a> {
    fn create_session(config: Cow<'a, ImapConfig>) -> Result<ImapSession> {
        let builder = TlsConnector::builder()
            .danger_accept_invalid_certs(config.insecure())
            .danger_accept_invalid_hostnames(config.insecure())
            .build()
            .map_err(Error::CreateTlsConnectorError)?;

        let mut client_builder = imap::ClientBuilder::new(&config.host, config.port);
        if config.starttls() {
            client_builder.starttls();
        }

        let client = if config.ssl() {
            client_builder.connect(|domain, tcp| {
                let connector = TlsConnector::connect(&builder, domain, tcp)?;
                Ok(ImapSessionStream::Tls(connector))
            })
        } else {
            client_builder.connect(|_, tcp| Ok(ImapSessionStream::Tcp(tcp)))
        }
        .map_err(Error::ConnectImapServerError)?;

        let mut session = client
            .login(&config.login, &config.passwd()?)
            .map_err(|res| Error::LoginImapServerError(res.0))?;
        session.debug = log_enabled!(Level::Trace);

        Result::Ok(session)
    }

    pub fn new(
        account_config: Cow<'a, AccountConfig>,
        imap_config: Cow<'a, ImapConfig>,
    ) -> Result<Self> {
        ImapBackendBuilder::default().build(account_config, imap_config)
    }

    pub fn session(&self) -> Result<MutexGuard<ImapSession>> {
        let mut cursor = self
            .sessions_pool_cursor
            .lock()
            .map_err(|err| Error::LockSessionsPoolCursorError(err.to_string()))?;
        let session = self
            .sessions_pool
            .get(*cursor)
            .ok_or(Error::FindSessionByCursorError(*cursor))?;
        // TODO: find a way to get the next available connection
        // instead of the next one in the list
        *cursor = (*cursor + 1) % self.sessions_pool_size;
        session
            .lock()
            .map_err(|err| Error::LockSessionError(err.to_string()))
    }

    pub fn close_sessions(&self) -> Result<()> {
        for session in &self.sessions_pool {
            let mut session = session
                .lock()
                .map_err(|err| Error::LockSessionError(err.to_string()))?;
            session.close().map_err(Error::CloseImapSessionError)?;
        }

        Ok(())
    }

    fn search_new_msgs(&self, session: &mut ImapSession, query: &str) -> Result<Vec<u32>> {
        let uids: Vec<u32> = session
            .uid_search(query)
            .map_err(Error::SearchNewMsgsError)?
            .into_iter()
            .collect();
        debug!("found {} new messages", uids.len());
        trace!("uids: {:?}", uids);

        Ok(uids)
    }

    pub fn notify(&self, keepalive: u64, mbox: &str) -> Result<()> {
        let mut session = self.session()?;

        session
            .examine(mbox)
            .map_err(|err| Error::ExamineMboxError(err, mbox.to_owned()))?;

        debug!("init messages hashset");
        let mut msgs_set: HashSet<u32> = self
            .search_new_msgs(&mut session, &self.imap_config.notify_query())?
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        trace!("messages hashset: {:?}", msgs_set);

        loop {
            debug!("begin loop");
            session
                .idle()
                .timeout(Duration::new(keepalive, 0))
                .wait_while(stop_on_any)
                .map_err(Error::StartIdleModeError)?;

            let uids: Vec<u32> = self
                .search_new_msgs(&mut session, &self.imap_config.notify_query())?
                .into_iter()
                .filter(|uid| -> bool { msgs_set.get(uid).is_none() })
                .collect();
            debug!("found {} new messages not in hashset", uids.len());
            trace!("messages hashet: {:?}", msgs_set);

            if !uids.is_empty() {
                let uids = uids
                    .iter()
                    .map(|uid| uid.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                let fetches = session
                    .uid_fetch(uids, "(UID ENVELOPE)")
                    .map_err(Error::FetchNewMsgsEnvelopeError)?;

                for fetch in fetches.iter() {
                    let msg = envelope::imap::from_raw(fetch)?;
                    let uid = fetch.uid.ok_or_else(|| Error::GetUidError(fetch.message))?;

                    let from = msg.sender.to_owned().into();
                    self.imap_config.run_notify_cmd(uid, &msg.subject, &from)?;

                    debug!("notify message: {}", uid);
                    trace!("message: {:?}", msg);

                    debug!("insert message {} in hashset", uid);
                    msgs_set.insert(uid);
                    trace!("messages hashset: {:?}", msgs_set);
                }
            }

            debug!("end loop");
        }
    }

    pub fn watch(&self, keepalive: u64, mbox: &str) -> Result<()> {
        debug!("examine folder: {}", mbox);
        let mut session = self.session()?;

        session
            .examine(mbox)
            .map_err(|err| Error::ExamineMboxError(err, mbox.to_owned()))?;

        loop {
            debug!("begin loop");

            let cmds = self.imap_config.watch_cmds().clone();
            thread::spawn(move || {
                debug!("batch execution of {} cmd(s)", cmds.len());
                cmds.iter().for_each(|cmd| match process::run(cmd, &[]) {
                    // TODO: manage errors
                    Err(_) => (),
                    Ok(_) => (),
                })
            });

            session
                .idle()
                .timeout(Duration::new(keepalive, 0))
                .wait_while(stop_on_any)
                .map_err(Error::StartIdleModeError)?;

            debug!("end loop");
        }
    }
}

impl<'a> Backend for ImapBackend<'a> {
    fn name(&self) -> String {
        self.account_config.name.clone()
    }

    fn add_folder(&self, folder: &str) -> backend::Result<()> {
        let mut session = self.session()?;
        let folder = encode_utf7(folder.to_owned());

        session
            .create(&folder)
            .map_err(|err| Error::CreateMboxError(err, folder.to_owned()))?;

        Ok(())
    }

    fn list_folders(&self) -> backend::Result<Folders> {
        let mut session = self.session()?;
        let folders = session
            .list(Some(""), Some("*"))
            .map_err(Error::ListMboxesError)?;
        let folders = Folders::from_iter(folders.iter().map(|imap_mbox| {
            Folder {
                delim: imap_mbox.delimiter().unwrap_or_default().into(),
                name: decode_utf7(imap_mbox.name().into()),
                desc: imap_mbox
                    .attributes()
                    .iter()
                    .map(|attr| format!("{:?}", attr))
                    .collect::<Vec<_>>()
                    .join(", "),
            }
        }));

        trace!("imap folders: {:?}", folders);
        Ok(folders)
    }

    fn purge_folder(&self, folder: &str) -> backend::Result<()> {
        let folder = encode_utf7(folder.to_owned());
        let flags = Flags::from_iter([Flag::Deleted]);
        let seq = String::from("1:*");

        let mut session = self.session()?;

        session
            .select(&folder)
            .map_err(|err| Error::SelectFolderError(err, folder.clone()))?;
        session
            .store(&seq, format!("+FLAGS ({})", flags.to_imap_query()))
            .map_err(|err| Error::AddFlagsError(err, flags.to_imap_query(), seq))?;
        session
            .expunge()
            .map_err(|err| Error::ExpungeError(err, folder.clone()))?;

        Ok(())
    }

    fn delete_folder(&self, folder: &str) -> backend::Result<()> {
        let mut session = self.session()?;
        let folder = encode_utf7(folder.to_owned());

        session
            .delete(&folder)
            .map_err(|err| Error::DeleteMboxError(err, folder.to_owned()))?;

        Ok(())
    }

    fn get_envelope(&self, folder: &str, id: &str) -> backend::Result<Envelope> {
        debug!("folder: {}", folder);
        debug!("id: {}", id);

        let folder = encode_utf7(folder.to_owned());
        debug!("utf7 encoded folder: {:?}", folder);

        let mut session = self.session()?;

        session
            .select(&folder)
            .map_err(|err| Error::SelectFolderError(err, folder.to_owned()))?;
        let fetches = session
            .fetch(id, "(UID ENVELOPE FLAGS INTERNALDATE)")
            .map_err(|err| Error::FetchMsgsByRangeError(err, id.to_owned()))?;
        let fetch = fetches
            .get(0)
            .ok_or_else(|| Error::GetEnvelopeError(id.to_owned()))?;
        let envelope = envelope::imap::from_raw(&fetch)?;

        Ok(envelope)
    }

    fn get_envelope_internal(&self, folder: &str, internal_id: &str) -> backend::Result<Envelope> {
        debug!("folder: {}", folder);
        debug!("internal id: {}", internal_id);

        let folder = encode_utf7(folder.to_owned());
        debug!("utf7 encoded folder: {:?}", folder);

        let mut session = self.session()?;

        session
            .select(&folder)
            .map_err(|err| Error::SelectFolderError(err, folder.to_owned()))?;
        let fetches = session
            .uid_fetch(internal_id, "(UID ENVELOPE FLAGS INTERNALDATE)")
            .map_err(|err| Error::FetchMsgsByRangeError(err, internal_id.to_owned()))?;
        let fetch = fetches
            .get(0)
            .ok_or_else(|| Error::GetEnvelopeError(internal_id.to_owned()))?;
        let envelope = envelope::imap::from_raw(&fetch)?;

        Ok(envelope)
    }

    fn list_envelopes(
        &self,
        folder: &str,
        page_size: usize,
        page: usize,
    ) -> backend::Result<Envelopes> {
        let mut session = self.session()?;
        let folder = encode_utf7(folder.to_owned());
        let last_seq = session
            .select(&folder)
            .map_err(|err| Error::SelectFolderError(err, folder.to_owned()))?
            .exists as usize;
        debug!("last sequence number: {:?}", last_seq);
        if last_seq == 0 {
            return Ok(Envelopes::default());
        }

        let range = if page_size > 0 {
            let cursor = page * page_size;
            let begin = 1.max(last_seq - cursor.min(last_seq));
            let end = begin - begin.min(page_size) + 1;
            format!("{}:{}", end, begin)
        } else {
            String::from("1:*")
        };
        debug!("range: {:?}", range);

        let fetches = session
            .fetch(&range, "(UID ENVELOPE FLAGS INTERNALDATE)")
            .map_err(|err| Error::FetchMsgsByRangeError(err, range.to_owned()))?;

        let envelopes = envelope::imap::from_raws(fetches)?;
        Ok(envelopes)
    }

    fn search_envelopes(
        &self,
        folder: &str,
        query: &str,
        sort: &str,
        page_size: usize,
        page: usize,
    ) -> backend::Result<Envelopes> {
        let mut session = self.session()?;
        let folder = encode_utf7(folder.to_owned());
        let last_seq = session
            .select(&folder)
            .map_err(|err| Error::SelectFolderError(err, folder.to_owned()))?
            .exists;
        debug!("last sequence number: {:?}", last_seq);
        if last_seq == 0 {
            return Ok(Envelopes::default());
        }

        let begin = page * page_size;
        let end = begin + (page_size - 1);
        let seqs: Vec<String> = if sort.is_empty() {
            session
                .search(query)
                .map_err(|err| Error::SearchMsgsError(err, folder.to_owned(), query.to_owned()))?
                .iter()
                .map(|seq| seq.to_string())
                .collect()
        } else {
            let sort: envelope::imap::SortCriteria = sort.try_into()?;
            session
                .sort(&sort, imap::extensions::sort::SortCharset::Utf8, query)
                .map_err(|err| Error::SortMsgsError(err, folder.to_owned(), query.to_owned()))?
                .iter()
                .map(|seq| seq.to_string())
                .collect()
        };
        if seqs.is_empty() {
            return Ok(Envelopes::default());
        }

        let range = seqs[begin..end.min(seqs.len())].join(",");
        let fetches = session
            .fetch(&range, "(UID ENVELOPE FLAGS INTERNALDATE)")
            .map_err(|err| Error::FetchMsgsByRangeError(err, range.to_owned()))?;

        let envelopes = envelope::imap::from_raws(fetches)?;
        Ok(envelopes)
    }

    fn add_email(&self, folder: &str, email: &[u8], flags: &Flags) -> backend::Result<String> {
        debug!("folder: {}", folder);
        debug!("flags: {:?}", flags);

        let folder = encode_utf7(folder.to_owned());
        debug!("utf7 encoded folder: {:?}", folder);

        let mut flags = flags.clone();
        flags.insert(Flag::Seen);

        let mut session = self.session()?;

        session
            .append(&folder, email)
            .flags(flags.into_imap_flags_vec())
            .finish()
            .map_err(|err| Error::AppendMsgError(err, folder.to_owned()))?;
        let last_seq = session
            .select(&folder)
            .map_err(|err| Error::SelectFolderError(err, folder.to_owned()))?
            .exists;

        Ok(last_seq.to_string())
    }

    fn add_email_internal(
        &self,
        folder: &str,
        email: &[u8],
        flags: &Flags,
    ) -> backend::Result<String> {
        debug!("folder: {}", folder);
        debug!("flags: {:?}", flags);

        let folder = encode_utf7(folder.to_owned());
        debug!("utf7 encoded folder: {:?}", folder);

        let mut flags = flags.clone();
        flags.insert(Flag::Seen);

        let mut session = self.session()?;

        session
            .append(&folder, email)
            .flags(flags.into_imap_flags_vec())
            .finish()
            .map_err(|err| Error::AppendMsgError(err, folder.to_owned()))?;
        let last_uid = session
            .select(&folder)
            .map_err(|err| Error::SelectFolderError(err, folder.to_owned()))?
            .uid_next
            .ok_or(Error::GetLastUidError)?;

        Ok(last_uid.to_string())
    }

    fn get_emails(&self, folder: &str, ids: Vec<&str>) -> backend::Result<Emails> {
        debug!("folder: {}", folder);
        debug!("ids: {:?}", ids);

        let folder = encode_utf7(folder.to_owned());
        debug!("utf7 encoded folder: {:?}", folder);

        let seq = ids.join(",");

        let mut session = self.session()?;

        session
            .select(&folder)
            .map_err(|err| Error::SelectFolderError(err, folder.clone()))?;
        let fetches = session
            .fetch(&seq, "BODY[]")
            .map_err(|err| Error::GetEmailsBySeqError(err, seq))?;

        Ok(Emails::try_from(fetches)?)
    }

    fn get_emails_internal(
        &self,
        folder: &str,
        internal_ids: Vec<&str>,
    ) -> backend::Result<Emails> {
        debug!("folder: {}", folder);
        debug!("internal ids: {:?}", internal_ids);

        let folder = encode_utf7(folder.to_owned());
        debug!("utf7 encoded folder: {:?}", folder);

        let seq = internal_ids.join(",");
        let mut session = self.session()?;

        session
            .select(&folder)
            .map_err(|err| Error::SelectFolderError(err, folder.clone()))?;
        let fetches = session
            .uid_fetch(&seq, "BODY[]")
            .map_err(|err| Error::GetEmailsBySeqError(err, seq))?;
        let emails = Emails::try_from(fetches)?;

        Ok(emails)
    }

    fn copy_emails(
        &self,
        from_folder: &str,
        to_folder: &str,
        ids: Vec<&str>,
    ) -> backend::Result<()> {
        debug!("ids: {:?}", ids);
        debug!("from folder: {}", from_folder);
        debug!("to folder: {}", to_folder);

        let seq = ids.join(",");
        let from_folder_encoded = encode_utf7(from_folder.to_owned());
        let to_folder_encoded = encode_utf7(to_folder.to_owned());
        debug!("from folder (utf7 encoded): {}", from_folder_encoded);
        debug!("to folder (utf7 encoded): {}", to_folder_encoded);

        let mut session = self.session()?;

        session
            .select(from_folder_encoded)
            .map_err(|err| Error::SelectFolderError(err, from_folder.to_owned()))?;
        session.copy(&seq, to_folder_encoded).map_err(|err| {
            Error::CopyEmailError(err, seq, from_folder.to_owned(), to_folder.to_owned())
        })?;

        Ok(())
    }

    fn copy_emails_internal(
        &self,
        from_folder: &str,
        to_folder: &str,
        internal_ids: Vec<&str>,
    ) -> backend::Result<()> {
        debug!("internal ids: {:?}", internal_ids);
        debug!("from folder: {}", from_folder);
        debug!("to folder: {}", to_folder);

        let uids = internal_ids.join(",");
        let from_folder_encoded = encode_utf7(from_folder.to_owned());
        let to_folder_encoded = encode_utf7(to_folder.to_owned());
        debug!("from folder (utf7 encoded): {}", from_folder_encoded);
        debug!("to folder (utf7 encoded): {}", to_folder_encoded);

        let mut session = self.session()?;

        session
            .select(from_folder_encoded)
            .map_err(|err| Error::SelectFolderError(err, from_folder.to_owned()))?;
        session.uid_copy(&uids, to_folder_encoded).map_err(|err| {
            Error::CopyEmailError(err, uids, from_folder.to_owned(), to_folder.to_owned())
        })?;

        Ok(())
    }

    fn move_emails(
        &self,
        from_folder: &str,
        to_folder: &str,
        ids: Vec<&str>,
    ) -> backend::Result<()> {
        debug!("from folder: {}", from_folder);
        debug!("to folder: {}", to_folder);
        debug!("ids: {:?}", ids);

        let seq = ids.join(",");
        let from_folder_encoded = encode_utf7(from_folder.to_owned());
        let to_folder_encoded = encode_utf7(to_folder.to_owned());
        debug!("from folder (utf7 encoded): {}", from_folder_encoded);
        debug!("to folder (utf7 encoded): {}", to_folder_encoded);

        let mut session = self.session()?;

        session
            .select(from_folder_encoded)
            .map_err(|err| Error::SelectFolderError(err, from_folder.to_owned()))?;
        session.mv(&seq, to_folder_encoded).map_err(|err| {
            Error::MoveEmailError(err, seq, from_folder.to_owned(), to_folder.to_owned())
        })?;

        Ok(())
    }

    fn move_emails_internal(
        &self,
        from_folder: &str,
        to_folder: &str,
        internal_ids: Vec<&str>,
    ) -> backend::Result<()> {
        debug!("from folder: {}", from_folder);
        debug!("to folder: {}", to_folder);
        debug!("internal ids: {:?}", internal_ids);

        let uids = internal_ids.join(",");
        let from_folder_encoded = encode_utf7(from_folder.to_owned());
        let to_folder_encoded = encode_utf7(to_folder.to_owned());
        debug!("from folder (utf7 encoded): {}", from_folder_encoded);
        debug!("to folder (utf7 encoded): {}", to_folder_encoded);

        let mut session = self.session()?;

        session
            .select(from_folder_encoded)
            .map_err(|err| Error::SelectFolderError(err, from_folder.to_owned()))?;
        session.uid_mv(&uids, to_folder_encoded).map_err(|err| {
            Error::MoveEmailError(err, uids, from_folder.to_owned(), to_folder.to_owned())
        })?;

        Ok(())
    }

    fn delete_emails(&self, folder: &str, ids: Vec<&str>) -> backend::Result<()> {
        self.add_flags(folder, ids, &Flags::from_iter([Flag::Deleted]))
    }

    fn delete_emails_internal(&self, folder: &str, internal_ids: Vec<&str>) -> backend::Result<()> {
        self.add_flags_internal(folder, internal_ids, &Flags::from_iter([Flag::Deleted]))
    }

    fn add_flags(&self, folder: &str, ids: Vec<&str>, flags: &Flags) -> backend::Result<()> {
        debug!("folder: {}", folder);
        debug!("ids: {:?}", ids);
        debug!("flags: {:?}", flags);

        let folder = encode_utf7(folder.to_owned());
        debug!("folder (utf7 encoded): {}", folder);

        let seq = ids.join(",");

        let mut session = self.session()?;

        session
            .select(&folder)
            .map_err(|err| Error::SelectFolderError(err, folder.clone()))?;
        session
            .store(&seq, format!("+FLAGS ({})", flags.to_imap_query()))
            .map_err(|err| Error::AddFlagsError(err, flags.to_imap_query(), seq))?;
        session
            .expunge()
            .map_err(|err| Error::ExpungeError(err, folder.clone()))?;

        Ok(())
    }

    fn add_flags_internal(
        &self,
        folder: &str,
        internal_ids: Vec<&str>,
        flags: &Flags,
    ) -> backend::Result<()> {
        debug!("folder: {}", folder);
        debug!("internal ids: {:?}", internal_ids);
        debug!("flags: {:?}", flags);

        let uids = internal_ids.join(",");
        let folder = encode_utf7(folder.to_owned());
        debug!("folder (utf7 encoded): {}", folder);

        let mut session = self.session()?;

        session
            .select(&folder)
            .map_err(|err| Error::SelectFolderError(err, folder.clone()))?;
        session
            .uid_store(&uids, format!("+FLAGS ({})", flags.to_imap_query()))
            .map_err(|err| Error::AddFlagsError(err, flags.to_imap_query(), uids))?;
        session
            .expunge()
            .map_err(|err| Error::ExpungeError(err, folder.clone()))?;

        Ok(())
    }

    fn set_flags(&self, folder: &str, ids: Vec<&str>, flags: &Flags) -> backend::Result<()> {
        debug!("folder: {}", folder);
        debug!("ids: {:?}", ids);
        debug!("flags: {:?}", flags);

        let seq = ids.join(",");
        let folder = encode_utf7(folder.to_owned());
        debug!("folder (utf7 encoded): {}", folder);

        let mut session = self.session()?;

        session
            .select(&folder)
            .map_err(|err| Error::SelectFolderError(err, folder.clone()))?;
        session
            .store(&seq, format!("FLAGS ({})", flags.to_imap_query()))
            .map_err(|err| Error::SetFlagsError(err, flags.to_imap_query(), seq))?;

        Ok(())
    }

    fn set_flags_internal(
        &self,
        folder: &str,
        internal_ids: Vec<&str>,
        flags: &Flags,
    ) -> backend::Result<()> {
        debug!("folder: {}", folder);
        debug!("internal ids: {:?}", internal_ids);
        debug!("flags: {:?}", flags);

        let uids = internal_ids.join(",");
        let folder = encode_utf7(folder.to_owned());
        debug!("folder (utf7 encoded): {}", folder);

        let mut session = self.session()?;

        session
            .select(&folder)
            .map_err(|err| Error::SelectFolderError(err, folder.clone()))?;
        session
            .uid_store(&uids, format!("FLAGS ({})", flags.to_imap_query()))
            .map_err(|err| Error::SetFlagsError(err, flags.to_imap_query(), uids))?;

        Ok(())
    }

    fn remove_flags(&self, folder: &str, ids: Vec<&str>, flags: &Flags) -> backend::Result<()> {
        debug!("folder: {}", folder);
        debug!("ids: {:?}", ids);
        debug!("flags: {:?}", flags);

        let seq = ids.join(",");
        let folder = encode_utf7(folder.to_owned());
        debug!("folder (utf7 encoded): {}", folder);

        let mut session = self.session()?;

        session
            .select(&folder)
            .map_err(|err| Error::SelectFolderError(err, folder.clone()))?;
        session
            .store(&seq, format!("-FLAGS ({})", flags.to_imap_query()))
            .map_err(|err| Error::DelFlagsError(err, flags.to_imap_query(), seq))?;

        Ok(())
    }

    fn remove_flags_internal(
        &self,
        folder: &str,
        internal_ids: Vec<&str>,
        flags: &Flags,
    ) -> backend::Result<()> {
        debug!("folder: {}", folder);
        debug!("internal ids: {:?}", internal_ids);
        debug!("flags: {:?}", flags);

        let uids = internal_ids.join(",");
        let folder = encode_utf7(folder.to_owned());
        debug!("folder (utf7 encoded): {}", folder);

        let mut session = self.session()?;

        session
            .select(&folder)
            .map_err(|err| Error::SelectFolderError(err, folder.clone()))?;
        session
            .uid_store(&uids, format!("-FLAGS ({})", flags.to_imap_query()))
            .map_err(|err| Error::DelFlagsError(err, flags.to_imap_query(), uids))?;

        Ok(())
    }

    fn sync(&self, dry_run: bool) -> backend::Result<()> {
        ThreadSafeBackend::sync(self, &self.account_config, dry_run)
            .map_err(|err| backend::Error::SyncError(Box::new(err), self.name()))
    }

    fn as_any(&self) -> &(dyn Any + 'a) {
        self
    }
}

impl ThreadSafeBackend for ImapBackend<'_> {}
