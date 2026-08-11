use std::path::{Path, PathBuf};

use anyhow::Result;

pub(crate) const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
#[cfg(unix)]
const MAX_TREE_BYTES: usize = 32 * 1024 * 1024;
#[cfg(unix)]
const MAX_TREE_MEMBERS: usize = 1_024;
#[cfg(unix)]
const MAX_TREE_DEPTH: usize = 32;
#[cfg(unix)]
const MAX_QUARANTINE_ENTRIES: usize = 16;
#[cfg(unix)]
const MAX_QUARANTINE_BYTES: u64 = 256 * 1024 * 1024;
#[cfg(unix)]
const MAX_QUARANTINE_SCAN_MEMBERS: usize = 8_192;

pub(crate) fn ensure_directory(root: &Path, relative: &Path) -> Result<PathBuf> {
    platform::ensure_directory(root, relative)
}

pub(crate) fn read_optional_file(root: &Path, relative: &Path) -> Result<Option<String>> {
    read_optional_file_with_limit(root, relative, MAX_FILE_BYTES)
}

pub(crate) fn read_optional_file_with_limit(
    root: &Path,
    relative: &Path,
    max_file_bytes: usize,
) -> Result<Option<String>> {
    validate_file_size_limit(max_file_bytes)?;
    platform::read_optional_file(root, relative, max_file_bytes)
}

pub(crate) fn read_tree(root: &Path, relative: &Path) -> Result<Vec<(String, String)>> {
    platform::read_tree(root, relative)
}

pub(crate) fn write_file_atomic(root: &Path, relative: &Path, body: &[u8]) -> Result<()> {
    write_file_atomic_with_limit(root, relative, body, MAX_FILE_BYTES)
}

pub(crate) fn write_file_atomic_with_limit(
    root: &Path,
    relative: &Path,
    body: &[u8],
    max_file_bytes: usize,
) -> Result<()> {
    validate_file_size_limit(max_file_bytes)?;
    if body.len() > max_file_bytes {
        anyhow::bail!(
            "workspace file exceeds the {} byte limit: {}",
            max_file_bytes,
            relative.display()
        );
    }
    platform::write_file_atomic(root, relative, body)
}

/// Atomically write with a caller-owned serialization lock while preserving
/// descriptor-anchored anti-alias guarantees.  Configuration owns a separate
/// cross-process lock and must not create this module's planning-runtime lock
/// directory below a reviewable `.akra` directory.
#[cfg(unix)]
pub(crate) fn write_file_atomic_unlocked_with_limit_and_unix_mode(
    root: &Path,
    relative: &Path,
    body: &[u8],
    max_file_bytes: usize,
    unix_mode: u32,
) -> Result<()> {
    validate_file_size_limit(max_file_bytes)?;
    if body.len() > max_file_bytes {
        anyhow::bail!(
            "workspace file exceeds the {} byte limit: {}",
            max_file_bytes,
            relative.display()
        );
    }
    if unix_mode == 0 || unix_mode & !0o777 != 0 {
        anyhow::bail!("workspace file mode must be a nonzero Unix permission mode");
    }
    platform::write_file_atomic_unlocked_with_mode(root, relative, body, unix_mode)
}

pub(crate) fn compare_and_swap_optional_file(
    root: &Path,
    relative: &Path,
    observed: Option<&str>,
    replacement: Option<&str>,
) -> Result<bool> {
    if replacement.is_some_and(|body| body.len() > MAX_FILE_BYTES) {
        anyhow::bail!(
            "workspace file exceeds the {} byte limit: {}",
            MAX_FILE_BYTES,
            relative.display()
        );
    }
    platform::compare_and_swap_optional_file(root, relative, observed, replacement)
}

fn validate_file_size_limit(max_file_bytes: usize) -> Result<()> {
    if max_file_bytes == 0 || max_file_bytes > MAX_FILE_BYTES {
        anyhow::bail!("workspace file limit must be between 1 and {MAX_FILE_BYTES} bytes");
    }
    Ok(())
}

#[cfg(all(test, unix))]
pub(crate) fn install_before_atomic_replace_hook(hook: impl FnOnce() + 'static) {
    platform::install_before_atomic_replace_hook(hook);
}

#[cfg(all(test, unix))]
pub(crate) fn install_after_atomic_replace_error_hook(message: impl Into<String>) {
    platform::install_after_atomic_replace_error_hook(message.into());
}

pub(crate) fn remove_entry(root: &Path, relative: &Path) -> Result<()> {
    platform::remove_entry(root, relative)
}

#[cfg(unix)]
pub(crate) fn remove_optional_file(root: &Path, relative: &Path) -> Result<()> {
    platform::remove_optional_file(root, relative)
}

#[cfg(unix)]
mod platform {
    #[cfg(test)]
    use std::cell::RefCell;
    use std::ffi::{CStr, CString, OsStr, OsString};
    use std::fs::File;
    use std::io::{Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::path::{Component, Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, MutexGuard, TryLockError};
    use std::time::{Duration, Instant};

    use anyhow::{Context, Result, anyhow, bail};
    use rand::RngCore;

    use super::{
        MAX_FILE_BYTES, MAX_QUARANTINE_BYTES, MAX_QUARANTINE_ENTRIES, MAX_QUARANTINE_SCAN_MEMBERS,
        MAX_TREE_BYTES, MAX_TREE_DEPTH, MAX_TREE_MEMBERS,
    };

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);
    static PROCESS_MUTATION_LOCK: Mutex<()> = Mutex::new(());
    #[cfg(test)]
    thread_local! {
        static BEFORE_ATOMIC_REPLACE_HOOK: RefCell<Option<Box<dyn FnOnce()>>> =
            RefCell::new(None);
        static AFTER_ATOMIC_REPLACE_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
    }
    const QUARANTINE_RELATIVE: &str = ".codex-exec-loop/runtime/planning-quarantine";
    const QUARANTINE_PREFIX: &[u8] = b".akra-preserved-";
    const MUTATION_LOCK_PARENT_RELATIVE: &str = ".codex-exec-loop/runtime";
    const MUTATION_LOCK_NAME: &str = "planning-workspace.lock";
    const MUTATION_LOCK_RELATIVE: &str = ".codex-exec-loop/runtime/planning-workspace.lock";
    const MUTATION_LOCK_TIMEOUT: Duration = Duration::from_secs(30);
    const MUTATION_LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(25);

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Identity {
        device: libc::dev_t,
        inode: libc::ino_t,
        mode: libc::mode_t,
        owner: libc::uid_t,
        links: libc::nlink_t,
        size: libc::off_t,
        modified_seconds: i64,
        modified_nanoseconds: i64,
        changed_seconds: i64,
        changed_nanoseconds: i64,
    }

    struct DirectoryStream(*mut libc::DIR);

    struct WorkspaceMutationLock {
        _file: File,
        _process_guard: MutexGuard<'static, ()>,
    }

    impl Drop for DirectoryStream {
        fn drop(&mut self) {
            // SAFETY: the pointer comes from a successful fdopendir call and is owned here.
            unsafe {
                libc::closedir(self.0);
            }
        }
    }

    pub(super) fn ensure_directory(root: &Path, relative: &Path) -> Result<PathBuf> {
        reject_internal_path(relative)?;
        let root_fd = open_workspace_root(root)?;
        let components = relative_components(relative)?;
        let _directory = traverse_directories(&root_fd, &components, true)?;
        Ok(root.join(relative))
    }

    pub(super) fn read_optional_file(
        root: &Path,
        relative: &Path,
        max_file_bytes: usize,
    ) -> Result<Option<String>> {
        reject_internal_path(relative)?;
        let root_fd = open_workspace_root(root)?;
        read_optional_file_from_root(root, &root_fd, relative, max_file_bytes)
    }

    fn read_optional_file_from_root(
        root: &Path,
        root_fd: &OwnedFd,
        relative: &Path,
        max_file_bytes: usize,
    ) -> Result<Option<String>> {
        let (parent, leaf) = match open_parent(root_fd, relative, false) {
            Ok(value) => value,
            Err(error) if error_chain_is_not_found(&error) => return Ok(None),
            Err(error) => return Err(error),
        };
        let Some((mut file, identity)) = open_regular_file(&parent, &leaf)? else {
            return Ok(None);
        };
        let body = read_bounded_utf8(&mut file, identity, &root.join(relative), max_file_bytes)?;
        if identity_from_fd(file.as_raw_fd())? != identity {
            bail!("planning file changed while it was being read");
        }
        ensure_path_identity(&parent, &leaf, identity)?;
        let reachable_parent = ensure_parent_path_reachable(root, root_fd, relative, &parent)?;
        ensure_path_identity(&reachable_parent, &leaf, identity)?;
        Ok(Some(body))
    }

    pub(super) fn read_tree(root: &Path, relative: &Path) -> Result<Vec<(String, String)>> {
        reject_internal_path(relative)?;
        let root_fd = open_workspace_root(root)?;
        let components = relative_components(relative)?;
        let directory = traverse_directories(&root_fd, &components, false)?;
        let mut records = Vec::new();
        let mut admission = TreeAdmission::default();
        read_directory_recursive(&directory, Path::new(""), 0, &mut admission, &mut records)?;
        records.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(records)
    }

    pub(super) fn write_file_atomic(root: &Path, relative: &Path, body: &[u8]) -> Result<()> {
        write_file_atomic_with_mode(root, relative, body, 0o600)
    }

    pub(super) fn write_file_atomic_with_mode(
        root: &Path,
        relative: &Path,
        body: &[u8],
        mode: u32,
    ) -> Result<()> {
        reject_internal_path(relative)?;
        let root_fd = open_workspace_root(root)?;
        let _mutation_lock = acquire_workspace_mutation_lock(&root_fd)?;
        write_file_atomic_locked(root, &root_fd, relative, body, mode)
    }

    pub(super) fn write_file_atomic_unlocked_with_mode(
        root: &Path,
        relative: &Path,
        body: &[u8],
        mode: u32,
    ) -> Result<()> {
        reject_internal_path(relative)?;
        let root_fd = open_workspace_root(root)?;
        write_file_atomic_locked(root, &root_fd, relative, body, mode)
    }

    fn write_file_atomic_locked(
        root: &Path,
        root_fd: &OwnedFd,
        relative: &Path,
        body: &[u8],
        mode: u32,
    ) -> Result<()> {
        let (parent, leaf) = open_parent(root_fd, relative, true)?;
        let original = open_regular_file(&parent, &leaf)?;

        let temp_name = private_temp_name();
        let temp_fd = open_new_private_file(&parent, &temp_name, mode)?;
        validate_regular_identity(identity_from_fd(temp_fd.as_raw_fd())?, &root.join(relative))?;
        let mut temp_file = File::from(temp_fd);
        let write_result = (|| -> Result<()> {
            temp_file
                .write_all(body)
                .with_context(|| format!("failed to write {}", root.join(relative).display()))?;
            temp_file
                .sync_all()
                .with_context(|| format!("failed to sync {}", root.join(relative).display()))?;
            let written_identity = identity_from_fd(temp_file.as_raw_fd())?;
            validate_regular_identity(written_identity, &root.join(relative))?;
            if written_identity.size != body.len() as libc::off_t {
                bail!("private planning temporary file changed during write");
            }
            run_before_atomic_replace_hook();
            ensure_destination_unchanged(&parent, &leaf, original.as_ref())?;
            rename_entry(&parent, &temp_name, &leaf)?;
            run_after_atomic_replace_error_hook()?;
            sync_directory(&parent)?;
            let (_, installed) = open_regular_file(&parent, &leaf)?
                .ok_or_else(|| anyhow!("atomic planning file replacement disappeared"))?;
            validate_regular_identity(installed, &root.join(relative))?;
            let installed_handle = identity_from_fd(temp_file.as_raw_fd())?;
            if installed != installed_handle || !same_object(installed, written_identity) {
                bail!("installed planning file does not match the private temporary file");
            }
            let reachable_parent = ensure_parent_path_reachable(root, root_fd, relative, &parent)?;
            ensure_path_identity(&reachable_parent, &leaf, installed)?;
            Ok(())
        })();
        if write_result.is_err() {
            let _ = unlink_file(&parent, &temp_name);
        }
        write_result
    }

    pub(super) fn compare_and_swap_optional_file(
        root: &Path,
        relative: &Path,
        observed: Option<&str>,
        replacement: Option<&str>,
    ) -> Result<bool> {
        reject_internal_path(relative)?;
        let root_fd = open_workspace_root(root)?;
        let _mutation_lock = acquire_workspace_mutation_lock(&root_fd)?;
        let current = read_optional_file_from_root(root, &root_fd, relative, MAX_FILE_BYTES)?;
        if current.as_deref() != observed {
            return Ok(false);
        }
        if replacement == observed {
            return Ok(true);
        }

        // The hook models a non-cooperating writer that ignores Akra's advisory workspace lock.
        // Re-read after it so even that race is rejected before the bounded mutation begins.
        run_before_atomic_replace_hook();
        let current = read_optional_file_from_root(root, &root_fd, relative, MAX_FILE_BYTES)?;
        if current.as_deref() != observed {
            return Ok(false);
        }

        match replacement {
            Some(body) => {
                write_file_atomic_locked(root, &root_fd, relative, body.as_bytes(), 0o600)?
            }
            None => remove_entry_locked(root, &root_fd, relative)?,
        }
        Ok(true)
    }

    pub(super) fn remove_entry(root: &Path, relative: &Path) -> Result<()> {
        reject_internal_path(relative)?;
        let root_fd = open_workspace_root(root)?;
        let _mutation_lock = acquire_workspace_mutation_lock(&root_fd)?;
        remove_entry_locked(root, &root_fd, relative)
    }

    pub(super) fn remove_optional_file(root: &Path, relative: &Path) -> Result<()> {
        reject_internal_path(relative)?;
        let root_fd = open_workspace_root(root)?;
        let _mutation_lock = acquire_workspace_mutation_lock(&root_fd)?;
        let (parent, leaf) = match open_parent(&root_fd, relative, false) {
            Ok(value) => value,
            Err(error) if error_chain_is_not_found(&error) => return Ok(()),
            Err(error) => return Err(error),
        };
        let Some((file, identity)) = open_regular_file(&parent, &leaf)? else {
            return Ok(());
        };
        validate_regular_identity(identity, &root.join(relative))?;
        let reachable_parent = ensure_parent_path_reachable(root, &root_fd, relative, &parent)?;
        ensure_path_identity(&reachable_parent, &leaf, identity)?;
        ensure_path_identity(&parent, &leaf, identity)?;
        unlink_file(&parent, &leaf)?;
        sync_directory(&parent)?;

        let unlinked = identity_from_fd(file.as_raw_fd())?;
        if !same_object(unlinked, identity) || unlinked.links != 0 {
            bail!("private planning file identity changed during removal");
        }
        if inspect_entry(&parent, &leaf)?.is_some() {
            bail!("private planning file path reappeared during removal");
        }
        let reachable_parent = ensure_parent_path_reachable(root, &root_fd, relative, &parent)?;
        if inspect_entry(&reachable_parent, &leaf)?.is_some() {
            bail!("private planning file path reappeared during removal");
        }
        Ok(())
    }

    fn remove_entry_locked(root: &Path, root_fd: &OwnedFd, relative: &Path) -> Result<()> {
        let (parent, leaf) = match open_parent(root_fd, relative, false) {
            Ok(value) => value,
            Err(error) if error_chain_is_not_found(&error) => return Ok(()),
            Err(error) => return Err(error),
        };
        let Some(identity) = inspect_entry(&parent, &leaf)? else {
            return Ok(());
        };
        match identity.mode & libc::S_IFMT {
            libc::S_IFREG => {
                validate_regular_identity(identity, &root.join(relative))?;
            }
            libc::S_IFDIR => {
                validate_directory_identity(identity, &root.join(relative), true)?;
            }
            _ => bail!(
                "refusing to remove non-regular planning entry: {}",
                root.join(relative).display()
            ),
        }
        preserve_entry_in_quarantine(root_fd, &parent, &leaf, identity)?;
        sync_directory(&parent)
    }

    fn open_workspace_root(root: &Path) -> Result<OwnedFd> {
        let requested = if root.is_absolute() {
            root.to_path_buf()
        } else {
            std::env::current_dir()
                .context("failed to resolve current directory")?
                .join(root)
        };
        let requested_name = CString::new(requested.as_os_str().as_bytes())
            .map_err(|_| anyhow!("workspace root contains a NUL byte"))?;
        // Open the caller-visible root before canonicalization. O_NOFOLLOW rejects a pool/workspace
        // root that was replaced with a link, while the retained identity lets us distinguish a
        // legitimate platform ancestor alias from a raced final component.
        let requested_fd = unsafe {
            libc::open(
                requested_name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        let requested_fd = owned_fd(requested_fd)
            .with_context(|| format!("failed to pin workspace root {}", requested.display()))?;
        let requested_identity = identity_from_fd(requested_fd.as_raw_fd())?;
        validate_directory_identity(requested_identity, &requested, true)?;
        // The workspace is the trusted anchor selected by the host. Canonicalize
        // it once so platform-owned aliases such as macOS /var -> /private/var
        // do not make every managed operation fail, then pin every canonical
        // component with O_NOFOLLOW. No managed relative component is canonicalized.
        let absolute = std::fs::canonicalize(&requested).with_context(|| {
            format!(
                "failed to canonicalize workspace root {}",
                requested.display()
            )
        })?;
        let mut current = open_absolute_root()?;
        let components = absolute_components(&absolute)?;
        for (index, component) in components.iter().enumerate() {
            current = open_directory_at(&current, component, false, index + 1 == components.len())
                .with_context(|| format!("failed to pin workspace root {}", absolute.display()))?;
        }
        let canonical_identity = identity_from_fd(current.as_raw_fd())?;
        validate_directory_identity(canonical_identity, &absolute, true)?;
        if !same_object(requested_identity, canonical_identity) {
            bail!("workspace root identity changed during canonicalization");
        }
        Ok(current)
    }

    fn open_absolute_root() -> Result<OwnedFd> {
        let name = CString::new("/").expect("root path contains no NUL");
        // SAFETY: the C string is valid and open returns an owned descriptor on success.
        let fd = unsafe {
            libc::open(
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        owned_fd(fd).context("failed to open filesystem root")
    }

    fn absolute_components(path: &Path) -> Result<Vec<OsString>> {
        let mut components = Vec::new();
        for component in path.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(value) => components.push(value.to_os_string()),
                _ => bail!(
                    "workspace root must be an absolute normalized path: {}",
                    path.display()
                ),
            }
        }
        Ok(components)
    }

    fn relative_components(path: &Path) -> Result<Vec<OsString>> {
        let mut components = Vec::new();
        for component in path.components() {
            match component {
                Component::Normal(value) => {
                    validate_portable_component(value)?;
                    components.push(value.to_os_string());
                }
                _ => bail!(
                    "planning path must be a normalized relative path: {}",
                    path.display()
                ),
            }
        }
        if components.is_empty() {
            bail!("planning path must not be empty");
        }
        Ok(components)
    }

    fn reject_internal_path(relative: &Path) -> Result<()> {
        let runtime = Path::new(MUTATION_LOCK_PARENT_RELATIVE);
        if relative.starts_with(runtime) || runtime.starts_with(relative) {
            bail!("the private planning runtime store is not an operator-managed path");
        }
        Ok(())
    }

    fn open_parent(root: &OwnedFd, relative: &Path, create: bool) -> Result<(OwnedFd, OsString)> {
        let mut components = relative_components(relative)?;
        let leaf = components.pop().expect("non-empty components");
        let parent = traverse_directories(root, &components, create)?;
        Ok((parent, leaf))
    }

    fn ensure_parent_path_reachable(
        root_path: &Path,
        pinned_root: &OwnedFd,
        relative: &Path,
        pinned_parent: &OwnedFd,
    ) -> Result<OwnedFd> {
        let reachable_root = open_workspace_root(root_path)?;
        if !same_object(
            identity_from_fd(reachable_root.as_raw_fd())?,
            identity_from_fd(pinned_root.as_raw_fd())?,
        ) {
            bail!("workspace root identity changed during operation");
        }
        let mut components = relative_components(relative)?;
        components.pop().expect("non-empty components");
        let reachable_parent = traverse_directories(&reachable_root, &components, false)
            .context("workspace file ancestor identity changed during operation")?;
        if !same_object(
            identity_from_fd(reachable_parent.as_raw_fd())?,
            identity_from_fd(pinned_parent.as_raw_fd())?,
        ) {
            bail!("workspace file ancestor identity changed during operation");
        }
        Ok(reachable_parent)
    }

    fn traverse_directories(
        root: &OwnedFd,
        components: &[OsString],
        create: bool,
    ) -> Result<OwnedFd> {
        let mut current = duplicate_fd(root.as_raw_fd())?;
        for component in components {
            current = open_directory_at(&current, component, create, true)?;
        }
        Ok(current)
    }

    fn open_directory_at(
        parent: &OwnedFd,
        name: &OsStr,
        create: bool,
        require_owner: bool,
    ) -> Result<OwnedFd> {
        let name = c_name(name)?;
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW;
        // SAFETY: parent is a live directory descriptor and name is a valid C string.
        let mut fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 && create && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT)
        {
            // SAFETY: mkdirat operates beneath the already pinned parent descriptor.
            let result = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) };
            if result < 0 {
                if std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST) {
                    return Err(std::io::Error::last_os_error())
                        .context("failed to create planning directory");
                }
            } else {
                sync_directory(parent)
                    .context("failed to persist a newly created planning directory")?;
            }
            // SAFETY: same pinned parent/name pair; O_NOFOLLOW rejects a raced symlink.
            fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        }
        let directory =
            owned_fd(fd).context("failed to open planning directory without following links")?;
        validate_directory_identity(
            identity_from_fd(directory.as_raw_fd())?,
            Path::new(name.to_str().unwrap_or("planning directory")),
            require_owner,
        )?;
        Ok(directory)
    }

    fn open_regular_file(parent: &OwnedFd, name: &OsStr) -> Result<Option<(File, Identity)>> {
        let name_c = c_name(name)?;
        // SAFETY: the descriptor and C string are valid; O_NOFOLLOW rejects final symlinks.
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name_c.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ENOENT) {
                return Ok(None);
            }
            return Err(error).context("failed to open planning file without following links");
        }
        let fd = owned_fd(fd)?;
        let identity = identity_from_fd(fd.as_raw_fd())?;
        validate_regular_identity(identity, Path::new(name))?;
        ensure_path_identity(parent, name, identity)?;
        Ok(Some((File::from(fd), identity)))
    }

    fn open_new_private_file(parent: &OwnedFd, name: &OsStr, mode: u32) -> Result<OwnedFd> {
        let name = c_name(name)?;
        // SAFETY: create is anchored to the pinned parent; O_EXCL and O_NOFOLLOW reject aliases.
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                mode as libc::c_uint,
            )
        };
        let fd = owned_fd(fd).context("failed to create private planning temporary file")?;
        // `openat` mode is filtered by the process umask.  The descriptor is
        // private and not yet reachable by its final name, so normalize its
        // requested mode before publication without a path-based chmod race.
        // SAFETY: `fd` is a live owned descriptor and `mode` was validated by
        // the caller that exposes configurable visibility.
        if unsafe { libc::fchmod(fd.as_raw_fd(), mode as libc::mode_t) } != 0 {
            return Err(std::io::Error::last_os_error())
                .context("failed to set temporary file permissions");
        }
        Ok(fd)
    }

    fn read_directory_recursive(
        directory: &OwnedFd,
        relative: &Path,
        depth: usize,
        admission: &mut TreeAdmission,
        records: &mut Vec<(String, String)>,
    ) -> Result<()> {
        if depth > MAX_TREE_DEPTH {
            bail!("planning tree exceeds the maximum depth of {MAX_TREE_DEPTH}");
        }
        let directory_identity = identity_from_fd(directory.as_raw_fd())?;
        let remaining_members = MAX_TREE_MEMBERS.saturating_sub(admission.members);
        for name in directory_names(directory, remaining_members)? {
            admission.members = admission.members.saturating_add(1);
            if admission.members > MAX_TREE_MEMBERS {
                bail!("planning tree exceeds the maximum member count of {MAX_TREE_MEMBERS}");
            }
            let identity = inspect_entry(directory, &name)?
                .ok_or_else(|| anyhow!("planning draft entry disappeared during read"))?;
            validate_portable_component(&name)?;
            let child_relative = relative.join(&name);
            match identity.mode & libc::S_IFMT {
                libc::S_IFDIR => {
                    validate_directory_identity(identity, &child_relative, true)?;
                    let child = open_directory_at(directory, &name, false, true)?;
                    ensure_fd_identity(&child, identity)?;
                    read_directory_recursive(
                        &child,
                        &child_relative,
                        depth + 1,
                        admission,
                        records,
                    )?;
                }
                libc::S_IFREG => {
                    validate_regular_identity(identity, &child_relative)?;
                    let (mut file, opened_identity) = open_regular_file(directory, &name)?
                        .ok_or_else(|| anyhow!("planning draft file disappeared during read"))?;
                    if opened_identity != identity {
                        bail!("planning draft file identity changed during read");
                    }
                    let body = read_bounded_utf8(
                        &mut file,
                        opened_identity,
                        &child_relative,
                        MAX_FILE_BYTES,
                    )?;
                    if identity_from_fd(file.as_raw_fd())? != opened_identity {
                        bail!("planning draft file changed while it was being read");
                    }
                    admission.bytes = admission.bytes.saturating_add(body.len());
                    if admission.bytes > MAX_TREE_BYTES {
                        bail!("planning tree exceeds the {MAX_TREE_BYTES} byte total limit");
                    }
                    ensure_path_identity(directory, &name, identity)?;
                    let child_relative = child_relative.to_str().ok_or_else(|| {
                        anyhow!("planning path must use valid UTF-8 portable components")
                    })?;
                    records.push((child_relative.to_string(), body));
                }
                _ => bail!(
                    "planning draft tree contains a link or special file: {}",
                    child_relative.display()
                ),
            }
        }
        if identity_from_fd(directory.as_raw_fd())? != directory_identity {
            bail!("planning directory changed while it was being read");
        }
        Ok(())
    }

    fn directory_names(directory: &OwnedFd, max_entries: usize) -> Result<Vec<OsString>> {
        let duplicated = duplicate_raw_fd(directory.as_raw_fd())?;
        // SAFETY: fdopendir takes ownership of the duplicated directory descriptor.
        let raw = unsafe { libc::fdopendir(duplicated) };
        if raw.is_null() {
            // SAFETY: fdopendir did not take ownership on failure.
            unsafe { libc::close(duplicated) };
            return Err(std::io::Error::last_os_error())
                .context("failed to enumerate planning directory");
        }
        let stream = DirectoryStream(raw);
        let mut names = Vec::new();
        loop {
            errno::set_errno(errno::Errno(0));
            // SAFETY: stream remains live and readdir returns storage owned by the stream.
            let Some(entry) = classify_readdir_result(unsafe { libc::readdir(stream.0) })? else {
                break;
            };
            // SAFETY: d_name is NUL-terminated by the operating system.
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            if names.len() == max_entries {
                bail!("planning directory exceeds the bounded enumeration limit");
            }
            names.push(OsString::from_vec(name.to_vec()));
        }
        names.sort();
        Ok(names)
    }

    fn classify_readdir_result(entry: *mut libc::dirent) -> Result<Option<*mut libc::dirent>> {
        if !entry.is_null() {
            return Ok(Some(entry));
        }
        let error = errno::errno();
        if error.0 == 0 {
            return Ok(None);
        }
        Err(std::io::Error::from_raw_os_error(error.0))
            .context("failed while enumerating planning directory")
    }

    fn acquire_workspace_mutation_lock(root: &OwnedFd) -> Result<WorkspaceMutationLock> {
        acquire_workspace_mutation_lock_with_timeout(root, MUTATION_LOCK_TIMEOUT)
    }

    fn acquire_workspace_mutation_lock_with_timeout(
        root: &OwnedFd,
        timeout: Duration,
    ) -> Result<WorkspaceMutationLock> {
        let started = Instant::now();
        let process_guard = loop {
            match PROCESS_MUTATION_LOCK.try_lock() {
                Ok(guard) => break guard,
                Err(TryLockError::Poisoned(_)) => {
                    bail!("planning workspace mutation lock is poisoned")
                }
                Err(TryLockError::WouldBlock) => {
                    wait_for_mutation_lock_retry(started, timeout)?;
                }
            }
        };
        let components = MUTATION_LOCK_PARENT_RELATIVE
            .split('/')
            .map(OsString::from)
            .collect::<Vec<_>>();
        let parent = traverse_directories(root, &components, true)?;
        let name = OsStr::new(MUTATION_LOCK_NAME);
        let name_c = c_name(name)?;
        // SAFETY: creation/opening is anchored to the pinned private runtime directory.
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name_c.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                0o600,
            )
        };
        let fd = owned_fd(fd).context("failed to open planning workspace mutation lock")?;
        let identity = identity_from_fd(fd.as_raw_fd())?;
        validate_regular_identity(identity, Path::new(MUTATION_LOCK_RELATIVE))?;
        ensure_path_identity(&parent, name, identity)?;
        sync_directory(&parent).context("failed to persist planning workspace mutation lock")?;

        loop {
            // SAFETY: fd remains live in the returned File. The process mutex keeps local
            // threads on the same ordering boundary as cooperating external processes;
            // LOCK_NB keeps a stale external holder from blocking this thread indefinitely.
            if unsafe { libc::flock(fd.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                break;
            }
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                wait_for_mutation_lock_retry(started, timeout)?;
                continue;
            }
            if error.kind() != std::io::ErrorKind::WouldBlock
                && !error
                    .raw_os_error()
                    .is_some_and(|code| code == libc::EAGAIN || code == libc::EWOULDBLOCK)
            {
                return Err(error).context("failed to lock planning workspace mutation boundary");
            }
            wait_for_mutation_lock_retry(started, timeout)?;
        }
        if identity_from_fd(fd.as_raw_fd())? != identity {
            bail!("planning workspace mutation lock changed while it was acquired");
        }
        ensure_path_identity(&parent, name, identity)?;
        Ok(WorkspaceMutationLock {
            _file: File::from(fd),
            _process_guard: process_guard,
        })
    }

    fn wait_for_mutation_lock_retry(started: Instant, timeout: Duration) -> Result<()> {
        let elapsed = started.elapsed();
        if elapsed >= timeout {
            bail!(
                "planning workspace mutation lock timed out after {} ms",
                timeout.as_millis()
            );
        }
        std::thread::sleep(MUTATION_LOCK_RETRY_INTERVAL.min(timeout - elapsed));
        Ok(())
    }

    fn inspect_entry(parent: &OwnedFd, name: &OsStr) -> Result<Option<Identity>> {
        let name = c_name(name)?;
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: output storage is valid and AT_SYMLINK_NOFOLLOW inspects the entry itself.
        let result = unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ENOENT) {
                return Ok(None);
            }
            return Err(error).context("failed to inspect planning entry");
        }
        // SAFETY: successful fstatat initialized the structure.
        Ok(Some(identity_from_stat(unsafe { stat.assume_init() })))
    }

    fn identity_from_fd(fd: RawFd) -> Result<Identity> {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: fd is live and output storage is valid.
        if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } < 0 {
            return Err(std::io::Error::last_os_error())
                .context("failed to inspect planning handle");
        }
        // SAFETY: successful fstat initialized the structure.
        Ok(identity_from_stat(unsafe { stat.assume_init() }))
    }

    fn identity_from_stat(stat: libc::stat) -> Identity {
        let (modified_seconds, modified_nanoseconds, changed_seconds, changed_nanoseconds) =
            stat_version(&stat);
        Identity {
            device: stat.st_dev,
            inode: stat.st_ino,
            mode: stat.st_mode,
            owner: stat.st_uid,
            links: stat.st_nlink,
            size: stat.st_size,
            modified_seconds,
            modified_nanoseconds,
            changed_seconds,
            changed_nanoseconds,
        }
    }

    #[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
    fn stat_version(stat: &libc::stat) -> (i64, i64, i64, i64) {
        (
            stat.st_mtime,
            stat.st_mtime_nsec,
            stat.st_ctime,
            stat.st_ctime_nsec,
        )
    }

    #[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
    fn stat_version(_stat: &libc::stat) -> (i64, i64, i64, i64) {
        // Device/inode/size still provide object identity on less common Unix
        // targets. Version timestamps remain conservative until their libc
        // stat layouts are covered explicitly.
        (0, 0, 0, 0)
    }

    fn validate_regular_identity(identity: Identity, path: &Path) -> Result<()> {
        // SAFETY: geteuid has no preconditions.
        let current_user = unsafe { libc::geteuid() };
        if identity.mode & libc::S_IFMT != libc::S_IFREG
            || identity.owner != current_user
            || identity.links != 1
            || identity.mode & 0o022 != 0
        {
            bail!(
                "planning file must be a current-user-owned, non-writable-by-others, regular single-link file: {}",
                path.display()
            );
        }
        Ok(())
    }

    fn validate_directory_identity(
        identity: Identity,
        path: &Path,
        require_owner: bool,
    ) -> Result<()> {
        // SAFETY: geteuid has no preconditions.
        let current_user = unsafe { libc::geteuid() };
        if identity.mode & libc::S_IFMT != libc::S_IFDIR
            || (require_owner && identity.owner != current_user)
            || (require_owner && identity.mode & 0o022 != 0)
        {
            bail!(
                "planning directory must be stable, current-user-owned, and not group/world writable: {}",
                path.display()
            );
        }
        Ok(())
    }

    fn ensure_path_identity(parent: &OwnedFd, name: &OsStr, expected: Identity) -> Result<()> {
        if inspect_entry(parent, name)? != Some(expected) {
            bail!("planning path identity changed during operation");
        }
        Ok(())
    }

    fn ensure_fd_identity(fd: &OwnedFd, expected: Identity) -> Result<()> {
        if !same_object(identity_from_fd(fd.as_raw_fd())?, expected) {
            bail!("planning directory handle identity changed during operation");
        }
        Ok(())
    }

    fn same_object(left: Identity, right: Identity) -> bool {
        left.device == right.device
            && left.inode == right.inode
            && left.mode & libc::S_IFMT == right.mode & libc::S_IFMT
            && left.owner == right.owner
    }

    #[derive(Default)]
    struct TreeAdmission {
        members: usize,
        bytes: usize,
    }

    fn read_bounded_utf8(
        file: &mut File,
        identity: Identity,
        path: &Path,
        max_file_bytes: usize,
    ) -> Result<String> {
        if identity.size < 0 || identity.size as u128 > max_file_bytes as u128 {
            bail!(
                "workspace file exceeds the {max_file_bytes} byte limit: {}",
                path.display()
            );
        }
        let mut bytes = Vec::with_capacity(identity.size as usize);
        file.take((max_file_bytes + 1) as u64)
            .read_to_end(&mut bytes)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if bytes.len() > max_file_bytes {
            bail!(
                "workspace file grew beyond the {max_file_bytes} byte limit: {}",
                path.display()
            );
        }
        String::from_utf8(bytes)
            .with_context(|| format!("planning file is not valid UTF-8: {}", path.display()))
    }

    #[cfg(test)]
    pub(super) fn install_before_atomic_replace_hook(hook: impl FnOnce() + 'static) {
        BEFORE_ATOMIC_REPLACE_HOOK.with(|slot| {
            let previous = slot.borrow_mut().replace(Box::new(hook));
            assert!(
                previous.is_none(),
                "atomic replace test hook already installed"
            );
        });
    }

    #[cfg(test)]
    pub(super) fn install_after_atomic_replace_error_hook(message: String) {
        AFTER_ATOMIC_REPLACE_ERROR.with(|slot| {
            let previous = slot.borrow_mut().replace(message);
            assert!(
                previous.is_none(),
                "after atomic replace error hook already installed"
            );
        });
    }

    #[cfg(test)]
    fn run_before_atomic_replace_hook() {
        BEFORE_ATOMIC_REPLACE_HOOK.with(|slot| {
            if let Some(hook) = slot.borrow_mut().take() {
                hook();
            }
        });
    }

    #[cfg(not(test))]
    fn run_before_atomic_replace_hook() {}

    #[cfg(test)]
    fn run_after_atomic_replace_error_hook() -> Result<()> {
        AFTER_ATOMIC_REPLACE_ERROR.with(|slot| {
            if let Some(message) = slot.borrow_mut().take() {
                bail!(message);
            }
            Ok(())
        })
    }

    #[cfg(not(test))]
    fn run_after_atomic_replace_error_hook() -> Result<()> {
        Ok(())
    }

    #[derive(Default)]
    struct RetentionAdmission {
        members: usize,
        bytes: u64,
    }

    enum HeldEntry {
        File(File),
        Directory(OwnedFd),
    }

    impl HeldEntry {
        fn identity(&self) -> Result<Identity> {
            match self {
                Self::File(file) => identity_from_fd(file.as_raw_fd()),
                Self::Directory(directory) => identity_from_fd(directory.as_raw_fd()),
            }
        }
    }

    fn preserve_entry_in_quarantine(
        root: &OwnedFd,
        source_parent: &OwnedFd,
        source_name: &OsStr,
        expected: Identity,
    ) -> Result<()> {
        let mut admission = RetentionAdmission::default();
        measure_retained_entry(source_parent, source_name, expected, 0, &mut admission)?;

        let quarantine = open_private_quarantine(root)?;
        let retained_names = directory_names(&quarantine, MAX_QUARANTINE_ENTRIES)?;
        if retained_names.len() >= MAX_QUARANTINE_ENTRIES {
            bail!(quarantine_capacity_error());
        }
        for retained_name in &retained_names {
            if !retained_name.as_bytes().starts_with(QUARANTINE_PREFIX) {
                bail!(
                    "private planning quarantine contains an unexpected entry; operator cleanup is required at {QUARANTINE_RELATIVE}"
                );
            }
            let retained_identity = inspect_entry(&quarantine, retained_name)?
                .ok_or_else(|| anyhow!("private planning quarantine changed during preflight"))?;
            measure_retained_entry(
                &quarantine,
                retained_name,
                retained_identity,
                0,
                &mut admission,
            )?;
        }

        let destination_name = unused_quarantine_name(&quarantine)?;
        let held = hold_expected_entry(source_parent, source_name, expected)?;
        ensure_path_identity(source_parent, source_name, expected)?;
        rename_entry_between(source_parent, source_name, &quarantine, &destination_name)?;

        let retained = inspect_entry(&quarantine, &destination_name)?
            .ok_or_else(|| anyhow!("preserved planning entry disappeared after quarantine move"))?;
        if !same_object(retained, expected) || !same_object(held.identity()?, expected) {
            bail!("preserved planning entry identity changed during quarantine move");
        }
        if inspect_entry(source_parent, source_name)?.is_some() {
            bail!("planning source path was recreated during quarantine move");
        }
        sync_directory(&quarantine)?;
        Ok(())
    }

    fn open_private_quarantine(root: &OwnedFd) -> Result<OwnedFd> {
        let components = QUARANTINE_RELATIVE
            .split('/')
            .map(OsString::from)
            .collect::<Vec<_>>();
        let quarantine = traverse_directories(root, &components, true)?;
        let identity = identity_from_fd(quarantine.as_raw_fd())?;
        validate_directory_identity(identity, Path::new(QUARANTINE_RELATIVE), true)?;
        if identity.mode & 0o077 != 0 {
            bail!(
                "private planning quarantine must not grant group or world permissions: {QUARANTINE_RELATIVE}"
            );
        }
        Ok(quarantine)
    }

    fn measure_retained_entry(
        parent: &OwnedFd,
        name: &OsStr,
        expected: Identity,
        depth: usize,
        admission: &mut RetentionAdmission,
    ) -> Result<()> {
        if depth > MAX_TREE_DEPTH {
            bail!(quarantine_capacity_error());
        }
        admission.members = admission
            .members
            .checked_add(1)
            .ok_or_else(|| anyhow!(quarantine_capacity_error()))?;
        if admission.members > MAX_QUARANTINE_SCAN_MEMBERS {
            bail!(quarantine_capacity_error());
        }

        match expected.mode & libc::S_IFMT {
            libc::S_IFREG => {
                validate_regular_identity(expected, Path::new(name))?;
                let (file, opened) = open_regular_file(parent, name)?.ok_or_else(|| {
                    anyhow!("planning entry disappeared during retention preflight")
                })?;
                if opened != expected || identity_from_fd(file.as_raw_fd())? != expected {
                    bail!("planning file changed during retention preflight");
                }
                let size = u64::try_from(expected.size)
                    .map_err(|_| anyhow!("planning file has an invalid negative size"))?;
                admission.bytes = admission
                    .bytes
                    .checked_add(size)
                    .ok_or_else(|| anyhow!(quarantine_capacity_error()))?;
                if admission.bytes > MAX_QUARANTINE_BYTES {
                    bail!(quarantine_capacity_error());
                }
            }
            libc::S_IFDIR => {
                validate_directory_identity(expected, Path::new(name), true)?;
                let directory = open_directory_at(parent, name, false, true)?;
                if identity_from_fd(directory.as_raw_fd())? != expected {
                    bail!("planning directory changed during retention preflight");
                }
                let remaining = MAX_QUARANTINE_SCAN_MEMBERS.saturating_sub(admission.members);
                for child_name in directory_names(&directory, remaining)? {
                    let child_identity =
                        inspect_entry(&directory, &child_name)?.ok_or_else(|| {
                            anyhow!("planning entry disappeared during retention preflight")
                        })?;
                    measure_retained_entry(
                        &directory,
                        &child_name,
                        child_identity,
                        depth + 1,
                        admission,
                    )?;
                }
                if identity_from_fd(directory.as_raw_fd())? != expected {
                    bail!("planning directory changed during retention preflight");
                }
                ensure_path_identity(parent, name, expected)?;
            }
            _ => bail!("refusing to preserve a link or special planning entry"),
        }
        Ok(())
    }

    fn hold_expected_entry(
        parent: &OwnedFd,
        name: &OsStr,
        expected: Identity,
    ) -> Result<HeldEntry> {
        let held = match expected.mode & libc::S_IFMT {
            libc::S_IFREG => HeldEntry::File(
                open_regular_file(parent, name)?
                    .ok_or_else(|| anyhow!("planning file disappeared before quarantine move"))?
                    .0,
            ),
            libc::S_IFDIR => HeldEntry::Directory(open_directory_at(parent, name, false, true)?),
            _ => bail!("refusing to preserve a link or special planning entry"),
        };
        if held.identity()? != expected {
            bail!("planning entry changed before quarantine move");
        }
        Ok(held)
    }

    fn unused_quarantine_name(quarantine: &OwnedFd) -> Result<OsString> {
        for _ in 0..8 {
            let name = private_quarantine_name();
            if inspect_entry(quarantine, &name)?.is_none() {
                return Ok(name);
            }
        }
        bail!("failed to allocate a private planning quarantine name")
    }

    fn quarantine_capacity_error() -> &'static str {
        "private planning quarantine retention limit exceeded; no entry was removed and operator cleanup is required at .codex-exec-loop/runtime/planning-quarantine"
    }

    fn ensure_destination_unchanged(
        parent: &OwnedFd,
        name: &OsStr,
        expected: Option<&(File, Identity)>,
    ) -> Result<()> {
        match expected {
            Some((file, expected_identity)) => {
                if identity_from_fd(file.as_raw_fd())? != *expected_identity {
                    bail!("planning destination handle changed before atomic replacement");
                }
                let Some((current_file, current_identity)) = open_regular_file(parent, name)?
                else {
                    bail!("planning destination disappeared before atomic replacement");
                };
                if current_identity != *expected_identity
                    || identity_from_fd(current_file.as_raw_fd())? != *expected_identity
                {
                    bail!("planning destination changed before atomic replacement");
                }
            }
            None if inspect_entry(parent, name)?.is_some() => {
                bail!("planning destination appeared before atomic replacement");
            }
            None => {}
        }
        Ok(())
    }

    fn rename_entry(parent: &OwnedFd, source: &OsStr, destination: &OsStr) -> Result<()> {
        rename_entry_between(parent, source, parent, destination)
    }

    fn rename_entry_between(
        source_parent: &OwnedFd,
        source: &OsStr,
        destination_parent: &OwnedFd,
        destination: &OsStr,
    ) -> Result<()> {
        let source = c_name(source)?;
        let destination = c_name(destination)?;
        // SAFETY: both names are relative to pinned directory descriptors.
        if unsafe {
            libc::renameat(
                source_parent.as_raw_fd(),
                source.as_ptr(),
                destination_parent.as_raw_fd(),
                destination.as_ptr(),
            )
        } < 0
        {
            return Err(std::io::Error::last_os_error())
                .context("failed to atomically replace planning file");
        }
        Ok(())
    }

    fn unlink_file(parent: &OwnedFd, name: &OsStr) -> Result<()> {
        let name = c_name(name)?;
        // SAFETY: unlinkat acts on the entry below the pinned parent and does not follow it.
        if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), 0) } < 0 {
            return Err(std::io::Error::last_os_error()).context("failed to remove planning entry");
        }
        Ok(())
    }

    fn sync_directory(directory: &OwnedFd) -> Result<()> {
        // SAFETY: directory descriptor remains live for the call.
        if unsafe { libc::fsync(directory.as_raw_fd()) } < 0 {
            return Err(std::io::Error::last_os_error())
                .context("failed to sync planning directory");
        }
        Ok(())
    }

    fn private_temp_name() -> OsString {
        private_random_name(".akra-tmp-")
    }

    fn private_quarantine_name() -> OsString {
        private_random_name(".akra-preserved-")
    }

    fn private_random_name(prefix: &str) -> OsString {
        let mut random = [0_u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut random);
        let random = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        OsString::from(format!("{prefix}{sequence:016x}-{random}"))
    }

    fn duplicate_fd(fd: RawFd) -> Result<OwnedFd> {
        owned_fd(duplicate_raw_fd(fd)?)
    }

    fn duplicate_raw_fd(fd: RawFd) -> Result<RawFd> {
        // SAFETY: fcntl duplicates a live descriptor and returns a new owned descriptor.
        let duplicated = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) };
        if duplicated < 0 {
            return Err(std::io::Error::last_os_error())
                .context("failed to duplicate planning directory handle");
        }
        Ok(duplicated)
    }

    fn owned_fd(fd: RawFd) -> Result<OwnedFd> {
        if fd < 0 {
            return Err(std::io::Error::last_os_error()).context("filesystem operation failed");
        }
        // SAFETY: callers pass a newly owned descriptor after checking for failure.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    fn c_name(name: &OsStr) -> Result<CString> {
        CString::new(name.as_bytes()).map_err(|_| anyhow!("planning path contains a NUL byte"))
    }

    fn validate_portable_component(component: &OsStr) -> Result<()> {
        let component = component
            .to_str()
            .ok_or_else(|| anyhow!("planning path components must be valid UTF-8"))?;
        if component.contains('\\') || component.chars().any(char::is_control) {
            bail!("planning path components must not contain backslashes or control characters");
        }
        Ok(())
    }

    fn error_chain_is_not_found(error: &anyhow::Error) -> bool {
        error.chain().any(|cause| {
            cause
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        })
    }

    #[cfg(test)]
    mod tests {
        use std::path::Path;
        use std::sync::mpsc;
        use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

        #[cfg(target_os = "linux")]
        use super::MUTATION_LOCK_RELATIVE;
        use super::{
            acquire_workspace_mutation_lock, acquire_workspace_mutation_lock_with_timeout,
            classify_readdir_result, compare_and_swap_optional_file,
            install_before_atomic_replace_hook, open_workspace_root, read_tree, write_file_atomic,
        };

        fn workspace(name: &str) -> std::path::PathBuf {
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be available")
                .as_nanos();
            let workspace = std::env::temp_dir().join(format!(
                "codex-exec-loop-secure-fs-{name}-{}-{suffix}",
                std::process::id()
            ));
            std::fs::create_dir_all(&workspace).expect("workspace fixture should be created");
            workspace
        }

        #[test]
        fn mutation_lock_serializes_in_process_writers() {
            let workspace = workspace("mutation-lock");
            let root = open_workspace_root(&workspace).expect("workspace root should open");
            let lock = acquire_workspace_mutation_lock(&root).expect("first lock should acquire");
            let (started_tx, started_rx) = mpsc::channel();
            let (done_tx, done_rx) = mpsc::channel();
            let worker_workspace = workspace.clone();
            let worker = std::thread::spawn(move || {
                started_tx.send(()).expect("start signal should send");
                let result = write_file_atomic(&worker_workspace, Path::new("result.md"), b"new");
                done_tx.send(result).expect("result should send");
            });
            started_rx.recv().expect("worker should start");
            let completed_while_locked = done_rx.recv_timeout(Duration::from_millis(100));
            drop(lock);
            let result = done_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("writer should resume after the lock is released");
            worker.join().expect("writer thread should join");
            let _ = std::fs::remove_dir_all(&workspace);

            assert!(
                completed_while_locked.is_err(),
                "writer must not pass the in-process mutation boundary"
            );
            result.expect("writer should complete after serialization");
        }

        #[test]
        fn mutation_lock_times_out_instead_of_waiting_forever() {
            let workspace = workspace("mutation-timeout");
            let root = open_workspace_root(&workspace).expect("workspace root should open");
            let lock = acquire_workspace_mutation_lock(&root).expect("first lock should acquire");
            let started = Instant::now();
            let error = match acquire_workspace_mutation_lock_with_timeout(
                &root,
                Duration::from_millis(40),
            ) {
                Ok(_) => panic!("contended local lock must time out"),
                Err(error) => error,
            };
            let elapsed = started.elapsed();
            drop(lock);
            let _ = std::fs::remove_dir_all(&workspace);

            assert!(error.to_string().contains("timed out after 40 ms"));
            assert!(elapsed >= Duration::from_millis(40));
            assert!(elapsed < Duration::from_secs(1));
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn mutation_lock_times_out_for_kernel_lock_holder() {
            use std::os::fd::AsRawFd;

            let workspace = workspace("kernel-lock");
            let root = open_workspace_root(&workspace).expect("workspace root should open");
            let lock = acquire_workspace_mutation_lock(&root).expect("first lock should acquire");
            drop(lock);
            let external = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(workspace.join(MUTATION_LOCK_RELATIVE))
                .expect("external lock handle should open");
            // SAFETY: external is a live descriptor and no other holder remains here.
            assert_eq!(
                unsafe { libc::flock(external.as_raw_fd(), libc::LOCK_EX) },
                0
            );
            let started = Instant::now();
            let error = match acquire_workspace_mutation_lock_with_timeout(
                &root,
                Duration::from_millis(40),
            ) {
                Ok(_) => panic!("contended kernel lock must time out"),
                Err(error) => error,
            };
            let elapsed = started.elapsed();
            drop(external);
            let _ = std::fs::remove_dir_all(&workspace);

            assert!(error.to_string().contains("timed out after 40 ms"));
            assert!(elapsed >= Duration::from_millis(40));
            assert!(elapsed < Duration::from_secs(1));
        }

        #[test]
        fn readdir_error_is_not_treated_as_end_of_directory() {
            errno::set_errno(errno::Errno(libc::EIO));
            let error = classify_readdir_result(std::ptr::null_mut())
                .expect_err("readdir I/O error must fail closed");
            assert!(error.chain().any(|cause| {
                cause
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.raw_os_error() == Some(libc::EIO))
            }));

            errno::set_errno(errno::Errno(0));
            assert!(
                classify_readdir_result(std::ptr::null_mut())
                    .expect("zero errno should represent end of directory")
                    .is_none()
            );
        }

        #[test]
        fn fixed_length_temp_name_supports_long_valid_destination_leaf() {
            let workspace = workspace("long-leaf");
            let leaf = format!("{}.md", "a".repeat(236));
            write_file_atomic(&workspace, Path::new(&leaf), b"body")
                .expect("valid long destination should not overflow the temp name limit");
            assert_eq!(std::fs::read(workspace.join(leaf)).unwrap(), b"body");
            let _ = std::fs::remove_dir_all(&workspace);
        }

        #[test]
        fn optional_file_compare_and_swap_handles_create_replace_and_delete() {
            let workspace = workspace("optional-cas-lifecycle");
            let relative = Path::new("planning/result.md");

            assert!(
                compare_and_swap_optional_file(&workspace, relative, None, Some("first"))
                    .expect("missing file should be created")
            );
            assert!(
                !compare_and_swap_optional_file(
                    &workspace,
                    relative,
                    Some("stale"),
                    Some("must-not-write")
                )
                .expect("stale observed content should be a CAS miss")
            );
            assert_eq!(
                std::fs::read_to_string(workspace.join(relative)).unwrap(),
                "first"
            );
            assert!(
                compare_and_swap_optional_file(&workspace, relative, Some("first"), Some("second"))
                    .expect("matching file should be replaced")
            );
            assert!(
                compare_and_swap_optional_file(&workspace, relative, Some("second"), None)
                    .expect("matching file should be deleted")
            );
            assert!(!workspace.join(relative).exists());
            let _ = std::fs::remove_dir_all(&workspace);
        }

        #[test]
        fn optional_file_compare_and_swap_preserves_concurrent_operator_mutation() {
            let workspace = workspace("optional-cas-race");
            let relative = Path::new("planning/result.md");
            write_file_atomic(&workspace, relative, b"worker candidate")
                .expect("worker candidate should be seeded");
            let operator_path = workspace.join(relative);
            install_before_atomic_replace_hook(move || {
                std::fs::write(operator_path, "operator edit")
                    .expect("operator race mutation should write");
            });

            assert!(
                !compare_and_swap_optional_file(
                    &workspace,
                    relative,
                    Some("worker candidate"),
                    Some("pre-turn snapshot")
                )
                .expect("concurrent operator mutation should become a CAS miss")
            );
            assert_eq!(
                std::fs::read_to_string(workspace.join(relative)).unwrap(),
                "operator edit"
            );
            let _ = std::fs::remove_dir_all(&workspace);
        }

        #[test]
        fn planning_tree_rejects_backslash_aliases_instead_of_rewriting_their_identity() {
            let workspace = workspace("backslash-alias");
            let draft = workspace.join("draft");
            std::fs::create_dir(&draft).expect("draft fixture should create");
            std::fs::write(draft.join("nested\\result.md"), b"ambiguous")
                .expect("backslash fixture should write");

            let error = read_tree(&workspace, Path::new("draft"))
                .expect_err("a Unix backslash must not impersonate a path separator");
            assert!(error.to_string().contains("backslashes"));
            let _ = std::fs::remove_dir_all(&workspace);
        }

        #[test]
        fn planning_tree_rejects_non_utf8_names_instead_of_lossy_aliasing_them() {
            use std::os::unix::ffi::OsStringExt;

            let workspace = workspace("non-utf8-alias");
            let draft = workspace.join("draft");
            std::fs::create_dir(&draft).expect("draft fixture should create");
            let non_utf8_path = draft.join(std::ffi::OsString::from_vec(vec![b'x', 0xff]));
            if let Err(error) = std::fs::write(&non_utf8_path, b"ambiguous") {
                // APFS rejects invalid UTF-8 path components at the filesystem
                // boundary (EILSEQ), so the lossy-alias case cannot be created
                // on that host. Other failures remain a real fixture error.
                assert_eq!(error.raw_os_error(), Some(libc::EILSEQ), "{error}");
                let _ = std::fs::remove_dir_all(&workspace);
                return;
            }

            let error = read_tree(&workspace, Path::new("draft"))
                .expect_err("non-UTF-8 planning names must not be converted lossily");
            assert!(error.to_string().contains("valid UTF-8"));
            let _ = std::fs::remove_dir_all(&workspace);
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::path::{Path, PathBuf};

    use anyhow::{Result, bail};

    fn unsupported<T>(operation: &str) -> Result<T> {
        bail!(
            "secure direct planning filesystem {operation} is unsupported on Windows; use the repo-scoped authority store"
        )
    }

    pub(super) fn ensure_directory(_root: &Path, _relative: &Path) -> Result<PathBuf> {
        unsupported("directory creation")
    }

    pub(super) fn read_optional_file(
        _root: &Path,
        _relative: &Path,
        _max_file_bytes: usize,
    ) -> Result<Option<String>> {
        unsupported("file read")
    }

    pub(super) fn read_tree(_root: &Path, _relative: &Path) -> Result<Vec<(String, String)>> {
        unsupported("tree read")
    }

    pub(super) fn write_file_atomic(_root: &Path, _relative: &Path, _body: &[u8]) -> Result<()> {
        unsupported("file write")
    }

    pub(super) fn compare_and_swap_optional_file(
        _root: &Path,
        _relative: &Path,
        _observed: Option<&str>,
        _replacement: Option<&str>,
    ) -> Result<bool> {
        unsupported("file compare-and-swap")
    }

    pub(super) fn remove_entry(_root: &Path, _relative: &Path) -> Result<()> {
        unsupported("entry removal")
    }
}
