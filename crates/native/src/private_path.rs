//! Anchored private directory traversal shared by files and SQLite consumers.
//! Directory ownership, identity, no-follow and Windows rename-sharing semantics are preserved.

use anyhow::{Context, Result, bail};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::time::Duration;

mod pinned_directory;
pub use pinned_directory::PinnedPrivateDirectory;

struct DirectoryAnchor {
    path: PathBuf,
    file: File,
    private: bool,
}

impl DirectoryAnchor {
    fn verify(&self) -> Result<()> {
        platform::verify_directory(&self.path, &self.file, self.private)
    }
}

#[derive(Debug)]
struct EntryAppeared;

impl std::fmt::Display for EntryAppeared {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("private SQLite entry appeared during absence verification")
    }
}

impl std::error::Error for EntryAppeared {}

fn confirm_absence(path: &Path, directories: &[DirectoryAnchor]) -> Result<()> {
    for directory in directories {
        directory.verify()?;
    }
    let appeared = match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
                bail!("private SQLite missing entry is redirected or has an unsafe type");
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
                if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    bail!("private SQLite missing entry is a reparse point");
                }
            }
            true
        }
        Err(error) => return Err(error).context("failed to confirm private SQLite absence"),
    };
    for directory in directories {
        directory.verify()?;
    }
    if appeared {
        // Retry only an observed normal entry appearing, through all ordinary ACL,
        // identity, and schema checks. Permission/corruption failures are not retried.
        return Err(EntryAppeared.into());
    }
    Ok(())
}

pub fn validate_runtime_path(runtime: &Path) -> Result<()> {
    if !runtime.is_absolute() || runtime.as_os_str().as_encoded_bytes().contains(&0) {
        bail!("private SQLite runtime must be an absolute directory without NUL bytes")
    }
    for component in runtime.components() {
        match component {
            std::path::Component::Prefix(_) | std::path::Component::RootDir => {}
            std::path::Component::Normal(name) => {
                #[cfg(windows)]
                {
                    let name = name
                        .to_str()
                        .context("private SQLite directory name is not Unicode")?;
                    let stem = name
                        .split('.')
                        .next()
                        .unwrap_or_default()
                        .to_ascii_uppercase();
                    if name.ends_with(['.', ' '])
                        || name
                            .chars()
                            .any(|character| character < ' ' || "<>:\"|?*".contains(character))
                        || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                        || (stem.len() == 4
                            && (stem.starts_with("COM") || stem.starts_with("LPT"))
                            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
                    {
                        bail!("private SQLite directory name must not contain Windows aliases")
                    }
                }
                #[cfg(not(windows))]
                let _ = name;
            }
            _ => {
                bail!("private SQLite runtime must not contain parent or current-directory aliases")
            }
        }
    }
    Ok(())
}

/// An opaque retained ancestor chain. No constructor accepts unchecked handles.
pub struct DirectoryChain {
    directories: Vec<DirectoryAnchor>,
}

impl DirectoryChain {
    pub fn open(path: &Path, create: bool) -> Result<Self> {
        validate_runtime_path(path)?;
        Ok(Self {
            directories: platform::open_runtime_with_privacy(path, create, true)?,
        })
    }

    pub fn open_existing(path: &Path) -> Result<Option<Self>> {
        validate_runtime_path(path)?;
        Ok(platform::open_existing_runtime(path)?.map(|directories| Self { directories }))
    }

    pub fn verify(&self) -> Result<()> {
        for directory in &self.directories {
            directory.verify()?;
        }
        Ok(())
    }

    pub fn handle(&self) -> &File {
        &self
            .directories
            .last()
            .expect("validated directory anchor")
            .file
    }

    pub fn confirm_absence(&self, path: &Path) -> Result<()> {
        confirm_absence(path, &self.directories)
    }
}

/// Only an observed ordinary entry appearing is transient; unsafe identity errors are not.
pub fn entry_appeared(error: &anyhow::Error) -> bool {
    error.is::<EntryAppeared>()
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::path::Component;

    pub(super) fn open_runtime_with_privacy(
        runtime: &Path,
        create: bool,
        private_final: bool,
    ) -> Result<Vec<DirectoryAnchor>> {
        let components = runtime.components().collect::<Vec<_>>();
        let root = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open("/")?;
        let mut anchors = vec![DirectoryAnchor {
            path: PathBuf::from("/"),
            file: root,
            private: private_final && components.len() == 1,
        }];
        let mut path = PathBuf::from("/");
        for (index, component) in components.iter().enumerate() {
            match component {
                Component::RootDir => continue,
                Component::Normal(name) => {
                    let name = CString::new(name.as_bytes())?;
                    let parent = anchors.last().context("missing runtime parent")?;
                    if create {
                        parent.verify()?;
                    }
                    let flags =
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
                    let mut created = false;
                    let directory = match open_at(&parent.file, &name, flags, 0) {
                        Ok(directory) => directory,
                        Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
                            parent.verify()?;
                            created = create_directory_at(&parent.file, &name)?;
                            let directory = open_at(&parent.file, &name, flags, 0)?;
                            // Pin the entry and persist its parent before using a new directory.
                            parent
                                .file
                                .sync_all()
                                .context("failed to sync private SQLite parent directory")?;
                            parent.verify()?;
                            directory
                        }
                        Err(error) => {
                            return Err(error).context("failed to open private SQLite directory");
                        }
                    };
                    path.push(component.as_os_str());
                    let anchor = DirectoryAnchor {
                        path: path.clone(),
                        file: directory,
                        private: created || (private_final && index + 1 == components.len()),
                    };
                    if create {
                        anchor.verify()?;
                        parent.verify()?;
                    }
                    anchors.push(anchor);
                }
                _ => bail!("private SQLite runtime must not contain parent or prefix aliases"),
            }
        }
        for anchor in &anchors {
            anchor.verify()?;
        }
        Ok(anchors)
    }

    pub(super) fn open_existing_runtime(runtime: &Path) -> Result<Option<Vec<DirectoryAnchor>>> {
        let components = runtime.components().collect::<Vec<_>>();
        let root = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open("/")?;
        let mut anchors = vec![DirectoryAnchor {
            path: PathBuf::from("/"),
            file: root,
            private: components.len() == 1,
        }];
        anchors[0].verify()?;
        let mut path = PathBuf::from("/");
        for (index, component) in components.iter().enumerate() {
            match component {
                Component::RootDir => continue,
                Component::Normal(name) => {
                    let parent = anchors.last().context("missing runtime parent")?;
                    parent.verify()?;
                    let child = CString::new(name.as_bytes())?;
                    path.push(component.as_os_str());
                    let directory = match open_at(
                        &parent.file,
                        &child,
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                        0,
                    ) {
                        Ok(directory) => directory,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            confirm_absence(&path, &anchors)?;
                            return Ok(None);
                        }
                        Err(error) => {
                            return Err(error)
                                .context("failed to open existing private SQLite directory");
                        }
                    };
                    let anchor = DirectoryAnchor {
                        path: path.clone(),
                        file: directory,
                        private: index + 1 == components.len(),
                    };
                    anchor.verify()?;
                    parent.verify()?;
                    anchors.push(anchor);
                }
                _ => bail!("private SQLite runtime must not contain parent or prefix aliases"),
            }
        }
        for anchor in &anchors {
            anchor.verify()?;
        }
        Ok(Some(anchors))
    }

    fn create_directory_at(directory: &File, name: &CString) -> Result<bool> {
        // SAFETY: the parent descriptor and NUL-terminated child name remain valid.
        let status = unsafe { libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o700) };
        if status == 0 {
            return Ok(true);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            Ok(false)
        } else {
            Err(error).context("failed to create private SQLite directory")
        }
    }

    pub(super) fn verify_directory(path: &Path, directory: &File, private: bool) -> Result<()> {
        let opened = directory.metadata()?;
        let current = fs::symlink_metadata(path)?;
        // SAFETY: geteuid has no preconditions.
        let user = unsafe { libc::geteuid() };
        if !opened.is_dir()
            || current.file_type().is_symlink()
            || !current.is_dir()
            || opened.dev() != current.dev()
            || opened.ino() != current.ino()
            || (opened.uid() != user && opened.uid() != 0)
        {
            bail!("private SQLite runtime directory identity is unsafe")
        }
        if private {
            if opened.uid() != user || opened.mode() & 0o777 != 0o700 {
                bail!("private SQLite runtime is not a private directory")
            }
        } else if opened.mode() & 0o022 != 0 && opened.mode() & 0o1000 == 0 {
            bail!("private SQLite runtime ancestor is replaceable by other users")
        }
        Ok(())
    }

    fn open_at(
        directory: &File,
        name: &CString,
        flags: libc::c_int,
        mode: libc::c_uint,
    ) -> std::io::Result<File> {
        // SAFETY: the descriptor and NUL-terminated name remain valid; mode uses the C
        // variadic unsigned-int promotion required even where mode_t is narrower.
        let descriptor = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags, mode) };
        if descriptor < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            // SAFETY: openat returned a fresh descriptor, now owned by File.
            Ok(unsafe { File::from_raw_fd(descriptor) })
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use crate::private_fs::{
        WINDOWS_FILE_FLAG_BACKUP_SEMANTICS, WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
        WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ, WINDOWS_READ_CONTROL, WINDOWS_WRITE_DAC,
        set_windows_private_acl, validate_windows_path_identity,
        validate_windows_path_identity_only, validate_windows_private_owner_and_acl,
    };
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::Component;
    const TRANSIENT_ATTEMPTS: usize = 8;
    const TRANSIENT_DELAY: Duration = Duration::from_millis(5);

    pub(super) fn open_runtime_with_privacy(
        runtime: &Path,
        create: bool,
        private_final: bool,
    ) -> Result<Vec<DirectoryAnchor>> {
        let components = runtime.components().collect::<Vec<_>>();
        let mut path = PathBuf::new();
        let mut anchors: Vec<DirectoryAnchor> = Vec::new();
        for (index, component) in components.iter().enumerate() {
            match component {
                Component::Prefix(_) => {
                    path.push(component.as_os_str());
                    continue;
                }
                Component::RootDir | Component::Normal(_) => path.push(component.as_os_str()),
                _ => bail!("private SQLite runtime must not contain parent aliases"),
            }
            if create && let Some(parent) = anchors.last() {
                parent.verify()?;
            }
            let mut created = false;
            let directory = match open_directory(&path, create, false) {
                Ok(directory) => directory,
                Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
                    let parent = anchors
                        .last()
                        .context("missing private SQLite directory parent")?;
                    parent.verify()?;
                    created = create_private_directory(&path)?;
                    parent.verify()?;
                    let directory = open_directory(&path, true, created)?;
                    if created {
                        // The initial DACL is already owner-only. Apply directory inheritance only
                        // through this verified handle; never repair an existing directory.
                        validate_windows_path_identity(&path, &directory, true)?;
                        set_windows_private_acl(&directory, true)?;
                    }
                    directory
                }
                Err(error) => return Err(error).context("failed to open private SQLite directory"),
            };
            let anchor = DirectoryAnchor {
                path: path.clone(),
                file: directory,
                private: created || (private_final && index + 1 == components.len()),
            };
            if create {
                anchor.verify()?;
                if let Some(parent) = anchors.last() {
                    parent.verify()?;
                }
            }
            anchors.push(anchor);
        }
        for anchor in &anchors {
            anchor.verify()?;
        }
        Ok(anchors)
    }

    pub(super) fn open_existing_runtime(runtime: &Path) -> Result<Option<Vec<DirectoryAnchor>>> {
        let components = runtime.components().collect::<Vec<_>>();
        let mut path = PathBuf::new();
        let mut anchors: Vec<DirectoryAnchor> = Vec::new();
        for (index, component) in components.iter().enumerate() {
            match component {
                Component::Prefix(_) => {
                    path.push(component.as_os_str());
                    continue;
                }
                Component::RootDir | Component::Normal(_) => path.push(component.as_os_str()),
                _ => bail!("private SQLite runtime must not contain parent aliases"),
            }
            if let Some(parent) = anchors.last() {
                parent.verify()?;
            }
            // Retain non-delete-sharing handles even on the read path. Validate each
            // component before traversing beneath it, including dangling junctions.
            let directory = match open_directory(&path, true, false) {
                Ok(directory) => directory,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    confirm_absence(&path, &anchors)?;
                    return Ok(None);
                }
                Err(error) => {
                    return Err(error).context("failed to open existing private SQLite directory");
                }
            };
            let anchor = DirectoryAnchor {
                path: path.clone(),
                file: directory,
                private: index + 1 == components.len(),
            };
            anchor.verify()?;
            if let Some(parent) = anchors.last() {
                parent.verify()?;
            }
            anchors.push(anchor);
        }
        for anchor in &anchors {
            anchor.verify()?;
        }
        Ok(Some(anchors))
    }

    pub(super) fn open_directory(path: &Path, pin: bool, write_acl: bool) -> std::io::Result<File> {
        use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_DELETE;

        fs::OpenOptions::new()
            .read(true)
            .access_mode(
                WINDOWS_GENERIC_READ
                    | WINDOWS_READ_CONTROL
                    | if write_acl { WINDOWS_WRITE_DAC } else { 0 },
            )
            // Cooperating readers/writers remain compatible, but no ancestor can be renamed
            // while a creation transaction retains its directory anchors.
            .share_mode(if pin {
                WINDOWS_FILE_SHARE_ALL & !FILE_SHARE_DELETE
            } else {
                WINDOWS_FILE_SHARE_ALL
            })
            .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT | WINDOWS_FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
    }

    fn create_private_directory(path: &Path) -> Result<bool> {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
        use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;

        // Ancestor handles already pin and verify this existing parent. Canonicalize
        // only the FFI spelling so CreateDirectoryW accepts paths beyond MAX_PATH.
        let parent = path
            .parent()
            .context("missing private SQLite directory parent")?;
        let name = path
            .file_name()
            .context("missing private SQLite directory name")?;
        let path = fs::canonicalize(parent)
            .context("failed to resolve pinned private SQLite directory parent")?
            .join(name);
        let security = crate::private_fs::WindowsOwnerOnlySecurityDescriptor::for_current_user()?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: security.as_raw(),
            bInheritHandle: 0,
        };
        let wide = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        // SAFETY: the NUL-terminated path and security descriptor remain live for this call.
        if unsafe { CreateDirectoryW(wide.as_ptr(), &attributes) } != 0 {
            return Ok(true);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            Ok(false)
        } else {
            Err(error).context("failed to create private SQLite directory")
        }
    }

    pub(super) fn verify_directory(path: &Path, directory: &File, private: bool) -> Result<()> {
        retry_identity(|| {
            if private {
                validate_windows_path_identity(path, directory, true)?;
                validate_windows_private_owner_and_acl(path, directory)
            } else {
                validate_windows_path_identity_only(path, directory, true)
            }
        })
    }

    fn retry_identity(mut check: impl FnMut() -> Result<()>) -> Result<()> {
        for attempt in 0..TRANSIENT_ATTEMPTS {
            match check() {
                Err(_) if attempt + 1 < TRANSIENT_ATTEMPTS => {
                    std::thread::sleep(TRANSIENT_DELAY);
                }
                result => return result,
            }
        }
        unreachable!("bounded identity validation returns")
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    use super::*;
    pub(super) fn open_runtime_with_privacy(
        _: &Path,
        _: bool,
        _: bool,
    ) -> Result<Vec<DirectoryAnchor>> {
        bail!("private directory pinning is unsupported on this platform")
    }

    pub(super) fn open_existing_runtime(_: &Path) -> Result<Option<Vec<DirectoryAnchor>>> {
        bail!("private SQLite databases are unsupported on this platform")
    }

    pub(super) fn verify_directory(_: &Path, _: &File, _: bool) -> Result<()> {
        bail!("private SQLite databases are unsupported on this platform")
    }
}
