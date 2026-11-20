//! Atomic registry installation and the distinction between an unapplied write and a
//! replacement whose follow-up failed. Callers must reconcile an installed registry
//! even when they still need to report an error to the operator.

#[cfg(unix)]
use std::fs::File;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;

use anyhow::{Context, Result};
use toml_edit::DocumentMut;

use super::CONFIG_FILE;

/// Typed anyhow context survives additional error context across the local and
/// Gateway-routed backends without changing their public Result contract.
#[derive(Debug)]
pub(super) enum RegistryChangeFailure {
    DurabilityUncertain,
    FollowUp,
}

impl std::fmt::Display for RegistryChangeFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DurabilityUncertain => formatter.write_str(
                "MCP registry was replaced, but durability after a crash is uncertain; re-read the current registry before retrying",
            ),
            Self::FollowUp => formatter.write_str(
                "MCP registry was replaced, but follow-up failed; re-read the current registry before retrying",
            ),
        }
    }
}

pub(super) fn was_replaced(error: &anyhow::Error) -> bool {
    error.is::<RegistryChangeFailure>()
}

pub(super) fn sync_directory(home: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(home)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = home;
        Ok(())
    }
}

/// The injected directory sync is the final operation after the real rename. A
/// failure before that point is an ordinary error and never claims installation.
pub(super) fn write_document(
    home: &Path,
    document: &DocumentMut,
    sync_directory: fn(&Path) -> io::Result<()>,
) -> Result<()> {
    crate::private_directory::ensure_private_directory(home)?;
    let path = home.join(CONFIG_FILE);
    let mut temporary = None;
    for _ in 0..32 {
        let candidate = home.join(format!(
            ".{CONFIG_FILE}.{}.{}.tmp",
            std::process::id(),
            rand::random::<u64>()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&candidate) {
            Ok(file) => {
                temporary = Some((candidate, file));
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to create `{}`", candidate.display()));
            }
        }
    }
    let (temporary_path, mut temporary_file) =
        temporary.context("failed to allocate an MCP registry temporary file")?;
    let write_result = temporary_file
        .write_all(document.to_string().as_bytes())
        .and_then(|()| temporary_file.sync_all());
    drop(temporary_file);
    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(error)
            .with_context(|| format!("failed to write `{}`", temporary_path.display()));
    }
    if let Err(error) = std::fs::rename(&temporary_path, &path) {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(error).with_context(|| format!("failed to replace `{}`", path.display()));
    }
    sync_directory(home)
        .with_context(|| format!("failed to sync MCP registry directory `{}`", home.display()))
        .context(RegistryChangeFailure::DurabilityUncertain)
}
