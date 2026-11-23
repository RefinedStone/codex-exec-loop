//! Private, process-locked transactional state for a single extension scope.
use crate::application::port::outbound::extension_state_store_port::ExtensionStateStorePort;
use crate::domain::kernel_extension::ExtensionDurableState;
use crate::private_sqlite::{DatabaseFiles, PrivateConnection, Synchronous};
use anyhow::{Context, Result, ensure};
use rusqlite::{OptionalExtension, params};
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::sync::Mutex;
const FILES: DatabaseFiles = DatabaseFiles::new(
    "state.sqlite3",
    "state.sqlite3-journal",
    "state.sqlite3-wal",
    "state.sqlite3-shm",
);
pub(crate) struct SqliteExtensionStateStore {
    connection: Mutex<PrivateConnection>,
    _lock: File,
}
impl SqliteExtensionStateStore {
    pub fn open(root: &Path) -> Result<Self> {
        crate::private_directory::ensure_private_directory(root)?;
        let path = root.join("host.lock");
        if let Ok(metadata) = std::fs::symlink_metadata(&path) {
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "extension lock must be a private regular file"
            );
        }
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let lock = options.open(path)?;
        lock.try_lock()
            .context("another extension host owns this scope")?;
        let mut connection = retry_transient(|| {
            crate::private_sqlite::open_or_create(root, FILES, Synchronous::Full)
        })?;
        let (sql, guard) = connection.parts();
        guard.verify_identity()?;
        sql.execute_batch("CREATE TABLE IF NOT EXISTS extension_state(id INTEGER PRIMARY KEY CHECK(id=1),document TEXT NOT NULL);")?;
        guard.verify_identity()?;
        // Creation pins ancestors against rename. Steady connections must release those pins so
        // existing repository services can reopen their private directories on Windows.
        drop(connection);
        let connection =
            retry_transient(|| crate::private_sqlite::open(root, FILES, Synchronous::Full))?;
        Ok(Self {
            connection: Mutex::new(connection),
            _lock: lock,
        })
    }
}
impl ExtensionStateStorePort for SqliteExtensionStateStore {
    fn load(&self) -> Result<ExtensionDurableState> {
        let mut connection = self.connection.lock().unwrap_or_else(|p| p.into_inner());
        let (sql, guard) = connection.parts();
        guard.verify_identity()?;
        let saved = sql
            .query_row(
                "SELECT document FROM extension_state WHERE id=1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        guard.verify_identity()?;
        match saved {
            Some(saved) => {
                ensure!(
                    saved.len() <= 32 * 1024 * 1024,
                    "extension state is too large"
                );
                Ok(serde_json::from_str(&saved)?)
            }
            None => Ok(ExtensionDurableState::default()),
        }
    }
    fn save(&self, state: &ExtensionDurableState) -> Result<()> {
        let saved = serde_json::to_string(state)?;
        ensure!(
            saved.len() <= 32 * 1024 * 1024,
            "extension state is too large"
        );
        let mut connection = self.connection.lock().unwrap_or_else(|p| p.into_inner());
        let (sql, guard) = connection.parts();
        guard.verify_identity()?;
        let transaction = sql.transaction()?;
        transaction.execute("INSERT INTO extension_state(id,document) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET document=excluded.document",params![saved])?;
        guard.verify_identity()?;
        transaction.commit()?;
        guard.verify_identity()?;
        Ok(())
    }
}
fn retry_transient<T>(mut operation: impl FnMut() -> Result<T>) -> Result<T> {
    for attempt in 0..40 {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) if attempt < 39 && transient(&error) => {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("bounded retries return")
}
fn transient(error: &anyhow::Error) -> bool {
    #[cfg(windows)]
    {
        error
            .chain()
            .filter_map(|cause| cause.downcast_ref::<std::io::Error>())
            .any(|e| matches!(e.raw_os_error(), Some(5 | 32 | 303)))
    }
    #[cfg(not(windows))]
    {
        let _ = error;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_store_is_single_owner_and_recovers_the_saved_identity() {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-extension-store-{:032x}",
            rand::random::<u128>()
        ));
        let store = SqliteExtensionStateStore::open(&root).unwrap();
        assert!(SqliteExtensionStateStore::open(&root).is_err());
        let state = ExtensionDurableState {
            host_id: "stable-host".into(),
            ..Default::default()
        };
        store.save(&state).unwrap();
        drop(store);
        let restored = SqliteExtensionStateStore::open(&root).unwrap();
        assert_eq!(restored.load().unwrap().host_id, "stable-host");
        drop(restored);
        std::fs::remove_dir_all(root).unwrap();
    }
}
