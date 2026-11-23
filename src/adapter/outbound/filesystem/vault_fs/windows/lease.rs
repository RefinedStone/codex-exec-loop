use std::fs::{File, TryLockError};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::private_fs::{
    WindowsFileIdentitySnapshot, validate_windows_identity_snapshot, windows_file_identity_snapshot,
};
use crate::private_sqlite::PinnedPrivateDirectory;

use super::directory;
use super::files::{self, NewFilePurpose};

const RETRY_INTERVAL: Duration = Duration::from_millis(25);
const MUTATION_TIMEOUT: Duration = Duration::from_secs(30);
const MUTATION_LOCK_NAME: &str = "evidence-vault-mutation.lock";

pub(crate) struct BindingLease {
    lease: PrivateLease,
}

impl BindingLease {
    pub(crate) fn acquire(root: PathBuf, name: String, timeout: Duration) -> Result<Self> {
        let directory =
            directory::open_root(&root)?.context("the vault writer root has not been prepared")?;
        Ok(Self {
            lease: PrivateLease::acquire(directory, name, timeout)?,
        })
    }

    pub(crate) fn verify_identity(&self) -> Result<()> {
        self.lease.verify_identity()
    }
}

pub(super) struct PrivateLease {
    directory: PinnedPrivateDirectory,
    name: String,
    file: File,
    identity: WindowsFileIdentitySnapshot,
}

impl PrivateLease {
    pub(super) fn mutation(directory: PinnedPrivateDirectory) -> Result<Self> {
        Self::acquire(directory, MUTATION_LOCK_NAME.into(), MUTATION_TIMEOUT)
    }

    #[cfg(test)]
    pub(super) fn mutation_with_timeout(
        directory: PinnedPrivateDirectory,
        timeout: Duration,
    ) -> Result<Self> {
        Self::acquire(directory, MUTATION_LOCK_NAME.into(), timeout)
    }

    fn acquire(directory: PinnedPrivateDirectory, name: String, timeout: Duration) -> Result<Self> {
        directory.verify()?;
        let file = match files::create_private_file(&directory, &name, NewFilePurpose::Lease) {
            Ok(file) => {
                file.sync_all()
                    .context("the vault lease could not be flushed")?;
                file
            }
            Err(error) if files::is_already_exists(&error) => {
                files::open_existing_lease(&directory, &name)?
            }
            Err(error) => return Err(error),
        };
        if file.metadata()?.len() != 0 {
            bail!("the vault lease must be an empty private regular file");
        }
        let identity = windows_file_identity_snapshot(&file)?;
        let started = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) => {
                    if started.elapsed() >= timeout {
                        bail!("another vault transaction is still running");
                    }
                    std::thread::sleep(
                        RETRY_INTERVAL.min(timeout.saturating_sub(started.elapsed())),
                    );
                }
                Err(TryLockError::Error(error)) => {
                    return Err(error).context("the private vault lease could not be acquired");
                }
            }
        }
        let lease = Self {
            directory,
            name,
            file,
            identity,
        };
        lease.verify_identity()?;
        Ok(lease)
    }

    pub(super) fn directory(&self) -> &PinnedPrivateDirectory {
        &self.directory
    }

    pub(super) fn verify_identity(&self) -> Result<()> {
        files::verify_named_file(&self.directory, &self.name, &self.file)?;
        validate_windows_identity_snapshot(self.identity, &self.file, false)
    }
}

impl Drop for PrivateLease {
    fn drop(&mut self) {
        // The empty lock file stays in place. Removing it would split the lock domain.
        if let Err(error) = self.file.unlock() {
            tracing::warn!(error = %error, "private vault lease release failed");
        }
    }
}
