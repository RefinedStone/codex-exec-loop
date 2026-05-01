use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;

use super::super::ensure_directory_exists;
use super::{derive_default_pool_root, detect_canonical_repo_root};

const POOL_ALLOCATION_LOCK_DIR: &str = ".allocation-lock";
const POOL_ALLOCATION_LOCK_OWNER_FILE: &str = "owner";
const POOL_ALLOCATION_LOCK_TIMEOUT: Duration = Duration::from_secs(10);
const POOL_ALLOCATION_LOCK_RETRY: Duration = Duration::from_millis(25);
const POOL_ALLOCATION_LOCK_STALE_AFTER: Duration = Duration::from_secs(300);

pub(in crate::application::service::parallel_mode) struct PoolAllocationLock {
    lock_path: PathBuf,
    owner_token: String,
}

impl Drop for PoolAllocationLock {
    fn drop(&mut self) {
        release_pool_allocation_lock(&self.lock_path, &self.owner_token);
    }
}

pub(in crate::application::service::parallel_mode) fn acquire_pool_allocation_lock(
    planning_authority: &dyn PlanningAuthorityPort,
    workspace_dir: &str,
) -> Result<PoolAllocationLock, String> {
    let canonical_repo_root = detect_canonical_repo_root(planning_authority, workspace_dir)
        .ok_or_else(|| "canonical root inspection failed".to_string())?;
    let pool_root = derive_default_pool_root(&canonical_repo_root);
    ensure_directory_exists(&pool_root)
        .map_err(|error| format!("pool root creation failed before allocation lock: {error}"))?;
    acquire_pool_allocation_lock_at(&pool_root)
}

fn acquire_pool_allocation_lock_at(pool_root: &Path) -> Result<PoolAllocationLock, String> {
    let lock_path = pool_root.join(POOL_ALLOCATION_LOCK_DIR);
    let deadline = Instant::now() + POOL_ALLOCATION_LOCK_TIMEOUT;
    let owner_token = pool_allocation_lock_owner_token();

    loop {
        match fs::create_dir(&lock_path) {
            Ok(()) => {
                if let Err(error) = fs::write(
                    lock_path.join(POOL_ALLOCATION_LOCK_OWNER_FILE),
                    &owner_token,
                ) {
                    let _ = fs::remove_dir_all(&lock_path);
                    return Err(format!(
                        "pool allocation lock owner could not be written at `{}`: {error}",
                        lock_path.display()
                    ));
                }
                return Ok(PoolAllocationLock {
                    lock_path,
                    owner_token,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                remove_stale_pool_allocation_lock(&lock_path);
                if Instant::now() >= deadline {
                    return Err(format!(
                        "pool allocation lock is busy at `{}`",
                        lock_path.display()
                    ));
                }
                thread::sleep(POOL_ALLOCATION_LOCK_RETRY);
            }
            Err(error) => {
                return Err(format!(
                    "pool allocation lock could not be acquired at `{}`: {error}",
                    lock_path.display()
                ));
            }
        }
    }
}

fn pool_allocation_lock_owner_token() -> String {
    let created_at = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("pid={}\ncreated_at_ms={created_at}\n", std::process::id())
}

fn release_pool_allocation_lock(lock_path: &Path, owner_token: &str) {
    let owner_path = lock_path.join(POOL_ALLOCATION_LOCK_OWNER_FILE);
    let Ok(current_owner) = fs::read_to_string(&owner_path) else {
        return;
    };
    if current_owner == owner_token {
        let _ = fs::remove_dir_all(lock_path);
    }
}

fn remove_stale_pool_allocation_lock(lock_path: &Path) {
    let Ok(metadata) = fs::metadata(lock_path) else {
        return;
    };
    let Ok(modified_at) = metadata.modified() else {
        return;
    };
    let Ok(age) = SystemTime::now().duration_since(modified_at) else {
        return;
    };
    if age >= POOL_ALLOCATION_LOCK_STALE_AFTER {
        let owner_path = lock_path.join(POOL_ALLOCATION_LOCK_OWNER_FILE);
        if !matches!(
            fs::read_to_string(owner_path)
                .ok()
                .and_then(|owner| pool_allocation_lock_owner_pid(&owner))
                .map(pool_allocation_lock_owner_liveness),
            None | Some(PoolAllocationLockOwnerLiveness::Dead)
        ) {
            return;
        }
        let _ = fs::remove_dir_all(lock_path);
    }
}

fn pool_allocation_lock_owner_pid(owner_token: &str) -> Option<u32> {
    owner_token
        .lines()
        .find_map(|line| line.strip_prefix("pid=")?.parse::<u32>().ok())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PoolAllocationLockOwnerLiveness {
    Alive,
    Dead,
    Unknown,
}

fn pool_allocation_lock_owner_liveness(pid: u32) -> PoolAllocationLockOwnerLiveness {
    platform_process_liveness(pid)
}

#[cfg(unix)]
fn platform_process_liveness(pid: u32) -> PoolAllocationLockOwnerLiveness {
    match std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
    {
        Ok(status) if status.success() => PoolAllocationLockOwnerLiveness::Alive,
        Ok(_) => PoolAllocationLockOwnerLiveness::Dead,
        Err(_) => PoolAllocationLockOwnerLiveness::Unknown,
    }
}

#[cfg(windows)]
fn platform_process_liveness(pid: u32) -> PoolAllocationLockOwnerLiveness {
    let filter = format!("PID eq {pid}");
    match std::process::Command::new("tasklist")
        .args(["/FI", filter.as_str(), "/NH"])
        .output()
    {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if stdout
                .split_whitespace()
                .any(|field| field.trim() == pid.to_string())
            {
                PoolAllocationLockOwnerLiveness::Alive
            } else {
                PoolAllocationLockOwnerLiveness::Dead
            }
        }
        Ok(_) => PoolAllocationLockOwnerLiveness::Dead,
        Err(_) => PoolAllocationLockOwnerLiveness::Unknown,
    }
}

#[cfg(not(any(unix, windows)))]
fn platform_process_liveness(_pid: u32) -> PoolAllocationLockOwnerLiveness {
    PoolAllocationLockOwnerLiveness::Unknown
}
