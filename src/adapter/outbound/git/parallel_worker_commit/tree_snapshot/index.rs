use super::*;
use crate::private_sqlite::PinnedPrivateDirectory;
use std::fs::File;
#[cfg(windows)]
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom, Write};

const MAX_INDEX_BYTES: u64 = 16 * 1024 * 1024;

pub(super) struct OriginalIndex {
    directory: PinnedPrivateDirectory,
    name: String,
    file: File,
    identity: String,
    bytes: Vec<u8>,
}

impl OriginalIndex {
    pub(super) fn open(workspace: &Path) -> Result<Self, String> {
        let path = checked_git_text(workspace, ["rev-parse", "--git-path", "index"], None)?;
        let path = absolutize(workspace, Path::new(&path));
        let parent = path.parent().ok_or("Git index has no parent")?;
        let directory =
            PinnedPrivateDirectory::pin_ancestors(parent).map_err(|error| error.to_string())?;
        let name = path
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or("Git index filename is not UTF-8")?
            .to_string();
        directory
            .file_path(&name)
            .map_err(|error| error.to_string())?;
        let mut file = open_index(&directory, &name, false)?;
        validate_index_file(&directory, &name, &file)?;
        let identity = file_identity(&file)?;
        let bytes = read_index(&mut file)?;
        let mut original = Self {
            directory,
            name,
            file,
            identity,
            bytes,
        };
        original.verify()?;
        Ok(original)
    }

    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(super) fn verify(&mut self) -> Result<(), String> {
        validate_index_file(&self.directory, &self.name, &self.file)?;
        if file_identity(&self.file)? != self.identity
            || read_index(&mut self.file)? != self.bytes
            || file_identity(&self.file)? != self.identity
        {
            return Err("the real Git index changed during evidence observation".into());
        }
        validate_index_file(&self.directory, &self.name, &self.file)
    }
}

pub(super) struct TemporaryIndex {
    directory: Option<PinnedPrivateDirectory>,
    path: PathBuf,
}

impl TemporaryIndex {
    pub(super) fn new(bytes: &[u8]) -> Result<Self, String> {
        use rand::RngCore;
        let mut random = [0u8; 32];
        rand::rngs::OsRng
            .try_fill_bytes(&mut random)
            .map_err(|_| "secure temporary index randomness is unavailable")?;
        let name = format!(
            "akra-evidence-index-{}",
            random
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let parent =
            std::fs::canonicalize(std::env::temp_dir()).map_err(|error| error.to_string())?;
        let parent =
            PinnedPrivateDirectory::pin_ancestors(&parent).map_err(|error| error.to_string())?;
        let directory = parent
            .child(&name, true)
            .map_err(|error| error.to_string())?;
        let path = directory
            .file_path("index")
            .map_err(|error| error.to_string())?;
        let temporary = Self {
            directory: Some(directory),
            path,
        };
        let directory = temporary
            .directory
            .as_ref()
            .expect("live temporary index directory");
        let mut file = open_index(directory, "index", true)?;
        file.write_all(bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        validate_index_file(directory, "index", &file)?;
        directory.sync().map_err(|error| error.to_string())?;
        Ok(temporary)
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn verify(&self) -> Result<(), String> {
        let directory = self
            .directory
            .as_ref()
            .expect("live temporary index directory");
        directory.verify().map_err(|error| error.to_string())?;
        let file = open_index(directory, "index", false)?;
        validate_index_file(directory, "index", &file)
    }
}

impl Drop for TemporaryIndex {
    fn drop(&mut self) {
        let Some(directory) = self.directory.take() else {
            return;
        };
        let path = directory.path().to_path_buf();
        if directory.verify().is_err() {
            return;
        }
        // Never recursively delete an unexpected entry. Git is reaped before this
        // guard drops; these are the only two names the temporary-index path owns.
        for name in ["index.lock", "index"] {
            let _ = remove_index(&directory, name);
        }
        let _ = directory.sync();
        drop(directory);
        let _ = std::fs::remove_dir(path);
    }
}

fn read_index(file: &mut File) -> Result<Vec<u8>, String> {
    if file.metadata().map_err(|error| error.to_string())?.len() > MAX_INDEX_BYTES {
        return Err("the Git index exceeds the 16 MiB evidence bound".into());
    }
    file.seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_INDEX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_INDEX_BYTES {
        return Err("the Git index is empty or exceeds the evidence bound".into());
    }
    Ok(bytes)
}

fn validate_index_file(
    directory: &PinnedPrivateDirectory,
    name: &str,
    file: &File,
) -> Result<(), String> {
    directory.verify().map_err(|error| error.to_string())?;
    let path = directory
        .file_path(name)
        .map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_INDEX_BYTES {
        return Err("Git index must be a bounded regular file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid has no preconditions.
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
            || metadata.mode() & 0o022 != 0
        {
            return Err("Git index ownership, links, or permissions are unsafe".into());
        }
        let current = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if current.file_type().is_symlink()
            || metadata.dev() != current.dev()
            || metadata.ino() != current.ino()
        {
            return Err("Git index path identity changed".into());
        }
    }
    #[cfg(windows)]
    crate::private_fs::validate_windows_path_identity(&path, file, false)
        .map_err(|error| error.to_string())?;
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        return Err("Git index identity is unavailable".into());
    }
    directory.verify().map_err(|error| error.to_string())
}

fn file_identity(file: &File) -> Result<String, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let value = file.metadata().map_err(|error| error.to_string())?;
        Ok(format!(
            "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
            value.dev(),
            value.ino(),
            value.mode(),
            value.uid(),
            value.nlink(),
            value.len(),
            value.mtime(),
            value.mtime_nsec(),
            value.ctime(),
            value.ctime_nsec()
        ))
    }
    #[cfg(windows)]
    {
        crate::private_fs::windows_file_identity_snapshot(file)
            .map(|value| format!("{value:?}"))
            .map_err(|error| error.to_string())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        Err("Git index identity is unavailable".into())
    }
}

fn open_index(
    directory: &PinnedPrivateDirectory,
    name: &str,
    create: bool,
) -> Result<File, String> {
    #[cfg(unix)]
    {
        use std::os::fd::{AsRawFd, FromRawFd};
        let name = std::ffi::CString::new(name).map_err(|error| error.to_string())?;
        let flags = libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | libc::O_CLOEXEC
            | if create {
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL
            } else {
                libc::O_RDONLY
            };
        // SAFETY: pinned descriptor and validated child name remain live for openat.
        let fd =
            unsafe { libc::openat(directory.handle().as_raw_fd(), name.as_ptr(), flags, 0o600) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        // SAFETY: openat returned one owned descriptor.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    #[cfg(windows)]
    {
        use crate::private_fs::*;
        use std::os::windows::fs::OpenOptionsExt;
        let file = OpenOptions::new()
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
            .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT)
            .open(directory.path().join(name))
            .map_err(|error| error.to_string())?;
        if create {
            set_windows_private_acl(&file, false).map_err(|error| error.to_string())?;
        }
        Ok(file)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (directory, name, create);
        Err("private temporary indexes are unavailable".into())
    }
}

fn remove_index(directory: &PinnedPrivateDirectory, name: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let name = std::ffi::CString::new(name)?;
        // SAFETY: only a fixed owned child name is unlinked relative to the pinned directory.
        if unsafe { libc::unlinkat(directory.handle().as_raw_fd(), name.as_ptr(), 0) } == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
    #[cfg(not(unix))]
    {
        std::fs::remove_file(directory.path().join(name))
    }
}
