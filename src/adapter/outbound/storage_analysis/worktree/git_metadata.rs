//! Git may read only metadata whose paths were already observed without placeholders or links.
use anyhow::{Context, Result, bail, ensure};
use rusqlite::{OptionalExtension, params};
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

use super::super::{index::StorageIndex, native};

pub(super) struct GitMetadata {
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
    pub work_tree: Option<PathBuf>,
    // On Windows these handles prevent replacing the resolved Git directories during commands.
    _guards: Vec<native::NativeMetadata>,
    // Existing config and pointer files stay read-only until the Git subprocesses finish.
    _file_guards: Vec<fs::File>,
}

pub(super) fn key(path: &Path) -> String {
    let normalized = lexical(path);
    let value = normalized.to_string_lossy().into_owned();
    #[cfg(windows)]
    {
        let value = value.replace('/', "\\");
        let value = value.strip_prefix(r"\\?\").unwrap_or(&value);
        if let Some(unc) = value.strip_prefix(r"UNC\") {
            format!(r"\\{unc}").to_lowercase()
        } else {
            value.to_lowercase()
        }
    }
    #[cfg(not(windows))]
    {
        value
    }
}

/// Git for Windows does not accept verbatim drive/UNC spellings in --git-dir.
/// Identity checks and pinned handles keep the original path; only argv is converted.
pub(super) fn git_argument(path: &Path) -> std::ffi::OsString {
    #[cfg(windows)]
    if let Some(value) = path.to_str() {
        if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
            return std::ffi::OsString::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = value.strip_prefix(r"\\?\")
            && rest.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
            && rest.as_bytes().get(1) == Some(&b':')
            && rest.as_bytes().get(2) == Some(&b'\\')
        {
            return std::ffi::OsString::from(rest);
        }
    }
    path.as_os_str().to_owned()
}

fn lexical(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !result.pop() {
                    result.push(component.as_os_str());
                }
            }
            _ => result.push(component.as_os_str()),
        }
    }
    result
}

/// Both lookup and per-common-directory caches remain on the job's own bounded-cache SQLite file.
pub(super) fn prepare(db: &StorageIndex) -> Result<()> {
    let tx = db.conn.unchecked_transaction()?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS git_observed_paths(path_key TEXT PRIMARY KEY,node INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS git_worktree_cache(common_key TEXT PRIMARY KEY,records BLOB);",
    )?;
    {
        let mut rows =
            tx.prepare("SELECT id,path FROM nodes WHERE kind IN ('file','directory') ORDER BY id")?;
        let mut insert = tx.prepare("INSERT INTO git_observed_paths VALUES(?1,?2) ON CONFLICT(path_key) DO UPDATE SET node=-1")?;
        for row in rows.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })? {
            let (id, path) = row?;
            insert.execute(params![key(Path::new(&path)), id])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Runtime roots may have unreadable descendants; validate the observed directory itself.
pub(super) fn runtime_node(db: &StorageIndex, candidate: &Path) -> Result<Option<i64>> {
    let row: Option<(i64,String,Option<String>)> = db.conn.query_row(
        "SELECT n.id,n.path,n.object_key FROM git_observed_paths p JOIN nodes n ON n.id=p.node WHERE p.path_key=?1 AND n.kind='directory'",
        [key(candidate)],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).optional()?;
    let Some((id, path, identity)) = row else {
        return Ok(None);
    };
    // Volume catalog roots use ordinary drive paths; native containment checks use final paths.
    let root = fs::canonicalize(db.path("n1")?)?;
    let metadata = native::metadata(Path::new(&path), &root, None)?;
    ensure!(
        metadata.directory
            && metadata.issue.is_none()
            && identity.is_some()
            && metadata.identity == identity,
        "storage_runtime_path_changed"
    );
    Ok(Some(id))
}
fn observed(
    db: &StorageIndex,
    path: &Path,
    root: &Path,
) -> Result<(PathBuf, native::NativeMetadata)> {
    let record: Option<(String, bool, i64, String)> = db
        .conn
        .query_row(
            "SELECT n.path,n.unknown,n.pending,n.object_key FROM git_observed_paths p
         JOIN nodes n ON n.id=p.node WHERE p.path_key=?1",
            [key(path)],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let (path, unknown, pending, identity) = record.context("storage_git_metadata_unobserved")?;
    ensure!(!unknown && pending == 0, "storage_git_metadata_unverified");
    let path = PathBuf::from(path);
    let metadata = native::metadata(&path, root, None)?;
    ensure!(
        metadata.issue.is_none() && metadata.identity.as_deref() == Some(identity.as_str()),
        "storage_git_metadata_changed"
    );
    Ok((path, metadata))
}

fn optional_file(
    db: &StorageIndex,
    path: &Path,
    root: &Path,
    maximum: usize,
    file_guards: &mut Vec<fs::File>,
) -> Result<Option<Vec<u8>>> {
    // Attribute inspection of an absent optional metadata file does not read its contents.
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    let (path, metadata) = observed(db, path, root)?;
    ensure!(!metadata.directory, "storage_git_metadata_expected_file");
    ensure!(
        metadata.logical <= maximum as u64,
        "storage_git_metadata_too_large"
    );
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_OPEN_NO_RECALL, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
        };
        options
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_NO_RECALL | FILE_FLAG_OPEN_REPARSE_POINT);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(&path)?;
    let mut bytes = Vec::new();
    (&mut file)
        .take((maximum + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= maximum, "storage_git_metadata_too_large");
    file_guards.push(file);
    Ok(Some(bytes))
}

fn pointer(base: &Path, bytes: &[u8], prefix: Option<&str>) -> Result<PathBuf> {
    let text = std::str::from_utf8(bytes)?.trim_end_matches(['\r', '\n']);
    let value = if let Some(prefix) = prefix {
        text.strip_prefix(prefix)
            .context("storage_git_metadata_invalid_pointer")?
    } else {
        text
    };
    ensure!(
        !value.is_empty() && !value.contains(['\0', '\r', '\n']),
        "storage_git_metadata_invalid_pointer"
    );
    let target = Path::new(value);
    Ok(lexical(&if target.is_absolute() {
        target.to_path_buf()
    } else {
        base.join(target)
    }))
}

fn inspect_config(bytes: &[u8]) -> Result<()> {
    let text = std::str::from_utf8(bytes)?;
    for line in text.lines() {
        let line = line.trim_start_matches('\u{feff}').trim_start();
        if let Some(section) = line.strip_prefix('[') {
            let section = section.trim_start().to_ascii_lowercase();
            if section.starts_with("include") {
                bail!("storage_git_external_configuration");
            }
        }
    }
    Ok(())
}

pub(super) fn observe(db: &StorageIndex, path: &Path) -> Result<GitMetadata> {
    let root = fs::canonicalize(db.path("n1")?)?;
    let mut guards = Vec::new();
    let mut file_guards = Vec::new();
    let dot_git = path.join(".git");
    let exists = fs::symlink_metadata(&dot_git)
        .map(|_| true)
        .or_else(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Ok(false)
            } else {
                Err(error)
            }
        })?;
    let work_tree = exists.then(|| path.to_path_buf());
    let git_dir = if exists {
        let (actual, metadata) = observed(db, &dot_git, &root)?;
        let directory = metadata.directory;
        guards.push(metadata);
        if directory {
            actual
        } else {
            let bytes = optional_file(db, &dot_git, &root, 4096, &mut file_guards)?
                .context("storage_git_metadata_unavailable")?;
            pointer(path, &bytes, Some("gitdir: "))?
        }
    } else {
        path.to_path_buf()
    };
    let (git_dir, metadata) = observed(db, &git_dir, &root)?;
    ensure!(
        metadata.directory,
        "storage_git_metadata_expected_directory"
    );
    guards.push(metadata);
    let common_dir = match optional_file(
        db,
        &git_dir.join("commondir"),
        &root,
        4096,
        &mut file_guards,
    )? {
        Some(bytes) => pointer(&git_dir, &bytes, None)?,
        None => git_dir.clone(),
    };
    let (common_dir, metadata) = observed(db, &common_dir, &root)?;
    ensure!(
        metadata.directory,
        "storage_git_metadata_expected_directory"
    );
    guards.push(metadata);
    // These otherwise let Git read configuration or objects outside the verified metadata tree.
    for config in [common_dir.join("config"), git_dir.join("config.worktree")] {
        if let Some(bytes) = optional_file(db, &config, &root, 64 * 1024, &mut file_guards)? {
            inspect_config(&bytes)?;
        }
    }
    for alternate in ["alternates", "http-alternates"] {
        if optional_file(
            db,
            &common_dir.join("objects/info").join(alternate),
            &root,
            4096,
            &mut file_guards,
        )?
        .is_some_and(|bytes| !bytes.is_empty())
        {
            bail!("storage_git_external_objects");
        }
    }
    Ok(GitMetadata {
        git_dir,
        common_dir,
        work_tree,
        _guards: guards,
        _file_guards: file_guards,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_configuration_is_not_read_through_git() {
        assert!(inspect_config(b"[core]\nrepositoryformatversion = 0\n").is_ok());
        assert!(inspect_config(b"[include]\npath = ../outside\n").is_err());
        assert!(inspect_config(b" [ IncludeIf \"gitdir:*\" ]\npath = ../outside\n").is_err());
    }

    #[test]
    fn relative_git_metadata_pointers_are_normalized_without_filesystem_discovery() {
        let base = std::env::temp_dir()
            .join("root")
            .join("worktrees")
            .join("one");
        assert_eq!(
            key(&pointer(&base, b"../..\n", None).unwrap()),
            key(&std::env::temp_dir().join("root"))
        );
        assert!(pointer(&base, b"gitdir: a\nother", Some("gitdir: ")).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn git_argv_preserves_identity_while_normalizing_verbatim_drive_and_unc_paths() {
        assert_eq!(
            git_argument(Path::new(r"\\?\C:\fixture\repository\.git")),
            std::ffi::OsString::from(r"C:\fixture\repository\.git"),
        );
        assert_eq!(
            git_argument(Path::new(r"\\?\UNC\server\share\repository\.git")),
            std::ffi::OsString::from(r"\\server\share\repository\.git"),
        );
        let volume = Path::new(r"\\?\Volume{fixture-volume}\repository\.git");
        assert_eq!(git_argument(volume), volume.as_os_str());
    }
    #[cfg(windows)]
    #[test]
    fn storage_analysis_git_metadata_pins_existing_config_and_pointer_until_commands_finish() {
        use crate::adapter::outbound::storage_analysis::NativeStorageAnalysisAdapter;
        use crate::application::port::outbound::storage_analysis_port::StorageAnalysisPort;
        use crate::domain::storage_analysis::{ScanOptions, ScanRequest};
        use std::sync::atomic::AtomicBool;
        let base = crate::test_utils::platform_safe_temp_dir()
            .join(format!("storage-git-pins-{:032x}", rand::random::<u128>()));
        let source = base.join("source");
        let index = base.join("index");
        fs::create_dir_all(&source).unwrap();
        let mut command = crate::git_subprocess::command(["-C"]);
        let output = command.arg(&source).args(["init", "-q"]).output().unwrap();
        assert!(output.status.success());
        let pointer = source.join(".git/commondir");
        fs::write(&pointer, ".\n").unwrap();
        let adapter = NativeStorageAnalysisAdapter::fixture(index.clone(), source.clone());
        adapter
            .scan(
                &ScanRequest {
                    job_id: "pins".into(),
                    volume_id: "fixture-volume".into(),
                    options: ScanOptions::default(),
                },
                &AtomicBool::new(false),
                &|_| {},
            )
            .unwrap();
        let db = StorageIndex::open(&index.join("pins")).unwrap();
        let guarded = observe(&db, &source).unwrap();
        let config = source.join(".git/config");
        for file in [&config, &pointer] {
            assert!(
                fs::OpenOptions::new().write(true).open(file).is_err(),
                "verified metadata must reject concurrent writers"
            );
            assert!(
                fs::rename(file, file.with_extension("replacement")).is_err(),
                "verified metadata must reject replacement"
            );
        }
        let cancelled = AtomicBool::new(true);
        let error =
            super::super::git_output(&source, &guarded, &["rev-parse", "--git-dir"], &cancelled)
                .unwrap_err();
        assert_eq!(
            error.to_string(),
            "storage_git_cancelled",
            "subsequent Git commands stop at the common spawn boundary"
        );
        drop(guarded);
        assert!(fs::OpenOptions::new().write(true).open(&config).is_ok());
        assert!(fs::OpenOptions::new().write(true).open(&pointer).is_ok());
        drop(db);
        fs::remove_dir_all(&base).unwrap();
    }
}
