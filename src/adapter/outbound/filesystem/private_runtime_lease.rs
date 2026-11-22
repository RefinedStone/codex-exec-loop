//! Private runtime의 process-shared lease. The lock inode stays in place after release.
//! Every path component is pinned without following links; Unix-only until secure_fs supports more.
#[cfg(unix)]
mod implementation {
    use std::ffi::{CString, OsStr};
    use std::fs::{File, Metadata, OpenOptions, TryLockError};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::path::{Component, Path, PathBuf};
    use std::time::{Duration, Instant};

    use anyhow::{Context, Result, bail};

    const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(25);

    pub(crate) struct PrivateRuntimeLease {
        runtime: PathBuf,
        lock_name: String,
        root: File,
        lock: File,
    }

    impl PrivateRuntimeLease {
        pub(crate) fn acquire(
            runtime: PathBuf,
            lock_name: impl Into<String>,
            timeout: Duration,
        ) -> Result<Self> {
            let lock_name = lock_name.into();
            if Path::new(&lock_name).components().count() != 1
                || !matches!(
                    Path::new(&lock_name).components().next(),
                    Some(Component::Normal(_))
                )
            {
                bail!("the issue persistence lease name is invalid")
            }
            let root = open_private_runtime(&runtime)?;
            let name = CString::new(lock_name.as_str())
                .context("the issue persistence lease name is invalid")?;
            // SAFETY: the parent descriptor and name remain valid. NONBLOCK prevents FIFO
            // waits and NOFOLLOW plus descriptor checks reject symbolic and hard-link aliases.
            let raw = unsafe {
                libc::openat(
                    root.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDWR
                        | libc::O_CREAT
                        | libc::O_CLOEXEC
                        | libc::O_NOFOLLOW
                        | libc::O_NONBLOCK,
                    0o600,
                )
            };
            if raw < 0 {
                return Err(std::io::Error::last_os_error())
                    .context("the issue persistence lease could not be opened");
            }
            // SAFETY: successful openat transferred one new owned descriptor.
            let lock = unsafe { File::from_raw_fd(raw) };
            validate_lock(&root, &lock, &lock_name)?;
            root.sync_all()
                .context("the issue persistence lease could not be persisted")?;
            let started = Instant::now();
            loop {
                match lock.try_lock() {
                    Ok(()) => break,
                    Err(TryLockError::WouldBlock) => {
                        if started.elapsed() >= timeout {
                            bail!("another issue persistence transaction is still running")
                        }
                        std::thread::sleep(
                            LOCK_RETRY_INTERVAL.min(timeout.saturating_sub(started.elapsed())),
                        );
                    }
                    Err(TryLockError::Error(error)) => {
                        return Err(error)
                            .context("the issue persistence lease could not be acquired");
                    }
                }
            }
            let lease = Self {
                runtime,
                lock_name,
                root,
                lock,
            };
            lease.verify_identity()?;
            Ok(lease)
        }

        pub(crate) fn verify_identity(&self) -> Result<()> {
            let current = open_private_runtime(&self.runtime)?;
            if !same_object(&current.metadata()?, &self.root.metadata()?) {
                bail!("the issue persistence runtime directory changed")
            }
            validate_lock(&self.root, &self.lock, &self.lock_name)
        }
    }

    impl Drop for PrivateRuntimeLease {
        fn drop(&mut self) {
            // Explicitly release at guard drop even if a child inherited a handle.
            if let Err(error) = self.lock.unlock() {
                tracing::warn!(error = %error, "issue persistence lease release failed");
            }
        }
    }

    fn same_object(first: &Metadata, second: &Metadata) -> bool {
        first.dev() == second.dev() && first.ino() == second.ino()
    }

    /// Resolve every runtime component from a pinned descriptor, rejecting directory symlinks.
    fn open_private_runtime(path: &Path) -> Result<File> {
        if !path.is_absolute() {
            bail!("the issue persistence runtime must be an absolute directory")
        }
        let mut directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open("/")
            .context("the issue persistence runtime root could not be opened")?;
        for component in path.components() {
            let Component::Normal(name) = component else {
                if component == Component::RootDir {
                    continue;
                }
                bail!("the issue persistence runtime contains a noncanonical component")
            };
            let name = CString::new(name.as_bytes())
                .context("the issue persistence runtime contains an invalid component")?;
            // SAFETY: each component is opened relative to the preceding live directory.
            let raw = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                )
            };
            if raw < 0 {
                return Err(std::io::Error::last_os_error())
                    .context("the issue persistence runtime contains an unavailable directory");
            }
            // SAFETY: a successful openat gives a new owned directory descriptor.
            directory = unsafe { File::from_raw_fd(raw) };
        }
        let metadata = directory.metadata()?;
        // SAFETY: geteuid has no preconditions.
        let current_user = unsafe { libc::geteuid() };
        if !metadata.is_dir() || metadata.uid() != current_user || metadata.mode() & 0o077 != 0 {
            bail!("the issue persistence runtime is not a private directory")
        }
        Ok(directory)
    }

    // libc device/inode widths differ between Unix targets. MetadataExt normalizes to u64.
    #[allow(clippy::unnecessary_cast)]
    fn validate_lock(root: &File, lock: &File, lock_name: &str) -> Result<()> {
        let metadata = lock.metadata()?;
        // SAFETY: geteuid has no preconditions.
        let current_user = unsafe { libc::geteuid() };
        if !metadata.is_file()
            || metadata.uid() != current_user
            || metadata.nlink() != 1
            || metadata.mode() & 0o077 != 0
        {
            bail!("the issue persistence lease is not a private regular file")
        }
        let name = CString::new(OsStr::new(lock_name).as_bytes()).expect("constant has no NUL");
        let mut path_metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: the directory and C string live across the call and stat is writable.
        let result = unsafe {
            libc::fstatat(
                root.as_raw_fd(),
                name.as_ptr(),
                path_metadata.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result != 0 {
            return Err(std::io::Error::last_os_error())
                .context("the issue persistence lease disappeared");
        }
        // SAFETY: successful fstatat initialized the entire stat object.
        let path_metadata = unsafe { path_metadata.assume_init() };
        if metadata.dev() != path_metadata.st_dev as u64
            || metadata.ino() != path_metadata.st_ino as u64
            || path_metadata.st_mode & libc::S_IFMT != libc::S_IFREG
            || path_metadata.st_nlink != 1
        {
            bail!("the issue persistence lease changed identity")
        }
        Ok(())
    }
}

#[cfg(not(unix))]
mod implementation {
    use std::path::PathBuf;
    use std::time::Duration;

    use anyhow::{Result, bail};

    pub(crate) struct PrivateRuntimeLease;

    impl PrivateRuntimeLease {
        pub(crate) fn acquire(_: PathBuf, _: impl Into<String>, _: Duration) -> Result<Self> {
            bail!("the issue persistence lease is unavailable on this platform")
        }

        pub(crate) fn verify_identity(&self) -> Result<()> {
            bail!("the issue persistence lease is unavailable on this platform")
        }
    }
}

pub(crate) use implementation::PrivateRuntimeLease;
