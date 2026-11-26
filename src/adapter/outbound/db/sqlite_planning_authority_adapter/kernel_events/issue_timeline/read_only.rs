//! Timeline 조회는 store, marker, schema, 권한을 만들거나 갱신하지 않는다.
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags};

use super::super::super as authority;
use crate::domain::planning::PlanningAuthorityLocation;

pub(in crate::adapter::outbound::db::sqlite_planning_authority_adapter::kernel_events) struct ReadOnlyStore
{
    pub connection: Connection,
    // SQLite를 먼저 닫은 뒤 파일과 디렉터리 identity anchor를 해제한다.
    _anchors: Vec<File>,
}

pub(in crate::adapter::outbound::db::sqlite_planning_authority_adapter::kernel_events) fn open(
    location: &PlanningAuthorityLocation,
) -> Result<Option<ReadOnlyStore>> {
    let path = Path::new(&location.authority_store_path);
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("failed to inspect issue timeline store"),
    }
    let parent = path
        .parent()
        .context("issue timeline store has no parent")?;
    let managed_root = parent
        .ancestors()
        .nth(2)
        .context("issue timeline store layout is invalid")?;
    let storage_root = managed_root
        .parent()
        .context("issue timeline storage root is missing")?;
    #[cfg(unix)]
    authority::validate_authority_storage_ancestor_chain(storage_root)?;
    let mut anchors = vec![authority::open_and_validate_authority_storage_root(
        storage_root,
    )?];
    anchors.push(open_private_handle(managed_root, true)?);
    let mut current = managed_root.to_path_buf();
    for component in parent.strip_prefix(managed_root)?.components() {
        current.push(component.as_os_str());
        anchors.push(open_private_handle(&current, true)?);
    }

    let mut store = open_private_handle(path, false)?;
    let mut header = [0_u8; 20];
    if store.metadata()?.len() == 0 {
        return Ok(None);
    }
    store
        .read_exact(&mut header)
        .context("failed to read issue timeline store header")?;
    if &header[..16] != b"SQLite format 3\0" || header[18] != 1 || header[19] != 1 {
        // Authority stores use DELETE journals. A WAL read may write shared memory even with mode=ro.
        bail!("issue timeline requires an existing rollback-journal authority store");
    }
    anchor_sidecars(path, &mut anchors)?;
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .context("failed to open issue timeline store for reading")?;
    connection.busy_timeout(authority::AUTHORITY_STORE_BUSY_TIMEOUT)?;
    let journal_mode: String =
        connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
    if !journal_mode.eq_ignore_ascii_case("delete") {
        bail!("issue timeline store journal mode changed");
    }
    authority::validate_authority_store_path_identity(path, &store)?;
    authority::validate_authority_store_schema(&connection, location)?;
    anchor_sidecars(path, &mut anchors)?;
    anchors.push(store);
    Ok(Some(ReadOnlyStore {
        connection,
        _anchors: anchors,
    }))
}

fn anchor_sidecars(path: &Path, anchors: &mut Vec<File>) -> Result<()> {
    for suffix in authority::AUTHORITY_STORE_SIDECAR_SUFFIXES {
        let sidecar = authority::authority_store_sidecar_path(path, suffix);
        match open_private_handle(&sidecar, false) {
            Ok(anchor) => anchors.push(anchor),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn open_private_handle(path: &Path, directory: bool) -> Result<File> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let mut options = OpenOptions::new();
    options.read(true).custom_flags(
        libc::O_CLOEXEC | libc::O_NOFOLLOW | if directory { libc::O_DIRECTORY } else { 0 },
    );
    let file = options
        .open(path)
        .with_context(|| format!("failed to open {} for reading", path.display()))?;
    let metadata = file.metadata()?;
    authority::validate_path_identity(path, &metadata, directory)?;
    if (!directory && (!metadata.is_file() || metadata.nlink() != 1))
        || (directory && !metadata.is_dir())
        || metadata.mode() & 0o777 != if directory { 0o700 } else { 0o600 }
    {
        bail!("issue timeline path is not private: {}", path.display());
    }
    Ok(file)
}

#[cfg(windows)]
fn open_private_handle(path: &Path, directory: bool) -> Result<File> {
    use crate::private_fs::{
        WINDOWS_FILE_FLAG_BACKUP_SEMANTICS, WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
        WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ, WINDOWS_READ_CONTROL,
        validate_windows_path_identity, validate_windows_private_owner_and_acl,
    };
    use std::os::windows::fs::OpenOptionsExt;

    let file = OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
        .share_mode(WINDOWS_FILE_SHARE_ALL)
        .custom_flags(
            WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT
                | if directory {
                    WINDOWS_FILE_FLAG_BACKUP_SEMANTICS
                } else {
                    0
                },
        )
        .open(path)
        .with_context(|| format!("failed to open {} for reading", path.display()))?;
    validate_windows_path_identity(path, &file, directory)?;
    validate_windows_private_owner_and_acl(path, &file)?;
    Ok(file)
}

#[cfg(not(any(unix, windows)))]
fn open_private_handle(path: &Path, _directory: bool) -> Result<File> {
    bail!(
        "read-only authority-store access is unsupported: {}",
        path.display()
    )
}
