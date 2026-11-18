use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rand::RngCore;
use serde_json::json;

use crate::private_directory::ensure_private_directory;

pub(super) fn random_id(bytes: usize) -> Result<String> {
    let mut value = vec![0; bytes];
    rand::rngs::OsRng
        .try_fill_bytes(&mut value)
        .context("secure randomness unavailable")?;
    Ok(value.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(super) struct SessionFiles {
    pub directory: PathBuf,
    owned: HashSet<PathBuf>,
}

impl SessionFiles {
    pub fn create(run_dir: &Path) -> Result<Self> {
        ensure_private_directory(run_dir)?;
        let directory = run_dir.join(format!("browser-{}", random_id(16)?));
        // create_dir, not create_dir_all: never claim an existing instance directory.
        std::fs::create_dir(&directory).context("failed to create browser instance directory")?;
        let files = Self {
            directory,
            owned: HashSet::new(),
        };
        ensure_private_directory(&files.directory)?;
        Ok(files)
    }

    pub fn endpoint(&self) -> String {
        #[cfg(unix)]
        {
            self.directory.join("b.sock").display().to_string()
        }
        #[cfg(windows)]
        {
            format!(
                r"\\.\pipe\akra-{}",
                self.directory.file_name().unwrap().to_string_lossy()
            )
        }
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
        let data = serde_json::to_vec(&json!({
            "version":1, "endpoint":endpoint, "registrationId":registration_id, "token":token
        }))?;
        file.write_all(&data)
            .context("failed to write private browser session")?;
        file.flush()
            .context("failed to flush private browser session")?;
        Ok(path)
    }

    pub fn remove(&mut self, path: &Path) -> Result<()> {
        if self.owned.contains(path) {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).context("failed to remove private browser session");
                }
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
            Err(error) => Err(error).context("failed to remove browser instance directory"),
        }
    }
}

impl Drop for SessionFiles {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            tracing::warn!(%error, "browser session cleanup failed");
        }
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
    // The directory already has an owner-only ACL. No secret is written until
    // the file has its own protected ACL and its identity has been verified.
    options
        .open(path)
        .context("failed to create private browser session")
}
