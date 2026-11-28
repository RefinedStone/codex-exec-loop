//! The narrow private filesystem boundary used by encrypted Evidence Vault records.
//! It does not enable direct planning filesystem operations on Windows.
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};

#[cfg(all(test, any(unix, windows)))]
pub(crate) struct DisposableVault(pub(crate) PathBuf);

#[cfg(all(test, any(unix, windows)))]
impl DisposableVault {
    pub(crate) fn new() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-disposable-vault-{:032x}",
            rand::random::<u128>()
        ));
        prepare_writer_root(&root).unwrap();
        Self(std::fs::canonicalize(root).unwrap())
    }
}

#[cfg(all(test, any(unix, windows)))]
impl Drop for DisposableVault {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            eprintln!("disposable Vault fixture cleanup failed: {error}");
        }
    }
}

#[cfg(unix)]
#[path = "vault_fs/unix.rs"]
mod platform;
#[cfg(windows)]
#[path = "vault_fs/windows.rs"]
mod platform;
#[cfg(not(any(unix, windows)))]
#[path = "vault_fs/unsupported.rs"]
mod platform;

pub(crate) const MAX_FILE_BYTES: usize = super::secure_fs::MAX_FILE_BYTES;

/// Prepare a writer-owned root. Windows verifies the local NTFS boundary before
/// creating any directory and never repairs the ACL of an existing object.
pub(crate) fn prepare_writer_root(root: &Path) -> Result<()> {
    platform::prepare_writer_root(root)
}

/// Missing roots, record directories and records remain absent after a read.
pub(crate) fn read_optional_file(root: &Path, relative: &Path) -> Result<Option<String>> {
    record_name(relative)?;
    platform::read_optional_file(root, relative)
}

pub(crate) fn write_file_atomic(root: &Path, relative: &Path, body: &[u8]) -> Result<()> {
    record_name(relative)?;
    validate_size(body.len())?;
    platform::write_file_atomic(root, relative, body)
}

/// Vault generations and pointers are always installed, never deleted by CAS.
pub(crate) fn compare_and_swap_file(
    root: &Path,
    relative: &Path,
    observed: Option<&str>,
    replacement: &str,
) -> Result<bool> {
    record_name(relative)?;
    validate_size(replacement.len())?;
    platform::compare_and_swap_file(root, relative, observed, replacement)
}

/// Remove only the observed private record. A stale observation performs no mutation.
pub(crate) fn remove_observed_file(root: &Path, relative: &Path, observed: &str) -> Result<bool> {
    record_name(relative)?;
    platform::remove_observed_file(root, relative, observed)
}

/// A destination may have changed even if later flushing or verification failed.
pub(crate) fn write_outcome_unknown(error: &anyhow::Error) -> bool {
    platform::write_outcome_unknown(error)
}

pub(crate) struct VaultBindingLease {
    inner: platform::BindingLease,
}

impl VaultBindingLease {
    pub(crate) fn acquire(
        root: PathBuf,
        lock_name: impl Into<String>,
        timeout: Duration,
    ) -> Result<Self> {
        let lock_name = lock_name.into();
        let digest = lock_name
            .strip_prefix("evidence-vault-binding-")
            .and_then(|value| value.strip_suffix(".lock"))
            .context("the vault binding lease name is invalid")?;
        validate_digest(digest)?;
        Ok(Self {
            inner: platform::BindingLease::acquire(root, lock_name, timeout)?,
        })
    }

    pub(crate) fn verify_identity(&self) -> Result<()> {
        self.inner.verify_identity()
    }
}

fn validate_size(size: usize) -> Result<()> {
    if size > MAX_FILE_BYTES {
        bail!("the encrypted vault record exceeds its byte limit");
    }
    Ok(())
}

fn validate_digest(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("the vault record or lease name must contain a SHA-256 digest");
    }
    Ok(())
}

fn record_name(relative: &Path) -> Result<&str> {
    let text = relative
        .to_str()
        .context("the vault record path is not Unicode")?;
    let name = text.strip_prefix("records/");
    #[cfg(windows)]
    let name = name.or_else(|| text.strip_prefix("records\\"));
    let name = name.context("the vault record must be directly inside records")?;
    let digest = name
        .strip_suffix(".json")
        .context("the vault record name is invalid")?;
    validate_digest(digest)?;
    Ok(name)
}

#[cfg(all(test, any(unix, windows)))]
pub(crate) fn install_before_atomic_replace_hook(hook: impl FnOnce() + 'static) {
    platform::install_before_atomic_replace_hook(hook);
}

#[cfg(all(test, any(unix, windows)))]
pub(crate) fn install_after_atomic_replace_error_hook(message: impl Into<String>) {
    platform::install_after_atomic_replace_error_hook(message);
}

#[cfg(test)]
mod tests;
