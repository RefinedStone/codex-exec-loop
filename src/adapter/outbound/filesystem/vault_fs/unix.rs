use std::path::Path;

use anyhow::Result;

use super::super::secure_fs;

pub(super) use super::super::private_runtime_lease::PrivateRuntimeLease as BindingLease;

pub(super) fn prepare_writer_root(root: &Path) -> Result<()> {
    crate::private_directory::ensure_private_directory(root)
}

pub(super) fn read_optional_file(root: &Path, relative: &Path) -> Result<Option<String>> {
    match std::fs::symlink_metadata(root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
        Ok(_) => secure_fs::read_optional_file(root, relative),
    }
}

pub(super) fn write_file_atomic(root: &Path, relative: &Path, body: &[u8]) -> Result<()> {
    secure_fs::write_file_atomic(root, relative, body)
}

pub(super) fn compare_and_swap_file(
    root: &Path,
    relative: &Path,
    observed: Option<&str>,
    replacement: &str,
) -> Result<bool> {
    secure_fs::compare_and_swap_optional_file(root, relative, observed, Some(replacement))
}

pub(super) fn write_outcome_unknown(error: &anyhow::Error) -> bool {
    secure_fs::atomic_replacement_completed(error)
}

#[cfg(test)]
pub(super) use secure_fs::{
    install_after_atomic_replace_error_hook, install_before_atomic_replace_hook,
};
