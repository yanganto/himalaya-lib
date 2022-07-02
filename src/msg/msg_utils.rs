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

use log::{debug, trace};
use std::{env, fs, path};

use crate::msg::{Error, Result};

pub fn local_draft_path() -> path::PathBuf {
    trace!(">> get local draft path");

    let path = env::temp_dir().join("himalaya-draft.eml");
    debug!("local draft path: {:?}", path);

    trace!("<< get local draft path");
    path
}

pub fn remove_local_draft() -> Result<()> {
    trace!(">> remove local draft");

    let path = local_draft_path();
    fs::remove_file(&path).map_err(|err| Error::DeleteLocalDraftError(err, path))?;

    trace!("<< remove local draft");
    Ok(())
}
