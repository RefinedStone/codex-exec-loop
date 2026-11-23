use std::path::Path;

use anyhow::Result;

#[path = "windows/directory.rs"]
mod directory;
#[path = "windows/files.rs"]
mod files;
#[path = "windows/lease.rs"]
mod lease;
#[cfg(test)]
#[path = "windows/test_hooks.rs"]
mod test_hooks;
#[cfg(test)]
#[path = "windows/tests.rs"]
mod tests;
#[path = "windows/write.rs"]
mod write;

pub(super) use lease::BindingLease;

pub(super) fn prepare_writer_root(root: &Path) -> Result<()> {
    directory::prepare_writer_root(root)
}

pub(super) fn read_optional_file(root: &Path, relative: &Path) -> Result<Option<String>> {
    let name = super::record_name(relative)?;
    let Some(root) = directory::open_root(root)? else {
        return Ok(None);
    };
    let Some(records) = directory::open_records(&root, false)? else {
        return Ok(None);
    };
    let record = files::read_snapshot(&records, name)?;
    root.verify()?;
    Ok(record.map(|record| record.body))
}

pub(super) fn write_file_atomic(root: &Path, relative: &Path, body: &[u8]) -> Result<()> {
    write::write_file_atomic(root, super::record_name(relative)?, body)
}

pub(super) fn compare_and_swap_file(
    root: &Path,
    relative: &Path,
    observed: Option<&str>,
    replacement: &str,
) -> Result<bool> {
    write::compare_and_swap_file(root, super::record_name(relative)?, observed, replacement)
}

pub(super) fn write_outcome_unknown(error: &anyhow::Error) -> bool {
    error.downcast_ref::<write::WriteOutcomeUnknown>().is_some()
}

#[cfg(test)]
pub(super) use test_hooks::{
    install_after_atomic_replace_error_hook, install_before_atomic_replace_hook,
};
