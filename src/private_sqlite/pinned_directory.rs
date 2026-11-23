//! Reuse the SQLite boundary's directory traversal without opening a database.
//! Callers retain every ancestor handle while operating on host-owned private files.

use super::{DirectoryAnchor, platform, validate_runtime_path};
#[cfg(unix)]
use anyhow::Context;
use anyhow::{Result, bail};
use std::fs::File;
use std::path::{Component, Path, PathBuf};

pub(crate) struct PinnedPrivateDirectory {
    directories: Vec<DirectoryAnchor>,
}

impl PinnedPrivateDirectory {
    pub(crate) fn open(path: &Path, create: bool) -> Result<Self> {
        Self::open_with_privacy(path, create, true)
    }

    /// Pin an existing Git metadata/temporary parent chain without requiring its
    /// final directory to have private mode. New children still require privacy.
    pub(crate) fn pin_ancestors(path: &Path) -> Result<Self> {
        Self::open_with_privacy(path, false, false)
    }

    fn open_with_privacy(path: &Path, create: bool, private_final: bool) -> Result<Self> {
        validate_runtime_path(path)?;
        let mut directories = platform::open_runtime_with_privacy(path, create, private_final)?;
        // SQLite readers allow rename sharing. Artifact capture must also pin ancestors
        // during read-only verification, not only when creating a new directory.
        #[cfg(windows)]
        for anchor in &mut directories {
            let pinned = platform::open_directory(&anchor.path, true, false)?;
            // The newly pinned path cannot now be renamed. Confirm that it is still
            // the first handle's object before replacing that temporary reader handle.
            crate::private_fs::validate_windows_path_identity_only(
                &anchor.path,
                &anchor.file,
                true,
            )?;
            anchor.file = pinned;
        }
        #[cfg(not(windows))]
        let _ = &mut directories;
        let directory = Self { directories };
        directory.verify()?;
        Ok(directory)
    }

    pub(crate) fn child(&self, name: &str, create: bool) -> Result<Self> {
        validate_name(name)?;
        self.verify()?;
        let mut child = Self::open(&self.path().join(name), create)?;
        // Keep the privacy requirement of every previously pinned component. Otherwise
        // a runtime/evidence ancestor could silently become merely non-world-writable.
        for inherited in &self.directories {
            if let Some(anchor) = child
                .directories
                .iter_mut()
                .find(|anchor| anchor.path == inherited.path)
            {
                anchor.private |= inherited.private;
            }
        }
        self.verify()?;
        child.verify()?;
        Ok(child)
    }

    pub(crate) fn path(&self) -> &Path {
        &self
            .directories
            .last()
            .expect("validated directory anchor")
            .path
    }

    pub(crate) fn handle(&self) -> &File {
        &self
            .directories
            .last()
            .expect("validated directory anchor")
            .file
    }

    pub(crate) fn verify(&self) -> Result<()> {
        for directory in &self.directories {
            directory.verify()?;
        }
        Ok(())
    }

    pub(crate) fn file_path(&self, name: &str) -> Result<PathBuf> {
        validate_name(name)?;
        Ok(self.path().join(name))
    }

    pub(crate) fn sync(&self) -> Result<()> {
        self.verify()?;
        // Windows directory handles do not support FlushFileBuffers. File contents are
        // flushed before publishing; Unix additionally persists the directory entry.
        #[cfg(unix)]
        self.handle()
            .sync_all()
            .context("failed to sync private directory")?;
        self.verify()
    }
}

fn validate_name(name: &str) -> Result<()> {
    let path = Path::new(name);
    if name.is_empty()
        || name.as_bytes().contains(&0)
        || path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
        || name.contains(['/', '\\', ':'])
        || name.ends_with(['.', ' '])
    {
        bail!("private child must be one unambiguous path component");
    }
    // Also applies the existing Windows reserved-name validation before any creation.
    validate_runtime_path(&std::env::current_dir()?.join(name))
}
