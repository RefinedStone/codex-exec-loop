use crate::application::port::outbound::parallel_mode_runtime_port::ParallelPoolMutationPermit;
use rand::RngCore;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

const POOL_MUTATION_LOCK_FILE: &str = ".allocation-lock";

struct GitParallelPoolMutationPermit {
    platform_lock: platform::PlatformPoolMutationLock,
    lock_path: PathBuf,
    pool_root: PathBuf,
}

impl ParallelPoolMutationPermit for GitParallelPoolMutationPermit {
    fn verify_pool_root(&self, pool_root: &Path) -> Result<(), String> {
        if self.pool_root != pool_root {
            return Err(format!(
                "pool mutation permit for `{}` cannot mutate `{}`",
                self.pool_root.display(),
                pool_root.display()
            ));
        }
        self.platform_lock
            .verify_paths(pool_root, &self.lock_path)
            .map_err(|error| format!("pool mutation permit identity changed: {error}"))
    }
}

pub(super) fn acquire_pool_mutation_permit(
    canonical_repo_root: &Path,
    pool_root: &Path,
    timeout: Duration,
    retry_delay: Duration,
) -> Result<Box<dyn ParallelPoolMutationPermit>, String> {
    platform::ensure_private_pool_root(canonical_repo_root, pool_root)
        .map_err(|error| format!("pool root could not be created before mutation lock: {error}"))?;
    acquire_pool_mutation_permit_at(pool_root, timeout, retry_delay)
}

fn acquire_pool_mutation_permit_at(
    pool_root: &Path,
    timeout: Duration,
    retry_delay: Duration,
) -> Result<Box<dyn ParallelPoolMutationPermit>, String> {
    let lock_path = pool_root.join(POOL_MUTATION_LOCK_FILE);
    let deadline = Instant::now() + timeout;
    let owner_record = pool_mutation_lock_owner_record()?;
    loop {
        match try_acquire_pool_mutation_permit_at_with_owner(pool_root, &lock_path, &owner_record)?
        {
            Some(pool_mutation_permit) => return Ok(Box::new(pool_mutation_permit)),
            None if Instant::now() < deadline => thread::sleep(retry_delay),
            None => {
                return Err(format!(
                    "pool mutation lock is busy at `{}`",
                    lock_path.display()
                ));
            }
        }
    }
}

pub(super) fn try_acquire_pool_mutation_permit(
    pool_root: &Path,
) -> Result<Option<Box<dyn ParallelPoolMutationPermit>>, String> {
    let lock_path = pool_root.join(POOL_MUTATION_LOCK_FILE);
    let owner_record = pool_mutation_lock_owner_record()?;
    try_acquire_pool_mutation_permit_at_with_owner(pool_root, &lock_path, &owner_record)
        .map(|permit| permit.map(|permit| Box::new(permit) as Box<dyn ParallelPoolMutationPermit>))
}

fn try_acquire_pool_mutation_permit_at_with_owner(
    pool_root: &Path,
    lock_path: &Path,
    owner_record: &str,
) -> Result<Option<GitParallelPoolMutationPermit>, String> {
    platform::try_acquire(pool_root, lock_path, owner_record)
        .map(|platform_lock| {
            platform_lock.map(|platform_lock| GitParallelPoolMutationPermit {
                platform_lock,
                lock_path: lock_path.to_path_buf(),
                pool_root: pool_root.to_path_buf(),
            })
        })
        .map_err(|error| {
            format!(
                "pool mutation lock could not be acquired at `{}`: {error}",
                lock_path.display()
            )
        })
}

#[cfg(test)]
fn acquire_pool_mutation_lock_at(
    pool_root: &Path,
) -> Result<Box<dyn ParallelPoolMutationPermit>, String> {
    acquire_pool_mutation_permit_at(
        pool_root,
        Duration::from_secs(120),
        Duration::from_millis(25),
    )
}

fn pool_mutation_lock_owner_record() -> Result<String, String> {
    let mut nonce = [0_u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut nonce)
        .map_err(|error| {
            format!("operating-system randomness is required for pool locking: {error}")
        })?;
    let process_start_identity =
        crate::process_liveness::required_process_start_identity(std::process::id())
            .map_err(|error| format!("pool lock process identity is unavailable: {error:#}"))?;
    let nonce = nonce
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!(
        "pid={}\nprocess_start_identity={}\nnonce={}\n",
        std::process::id(),
        process_start_identity,
        nonce
    ))
}

fn write_owner_record(file: &mut std::fs::File, owner_record: &str) -> std::io::Result<()> {
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(owner_record.as_bytes())?;
    file.sync_all()
}

fn pool_root_creation_chain(
    canonical_repo_root: &Path,
    pool_root: &Path,
) -> std::io::Result<Vec<PathBuf>> {
    let parent = canonical_repo_root.parent().unwrap_or(canonical_repo_root);
    let relative = pool_root.strip_prefix(parent).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "pool root must be beneath the canonical repository parent",
        )
    })?;
    let mut current = parent.to_path_buf();
    let mut chain = Vec::new();
    for component in relative.components() {
        let std::path::Component::Normal(component) = component else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "pool root creation chain contains a non-normal component",
            ));
        };
        current.push(component);
        chain.push(current.clone());
    }
    if chain.len() != 3 || chain.last().is_none_or(|path| path != pool_root) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "pool root must contain the repository pool, identity, and leaf components",
        ));
    }
    Ok(chain)
}

#[cfg(unix)]
mod platform {
    use super::{pool_root_creation_chain, write_owner_record};
    use std::fs::{DirBuilder, File, OpenOptions};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::path::Path;

    pub(super) struct PlatformPoolMutationLock {
        file: File,
        root_device: u64,
        root_inode: u64,
        lock_device: u64,
        lock_inode: u64,
    }

    impl PlatformPoolMutationLock {
        pub(super) fn verify_paths(
            &self,
            pool_root: &Path,
            lock_path: &Path,
        ) -> std::io::Result<()> {
            let root = open_private_pool_root(pool_root)?;
            let root_metadata = root.metadata()?;
            let lock_metadata = std::fs::symlink_metadata(lock_path)?;
            if root_metadata.dev() != self.root_device
                || root_metadata.ino() != self.root_inode
                || lock_metadata.dev() != self.lock_device
                || lock_metadata.ino() != self.lock_inode
                || self.file.metadata()?.dev() != self.lock_device
                || self.file.metadata()?.ino() != self.lock_inode
            {
                return Err(std::io::Error::other(
                    "pool root or mutation lock path no longer identifies the acquired object",
                ));
            }
            validate_lock_metadata(&lock_metadata)
        }
    }

    impl Drop for PlatformPoolMutationLock {
        fn drop(&mut self) {
            // SAFETY: the descriptor remains owned by self for the duration of this call.
            unsafe {
                libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
            }
        }
    }

    pub(super) fn ensure_private_pool_root(
        canonical_repo_root: &Path,
        pool_root: &Path,
    ) -> std::io::Result<()> {
        let creation_parent = canonical_repo_root.parent().unwrap_or(canonical_repo_root);
        let creation_parent_handle = open_trusted_creation_parent(creation_parent)?;
        for path in pool_root_creation_chain(canonical_repo_root, pool_root)? {
            match DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
            open_private_pool_root(&path)?;
        }
        validate_trusted_creation_parent(creation_parent, &creation_parent_handle)
    }

    pub(super) fn try_acquire(
        pool_root: &Path,
        lock_path: &Path,
        owner_record: &str,
    ) -> std::io::Result<Option<PlatformPoolMutationLock>> {
        let root = open_private_pool_root(pool_root)?;
        let root_metadata = root.metadata()?;
        reject_legacy_lock_directory(lock_path)?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(lock_path)?;
        let metadata = file.metadata()?;
        validate_lock_metadata(&metadata)?;
        // SAFETY: flock only observes the live owned descriptor and does not retain a Rust pointer.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EWOULDBLOCK)
                || error.raw_os_error() == Some(libc::EAGAIN)
            {
                return Ok(None);
            }
            return Err(error);
        }
        let path_metadata = std::fs::symlink_metadata(lock_path)?;
        if path_metadata.dev() != metadata.dev() || path_metadata.ino() != metadata.ino() {
            return Err(std::io::Error::other(
                "pool mutation lock path changed during acquisition",
            ));
        }
        write_owner_record(&mut file, owner_record)?;
        let lock = PlatformPoolMutationLock {
            file,
            root_device: root_metadata.dev(),
            root_inode: root_metadata.ino(),
            lock_device: metadata.dev(),
            lock_inode: metadata.ino(),
        };
        lock.verify_paths(pool_root, lock_path)?;
        Ok(Some(lock))
    }

    fn open_private_pool_root(pool_root: &Path) -> std::io::Result<File> {
        let root = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(pool_root)?;
        let metadata = root.metadata()?;
        if !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o022 != 0
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "pool root must be an owner-controlled non-writable-by-others directory",
            ));
        }
        Ok(root)
    }

    fn open_trusted_creation_parent(path: &Path) -> std::io::Result<File> {
        let parent = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(path)?;
        validate_trusted_creation_parent(path, &parent)?;
        Ok(parent)
    }

    fn validate_trusted_creation_parent(path: &Path, parent: &File) -> std::io::Result<()> {
        let opened = parent.metadata()?;
        let current = std::fs::symlink_metadata(path)?;
        let owner_is_trusted = opened.uid() == unsafe { libc::geteuid() } || opened.uid() == 0;
        let mode = opened.permissions().mode();
        let entry_mutation_is_restricted = mode & 0o022 == 0 || mode & 0o1000 != 0;
        if !opened.is_dir()
            || opened.dev() != current.dev()
            || opened.ino() != current.ino()
            || !owner_is_trusted
            || !entry_mutation_is_restricted
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "pool creation parent must be root/current-user owned and either non-writable by others or sticky",
            ));
        }
        Ok(())
    }

    fn validate_lock_metadata(metadata: &std::fs::Metadata) -> std::io::Result<()> {
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "pool mutation lock must be an owner-private single-link regular file",
            ));
        }
        Ok(())
    }

    fn reject_legacy_lock_directory(lock_path: &Path) -> std::io::Result<()> {
        match std::fs::symlink_metadata(lock_path) {
            Ok(metadata) if metadata.file_type().is_dir() => Err(std::io::Error::other(
                "legacy `.allocation-lock` directory found; confirm no older Akra process is running, remove that directory, and retry",
            )),
            Ok(_) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::{pool_root_creation_chain, write_owner_record};
    use crate::private_fs::{
        WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT, WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ,
        WINDOWS_GENERIC_WRITE, WINDOWS_READ_CONTROL, WINDOWS_WRITE_DAC, set_windows_private_acl,
        validate_windows_path_identity, validate_windows_path_identity_only,
        validate_windows_private_owner_and_acl, validate_windows_trusted_executable_acl,
    };
    use std::fs::{File, OpenOptions};
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;
    use windows_sys::Win32::Foundation::{ERROR_LOCK_VIOLATION, HANDLE};
    use windows_sys::Win32::Storage::FileSystem::{LockFile, UnlockFile};

    pub(super) struct PlatformPoolMutationLock {
        file: File,
        root: File,
    }

    impl PlatformPoolMutationLock {
        pub(super) fn verify_paths(
            &self,
            pool_root: &Path,
            lock_path: &Path,
        ) -> std::io::Result<()> {
            validate_windows_path_identity(pool_root, &self.root, true)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            validate_windows_private_owner_and_acl(pool_root, &self.root)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            validate_windows_path_identity(lock_path, &self.file, false)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            validate_windows_private_owner_and_acl(lock_path, &self.file)
                .map_err(|error| std::io::Error::other(error.to_string()))
        }
    }

    impl Drop for PlatformPoolMutationLock {
        fn drop(&mut self) {
            // SAFETY: the owned file handle remains valid and the same byte range was locked.
            unsafe {
                UnlockFile(self.file.as_raw_handle() as HANDLE, 0, 0, 1, 0);
            }
        }
    }

    pub(super) fn ensure_private_pool_root(
        canonical_repo_root: &Path,
        pool_root: &Path,
    ) -> std::io::Result<()> {
        let creation_parent = canonical_repo_root.parent().unwrap_or(canonical_repo_root);
        let creation_parent_handle = open_trusted_creation_parent(creation_parent)?;
        for path in pool_root_creation_chain(canonical_repo_root, pool_root)? {
            let created = match std::fs::create_dir(&path) {
                Ok(()) => true,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
                Err(error) => return Err(error),
            };
            let directory = OpenOptions::new()
                .read(true)
                .access_mode(
                    WINDOWS_GENERIC_READ
                        | WINDOWS_READ_CONTROL
                        | if created { WINDOWS_WRITE_DAC } else { 0 },
                )
                .share_mode(WINDOWS_FILE_SHARE_ALL)
                .custom_flags(
                    crate::private_fs::WINDOWS_FILE_FLAG_BACKUP_SEMANTICS
                        | WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
                )
                .open(&path)?;
            validate_windows_path_identity(&path, &directory, true)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            if created {
                set_windows_private_acl(&directory, true)
                    .map_err(|error| std::io::Error::other(error.to_string()))?;
            }
            validate_windows_private_owner_and_acl(&path, &directory)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            validate_windows_path_identity(&path, &directory, true)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
        }
        validate_windows_path_identity_only(creation_parent, &creation_parent_handle, true)
            .map_err(|error| std::io::Error::other(error.to_string()))
    }

    pub(super) fn try_acquire(
        pool_root: &Path,
        lock_path: &Path,
        owner_record: &str,
    ) -> std::io::Result<Option<PlatformPoolMutationLock>> {
        let root = OpenOptions::new()
            .read(true)
            .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
            .share_mode(WINDOWS_FILE_SHARE_ALL)
            .custom_flags(
                crate::private_fs::WINDOWS_FILE_FLAG_BACKUP_SEMANTICS
                    | WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
            )
            .open(pool_root)?;
        validate_windows_path_identity(pool_root, &root, true)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        validate_windows_private_owner_and_acl(pool_root, &root)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        reject_legacy_lock_directory(lock_path)?;

        let mut create_options = OpenOptions::new();
        create_options
            .read(true)
            .write(true)
            .create_new(true)
            .access_mode(
                WINDOWS_GENERIC_READ
                    | WINDOWS_GENERIC_WRITE
                    | WINDOWS_READ_CONTROL
                    | WINDOWS_WRITE_DAC,
            )
            .share_mode(WINDOWS_FILE_SHARE_ALL)
            .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT);
        let (mut file, created) = match create_options.open(lock_path) {
            Ok(file) => (file, true),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .access_mode(
                        WINDOWS_GENERIC_READ | WINDOWS_GENERIC_WRITE | WINDOWS_READ_CONTROL,
                    )
                    .share_mode(WINDOWS_FILE_SHARE_ALL)
                    .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT)
                    .open(lock_path)?,
                false,
            ),
            Err(error) => return Err(error),
        };
        validate_windows_path_identity(lock_path, &file, false)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        if created {
            set_windows_private_acl(&file, false)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
        }
        validate_windows_private_owner_and_acl(lock_path, &file)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        validate_windows_path_identity(lock_path, &file, false)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        // SAFETY: LockFile receives a valid owned handle and a one-byte range at offset zero.
        if unsafe { LockFile(file.as_raw_handle() as HANDLE, 0, 0, 1, 0) } == 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32) {
                return Ok(None);
            }
            return Err(error);
        }
        validate_windows_path_identity(lock_path, &file, false)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        validate_windows_private_owner_and_acl(lock_path, &file)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        write_owner_record(&mut file, owner_record)?;
        let lock = PlatformPoolMutationLock { file, root };
        lock.verify_paths(pool_root, lock_path)?;
        Ok(Some(lock))
    }

    fn reject_legacy_lock_directory(lock_path: &Path) -> std::io::Result<()> {
        match std::fs::symlink_metadata(lock_path) {
            Ok(metadata) if metadata.file_type().is_dir() => Err(std::io::Error::other(
                "legacy `.allocation-lock` directory found; confirm no older Akra process is running, remove that directory, and retry",
            )),
            Ok(_) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn open_trusted_creation_parent(path: &Path) -> std::io::Result<File> {
        let parent = OpenOptions::new()
            .read(true)
            .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
            .share_mode(WINDOWS_FILE_SHARE_ALL)
            .custom_flags(
                crate::private_fs::WINDOWS_FILE_FLAG_BACKUP_SEMANTICS
                    | WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
            )
            .open(path)?;
        validate_windows_path_identity_only(path, &parent, true)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        validate_windows_trusted_executable_acl(path, &parent, true)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        validate_windows_path_identity_only(path, &parent, true)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        Ok(parent)
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    use std::path::Path;

    pub(super) struct PlatformPoolMutationLock;

    impl PlatformPoolMutationLock {
        pub(super) fn verify_paths(
            &self,
            _pool_root: &Path,
            _lock_path: &Path,
        ) -> std::io::Result<()> {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "pool mutation locking is unsupported on this platform",
            ))
        }
    }

    pub(super) fn ensure_private_pool_root(
        _canonical_repo_root: &Path,
        _pool_root: &Path,
    ) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "pool mutation locking is unsupported on this platform",
        ))
    }

    pub(super) fn try_acquire(
        _pool_root: &Path,
        _lock_path: &Path,
        _owner_record: &str,
    ) -> std::io::Result<Option<PlatformPoolMutationLock>> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "pool mutation locking is unsupported on this platform",
        ))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{POOL_MUTATION_LOCK_FILE, acquire_pool_mutation_lock_at, platform};
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    fn private_pool_root(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "akra-pool-lock-{label}-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("private pool root should create");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .expect("private pool root should be owner-only");
        path
    }

    #[test]
    fn mutation_lock_rejects_owner_path_symlink_without_touching_target() {
        let pool_root = private_pool_root("symlink");
        let sentinel = pool_root.parent().unwrap().join(format!(
            "akra-pool-lock-sentinel-{}",
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&sentinel, "sentinel").expect("sentinel should write");
        symlink(&sentinel, pool_root.join(POOL_MUTATION_LOCK_FILE))
            .expect("lock symlink should create");

        let error = acquire_pool_mutation_lock_at(&pool_root)
            .err()
            .expect("symlink lock path must be rejected");
        assert!(error.contains("could not be acquired"));
        assert_eq!(fs::read_to_string(&sentinel).unwrap(), "sentinel");
        let _ = fs::remove_dir_all(pool_root);
        let _ = fs::remove_file(sentinel);
    }

    #[test]
    fn private_pool_setup_creates_and_validates_the_full_missing_ancestor_chain() {
        let fixture_root = private_pool_root("missing-chain");
        let canonical_repo_root = fixture_root.join("repo");
        fs::create_dir(&canonical_repo_root).expect("canonical repository root should create");
        fs::set_permissions(&canonical_repo_root, fs::Permissions::from_mode(0o700))
            .expect("canonical repository root should be private");
        let pool_root = fixture_root
            .join("repo-akra-worktrees")
            .join("0123456789ab")
            .join("akra-pool");

        platform::ensure_private_pool_root(&canonical_repo_root, &pool_root)
            .expect("fresh pool setup should create every missing ancestor");

        for path in [
            fixture_root.join("repo-akra-worktrees"),
            fixture_root
                .join("repo-akra-worktrees")
                .join("0123456789ab"),
            pool_root,
        ] {
            let metadata = fs::metadata(path).expect("pool chain component should exist");
            assert!(metadata.is_dir());
            assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
            assert_eq!(metadata.permissions().mode() & 0o077, 0);
        }
        let _ = fs::remove_dir_all(fixture_root);
    }

    #[test]
    fn private_pool_setup_rejects_a_non_sticky_shared_creation_parent() {
        let fixture_root = private_pool_root("unsafe-shared-parent");
        let shared_parent = fixture_root.join("shared");
        fs::create_dir(&shared_parent).expect("shared creation parent should create");
        fs::set_permissions(&shared_parent, fs::Permissions::from_mode(0o777))
            .expect("shared creation parent should become writable by other users");
        let canonical_repo_root = shared_parent.join("repo");
        fs::create_dir(&canonical_repo_root).expect("canonical repository root should create");
        fs::set_permissions(&canonical_repo_root, fs::Permissions::from_mode(0o700))
            .expect("canonical repository root should be private");
        let pool_root = shared_parent
            .join("repo-akra-worktrees")
            .join("0123456789ab")
            .join("akra-pool");

        let error = platform::ensure_private_pool_root(&canonical_repo_root, &pool_root)
            .expect_err("non-sticky shared parent must not authorize pool entries");

        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("pool creation parent"));
        assert!(!pool_root.exists());
        let _ = fs::remove_dir_all(fixture_root);
    }

    #[test]
    fn dropping_permits_never_deletes_or_replaces_the_persistent_lock_object() {
        let pool_root = private_pool_root("persistent");
        let first = acquire_pool_mutation_lock_at(&pool_root).expect("first lock should acquire");
        let lock_path = pool_root.join(POOL_MUTATION_LOCK_FILE);
        let first_metadata = fs::metadata(&lock_path).expect("lock file should exist");
        drop(first);
        let second = acquire_pool_mutation_lock_at(&pool_root).expect("second lock should acquire");
        let second_metadata = fs::metadata(&lock_path).expect("lock file should remain");
        assert_eq!(first_metadata.dev(), second_metadata.dev());
        assert_eq!(first_metadata.ino(), second_metadata.ino());
        drop(second);
        assert!(lock_path.is_file());
        let _ = fs::remove_dir_all(pool_root);
    }

    #[test]
    fn mutation_permit_rejects_a_different_pool_root() {
        let acquired_root = private_pool_root("scope-acquired");
        let replacement_root = private_pool_root("scope-replacement");
        let permit =
            acquire_pool_mutation_lock_at(&acquired_root).expect("mutation lock should acquire");

        let error = permit
            .verify_pool_root(&replacement_root)
            .expect_err("one repository permit must not authorize another pool root");

        assert!(error.contains("cannot mutate"));
        assert!(!replacement_root.join(POOL_MUTATION_LOCK_FILE).exists());
        drop(permit);
        let _ = fs::remove_dir_all(acquired_root);
        let _ = fs::remove_dir_all(replacement_root);
    }

    #[test]
    fn legacy_directory_lock_fails_with_explicit_recovery_guidance() {
        let pool_root = private_pool_root("legacy-directory");
        fs::create_dir(pool_root.join(POOL_MUTATION_LOCK_FILE))
            .expect("legacy lock directory should create");

        let error = acquire_pool_mutation_lock_at(&pool_root)
            .err()
            .expect("legacy lock directory must not be migrated while ownership is ambiguous");

        assert!(error.contains("legacy `.allocation-lock` directory"));
        assert!(error.contains("older Akra process"));
        let _ = fs::remove_dir_all(pool_root);
    }
}
