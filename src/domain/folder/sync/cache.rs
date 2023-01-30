use rusqlite::Connection;
pub use rusqlite::Error;
use std::path::PathBuf;

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
    SELECT name
    FROM folders
    WHERE account = ?
";

pub struct Cache<'a> {
    account_config: &'a AccountConfig,
    db_path: PathBuf,
}

impl<'a> Cache<'a> {
    const LOCAL_SUFFIX: &str = ":cache";

    fn db(&self) -> Result<rusqlite::Connection> {
        let db = Connection::open(&self.db_path)?;
        db.execute(CREATE_FOLDERS_TABLE, [])?;
        Ok(db)
    }

    pub fn new(account_config: &'a AccountConfig) -> Result<Self> {
        Ok(Self {
            account_config,
            db_path: account_config.sync_dir()?.join(".database.sqlite"),
        })
    }

    fn list_folders<A>(&self, account: A) -> Result<FoldersName>
    where
        A: AsRef<str>,
    {
        let db = self.db()?;
        let mut stmt = db.prepare(SELECT_FOLDERS)?;
        let folders: Vec<String> = stmt
            .query_map([account.as_ref()], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;

        Ok(FoldersName::from_iter(folders))
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
        self.db()?
            .execute(INSERT_FOLDER, [account.as_ref(), folder.as_ref()])?;
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
        self.db()?
            .execute(DELETE_FOLDER, [account.as_ref(), folder.as_ref()])?;
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
