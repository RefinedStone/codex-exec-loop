use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Component, Path, PathBuf, Prefix};

use anyhow::{Context, Result, bail};
use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

use crate::private_fs::{
    WINDOWS_FILE_FLAG_BACKUP_SEMANTICS, WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT, WINDOWS_GENERIC_READ,
    WINDOWS_READ_CONTROL, validate_windows_identity_snapshot, validate_windows_path_identity_only,
    validate_windows_trusted_executable_acl, windows_file_identity_snapshot,
};
use crate::private_sqlite::PinnedPrivateDirectory;

/// This reader is intentionally scoped to issue templates. Enabling the Windows
/// secure_fs reader would also enable direct planning/Vault authority reads.
pub(super) fn read_template(
    root: &Path,
    relative: &Path,
    max_bytes: usize,
) -> Result<Option<String>> {
    if max_bytes == 0 || max_bytes > super::MAX_TEMPLATE_BYTES {
        bail!(
            "issue template byte limit must be between 1 and {}",
            super::MAX_TEMPLATE_BYTES
        );
    }
    validate_root(root)?;
    let components = relative_components(relative)?;
    reject_private_runtime(&components, true)?;
    let (name, parents) = components
        .split_last()
        .context("issue template path must not be empty")?;

    // The selected workspace must exist. Only a missing component beneath a
    // successfully pinned root may mean that the optional template is absent.
    let mut parent = PinnedPrivateDirectory::pin_ancestors(root)
        .context("failed to pin the issue template workspace root")?;
    let mut directories = vec![TrustedDirectory::open(root)?];
    verify_scope(&parent, &directories)?;
    let canonical_root = std::fs::canonicalize(root)
        .context("failed to resolve the pinned issue template workspace")?;
    verify_scope(&parent, &directories)?;

    for component in parents {
        verify_scope(&parent, &directories)?;
        let path = parent.file_path(component)?;
        let next = match PinnedPrivateDirectory::pin_ancestors(&path) {
            Ok(next) => next,
            Err(error) => return confirm_absence(error, &path, &parent, &directories),
        };
        // Walk one component at a time. A dangling junction must be rejected
        // before an absent descendant can be mistaken for a missing template.
        directories.push(TrustedDirectory::open(&path)?);
        verify_scope(&parent, &directories)?;
        parent = next;
        reject_canonical_private_runtime(&canonical_root, parent.path())?;
        verify_scope(&parent, &directories)?;
    }

    verify_scope(&parent, &directories)?;
    let path = parent.file_path(name)?;
    let mut file = match OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) => {
            return confirm_absence(error.into(), &path, &parent, &directories);
        }
    };
    verify_file(&path, &file)?;
    let identity = windows_file_identity_snapshot(&file)?;
    let expected_size = file.metadata()?.len();
    if expected_size > max_bytes as u64 {
        bail!("issue template exceeds the {max_bytes} byte limit");
    }
    #[cfg(test)]
    run_hook(HookStage::FileOpened);

    let mut bytes = Vec::with_capacity(expected_size as usize);
    (&mut file)
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .context("failed to read issue template")?;
    if bytes.len() > max_bytes || bytes.len() as u64 != expected_size {
        bail!("issue template changed size or exceeded its byte limit while being read");
    }
    #[cfg(test)]
    run_hook(HookStage::BytesRead);

    // Retain both the leaf and every parent pin until all checks finish.
    // FILE_SHARE_READ refuses concurrent write/delete handles; metadata and ACL
    // revalidation also catch changes that do not require a data-write handle.
    verify_scope(&parent, &directories)?;
    validate_windows_identity_snapshot(identity, &file, false)
        .context("issue template changed while being read")?;
    verify_file(&path, &file)?;
    verify_scope(&parent, &directories)?;
    String::from_utf8(bytes)
        .map(Some)
        .context("issue template is not valid UTF-8")
}

struct TrustedDirectory {
    path: PathBuf,
    handle: File,
}

impl TrustedDirectory {
    fn open(path: &Path) -> Result<Self> {
        let handle = OpenOptions::new()
            .read(true)
            .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT | WINDOWS_FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .context("failed to open issue template directory")?;
        let directory = Self {
            path: path.to_path_buf(),
            handle,
        };
        directory.verify()?;
        Ok(directory)
    }

    fn verify(&self) -> Result<()> {
        validate_windows_path_identity_only(&self.path, &self.handle, true)?;
        validate_template_acl(&self.path, &self.handle, true)
    }
}

fn verify_scope(parent: &PinnedPrivateDirectory, directories: &[TrustedDirectory]) -> Result<()> {
    parent.verify()?;
    for directory in directories {
        directory.verify()?;
    }
    parent.verify()
}

fn verify_file(path: &Path, file: &File) -> Result<()> {
    if !file.metadata()?.is_file() {
        bail!("issue template must be a regular file");
    }
    validate_windows_path_identity_only(path, file, false)?;
    validate_template_acl(path, file, false)
}

fn validate_template_acl(path: &Path, file: &File, directory: bool) -> Result<()> {
    // Reuse the established Windows trust policy, not executable lookup. Files
    // reject untrusted writes; directories reject untrusted replacement, deletion,
    // and ACL changes. Inherited read access remains permitted. Trust includes the
    // current user and the existing Windows system principals, and is checked
    // without changing ownership or permissions.
    validate_windows_trusted_executable_acl(path, file, directory)
        .context("issue template path permits untrusted mutation or has an untrusted owner")
}

fn confirm_absence(
    error: anyhow::Error,
    path: &Path,
    parent: &PinnedPrivateDirectory,
    directories: &[TrustedDirectory],
) -> Result<Option<String>> {
    if !error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    }) {
        return Err(error).context("failed to open issue template path");
    }

    #[cfg(test)]
    run_hook(HookStage::MissingObserved);
    verify_scope(parent, directories)?;
    match std::fs::symlink_metadata(path) {
        Err(missing) if missing.kind() == std::io::ErrorKind::NotFound => {
            verify_scope(parent, directories)?;
            Ok(None)
        }
        Ok(_) => Err(error).context("issue template path changed while checking its absence"),
        Err(current) => Err(current).context("failed to confirm issue template path absence"),
    }
}

fn reject_canonical_private_runtime(canonical_root: &Path, directory: &Path) -> Result<()> {
    // Resolve only after no-reparse pins are held. This is an exclusion check,
    // never an alternate path used to open data. Normalized final paths expand
    // DOS short names that could otherwise alias the private runtime namespace.
    let canonical = std::fs::canonicalize(directory)
        .context("failed to resolve the pinned issue template directory")?;
    let relative = canonical
        .strip_prefix(canonical_root)
        .context("issue template directory is outside the selected workspace")?;
    let components = relative_components(relative)?;
    reject_private_runtime(&components, false)
}

fn reject_private_runtime(components: &[&str], reject_ancestors: bool) -> Result<()> {
    const RUNTIME: [&str; 2] = [".codex-exec-loop", "runtime"];
    let is_prefix = components
        .iter()
        .zip(RUNTIME)
        .all(|(component, expected)| component.eq_ignore_ascii_case(expected));
    if is_prefix && (reject_ancestors || components.len() >= RUNTIME.len()) {
        bail!("the private planning runtime store is not an issue template path");
    }
    Ok(())
}

fn validate_root(root: &Path) -> Result<()> {
    let mut components = root.components();
    if !matches!(
        components.next(),
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
    ) || !matches!(components.next(), Some(Component::RootDir))
    {
        bail!("issue template workspace must be an absolute local drive path");
    }
    for component in components {
        let Component::Normal(name) = component else {
            bail!("issue template workspace must not contain path aliases");
        };
        validate_component(
            name.to_str()
                .context("issue template workspace is not Unicode")?,
        )?;
    }
    Ok(())
}

fn relative_components(relative: &Path) -> Result<Vec<&str>> {
    let text = relative
        .to_str()
        .context("issue template path must be Unicode")?;
    let mut components = Vec::new();
    for component in text.split(['/', '\\']) {
        validate_component(component)?;
        components.push(component);
    }
    Ok(components)
}

fn validate_component(name: &str) -> Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|character| character.is_control() || "<>:\"|?*/\\".contains(character))
    {
        bail!("issue template path contains a Windows path alias");
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    let device_number = stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"));
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || device_number.is_some_and(|number| {
        matches!(
            number,
            "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
        )
    }) {
        bail!("issue template path must not name a Windows device");
    }
    Ok(())
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum HookStage {
    FileOpened,
    BytesRead,
    MissingObserved,
}

#[cfg(test)]
type ReadHook = (HookStage, Box<dyn FnOnce()>);

#[cfg(test)]
thread_local! {
    static READ_HOOK: std::cell::RefCell<Option<ReadHook>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn run_hook(stage: HookStage) {
    let hook = READ_HOOK.with(|slot| {
        if slot
            .borrow()
            .as_ref()
            .is_some_and(|(expected, _)| *expected == stage)
        {
            slot.borrow_mut().take().map(|(_, hook)| hook)
        } else {
            None
        }
    });
    if let Some(hook) = hook {
        hook();
    }
}

#[cfg(test)]
mod tests;
