use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, bail};

pub(super) fn prepare_writer_root(_: &Path) -> Result<()> {
    bail!("private vault files are unavailable on this platform")
}

pub(super) fn read_optional_file(_: &Path, _: &Path) -> Result<Option<String>> {
    bail!("private vault files are unavailable on this platform")
}

pub(super) fn write_file_atomic(_: &Path, _: &Path, _: &[u8]) -> Result<()> {
    bail!("private vault files are unavailable on this platform")
}

pub(super) fn compare_and_swap_file(_: &Path, _: &Path, _: Option<&str>, _: &str) -> Result<bool> {
    bail!("private vault files are unavailable on this platform")
}

pub(super) fn remove_observed_file(_: &Path, _: &Path, _: &str) -> Result<bool> {
    anyhow::bail!("private vault record removal is unavailable on this platform")
}

pub(super) fn write_outcome_unknown(_: &anyhow::Error) -> bool {
    false
}

pub(super) struct BindingLease;

impl BindingLease {
    pub(super) fn acquire(_: PathBuf, _: String, _: Duration) -> Result<Self> {
        bail!("private vault leases are unavailable on this platform")
    }

    pub(super) fn verify_identity(&self) -> Result<()> {
        bail!("private vault leases are unavailable on this platform")
    }
}
