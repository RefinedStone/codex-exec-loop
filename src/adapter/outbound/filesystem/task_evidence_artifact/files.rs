use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

use anyhow::{Context, Result, bail};

use crate::private_sqlite::PinnedPrivateDirectory;

pub(super) struct StableFile {
    file: File,
    name: String,
    identity: platform::Identity,
}

impl StableFile {
    pub(super) fn open(directory: &PinnedPrivateDirectory, name: &str) -> Result<Self> {
        directory.file_path(name)?;
        directory.verify()?;
        let file = platform::open_file(directory, name, false)?;
        verify_file(directory, name, &file)?;
        let identity = platform::identity(&file)?;
        Ok(Self {
            file,
            name: name.into(),
            identity,
        })
    }

    pub(super) fn verify(&self, directory: &PinnedPrivateDirectory) -> Result<()> {
        verify_file(directory, &self.name, &self.file)?;
        if platform::identity(&self.file)? != self.identity {
            bail!("task evidence artifact identity changed while being read");
        }
        Ok(())
    }

    pub(super) fn read_bounded(
        &mut self,
        directory: &PinnedPrivateDirectory,
        max_bytes: u64,
    ) -> Result<Vec<u8>> {
        self.verify(directory)?;
        if self.file.metadata()?.len() > max_bytes {
            bail!("task evidence artifact exceeds its byte limit");
        }
        let limit = max_bytes
            .checked_add(1)
            .context("artifact read limit overflow")?;
        self.file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        (&mut self.file).take(limit).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > max_bytes || bytes.len() as u64 != self.file.metadata()?.len() {
            bail!("task evidence artifact changed size or exceeded its byte limit");
        }
        self.verify(directory)?;
        Ok(bytes)
    }
}

pub(super) fn create_empty_if_missing(
    directory: &PinnedPrivateDirectory,
    name: &str,
) -> Result<()> {
    directory.file_path(name)?;
    directory.verify()?;
    match platform::open_file(directory, name, true) {
        Ok(file) => {
            verify_file(directory, name, &file)?;
            file.sync_all()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            // Never truncate or repair an existing entry.
            StableFile::open(directory, name)?;
        }
        Err(error) => return Err(error).context("failed to prepare private artifact file"),
    }
    directory.verify()
}

pub(super) struct TemporaryFile<'a> {
    directory: &'a PinnedPrivateDirectory,
    name: String,
    file: File,
    published: bool,
}

impl<'a> TemporaryFile<'a> {
    pub(super) fn create(directory: &'a PinnedPrivateDirectory, name: &str) -> Result<Self> {
        directory.file_path(name)?;
        directory.verify()?;
        let file = match platform::open_file(directory, name, true) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                // Only the fixed private scratch file may be reclaimed after a crash.
                // Never repair an unsafe entry or touch the sealed destination.
                let previous = StableFile::open(directory, name)?;
                previous.verify(directory)?;
                platform::remove_file(directory, name)?;
                directory.sync()?;
                platform::open_file(directory, name, true)?
            }
            Err(error) => return Err(error).context("failed to allocate artifact capture file"),
        };
        verify_file(directory, name, &file)?;
        Ok(Self {
            directory,
            name: name.into(),
            file,
            published: false,
        })
    }

    pub(super) fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        self.directory.verify()?;
        self.file.write_all(bytes)?;
        self.file.sync_all()?;
        verify_file(self.directory, &self.name, &self.file)
    }

    pub(super) fn publish_noreplace(&mut self, destination: &str) -> Result<()> {
        self.directory.file_path(destination)?;
        verify_file(self.directory, &self.name, &self.file)?;
        match platform::publish_noreplace(self.directory, &self.name, destination) {
            Ok(()) => self.published = true,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error).context("failed to publish immutable task evidence"),
        }
        self.directory.sync()
    }
}

impl Drop for TemporaryFile<'_> {
    fn drop(&mut self) {
        if !self.published && verify_file(self.directory, &self.name, &self.file).is_ok() {
            let _ = platform::remove_file(self.directory, &self.name);
            let _ = self.directory.sync();
        }
    }
}

fn verify_file(directory: &PinnedPrivateDirectory, name: &str, file: &File) -> Result<()> {
    directory.verify()?;
    platform::validate(file, &directory.file_path(name)?)?;
    let reopened = platform::open_file(directory, name, false)?;
    platform::validate(&reopened, &directory.file_path(name)?)?;
    if platform::identity(file)? != platform::identity(&reopened)? {
        bail!("task evidence artifact path no longer names the opened file");
    }
    directory.verify()
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(super) struct Identity {
        device: u64,
        inode: u64,
        mode: u32,
        owner: u32,
        links: u64,
        size: u64,
        modified: (i64, i64),
        changed: (i64, i64),
    }

    pub(super) fn identity(file: &File) -> Result<Identity> {
        let metadata = file.metadata()?;
        Ok(Identity {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            owner: metadata.uid(),
            links: metadata.nlink(),
            size: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        })
    }

    pub(super) fn validate(file: &File, _: &Path) -> Result<()> {
        let metadata = file.metadata()?;
        // SAFETY: geteuid has no preconditions.
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
            || metadata.mode() & 0o777 != 0o600
        {
            bail!("task evidence requires an owned private regular file with one link");
        }
        Ok(())
    }

    pub(super) fn open_file(
        directory: &PinnedPrivateDirectory,
        name: &str,
        create: bool,
    ) -> std::io::Result<File> {
        let name = CString::new(name)?;
        let flags = libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if create {
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL
            } else {
                libc::O_RDONLY
            };
        // SAFETY: the pinned descriptor and single NUL-terminated component are live.
        let descriptor =
            unsafe { libc::openat(directory.handle().as_raw_fd(), name.as_ptr(), flags, 0o600) };
        if descriptor < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: a successful openat returns one owned descriptor.
        Ok(unsafe { File::from_raw_fd(descriptor) })
    }

    pub(super) fn remove_file(
        directory: &PinnedPrivateDirectory,
        name: &str,
    ) -> std::io::Result<()> {
        let name = CString::new(name)?;
        // SAFETY: the pinned descriptor and single component remain valid.
        if unsafe { libc::unlinkat(directory.handle().as_raw_fd(), name.as_ptr(), 0) } == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    pub(super) fn publish_noreplace(
        directory: &PinnedPrivateDirectory,
        source: &str,
        destination: &str,
    ) -> std::io::Result<()> {
        let source = CString::new(source)?;
        let destination = CString::new(destination)?;
        let descriptor = directory.handle().as_raw_fd();
        #[cfg(target_os = "linux")]
        // SAFETY: both names and the pinned directory descriptor remain valid.
        let result = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                descriptor,
                source.as_ptr(),
                descriptor,
                destination.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        #[cfg(target_os = "macos")]
        // SAFETY: both names and the pinned directory descriptor remain valid.
        let result = unsafe {
            libc::renameatx_np(
                descriptor,
                source.as_ptr(),
                descriptor,
                destination.as_ptr(),
                libc::RENAME_EXCL,
            )
        } as libc::c_long;
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = (descriptor, source, destination);
            return Err(std::io::Error::other(
                "atomic no-replace artifact publication is unavailable",
            ));
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if result == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use crate::private_fs::*;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::Path;

    pub(super) type Identity = WindowsFileIdentitySnapshot;
    pub(super) fn identity(file: &File) -> Result<Identity> {
        windows_file_identity_snapshot(file)
    }
    pub(super) fn validate(file: &File, path: &Path) -> Result<()> {
        validate_windows_path_identity(path, file, false)?;
        validate_windows_private_owner_and_acl(path, file)
    }

    pub(super) fn open_file(
        directory: &PinnedPrivateDirectory,
        name: &str,
        create: bool,
    ) -> std::io::Result<File> {
        let mut options = std::fs::OpenOptions::new();
        options
            .read(true)
            .write(create)
            .create_new(create)
            .access_mode(
                WINDOWS_GENERIC_READ
                    | WINDOWS_READ_CONTROL
                    | if create {
                        WINDOWS_GENERIC_WRITE | WINDOWS_WRITE_DAC
                    } else {
                        0
                    },
            )
            .share_mode(WINDOWS_FILE_SHARE_ALL)
            .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT);
        // The pinned parent has a validated owner-only inheritable DACL. Only new
        // files receive an explicit ACL; an unsafe existing file is never repaired.
        let file = options.open(directory.path().join(name))?;
        if create {
            set_windows_private_acl(&file, false).map_err(std::io::Error::other)?;
        }
        Ok(file)
    }

    pub(super) fn remove_file(
        directory: &PinnedPrivateDirectory,
        name: &str,
    ) -> std::io::Result<()> {
        std::fs::remove_file(directory.path().join(name))
    }

    pub(super) fn publish_noreplace(
        directory: &PinnedPrivateDirectory,
        source: &str,
        destination: &str,
    ) -> std::io::Result<()> {
        use windows_sys::Win32::Storage::FileSystem::MoveFileW;
        // The verified ancestor handles remain pinned while resolving the extended
        // Windows spelling required by this raw API for paths beyond MAX_PATH.
        let directory = std::fs::canonicalize(directory.path())?;
        let wide = |name: &str| {
            directory
                .join(name)
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect::<Vec<_>>()
        };
        let source = wide(source);
        let destination = wide(destination);
        // SAFETY: both NUL-terminated buffers are live; all ancestor handles deny rename.
        if unsafe { MoveFileW(source.as_ptr(), destination.as_ptr()) } != 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    use super::*;
    use std::path::Path;
    #[derive(Clone, PartialEq, Eq)]
    pub(super) struct Identity;
    pub(super) fn identity(_: &File) -> Result<Identity> {
        bail!("artifact identity is unavailable")
    }
    pub(super) fn validate(_: &File, _: &Path) -> Result<()> {
        bail!("private artifact files are unavailable")
    }
    pub(super) fn open_file(_: &PinnedPrivateDirectory, _: &str, _: bool) -> std::io::Result<File> {
        Err(std::io::Error::other(
            "private artifact files are unavailable",
        ))
    }
    pub(super) fn remove_file(_: &PinnedPrivateDirectory, _: &str) -> std::io::Result<()> {
        Err(std::io::Error::other(
            "private artifact files are unavailable",
        ))
    }
    pub(super) fn publish_noreplace(
        _: &PinnedPrivateDirectory,
        _: &str,
        _: &str,
    ) -> std::io::Result<()> {
        Err(std::io::Error::other(
            "private artifact files are unavailable",
        ))
    }
}
