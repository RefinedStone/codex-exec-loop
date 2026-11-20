//! Private file boundary for the independent issue automation database.
//! No schema is created here; callers keep the guard alive through their transaction.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags};

pub(super) const DATABASE_NAME: &str = "issue-automation.db";
const ROLLBACK_JOURNAL_NAME: &str = "issue-automation.db-journal";
const TRANSIENT_ATTEMPTS: usize = 8;
const TRANSIENT_DELAY: Duration = Duration::from_millis(5);

/// Field order matters: close SQLite before releasing its independently opened file anchor.
pub(super) struct PrivateConnection {
    connection: Connection,
    guard: PrivateStoreGuard,
}

impl PrivateConnection {
    pub(super) fn parts(&mut self) -> (&mut Connection, &PrivateStoreGuard) {
        (&mut self.connection, &self.guard)
    }
}

struct DatabaseAnchor {
    file: File,
    // An unexpected rename may reveal a live inode only after open. Defer that descriptor's
    // close too: closing any descriptor of the inode can release POSIX SQLite locks.
    #[cfg(unix)]
    deferred_files: std::sync::Mutex<Vec<File>>,
}

impl DatabaseAnchor {
    fn new(file: File) -> Self {
        Self {
            file,
            #[cfg(unix)]
            deferred_files: std::sync::Mutex::new(Vec::new()),
        }
    }
}

pub(super) struct PrivateStoreGuard {
    runtime: PathBuf,
    directories: Vec<(PathBuf, File)>,
    database: Option<Arc<DatabaseAnchor>>,
}

impl Drop for PrivateStoreGuard {
    fn drop(&mut self) {
        platform::release_database(self.database.take());
    }
}

impl PrivateStoreGuard {
    fn database_file(&self) -> &File {
        &self
            .database
            .as_ref()
            .expect("database anchor is live until guard drop")
            .file
    }

    pub(super) fn verify_identity(&self) -> Result<()> {
        self.verify_directories()?;
        platform::verify_file(&self.runtime.join(DATABASE_NAME), self.database_file())?;
        reject_wal_sidecars(&self.runtime)?;
        let _journal = self.rollback_journal()?;
        self.verify_directories()
    }

    fn verify_directories(&self) -> Result<()> {
        for (index, (path, directory)) in self.directories.iter().enumerate() {
            platform::verify_directory(path, directory, index + 1 == self.directories.len())?;
        }
        Ok(())
    }

    fn rollback_journal(&self) -> Result<Option<File>> {
        let path = self.runtime.join(ROLLBACK_JOURNAL_NAME);
        let directory = &self
            .directories
            .last()
            .context("missing private runtime anchor")?
            .1;
        for attempt in 0..TRANSIENT_ATTEMPTS {
            match platform::open_journal(&path, directory) {
                Ok(file) => return Ok(file),
                Err(error) => {
                    // SQLite can unlink its rollback journal between inspection and open.
                    // Absence is acceptable only after a fresh, successful directory-entry check.
                    match fs::symlink_metadata(&path) {
                        Err(missing) if missing.kind() == std::io::ErrorKind::NotFound => {
                            return Ok(None);
                        }
                        _ if attempt + 1 < TRANSIENT_ATTEMPTS => {
                            std::thread::sleep(TRANSIENT_DELAY);
                        }
                        _ => return Err(error),
                    }
                }
            }
        }
        unreachable!("bounded journal validation returns")
    }
}

pub(super) fn open(runtime: &Path) -> Result<PrivateConnection> {
    if !runtime.is_absolute() {
        bail!("issue automation runtime must be an absolute private directory")
    }
    let directories = platform::open_runtime(runtime)?;
    reject_wal_sidecars(runtime)?;
    let directory = &directories
        .last()
        .context("missing private runtime anchor")?
        .1;
    let database = platform::open_database(&runtime.join(DATABASE_NAME), directory)?;
    let guard = PrivateStoreGuard {
        runtime: runtime.to_path_buf(),
        directories,
        database: Some(database),
    };
    guard.verify_identity()?;
    // Keep an existing hot journal pinned while SQLite performs normal rollback recovery.
    let hot_journal = guard.rollback_journal()?;
    validate_database_header(guard.database_file(), hot_journal.as_ref())?;

    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_CREATE
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_NOFOLLOW;
    let connection = Connection::open_with_flags(runtime.join(DATABASE_NAME), flags)
        .context("failed to open private issue automation database")?;
    connection.busy_timeout(Duration::from_secs(5))?;
    guard.verify_identity()?;
    let mode = connection.query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))?;
    if !mode.eq_ignore_ascii_case("delete") {
        bail!("issue automation database requires DELETE rollback journaling")
    }
    // journal_mode loads the schema, so SQLite has now attempted hot-journal recovery.
    // No damaged-header exception remains after that native recovery opportunity.
    guard.verify_identity()?;
    validate_database_header(guard.database_file(), None)?;
    connection.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA secure_delete = ON;
         PRAGMA temp_store = MEMORY;
         PRAGMA synchronous = FULL;",
    )?;
    guard.verify_identity()?;
    Ok(PrivateConnection { connection, guard })
}

fn reject_wal_sidecars(runtime: &Path) -> Result<()> {
    for name in ["issue-automation.db-wal", "issue-automation.db-shm"] {
        match fs::symlink_metadata(runtime.join(name)) {
            Ok(_) => bail!("issue automation database does not accept WAL or SHM files"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("failed to inspect issue automation sidecars"),
        }
    }
    Ok(())
}

fn validate_database_header(file: &File, recovery_journal: Option<&File>) -> Result<()> {
    let mut header = [0u8; 20];
    let read = read_prefix(file, &mut header)?;
    if read == 0
        || (read == header.len()
            && &header[..16] == b"SQLite format 3\0"
            && header[18] == 1
            && header[19] == 1)
    {
        return Ok(());
    }
    // A known WAL format is rejected even with a rollback journal or a damaged magic.
    // This deliberately also rejects a torn write that happens to leave a format byte of 2.
    if (read > 18 && header[18] == 2) || (read > 19 && header[19] == 2) {
        bail!("issue automation database has an unsupported file format")
    }
    if let Some(journal) = recovery_journal
        && has_rollback_recovery_header(journal)?
    {
        return Ok(());
    }
    bail!("issue automation database has an unsupported file format")
}

fn has_rollback_recovery_header(journal: &File) -> Result<bool> {
    let mut header = [0u8; 28];
    if read_prefix(journal, &mut header)? != header.len()
        || header[..8] != [0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7]
    {
        return Ok(false);
    }
    let field = |offset| u32::from_be_bytes(header[offset..offset + 4].try_into().unwrap());
    let sector_size = field(20);
    let page_size = field(24);
    // Only authorize a native recovery attempt. SQLite owns hot-journal locking,
    // record/checksum validation and playback; this never reimplements recovery.
    Ok(field(8) != 0
        && field(16) != 0
        && (512..=65536).contains(&sector_size)
        && sector_size.is_power_of_two()
        && (512..=65536).contains(&page_size)
        && page_size.is_power_of_two()
        && journal.metadata()?.len() >= u64::from(sector_size) + u64::from(page_size) + 8)
}

fn read_prefix(file: &File, buffer: &mut [u8]) -> Result<usize> {
    let mut read = 0;
    while read < buffer.len() {
        let count = platform::read_at(file, &mut buffer[read..], read as u64)?;
        if count == 0 {
            break;
        }
        read += count;
    }
    Ok(read)
}

#[cfg(unix)]
mod platform {
    use std::collections::HashMap;
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::path::Component;
    use std::sync::{LazyLock, Mutex, Weak};

    use super::*;

    type Inode = (u64, u64);
    static DATABASE_ANCHORS: LazyLock<Mutex<HashMap<Inode, Weak<DatabaseAnchor>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    pub(super) fn open_runtime(runtime: &Path) -> Result<Vec<(PathBuf, File)>> {
        let root = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open("/")?;
        let mut anchors = vec![(PathBuf::from("/"), root)];
        let mut path = PathBuf::from("/");
        for component in runtime.components() {
            match component {
                Component::RootDir => continue,
                Component::Normal(name) => {
                    let name = CString::new(name.as_bytes())?;
                    let parent = &anchors.last().context("missing runtime parent")?.1;
                    let directory = open_at(
                        parent,
                        &name,
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                        0,
                    )?;
                    path.push(component.as_os_str());
                    anchors.push((path.clone(), directory));
                }
                _ => bail!("issue automation runtime must not contain parent or prefix aliases"),
            }
        }
        for (index, (path, directory)) in anchors.iter().enumerate() {
            verify_directory(path, directory, index + 1 == anchors.len())?;
        }
        Ok(anchors)
    }

    pub(super) fn verify_directory(path: &Path, directory: &File, private: bool) -> Result<()> {
        let opened = directory.metadata()?;
        let current = fs::symlink_metadata(path)?;
        // SAFETY: geteuid has no preconditions.
        let user = unsafe { libc::geteuid() };
        if !opened.is_dir()
            || current.file_type().is_symlink()
            || !current.is_dir()
            || opened.dev() != current.dev()
            || opened.ino() != current.ino()
            || (opened.uid() != user && opened.uid() != 0)
        {
            bail!("issue automation runtime directory identity is unsafe")
        }
        if private {
            if opened.uid() != user || opened.mode() & 0o777 != 0o700 {
                bail!("issue automation runtime is not a private directory")
            }
        } else if opened.mode() & 0o022 != 0 && opened.mode() & 0o1000 == 0 {
            bail!("issue automation runtime ancestor is replaceable by other users")
        }
        Ok(())
    }

    pub(super) fn open_database(path: &Path, directory: &File) -> Result<Arc<DatabaseAnchor>> {
        let mut anchors = DATABASE_ANCHORS
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        anchors.retain(|_, anchor| anchor.strong_count() != 0);
        // Do not open another descriptor just to look up an existing inode. Its close would
        // release the process's POSIX locks even while another SQLite connection remains live.
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                validate_owned_file(&metadata)?;
                if let Some(anchor) = anchors
                    .get(&(metadata.dev(), metadata.ino()))
                    .and_then(Weak::upgrade)
                {
                    verify_file(path, &anchor.file)?;
                    return Ok(anchor);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("failed to inspect issue automation database"),
        }
        let file = open_at(
            directory,
            &CString::new(DATABASE_NAME)?,
            libc::O_RDWR | libc::O_CREAT | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            0o600,
        )?;
        let metadata = file.metadata()?;
        let inode = (metadata.dev(), metadata.ino());
        if let Some(anchor) = anchors.get(&inode).and_then(Weak::upgrade) {
            anchor
                .deferred_files
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(file);
            verify_file(path, &anchor.file)?;
            return Ok(anchor);
        }
        validate_owned_file(&metadata)?;
        verify_same_file(path, &metadata)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        verify_file(path, &file)?;
        let anchor = Arc::new(DatabaseAnchor::new(file));
        anchors.insert(inode, Arc::downgrade(&anchor));
        Ok(anchor)
    }

    pub(super) fn release_database(database: Option<Arc<DatabaseAnchor>>) {
        // Serialize the final File close with registry lookup/open, not only Arc creation.
        // A zero strong count must never let a new SQLite connection race an old FD close.
        let _anchors = DATABASE_ANCHORS
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        drop(database);
    }

    pub(super) fn open_journal(path: &Path, directory: &File) -> Result<Option<File>> {
        match fs::symlink_metadata(path) {
            Ok(metadata) => validate_owned_file(&metadata)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("failed to inspect issue automation journal"),
        }
        let file = match open_at(
            directory,
            &CString::new(ROLLBACK_JOURNAL_NAME)?,
            libc::O_RDWR | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            0,
        ) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("failed to open issue automation journal"),
        };
        verify_file(path, &file)?;
        Ok(Some(file))
    }

    fn validate_owned_file(metadata: &fs::Metadata) -> Result<()> {
        // SAFETY: geteuid has no preconditions.
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
        {
            bail!("issue automation file must be an owned regular file with one link")
        }
        Ok(())
    }

    fn verify_same_file(path: &Path, opened: &fs::Metadata) -> Result<()> {
        let current = fs::symlink_metadata(path)?;
        validate_owned_file(&current)?;
        if current.file_type().is_symlink()
            || current.dev() != opened.dev()
            || current.ino() != opened.ino()
        {
            bail!("issue automation database path identity changed")
        }
        Ok(())
    }

    pub(super) fn verify_file(path: &Path, file: &File) -> Result<()> {
        let opened = file.metadata()?;
        validate_owned_file(&opened)?;
        if opened.mode() & 0o777 != 0o600 {
            bail!("issue automation database file is not private")
        }
        verify_same_file(path, &opened)
    }

    fn open_at(
        directory: &File,
        name: &CString,
        flags: libc::c_int,
        mode: libc::c_uint,
    ) -> std::io::Result<File> {
        // SAFETY: the descriptor and NUL-terminated name remain valid; mode uses the C
        // variadic unsigned-int promotion required even where mode_t is narrower.
        let descriptor = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags, mode) };
        if descriptor < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            // SAFETY: openat returned a fresh descriptor, now owned by File.
            Ok(unsafe { File::from_raw_fd(descriptor) })
        }
    }

    pub(super) fn read_at(file: &File, buffer: &mut [u8], offset: u64) -> std::io::Result<usize> {
        file.read_at(buffer, offset)
    }
}

#[cfg(windows)]
mod platform {
    use std::os::windows::fs::{FileExt, OpenOptionsExt};
    use std::path::Component;

    use crate::private_fs::{
        WINDOWS_FILE_FLAG_BACKUP_SEMANTICS, WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
        WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ, WINDOWS_GENERIC_WRITE, WINDOWS_READ_CONTROL,
        WINDOWS_WRITE_DAC, set_windows_private_acl, validate_windows_path_identity,
        validate_windows_path_identity_only, validate_windows_private_owner_and_acl,
    };

    use super::*;

    pub(super) fn open_runtime(runtime: &Path) -> Result<Vec<(PathBuf, File)>> {
        let mut path = PathBuf::new();
        let mut anchors = Vec::new();
        for component in runtime.components() {
            match component {
                Component::Prefix(_) => {
                    path.push(component.as_os_str());
                    continue;
                }
                Component::RootDir | Component::Normal(_) => path.push(component.as_os_str()),
                _ => bail!("issue automation runtime must not contain parent aliases"),
            }
            let directory = fs::OpenOptions::new()
                .read(true)
                .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
                .share_mode(WINDOWS_FILE_SHARE_ALL)
                .custom_flags(
                    WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT | WINDOWS_FILE_FLAG_BACKUP_SEMANTICS,
                )
                .open(&path)?;
            anchors.push((path.clone(), directory));
        }
        for (index, (path, directory)) in anchors.iter().enumerate() {
            verify_directory(path, directory, index + 1 == anchors.len())?;
        }
        Ok(anchors)
    }

    pub(super) fn verify_directory(path: &Path, directory: &File, private: bool) -> Result<()> {
        retry_identity(|| {
            if private {
                validate_windows_path_identity(path, directory, true)?;
                validate_windows_private_owner_and_acl(path, directory)
            } else {
                validate_windows_path_identity_only(path, directory, true)
            }
        })
    }

    pub(super) fn open_database(path: &Path, _: &File) -> Result<Arc<DatabaseAnchor>> {
        let file = file_options().create(true).open(path)?;
        secure_file(path, &file)?;
        Ok(Arc::new(DatabaseAnchor::new(file)))
    }

    pub(super) fn release_database(database: Option<Arc<DatabaseAnchor>>) {
        drop(database);
    }

    pub(super) fn open_journal(path: &Path, _: &File) -> Result<Option<File>> {
        let file = match file_options().open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("failed to open issue automation journal"),
        };
        secure_file(path, &file)?;
        Ok(Some(file))
    }

    fn file_options() -> fs::OpenOptions {
        let mut options = fs::OpenOptions::new();
        options
            .read(true)
            .write(true)
            .access_mode(
                WINDOWS_GENERIC_READ
                    | WINDOWS_GENERIC_WRITE
                    | WINDOWS_READ_CONTROL
                    | WINDOWS_WRITE_DAC,
            )
            .share_mode(WINDOWS_FILE_SHARE_ALL)
            .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT);
        options
    }

    fn secure_file(path: &Path, file: &File) -> Result<()> {
        retry_identity(|| validate_windows_path_identity(path, file, false))?;
        set_windows_private_acl(file, false)?;
        verify_file(path, file)
    }

    pub(super) fn verify_file(path: &Path, file: &File) -> Result<()> {
        retry_identity(|| {
            validate_windows_path_identity(path, file, false)?;
            validate_windows_private_owner_and_acl(path, file)
        })
    }

    fn retry_identity(mut check: impl FnMut() -> Result<()>) -> Result<()> {
        for attempt in 0..TRANSIENT_ATTEMPTS {
            match check() {
                Err(_) if attempt + 1 < TRANSIENT_ATTEMPTS => {
                    std::thread::sleep(TRANSIENT_DELAY);
                }
                result => return result,
            }
        }
        unreachable!("bounded identity validation returns")
    }

    pub(super) fn read_at(file: &File, buffer: &mut [u8], offset: u64) -> std::io::Result<usize> {
        file.seek_read(buffer, offset)
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    use super::*;

    pub(super) fn open_runtime(_: &Path) -> Result<Vec<(PathBuf, File)>> {
        bail!("private issue automation databases are unsupported on this platform")
    }

    pub(super) fn open_database(_: &Path, _: &File) -> Result<Arc<DatabaseAnchor>> {
        bail!("private issue automation databases are unsupported on this platform")
    }

    pub(super) fn release_database(database: Option<Arc<DatabaseAnchor>>) {
        drop(database);
    }

    pub(super) fn open_journal(_: &Path, _: &File) -> Result<Option<File>> {
        bail!("private issue automation databases are unsupported on this platform")
    }

    pub(super) fn verify_directory(_: &Path, _: &File, _: bool) -> Result<()> {
        bail!("private issue automation databases are unsupported on this platform")
    }

    pub(super) fn verify_file(_: &Path, _: &File) -> Result<()> {
        bail!("private issue automation databases are unsupported on this platform")
    }

    pub(super) fn read_at(_: &File, _: &mut [u8], _: u64) -> std::io::Result<usize> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "private issue automation databases are unsupported on this platform",
        ))
    }
}

#[cfg(test)]
mod tests;
