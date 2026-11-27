//! Private file boundary for rollback-journal SQLite databases.
//! No schema is created here; callers keep the guard alive through their transaction.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags};

pub(crate) use akra_native::private_path::PinnedPrivateDirectory;
use akra_native::private_path::{DirectoryChain, entry_appeared, validate_runtime_path};

/// File names owned by one rollback-journal database, all relative to its private runtime.
#[derive(Clone, Copy)]
pub(crate) struct DatabaseFiles {
    database: &'static str,
    rollback_journal: &'static str,
    wal: &'static str,
    shm: &'static str,
}

impl DatabaseFiles {
    pub(crate) const fn new(
        database: &'static str,
        rollback_journal: &'static str,
        wal: &'static str,
        shm: &'static str,
    ) -> Self {
        Self {
            database,
            rollback_journal,
            wal,
            shm,
        }
    }

    fn validate(self) -> Result<()> {
        for name in [self.database, self.rollback_journal, self.wal, self.shm] {
            validate_file_name(name)?;
        }
        if self.rollback_journal != format!("{}-journal", self.database)
            || self.wal != format!("{}-wal", self.database)
            || self.shm != format!("{}-shm", self.database)
        {
            bail!("private SQLite companion names must match the database filename")
        }
        Ok(())
    }
}

fn validate_file_name(name: &str) -> Result<()> {
    // Keep the same literal filename on Unix and Windows: no separators, aliases, ADS,
    // trailing dots, or device names may redirect a supposedly private database file.
    if name.is_empty()
        || name.len() > 255
        || name == "."
        || name == ".."
        || name.ends_with('.')
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        bail!("private SQLite filename must be a safe single component")
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
    {
        bail!("private SQLite filename must not name a reserved device")
    }
    Ok(())
}

/// Required SQLite connection synchronization, checked after configuration.
#[derive(Clone, Copy)]
pub(crate) enum Synchronous {
    Full,
    Extra,
}

impl Synchronous {
    fn pragma(self) -> &'static str {
        match self {
            Self::Full => "PRAGMA synchronous = FULL;",
            Self::Extra => "PRAGMA synchronous = EXTRA;",
        }
    }

    fn value(self) -> i64 {
        match self {
            Self::Full => 2,
            Self::Extra => 3,
        }
    }
}
#[derive(Clone, Copy)]
enum DatabaseOpenMode {
    LegacyCreate,
    StrictCreate,
    Existing,
}

const TRANSIENT_ATTEMPTS: usize = 8;
const TRANSIENT_DELAY: Duration = Duration::from_millis(5);

/// Field order matters: close SQLite before releasing its independently opened file anchor.
pub(crate) struct PrivateConnection {
    connection: Connection,
    guard: PrivateStoreGuard,
}

impl PrivateConnection {
    pub(crate) fn parts(&mut self) -> (&mut Connection, &PrivateStoreGuard) {
        (&mut self.connection, &self.guard)
    }

    #[cfg(windows)]
    pub(crate) fn verify_identity(&self) -> Result<()> {
        self.guard.verify_identity()
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

pub(crate) struct PrivateStoreGuard {
    runtime: PathBuf,
    files: DatabaseFiles,
    directories: DirectoryChain,
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

    pub(crate) fn verify_identity(&self) -> Result<()> {
        self.verify_directories()?;
        platform::verify_file(
            &self.runtime.join(self.files.database),
            self.database_file(),
        )?;
        reject_wal_sidecars(&self.runtime, self.files)?;
        let _journal = self.rollback_journal()?;
        self.verify_directories()
    }

    fn verify_directories(&self) -> Result<()> {
        self.directories.verify()
    }

    fn rollback_journal(&self) -> Result<Option<File>> {
        let path = self.runtime.join(self.files.rollback_journal);
        let directory = self.directories.handle();
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

/// Open an existing private runtime without creating or repairing directories.
pub(crate) fn open(
    runtime: &Path,
    files: DatabaseFiles,
    synchronous: Synchronous,
) -> Result<PrivateConnection> {
    open_database(runtime, files, synchronous, false, Duration::from_secs(5))
}

/// Open only an existing database. A confirmed missing component is empty state;
/// missing directories/database are never created and existing permissions are not repaired.
/// Dangling links remain errors. Existing rollback-journal handling is retained: Windows
/// normalizes journal ACLs before SQLite performs native hot-journal recovery.
pub(crate) fn open_existing(
    runtime: &Path,
    files: DatabaseFiles,
    synchronous: Synchronous,
) -> Result<Option<PrivateConnection>> {
    open_existing_with_busy_timeout(runtime, files, synchronous, Duration::from_secs(5))
}

/// Existing-only access with a caller-owned SQLite contention budget.
pub(crate) fn open_existing_with_busy_timeout(
    runtime: &Path,
    files: DatabaseFiles,
    synchronous: Synchronous,
    busy_timeout: Duration,
) -> Result<Option<PrivateConnection>> {
    files.validate()?;
    validate_runtime_path(runtime)?;
    for attempt in 0..TRANSIENT_ATTEMPTS {
        match open_existing_once(runtime, files, synchronous, busy_timeout) {
            Err(error) if entry_appeared(&error) && attempt + 1 < TRANSIENT_ATTEMPTS => {
                std::thread::sleep(TRANSIENT_DELAY);
            }
            result => return result,
        }
    }
    unreachable!("bounded existing database lookup returns")
}

fn open_existing_once(
    runtime: &Path,
    files: DatabaseFiles,
    synchronous: Synchronous,
    busy_timeout: Duration,
) -> Result<Option<PrivateConnection>> {
    let Some(directories) = DirectoryChain::open_existing(runtime)? else {
        return Ok(None);
    };
    reject_wal_sidecars(runtime, files)?;
    let directory = directories.handle();
    let Some(database) = platform::open_database(
        &runtime.join(files.database),
        directory,
        DatabaseOpenMode::Existing,
    )?
    else {
        directories.confirm_absence(&runtime.join(files.database))?;
        // A journal without its database is damaged state, not an empty registry.
        directories.confirm_absence(&runtime.join(files.rollback_journal))?;
        reject_wal_sidecars(runtime, files)?;
        return Ok(None);
    };
    finish_open(
        runtime,
        files,
        synchronous,
        busy_timeout,
        directories,
        database,
        false,
    )
    .map(Some)
}

/// Bound the complete SQLite acquisition wait before accessing its schema.
#[cfg(windows)]
pub(crate) fn open_with_busy_timeout(
    runtime: &Path,
    files: DatabaseFiles,
    synchronous: Synchronous,
    busy_timeout: Duration,
) -> Result<PrivateConnection> {
    open_database(runtime, files, synchronous, false, busy_timeout)
}

/// Create missing runtime components privately while keeping every directory pinned.
/// Existing directory ownership, permissions, and identity must already be acceptable.
pub(crate) fn open_or_create(
    runtime: &Path,
    files: DatabaseFiles,
    synchronous: Synchronous,
) -> Result<PrivateConnection> {
    open_database(runtime, files, synchronous, true, Duration::from_secs(5))
}

/// Create privately, but never repair a pre-existing database's permissions.
/// The legacy entry points retain their historical permission behavior.
pub(crate) fn open_or_create_strict(
    runtime: &Path,
    files: DatabaseFiles,
    synchronous: Synchronous,
) -> Result<PrivateConnection> {
    open_or_create_strict_with_busy_timeout(runtime, files, synchronous, Duration::from_secs(5))
}

/// A profile owner lock must reject a live owner without waiting or repairing its files.
pub(crate) fn open_or_create_strict_with_busy_timeout(
    runtime: &Path,
    files: DatabaseFiles,
    synchronous: Synchronous,
    busy_timeout: Duration,
) -> Result<PrivateConnection> {
    open_database_with_mode(
        runtime,
        files,
        synchronous,
        true,
        busy_timeout,
        DatabaseOpenMode::StrictCreate,
    )
}

fn open_database(
    runtime: &Path,
    files: DatabaseFiles,
    synchronous: Synchronous,
    create: bool,
    busy_timeout: Duration,
) -> Result<PrivateConnection> {
    open_database_with_mode(
        runtime,
        files,
        synchronous,
        create,
        busy_timeout,
        DatabaseOpenMode::LegacyCreate,
    )
}

fn open_database_with_mode(
    runtime: &Path,
    files: DatabaseFiles,
    synchronous: Synchronous,
    create: bool,
    busy_timeout: Duration,
    mode: DatabaseOpenMode,
) -> Result<PrivateConnection> {
    // Validate the complete request before creating even the first directory.
    files.validate()?;
    validate_runtime_path(runtime)?;
    let directories = DirectoryChain::open(runtime, create)?;
    reject_wal_sidecars(runtime, files)?;
    let directory = directories.handle();
    let database = platform::open_database(&runtime.join(files.database), directory, mode)?
        .context("private SQLite database creation returned no file")?;
    finish_open(
        runtime,
        files,
        synchronous,
        busy_timeout,
        directories,
        database,
        true,
    )
}

fn finish_open(
    runtime: &Path,
    files: DatabaseFiles,
    synchronous: Synchronous,
    busy_timeout: Duration,
    directories: DirectoryChain,
    database: Arc<DatabaseAnchor>,
    create_database: bool,
) -> Result<PrivateConnection> {
    let guard = PrivateStoreGuard {
        runtime: runtime.to_path_buf(),
        files,
        directories,
        database: Some(database),
    };
    guard.verify_identity()?;
    // Keep an existing hot journal pinned while SQLite performs normal rollback recovery.
    let hot_journal = guard.rollback_journal()?;
    validate_database_header(guard.database_file(), hot_journal.as_ref())?;

    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | if create_database {
            OpenFlags::SQLITE_OPEN_CREATE
        } else {
            OpenFlags::empty()
        }
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_NOFOLLOW;
    let connection = Connection::open_with_flags(runtime.join(files.database), flags)
        .context("failed to open private SQLite database")?;
    connection.busy_timeout(busy_timeout)?;
    guard.verify_identity()?;
    let mode = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
        .context("failed to load private SQLite schema and journal mode")?;
    if !mode.eq_ignore_ascii_case("delete") {
        bail!("private SQLite database requires DELETE rollback journaling")
    }
    // journal_mode loads the schema, so SQLite has now attempted hot-journal recovery.
    // No damaged-header exception remains after that native recovery opportunity.
    guard.verify_identity()?;
    validate_database_header(guard.database_file(), None)?;
    connection.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA secure_delete = ON;
         PRAGMA temp_store = MEMORY;",
    )?;
    connection.execute_batch(synchronous.pragma())?;
    let actual = connection.query_row("PRAGMA synchronous", [], |row| row.get::<_, i64>(0))?;
    if actual != synchronous.value() {
        bail!("private SQLite synchronous setting does not match its required policy")
    }
    guard.verify_identity()?;
    Ok(PrivateConnection { connection, guard })
}

fn reject_wal_sidecars(runtime: &Path, files: DatabaseFiles) -> Result<()> {
    for name in [files.wal, files.shm] {
        match fs::symlink_metadata(runtime.join(name)) {
            Ok(_) => bail!("private SQLite database does not accept WAL or SHM files"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("failed to inspect private SQLite sidecars"),
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
        bail!("private SQLite database has an unsupported file format")
    }
    if let Some(journal) = recovery_journal
        && has_rollback_recovery_header(journal)?
    {
        return Ok(());
    }
    bail!("private SQLite database has an unsupported file format")
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
    use std::os::unix::fs::{FileExt, MetadataExt, PermissionsExt};
    use std::sync::{LazyLock, Mutex, Weak};

    use super::*;

    type Inode = (u64, u64);
    static DATABASE_ANCHORS: LazyLock<Mutex<HashMap<Inode, Weak<DatabaseAnchor>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    pub(super) fn open_database(
        path: &Path,
        directory: &File,
        mode: DatabaseOpenMode,
    ) -> Result<Option<Arc<DatabaseAnchor>>> {
        let create = !matches!(mode, DatabaseOpenMode::Existing);
        let strict = !matches!(mode, DatabaseOpenMode::LegacyCreate);
        let mut anchors = DATABASE_ANCHORS
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        anchors.retain(|_, anchor| anchor.strong_count() != 0);
        // Do not open another descriptor just to look up an existing inode. Its close would
        // release the process's POSIX locks even while another SQLite connection remains live.
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                validate_owned_file(&metadata)?;
                if strict && metadata.mode() & 0o777 != 0o600 {
                    bail!("existing private SQLite database file is not private");
                }
                if let Some(anchor) = anchors
                    .get(&(metadata.dev(), metadata.ino()))
                    .and_then(Weak::upgrade)
                {
                    verify_file(path, &anchor.file)?;
                    return Ok(Some(anchor));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !create => {
                return Ok(None);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("failed to inspect private SQLite database"),
        }
        let name = CString::new(
            path.file_name()
                .context("missing database filename")?
                .as_bytes(),
        )?;
        let flags = libc::O_RDWR | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;
        let (file, created) = if create {
            match open_at(
                directory,
                &name,
                flags | libc::O_CREAT | libc::O_EXCL,
                0o600,
            ) {
                Ok(file) => (file, true),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    (open_at(directory, &name, flags, 0)?, false)
                }
                Err(error) => {
                    return Err(error).context("failed to create private SQLite database");
                }
            }
        } else {
            (open_at(directory, &name, flags, 0)?, false)
        };
        let metadata = file.metadata()?;
        let inode = (metadata.dev(), metadata.ino());
        if let Some(anchor) = anchors.get(&inode).and_then(Weak::upgrade) {
            anchor
                .deferred_files
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(file);
            verify_file(path, &anchor.file)?;
            return Ok(Some(anchor));
        }
        validate_owned_file(&metadata)?;
        verify_same_file(path, &metadata)?;
        if created || !strict {
            // Strict operations correct a restrictive umask only on their new inode.
            // Legacy consumers retain their existing permission-normalization contract.
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        verify_file(path, &file)?;
        let anchor = Arc::new(DatabaseAnchor::new(file));
        anchors.insert(inode, Arc::downgrade(&anchor));
        Ok(Some(anchor))
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
            Err(error) => return Err(error).context("failed to inspect private SQLite journal"),
        }
        let file = match open_at(
            directory,
            &CString::new(
                path.file_name()
                    .context("missing journal filename")?
                    .as_bytes(),
            )?,
            libc::O_RDWR | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            0,
        ) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("failed to open private SQLite journal"),
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
            bail!("private SQLite file must be an owned regular file with one link")
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
            bail!("private SQLite database path identity changed")
        }
        Ok(())
    }

    pub(super) fn verify_file(path: &Path, file: &File) -> Result<()> {
        let opened = file.metadata()?;
        validate_owned_file(&opened)?;
        if opened.mode() & 0o777 != 0o600 {
            bail!("private SQLite database file is not private")
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

    use crate::private_fs::{
        WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT, WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ,
        WINDOWS_GENERIC_WRITE, WINDOWS_READ_CONTROL, WINDOWS_WRITE_DAC, set_windows_private_acl,
        validate_windows_path_identity, validate_windows_private_owner_and_acl,
    };

    use super::*;

    pub(super) fn open_database(
        path: &Path,
        _: &File,
        mode: DatabaseOpenMode,
    ) -> Result<Option<Arc<DatabaseAnchor>>> {
        if matches!(mode, DatabaseOpenMode::Existing) {
            let file = match file_options().open(path) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => {
                    return Err(error).context("failed to open existing private SQLite database");
                }
            };
            verify_file(path, &file)?;
            return Ok(Some(Arc::new(DatabaseAnchor::new(file))));
        }
        let file = match file_options().create_new(true).open(path) {
            Ok(file) => {
                secure_file(path, &file)?;
                file
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let file = file_options().open(path)?;
                // Only an atomically created database may receive its initial ACL.
                // Existing databases must never have unsafe permissions repaired.
                verify_file(path, &file)?;
                file
            }
            Err(error) => return Err(error).context("failed to create private SQLite database"),
        };
        Ok(Some(Arc::new(DatabaseAnchor::new(file))))
    }

    pub(super) fn release_database(database: Option<Arc<DatabaseAnchor>>) {
        drop(database);
    }

    pub(super) fn open_journal(path: &Path, _: &File) -> Result<Option<File>> {
        let file = match file_options().open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("failed to open private SQLite journal"),
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
        // A live rollback journal can be inspected by several processes. Reapplying the
        // same ACL changes its metadata and races their identity snapshots while a
        // writer holds SQLite locks. Normalize an inherited journal once, then read only.
        if validate_windows_private_owner_and_acl(path, file).is_err() {
            set_windows_private_acl(file, false)?;
        }
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

    pub(super) fn open_database(
        _: &Path,
        _: &File,
        _: DatabaseOpenMode,
    ) -> Result<Option<Arc<DatabaseAnchor>>> {
        bail!("private SQLite databases are unsupported on this platform")
    }

    pub(super) fn release_database(database: Option<Arc<DatabaseAnchor>>) {
        drop(database);
    }

    pub(super) fn open_journal(_: &Path, _: &File) -> Result<Option<File>> {
        bail!("private SQLite databases are unsupported on this platform")
    }

    pub(super) fn verify_file(_: &Path, _: &File) -> Result<()> {
        bail!("private SQLite databases are unsupported on this platform")
    }

    pub(super) fn read_at(_: &File, _: &mut [u8], _: u64) -> std::io::Result<usize> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "private SQLite databases are unsupported on this platform",
        ))
    }
}

#[cfg(test)]
mod tests;
