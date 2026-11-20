use super::protocol::nonce;
use crate::private_directory::ensure_private_directory;
use anyhow::{Context, Result, ensure};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const PURPOSE: &str = "issue-human-confirmation";
const MAX_SESSION_BYTES: u64 = 16 * 1024;

pub(crate) fn random_id(bytes: usize) -> Result<String> {
    let mut value = vec![0; bytes];
    rand::rngs::OsRng
        .try_fill_bytes(&mut value)
        .context("secure randomness unavailable")?;
    Ok(value.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionRecord {
    pub version: u32,
    pub purpose: String,
    pub endpoint: String,
    pub registration_id: String,
    pub token: String,
}

pub(super) struct SessionFiles {
    pub directory: PathBuf,
    owned: HashSet<PathBuf>,
}
impl SessionFiles {
    pub fn create(run_dir: &Path) -> Result<Self> {
        ensure_private_directory(run_dir)?;
        let directory = run_dir.join(format!("hc-{}", random_id(16)?));
        std::fs::create_dir(&directory).context("failed to create confirmation directory")?;
        let files = Self {
            directory,
            owned: HashSet::new(),
        };
        ensure_private_directory(&files.directory)?;
        Ok(files)
    }
    pub fn endpoint(&self) -> String {
        endpoint(&self.directory)
    }
    pub fn prepare(
        &mut self,
        endpoint: &str,
        registration_id: &str,
        token: &str,
    ) -> Result<PathBuf> {
        let path = self.directory.join(format!("{registration_id}.json"));
        let mut file = create_private_file(&path)?;
        self.owned.insert(path.clone());
        #[cfg(windows)]
        {
            crate::private_fs::validate_windows_path_identity(&path, &file, false)?;
            crate::private_fs::set_windows_private_acl(&file, false)?;
            crate::private_fs::validate_windows_private_owner_and_acl(&path, &file)?;
        }
        let record = SessionRecord {
            version: 1,
            purpose: PURPOSE.to_owned(),
            endpoint: endpoint.to_owned(),
            registration_id: registration_id.to_owned(),
            token: token.to_owned(),
        };
        file.write_all(&serde_json::to_vec(&record)?)
            .context("failed to write confirmation session")?;
        file.flush()
            .context("failed to flush confirmation session")?;
        Ok(path)
    }
    pub fn remove(&mut self, path: &Path) -> Result<()> {
        if self.owned.contains(path) {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => anyhow::bail!("failed to remove confirmation session"),
            }
            self.owned.remove(path);
        }
        Ok(())
    }
    pub fn cleanup(&mut self) -> Result<()> {
        for path in self.owned.clone() {
            self.remove(&path)?;
        }
        match std::fs::remove_dir(&self.directory) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => anyhow::bail!("failed to remove confirmation directory"),
        }
    }
}
impl Drop for SessionFiles {
    fn drop(&mut self) {
        if self.cleanup().is_err() {
            tracing::warn!("confirmation session cleanup failed");
        }
    }
}

fn endpoint(directory: &Path) -> String {
    #[cfg(unix)]
    {
        directory.join("c.sock").display().to_string()
    }
    #[cfg(windows)]
    {
        format!(
            r"\\.\pipe\akra-{}",
            directory.file_name().unwrap().to_string_lossy()
        )
    }
}

fn create_private_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.create_new(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(windows)]
    {
        use crate::private_fs::*;
        use std::os::windows::fs::OpenOptionsExt;
        options
            .access_mode(
                WINDOWS_GENERIC_READ
                    | WINDOWS_GENERIC_WRITE
                    | WINDOWS_READ_CONTROL
                    | WINDOWS_WRITE_DAC,
            )
            .share_mode(WINDOWS_FILE_SHARE_ALL)
            .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options
        .open(path)
        .context("failed to create private confirmation session")
}

/// Read only an owner-controlled file in this application's private runtime namespace.
/// The caller suppresses these filesystem errors rather than publishing paths or credentials.
pub(crate) fn read_session(
    path: &Path,
    registration_id: &str,
    run_dir: &Path,
) -> Result<SessionRecord> {
    ensure!(
        path.is_absolute() && nonce(registration_id, 16),
        "invalid confirmation registration"
    );
    let directory = path.parent().context("invalid confirmation path")?;
    let parent = directory.parent().context("invalid confirmation path")?;
    let name = directory
        .file_name()
        .and_then(|name| name.to_str())
        .context("invalid confirmation path")?;
    ensure!(
        name.strip_prefix("hc-")
            .is_some_and(|value| nonce(value, 16)),
        "invalid confirmation namespace"
    );
    ensure!(
        path.file_name().and_then(|name| name.to_str())
            == Some(format!("{registration_id}.json").as_str()),
        "invalid confirmation filename"
    );
    ensure!(
        std::fs::canonicalize(parent)? == std::fs::canonicalize(run_dir)?,
        "invalid confirmation location"
    );
    let _run = open_private(parent, true)?;
    let _directory = open_private(directory, true)?;
    let mut file = open_private(path, false)?;
    ensure!(
        file.metadata()?.len() <= MAX_SESSION_BYTES,
        "confirmation session exceeds limit"
    );
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_SESSION_BYTES + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_SESSION_BYTES,
        "confirmation session exceeds limit"
    );
    validate_handle(path, &file, false)?;
    let record: SessionRecord =
        serde_json::from_slice(&bytes).context("invalid confirmation session")?;
    ensure!(
        record.version == 1
            && record.purpose == PURPOSE
            && record.registration_id == registration_id
            && nonce(&record.token, 32)
            && record.endpoint == endpoint(directory),
        "invalid confirmation session"
    );
    Ok(record)
}

fn open_private(path: &Path, directory: bool) -> Result<File> {
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        !metadata.file_type().is_symlink()
            && metadata.is_dir() == directory
            && (directory || metadata.is_file()),
        "unsafe confirmation path"
    );
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    #[cfg(windows)]
    {
        use crate::private_fs::*;
        use std::os::windows::fs::OpenOptionsExt;
        options
            .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
            .share_mode(WINDOWS_FILE_SHARE_ALL)
            .custom_flags(
                WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT
                    | if directory {
                        WINDOWS_FILE_FLAG_BACKUP_SEMANTICS
                    } else {
                        0
                    },
            );
    }
    let file = options.open(path)?;
    validate_handle(path, &file, directory)?;
    Ok(file)
}
fn validate_handle(path: &Path, file: &File, directory: bool) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let held = file.metadata()?;
        let current = std::fs::symlink_metadata(path)?;
        // SAFETY: geteuid has no preconditions and does not retain pointers.
        let owner = unsafe { libc::geteuid() };
        ensure!(
            held.uid() == owner
                && held.mode() & 0o777 == if directory { 0o700 } else { 0o600 }
                && held.is_dir() == directory
                && (directory || held.is_file())
                && held.dev() == current.dev()
                && held.ino() == current.ino()
                && !current.file_type().is_symlink()
                && (directory || held.nlink() == 1),
            "unsafe confirmation file identity or permissions"
        );
    }
    #[cfg(windows)]
    {
        crate::private_fs::validate_windows_path_identity(path, file, directory)?;
        crate::private_fs::validate_windows_private_owner_and_acl(path, file)?;
        if !directory {
            ensure!(
                crate::private_fs::windows_file_link_count(file)? == 1,
                "unsafe confirmation links"
            );
        }
    }
    Ok(())
}
