pub use sqlite::Error;
use sqlite::{Connection, ConnectionWithFullMutex};
use std::{borrow::Cow, path::Path};

use crate::AccountConfig;

use super::{FoldersName, Result};

const CREATE_FOLDERS_TABLE: &str = "
    CREATE TABLE IF NOT EXISTS folders (
        account TEXT NOT NULL,
        name    TEXT NOT NULL,
        UNIQUE(name, account)
    )
";

const INSERT_FOLDER: &str = "
    INSERT INTO folders
    VALUES (?, ?)
";

const DELETE_FOLDER: &str = "
    DELETE FROM folders
    WHERE account = ?
    AND name = ?
";

const SELECT_FOLDERS: &str = "
    SELECT name, account
    FROM folders
    WHERE account = ?
";

pub struct Cache<'a> {
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
        conn.execute(CREATE_FOLDERS_TABLE)?;

        Ok(Self {
            account_config,
            conn,
        })
    }

    fn list_folders<A>(&self, account: A) -> Result<FoldersName>
    where
        A: AsRef<str>,
    {
        Ok(FoldersName::from_iter(
            self.conn
                .prepare(SELECT_FOLDERS)?
                .into_iter()
                .bind((1, account.as_ref()))?
                .collect::<sqlite::Result<Vec<_>>>()?
                .iter()
                .map(|row| row.read::<&str, _>("name").into()),
        ))
    }

    pub fn list_local_folders(&self) -> Result<FoldersName> {
        self.list_folders(self.account_config.name.clone() + Self::LOCAL_SUFFIX)
    }

    pub fn list_remote_folders(&self) -> Result<FoldersName> {
        self.list_folders(&self.account_config.name)
    }

    fn insert_folder<A, F>(&self, account: A, folder: F) -> Result<()>
    where
        A: AsRef<str>,
        F: AsRef<str>,
    {
        let mut statement = self.conn.prepare(INSERT_FOLDER)?;
        statement.bind((1, account.as_ref()))?;
        statement.bind((2, folder.as_ref()))?;
        statement.next()?;
        Ok(())
    }

    pub fn insert_local_folder<F>(&self, folder: F) -> Result<()>
    where
        F: AsRef<str>,
    {
        self.insert_folder(
            self.account_config.name.clone() + Self::LOCAL_SUFFIX,
            folder,
        )
    }

    pub fn insert_remote_folder<F>(&self, folder: F) -> Result<()>
    where
        F: AsRef<str>,
    {
        self.insert_folder(&self.account_config.name, folder)
    }

    fn delete_folder<A, F>(&self, account: A, folder: F) -> Result<()>
    where
        A: AsRef<str>,
        F: AsRef<str>,
    {
        let mut statement = self.conn.prepare(DELETE_FOLDER)?;
        statement.bind((1, account.as_ref()))?;
        statement.bind((2, folder.as_ref()))?;
        statement.next()?;
        Ok(())
    }

    pub fn delete_local_folder<F>(&self, folder: F) -> Result<()>
    where
        F: AsRef<str>,
    {
        self.delete_folder(
            self.account_config.name.clone() + Self::LOCAL_SUFFIX,
            folder,
        )
    }

    pub fn delete_remote_folder<F>(&self, folder: F) -> Result<()>
    where
        F: AsRef<str>,
    {
        self.delete_folder(&self.account_config.name, folder)
    }
}
