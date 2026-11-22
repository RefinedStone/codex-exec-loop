//! Issue automation keeps its original filename and FULL rollback-journal policy.

use std::path::Path;

use anyhow::Result;

use crate::private_sqlite::{DatabaseFiles, Synchronous};
pub(super) use crate::private_sqlite::{PrivateConnection, PrivateStoreGuard};

pub(super) const DATABASE_NAME: &str = "issue-automation.db";
const ROLLBACK_JOURNAL_NAME: &str = "issue-automation.db-journal";
const FILES: DatabaseFiles = DatabaseFiles::new(
    DATABASE_NAME,
    ROLLBACK_JOURNAL_NAME,
    "issue-automation.db-wal",
    "issue-automation.db-shm",
);

pub(super) fn open(runtime: &Path) -> Result<PrivateConnection> {
    crate::private_sqlite::open(runtime, FILES, Synchronous::Full)
}

#[cfg(test)]
mod tests;
