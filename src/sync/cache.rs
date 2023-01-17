use chrono::{DateTime, Local};
use log::warn;
use sqlite::{Connection, ConnectionWithFullMutex, Result};
use std::{borrow::Cow, path::Path};

pub(crate) use sqlite::Error;

use crate::{AccountConfig, Envelope, Envelopes, Flag, Flags};

const CREATE_ENVELOPES_TABLE: &str = "
CREATE TABLE IF NOT EXISTS envelopes (
    id          TEXT NOT NULL,
    internal_id TEXT NOT NULL,
    hash        TEXT NOT NULL,
    account     TEXT NOT NULL,
    folder      TEXT NOT NULL,
    flag        TEXT NOT NULL,
    message_id  TEXT NOT NULL,
    sender      TEXT NOT NULL,
    subject     TEXT NOT NULL,
    date        DATETIME
)";

const INSERT_ENVELOPE: &str = "INSERT INTO envelopes VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";

const DELETE_ENVELOPE: &str = "DELETE FROM envelopes WHERE account = ? AND folder = ? AND id = ?";

const SELECT_ENVELOPES: &str = "
    SELECT id, internal_id, hash, account, folder, GROUP_CONCAT(flag) AS flags, message_id, sender, subject, date
    FROM envelopes
    WHERE account = ?
    AND folder = ?
    GROUP BY hash
";

pub(crate) struct Cache<'a> {
    account_config: Cow<'a, AccountConfig>,
    conn: ConnectionWithFullMutex,
}

impl<'a> Cache<'a> {
    const LOCAL_SUFFIX: &str = ":cache";

    pub fn new<P>(account_config: Cow<'a, AccountConfig>, sync_dir: P) -> Result<Self>
    where
        P: AsRef<Path>,
    {
        let conn = Connection::open_with_full_mutex(sync_dir.as_ref().join("database.sqlite"))?;
        conn.execute(CREATE_ENVELOPES_TABLE)?;

        Ok(Self {
            account_config,
            conn,
        })
    }

    fn list_envelopes<A, F>(&self, account: A, folder: F) -> Result<Envelopes>
    where
        A: AsRef<str>,
        F: AsRef<str>,
    {
        Ok(Envelopes::from_iter(
            self.conn
                .prepare(SELECT_ENVELOPES)?
                .into_iter()
                .bind((1, account.as_ref()))?
                .bind((2, folder.as_ref()))?
                .collect::<sqlite::Result<Vec<_>>>()?
                .iter()
                .map(|row| Envelope {
                    id: row.read::<&str, _>("id").into(),
                    internal_id: row.read::<&str, _>("internal_id").into(),
                    flags: Flags::from_iter(
                        row.read::<&str, _>("flags").split(",").map(Flag::from),
                    ),
                    message_id: row.read::<&str, _>("message_id").into(),
                    sender: row.read::<&str, _>("sender").into(),
                    subject: row.read::<&str, _>("subject").into(),
                    date: {
                        let date_str = row.read::<&str, _>("date");
                        match DateTime::parse_from_rfc3339(date_str) {
                            Ok(date) => Some(date.with_timezone(&Local)),
                            Err(err) => {
                                warn!("invalid date {}, skipping it: {}", date_str, err);
                                None
                            }
                        }
                    },
                }),
        ))
    }

    pub fn list_local_envelopes<F>(&self, folder: F) -> Result<Envelopes>
    where
        F: AsRef<str>,
    {
        self.list_envelopes(
            self.account_config.name.clone() + Self::LOCAL_SUFFIX,
            folder,
        )
    }

    pub fn list_remote_envelopes<F: AsRef<str>>(&self, folder: F) -> Result<Envelopes> {
        self.list_envelopes(&self.account_config.name, folder)
    }

    fn insert_envelope<A, F>(&self, account: A, folder: F, envelope: Envelope) -> Result<()>
    where
        A: AsRef<str>,
        F: AsRef<str>,
    {
        let mut statement = self.conn.prepare(INSERT_ENVELOPE)?;

        for flag in envelope.flags.iter() {
            statement.reset()?;
            statement.bind((1, envelope.id.as_str()))?;
            statement.bind((2, envelope.internal_id.as_str()))?;
            statement.bind((3, envelope.hash(&folder).as_str()))?;
            statement.bind((4, account.as_ref()))?;
            statement.bind((5, folder.as_ref()))?;
            statement.bind((6, flag.to_string().as_str()))?;
            statement.bind((7, envelope.message_id.as_str()))?;
            statement.bind((8, envelope.sender.as_str()))?;
            statement.bind((9, envelope.subject.as_str()))?;
            statement.bind((
                10,
                match envelope.date {
                    Some(date) => date.to_rfc3339().into(),
                    None => sqlite::Value::Null,
                },
            ))?;
            statement.next()?;
        }

        Ok(())
    }

    pub fn insert_local_envelope<F>(&self, folder: F, envelope: Envelope) -> Result<()>
    where
        F: AsRef<str>,
    {
        self.insert_envelope(
            self.account_config.name.clone() + Self::LOCAL_SUFFIX,
            folder,
            envelope,
        )
    }

    pub fn insert_remote_envelope<F>(&self, folder: F, envelope: Envelope) -> Result<()>
    where
        F: AsRef<str>,
    {
        self.insert_envelope(&self.account_config.name, folder, envelope)
    }

    fn delete_envelope<A, F, I>(&self, account: A, folder: F, internal_id: I) -> Result<()>
    where
        A: AsRef<str>,
        F: AsRef<str>,
        I: AsRef<str>,
    {
        let mut statement = self.conn.prepare(DELETE_ENVELOPE)?;
        statement.bind((1, account.as_ref()))?;
        statement.bind((2, folder.as_ref()))?;
        statement.bind((3, internal_id.as_ref()))?;
        statement.next()?;
        Ok(())
    }

    pub fn delete_local_envelope<F, I>(&self, folder: F, internal_id: I) -> Result<()>
    where
        F: AsRef<str>,
        I: AsRef<str>,
    {
        self.delete_envelope(
            self.account_config.name.clone() + Self::LOCAL_SUFFIX,
            folder,
            internal_id,
        )
    }

    pub fn delete_remote_envelope<F, I>(&self, folder: F, internal_id: I) -> Result<()>
    where
        F: AsRef<str>,
        I: AsRef<str>,
    {
        self.delete_envelope(&self.account_config.name, folder, internal_id)
    }
}
