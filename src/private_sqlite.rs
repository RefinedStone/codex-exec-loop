//! Private file boundary for rollback-journal SQLite databases.
//! No schema is created here; callers keep the guard alive through their transaction.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags};

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

struct DirectoryAnchor {
    path: PathBuf,
    file: File,
    private: bool,
}

impl DirectoryAnchor {
    fn verify(&self) -> Result<()> {
        platform::verify_directory(&self.path, &self.file, self.private)
    }
}

pub(crate) struct PrivateStoreGuard {
    runtime: PathBuf,
    files: DatabaseFiles,
    directories: Vec<DirectoryAnchor>,
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
        for directory in &self.directories {
            directory.verify()?;
        }
        Ok(())
    }

    fn rollback_journal(&self) -> Result<Option<File>> {
        let path = self.runtime.join(self.files.rollback_journal);
        let directory = &self
            .directories
            .last()
            .context("missing private runtime anchor")?
            .file;
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
    open_database(runtime, files, synchronous, false)
}

/// Create missing runtime components privately while keeping every directory pinned.
/// Existing directory ownership, permissions, and identity must already be acceptable.
pub(crate) fn open_or_create(
    runtime: &Path,
    files: DatabaseFiles,
    synchronous: Synchronous,
) -> Result<PrivateConnection> {
    open_database(runtime, files, synchronous, true)
}

fn validate_runtime_path(runtime: &Path) -> Result<()> {
    if !runtime.is_absolute() || runtime.as_os_str().as_encoded_bytes().contains(&0) {
        bail!("private SQLite runtime must be an absolute directory without NUL bytes")
    }
    for component in runtime.components() {
        match component {
            std::path::Component::Prefix(_) | std::path::Component::RootDir => {}
            std::path::Component::Normal(name) => {
                #[cfg(windows)]
                {
                    let name = name
                        .to_str()
                        .context("private SQLite directory name is not Unicode")?;
                    let stem = name
                        .split('.')
                        .next()
                        .unwrap_or_default()
                        .to_ascii_uppercase();
                    if name.ends_with(['.', ' '])
                        || name
                            .chars()
                            .any(|character| character < ' ' || "<>:\"|?*".contains(character))
                        || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                        || (stem.len() == 4
                            && (stem.starts_with("COM") || stem.starts_with("LPT"))
                            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
                    {
                        bail!("private SQLite directory name must not contain Windows aliases")
                    }
                }
                #[cfg(not(windows))]
                let _ = name;
            }
            _ => {
                bail!("private SQLite runtime must not contain parent or current-directory aliases")
            }
        }
    }
    Ok(())
}

fn open_database(
    runtime: &Path,
    files: DatabaseFiles,
    synchronous: Synchronous,
    create: bool,
) -> Result<PrivateConnection> {
    // Validate the complete request before creating even the first directory.
    files.validate()?;
    validate_runtime_path(runtime)?;
    let directories = platform::open_runtime(runtime, create)?;
    reject_wal_sidecars(runtime, files)?;
    let directory = &directories
        .last()
        .context("missing private runtime anchor")?
        .file;
    let database = platform::open_database(&runtime.join(files.database), directory)?;
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
        | OpenFlags::SQLITE_OPEN_CREATE
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_NOFOLLOW;
    let connection = Connection::open_with_flags(runtime.join(files.database), flags)
        .context("failed to open private SQLite database")?;
    connection.busy_timeout(Duration::from_secs(5))?;
    guard.verify_identity()?;
    let mode = connection.query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))?;
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
    use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::path::Component;
    use std::sync::{LazyLock, Mutex, Weak};

    use super::*;

    type Inode = (u64, u64);
    static DATABASE_ANCHORS: LazyLock<Mutex<HashMap<Inode, Weak<DatabaseAnchor>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    pub(super) fn open_runtime(runtime: &Path, create: bool) -> Result<Vec<DirectoryAnchor>> {
        let components = runtime.components().collect::<Vec<_>>();
        let root = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open("/")?;
        let mut anchors = vec![DirectoryAnchor {
            path: PathBuf::from("/"),
            file: root,
            private: components.len() == 1,
        }];
        let mut path = PathBuf::from("/");
        for (index, component) in components.iter().enumerate() {
            match component {
                Component::RootDir => continue,
                Component::Normal(name) => {
                    let name = CString::new(name.as_bytes())?;
                    let parent = anchors.last().context("missing runtime parent")?;
                    if create {
                        parent.verify()?;
                    }
                    let flags =
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
                    let mut created = false;
                    let directory = match open_at(&parent.file, &name, flags, 0) {
                        Ok(directory) => directory,
                        Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
                            parent.verify()?;
                            created = create_directory_at(&parent.file, &name)?;
                            let directory = open_at(&parent.file, &name, flags, 0)?;
                            // Pin the entry and persist its parent before using a new directory.
                            parent
                                .file
                                .sync_all()
                                .context("failed to sync private SQLite parent directory")?;
                            parent.verify()?;
                            directory
                        }
                        Err(error) => {
                            return Err(error).context("failed to open private SQLite directory");
                        }
                    };
                    path.push(component.as_os_str());
                    let anchor = DirectoryAnchor {
                        path: path.clone(),
                        file: directory,
                        private: created || index + 1 == components.len(),
                    };
                    if create {
                        anchor.verify()?;
                        parent.verify()?;
                    }
                    anchors.push(anchor);
                }
                _ => bail!("private SQLite runtime must not contain parent or prefix aliases"),
            }
        }
        for anchor in &anchors {
            anchor.verify()?;
        }
        Ok(anchors)
    }

    fn create_directory_at(directory: &File, name: &CString) -> Result<bool> {
        // SAFETY: the parent descriptor and NUL-terminated child name remain valid.
        let status = unsafe { libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o700) };
        if status == 0 {
            return Ok(true);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            Ok(false)
        } else {
            Err(error).context("failed to create private SQLite directory")
        }
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
            bail!("private SQLite runtime directory identity is unsafe")
        }
        if private {
            if opened.uid() != user || opened.mode() & 0o777 != 0o700 {
                bail!("private SQLite runtime is not a private directory")
            }
        } else if opened.mode() & 0o022 != 0 && opened.mode() & 0o1000 == 0 {
            bail!("private SQLite runtime ancestor is replaceable by other users")
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
            Err(error) => return Err(error).context("failed to inspect private SQLite database"),
        }
        let file = open_at(
            directory,
            &CString::new(
                path.file_name()
                    .context("missing database filename")?
                    .as_bytes(),
            )?,
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
    use std::path::Component;

    use crate::private_fs::{
        WINDOWS_FILE_FLAG_BACKUP_SEMANTICS, WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
        WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ, WINDOWS_GENERIC_WRITE, WINDOWS_READ_CONTROL,
        WINDOWS_WRITE_DAC, set_windows_private_acl, validate_windows_path_identity,
        validate_windows_path_identity_only, validate_windows_private_owner_and_acl,
    };

    use super::*;

    pub(super) fn open_runtime(runtime: &Path, create: bool) -> Result<Vec<DirectoryAnchor>> {
        let components = runtime.components().collect::<Vec<_>>();
        let mut path = PathBuf::new();
        let mut anchors: Vec<DirectoryAnchor> = Vec::new();
        for (index, component) in components.iter().enumerate() {
            match component {
                Component::Prefix(_) => {
                    path.push(component.as_os_str());
                    continue;
                }
                Component::RootDir | Component::Normal(_) => path.push(component.as_os_str()),
                _ => bail!("private SQLite runtime must not contain parent aliases"),
            }
            if create && let Some(parent) = anchors.last() {
                parent.verify()?;
            }
            let mut created = false;
            let directory = match open_directory(&path, create, false) {
                Ok(directory) => directory,
                Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
                    let parent = anchors
                        .last()
                        .context("missing private SQLite directory parent")?;
                    parent.verify()?;
                    created = create_private_directory(&path)?;
                    parent.verify()?;
                    let directory = open_directory(&path, true, created)?;
                    if created {
                        // The initial DACL is already owner-only. Apply directory inheritance only
                        // through this verified handle; never repair an existing directory.
                        validate_windows_path_identity(&path, &directory, true)?;
                        set_windows_private_acl(&directory, true)?;
                    }
                    directory
                }
                Err(error) => return Err(error).context("failed to open private SQLite directory"),
            };
            let anchor = DirectoryAnchor {
                path: path.clone(),
                file: directory,
                private: created || index + 1 == components.len(),
            };
            if create {
                anchor.verify()?;
                if let Some(parent) = anchors.last() {
                    parent.verify()?;
                }
            }
            anchors.push(anchor);
        }
        for anchor in &anchors {
            anchor.verify()?;
        }
        Ok(anchors)
    }

    fn open_directory(path: &Path, pin: bool, write_acl: bool) -> std::io::Result<File> {
        use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_DELETE;

        fs::OpenOptions::new()
            .read(true)
            .access_mode(
                WINDOWS_GENERIC_READ
                    | WINDOWS_READ_CONTROL
                    | if write_acl { WINDOWS_WRITE_DAC } else { 0 },
            )
            // Cooperating readers/writers remain compatible, but no ancestor can be renamed
            // while a creation transaction retains its directory anchors.
            .share_mode(if pin {
                WINDOWS_FILE_SHARE_ALL & !FILE_SHARE_DELETE
            } else {
                WINDOWS_FILE_SHARE_ALL
            })
            .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT | WINDOWS_FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
    }

    fn create_private_directory(path: &Path) -> Result<bool> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
        use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;

        let security = crate::private_fs::WindowsOwnerOnlySecurityDescriptor::for_current_user()?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: security.as_raw(),
            bInheritHandle: 0,
        };
        let wide = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        // SAFETY: the NUL-terminated path and security descriptor remain live for this call.
        if unsafe { CreateDirectoryW(wide.as_ptr(), &attributes) } != 0 {
            return Ok(true);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            Ok(false)
        } else {
            Err(error).context("failed to create private SQLite directory")
        }
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

    pub(super) fn open_runtime(_: &Path, _: bool) -> Result<Vec<DirectoryAnchor>> {
        bail!("private SQLite databases are unsupported on this platform")
    }

    pub(super) fn open_database(_: &Path, _: &File) -> Result<Arc<DatabaseAnchor>> {
        bail!("private SQLite databases are unsupported on this platform")
    }

    pub(super) fn release_database(database: Option<Arc<DatabaseAnchor>>) {
        drop(database);
    }

    pub(super) fn open_journal(_: &Path, _: &File) -> Result<Option<File>> {
        bail!("private SQLite databases are unsupported on this platform")
    }

    pub(super) fn verify_directory(_: &Path, _: &File, _: bool) -> Result<()> {
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
