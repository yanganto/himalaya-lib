use chrono::{DateTime, Local};
use log::warn;
use rusqlite::Connection;
use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

use crate::{envelope::Mailbox, AccountConfig, Envelope, Envelopes};

use super::Result;

const CREATE_ENVELOPES_TABLE: &str = "
    CREATE TABLE IF NOT EXISTS envelopes (
        id          TEXT     NOT NULL,
        internal_id TEXT     NOT NULL,
        hash        TEXT     NOT NULL,
        account     TEXT     NOT NULL,
        folder      TEXT     NOT NULL,
        flag        TEXT     NOT NULL,
        message_id  TEXT     NOT NULL,
        sender      TEXT     NOT NULL,
        subject     TEXT     NOT NULL,
        date        DATETIME NOT NULL,
        UNIQUE(internal_id, hash, account, folder, flag)
    )
";

const INSERT_ENVELOPE: &str = "
    INSERT INTO envelopes
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
";

const DELETE_ENVELOPE: &str = "
    DELETE FROM envelopes
    WHERE account = ?
    AND folder = ?
    AND internal_id = ?
";

const SELECT_ENVELOPES: &str = "
    SELECT id, internal_id, hash, account, folder, GROUP_CONCAT(flag, ' ') AS flags, message_id, sender, subject, date
    FROM envelopes
    WHERE account = ?
    AND folder = ?
    GROUP BY hash
    ORDER BY date DESC
";

pub struct Cache<'a> {
    account_config: Cow<'a, AccountConfig>,
    db_path: PathBuf,
}

impl<'a> Cache<'a> {
    const LOCAL_SUFFIX: &str = ":cache";

    fn db(&self) -> Result<rusqlite::Connection> {
        let db = Connection::open(&self.db_path)?;
        db.execute(CREATE_ENVELOPES_TABLE, [])?;
        Ok(db)
    }

    pub fn new<P>(account_config: Cow<'a, AccountConfig>, sync_dir: P) -> Self
    where
        P: AsRef<Path>,
    {
        Self {
            account_config,
            db_path: sync_dir.as_ref().join(".database.sqlite"),
        }
    }

    fn list_envelopes<A, F>(&self, account: A, folder: F) -> Result<Envelopes>
    where
        A: AsRef<str>,
        F: AsRef<str>,
    {
        let db = self.db()?;
        let mut stmt = db.prepare(SELECT_ENVELOPES)?;
        let envelopes: Vec<Envelope> = stmt
            .query_map([account.as_ref(), folder.as_ref()], |row| {
                Ok(Envelope {
                    id: row.get(0)?,
                    internal_id: row.get(1)?,
                    flags: row.get::<usize, String>(5)?.as_str().into(),
                    message_id: row.get(6)?,
                    from: Mailbox::new_nameless(row.get::<usize, String>(7)?),
                    subject: row.get(8)?,
                    date: {
                        let date: String = row.get(9)?;
                        match DateTime::parse_from_rfc3339(&date) {
                            Ok(date) => date.with_timezone(&Local),
                            Err(err) => {
                                warn!("invalid date {}, skipping it: {}", date, err);
                                DateTime::default()
                            }
                        }
                    },
                })
            })?
            .collect::<rusqlite::Result<_>>()?;

        Ok(Envelopes::from_iter(envelopes))
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
        for flag in envelope.flags.iter() {
            self.db()?.execute(
                INSERT_ENVELOPE,
                [
                    envelope.id.as_str(),
                    envelope.internal_id.as_str(),
                    envelope.hash(folder.as_ref()).as_str(),
                    account.as_ref(),
                    folder.as_ref(),
                    flag.to_string().as_str(),
                    envelope.message_id.as_str(),
                    envelope.from.addr.as_str(),
                    envelope.subject.as_str(),
                    envelope.date.to_rfc3339().as_str(),
                ],
            )?;
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
        self.db()?.execute(
            DELETE_ENVELOPE,
            [account.as_ref(), folder.as_ref(), internal_id.as_ref()],
        )?;
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
