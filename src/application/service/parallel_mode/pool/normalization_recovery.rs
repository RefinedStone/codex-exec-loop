use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Output;

use rand::RngCore;

use super::super::git_sequence::{
    GitCommandSequenceReport, GitCommandStep, GitCommandStepReport, run_git_sequence,
};
use super::{
    PoolMutationLock, SlotGitStatus, git_command_directory, git_worktree_destination,
    inspect_slot_git_status, parse_worktree_records, worktree_paths_match,
};

const MAX_NORMALIZATION_PATHS: usize = 64;
const MAX_NORMALIZATION_STATUS_BYTES: usize = 1024 * 1024;
const MAX_NORMALIZATION_FILE_BYTES: usize = 1024 * 1024;
const NORMALIZATION_QUARANTINE_PREFIX: &str = ".normalization-recovery-";
const NORMALIZATION_REPLACEMENT_PREFIX: &str = ".normalization-replacement-";
const SAFE_TEXT_ATTRIBUTES: [(&str, &[&str]); 6] = [
    ("text", &["set", "auto"]),
    ("eol", &["lf"]),
    ("filter", &["unspecified", "unset"]),
    ("working-tree-encoding", &["unspecified", "unset"]),
    ("ident", &["unspecified", "unset"]),
    ("diff", &["unspecified", "unset"]),
];

#[derive(Debug)]
struct NormalizationPathProof {
    path: String,
    mode: String,
    index_oid: String,
}

pub(super) struct NormalizationRecoveryRequest<'a> {
    pub(super) repo_root: &'a str,
    pub(super) pool_root: &'a Path,
    pub(super) slot_id: &'a str,
    pub(super) slot_path: &'a Path,
    pub(super) source_oid: &'a str,
    pub(super) target_ref: &'a str,
}

pub(super) struct NormalizationRecoveryOutcome {
    pub(super) report: GitCommandSequenceReport,
    pub(super) quarantine_path: Option<PathBuf>,
}

/*
An old pool baseline can contain CRLF or mixed blobs even though `.gitattributes` declares
`text eol=lf`. Git then reports a freshly checked-out file as unstaged because its clean
conversion differs from the malformed index blob, although the raw worktree bytes are identical
to that blob. This classifier proves that narrow state without weakening the ordinary dirty-slot
gate used by allocation, cleanup, and delivery.

Every path must be a regular unstaged-only porcelain-v2 record, its raw bytes must still equal the
current index OID, only built-in LF normalization may be configured, and Rust-normalized bytes plus
mode must already equal the frozen target tree. `hash-object --no-filters --stdin` calculates blob
identities without invoking a repository filter between proof steps.
*/
pub(super) fn has_target_equivalent_lf_normalization_drift(
    slot_path: &Path,
    slot_status: SlotGitStatus,
    target_ref: &str,
) -> bool {
    if !slot_status.has_only_unstaged_changes()
        || target_ref.trim().is_empty()
        || crate::git_execution_guard::ensure_host_git_execution_config_safe(slot_path).is_err()
        || !index_has_only_normal_entries(slot_path)
        || index_has_unmerged_entries(slot_path)
    {
        return false;
    }

    let Some(target_oid) = resolve_target_oid(slot_path, target_ref) else {
        return false;
    };
    let Some(path_proofs) = load_normalization_path_proofs(slot_path) else {
        return false;
    };
    if path_proofs.is_empty() || path_proofs.len() > MAX_NORMALIZATION_PATHS {
        return false;
    }

    path_proofs.iter().all(|proof| {
        path_bytes_match_index_and_target(slot_path, &target_oid, proof)
            && path_has_safe_lf_attributes(slot_path, &proof.path, false)
            && path_has_safe_lf_attributes(slot_path, &proof.path, true)
    }) && differs_only_by_cr_at_eol(slot_path)
}

/*
Proof alone cannot make an external editor or Git process stop writing. Instead of checking out
over the dirty worktree, provision and verify the clean target at a hidden staging path first.
Then move the entire legacy worktree (including its index) to quarantine and atomically rename the
staged worktree into the canonical lane. The canonical path never hosts a Git checkout: a writer
using an old file descriptor follows the legacy inode into quarantine, while a path-based writer
that recreates the brief canonical gap makes the final move fail without overwriting those bytes.
*/
pub(super) fn quarantine_normalization_drift_and_replace_slot(
    request: NormalizationRecoveryRequest<'_>,
    mutation_lock: &PoolMutationLock,
    recheck_unowned_authority: &dyn Fn() -> Result<(), String>,
) -> NormalizationRecoveryOutcome {
    let quarantine_path =
        new_normalization_quarantine_path(request.pool_root, request.slot_id, request.source_oid);
    let result = (|| {
        mutation_lock.verify_pool_root(request.pool_root)?;
        let canonical_slot_path = request.pool_root.join(request.slot_id);
        if !worktree_paths_match(request.slot_path, &canonical_slot_path) {
            return Err(
                "normalization recovery slot path is outside its canonical lane".to_string(),
            );
        }
        let incomplete_replacements =
            normalization_replacement_artifacts_for_slot(request.pool_root, request.slot_id)?;
        if !incomplete_replacements.is_empty() {
            return Err(format!(
                "normalization recovery for slot `{}` is already incomplete; preserved replacement artifact(s): {}",
                request.slot_id,
                incomplete_replacements
                    .iter()
                    .map(|path| format!("`{}`", path.display()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let target_oid = resolve_target_oid(request.slot_path, request.target_ref)
            .ok_or_else(|| "normalization recovery target OID could not be resolved".to_string())?;
        let quarantine_path = quarantine_path.as_ref().map_err(Clone::clone)?;
        let replacement_path =
            new_normalization_replacement_path(request.pool_root, request.slot_id, &target_oid)?;
        ensure_path_absent(quarantine_path, "quarantine")?;
        ensure_path_absent(&replacement_path, "replacement staging")?;
        let canonical_repo_root = std::fs::canonicalize(request.repo_root).map_err(|error| {
            format!("normalization recovery repository root could not be pinned: {error}")
        })?;
        let git_repo_root = git_command_directory(&canonical_repo_root)?;
        crate::git_execution_guard::ensure_host_git_execution_config_safe(Path::new(
            request.repo_root,
        ))
        .map_err(|error| format!("normalization recovery Git execution is unsafe: {error}"))?;
        recheck_unowned_authority()?;
        verify_registered_detached_worktree(
            &git_repo_root,
            request.slot_path,
            request.source_oid,
            false,
        )?;
        mutation_lock.verify_pool_root(request.pool_root)?;

        let git_slot_path = git_worktree_path(&canonical_repo_root, request.slot_path)?;
        let git_quarantine_path = git_worktree_path(&canonical_repo_root, quarantine_path)?;
        let git_replacement_path = git_worktree_path(&canonical_repo_root, &replacement_path)?;
        let staging_directory = create_private_staging_directory(&replacement_path)?;
        mutation_lock.verify_pool_root(request.pool_root)?;
        run_before_normalization_staging_provision_hook(request.slot_path, &replacement_path);
        staging_directory.verify()?;
        let mut report = run_git_sequence(
            "quarantine target-equivalent LF normalization drift",
            vec![GitCommandStep::new(
                "provision clean replacement in normalization staging",
                [
                    "-C",
                    git_repo_root.as_str(),
                    "worktree",
                    "add",
                    "--detach",
                    git_replacement_path.as_str(),
                    target_oid.as_str(),
                ],
            )],
        );
        if !report.succeeded() {
            return Ok(report);
        }
        if let Err(detail) = staging_directory.verify() {
            append_verification_failure(
                &mut report,
                "verify normalization replacement staging identity",
                detail,
            );
            return Ok(report);
        }
        if let Err(detail) = verify_registered_detached_worktree(
            &git_repo_root,
            &replacement_path,
            &target_oid,
            true,
        ) {
            append_verification_failure(
                &mut report,
                "verify clean normalization replacement staging",
                detail,
            );
            return Ok(report);
        }

        run_before_normalization_quarantine_hook(request.slot_path);
        let pre_move_validation = (|| {
            mutation_lock.verify_pool_root(request.pool_root)?;
            recheck_unowned_authority()?;
            ensure_path_absent(quarantine_path, "quarantine")?;
            verify_registered_detached_worktree(
                &git_repo_root,
                request.slot_path,
                request.source_oid,
                false,
            )?;
            staging_directory.verify()?;
            verify_registered_detached_worktree(
                &git_repo_root,
                &replacement_path,
                &target_oid,
                true,
            )
        })();
        if let Err(detail) = pre_move_validation {
            append_verification_failure(
                &mut report,
                "revalidate normalization recovery before quarantine move",
                detail,
            );
            return Ok(report);
        }

        run_before_normalization_atomic_rename_hook(quarantine_path);
        if !append_atomic_rename_step(
            &mut report,
            "atomically move legacy slot to normalization recovery quarantine",
            request.slot_path,
            quarantine_path,
        ) {
            return Ok(report);
        }
        if let Err(detail) = mutation_lock.verify_pool_root(request.pool_root) {
            append_verification_failure(
                &mut report,
                "revalidate pool root after quarantining legacy slot",
                detail,
            );
            return Ok(report);
        }
        let repair_quarantine_report = run_git_sequence(
            "quarantine target-equivalent LF normalization drift",
            vec![GitCommandStep::new(
                "repair quarantined worktree registration",
                [
                    "-C",
                    git_repo_root.as_str(),
                    "worktree",
                    "repair",
                    git_quarantine_path.as_str(),
                ],
            )],
        );
        report.steps.extend(repair_quarantine_report.steps);
        if !report.succeeded() {
            return Ok(report);
        }

        run_after_normalization_quarantine_move_hook(request.slot_path);
        let pre_install_validation = (|| {
            mutation_lock.verify_pool_root(request.pool_root)?;
            recheck_unowned_authority()?;
            ensure_path_absent(request.slot_path, "canonical slot")?;
            verify_registered_detached_worktree(
                &git_repo_root,
                quarantine_path,
                request.source_oid,
                false,
            )?;
            staging_directory.verify()?;
            verify_registered_detached_worktree(
                &git_repo_root,
                &replacement_path,
                &target_oid,
                true,
            )
        })();
        if let Err(detail) = pre_install_validation {
            append_verification_failure(
                &mut report,
                "revalidate normalization recovery before canonical move",
                detail,
            );
            return Ok(report);
        }

        // Windows pins the staging directory without FILE_SHARE_DELETE, so release the verified
        // handle immediately before the no-replace move into the canonical lane.
        drop(staging_directory);
        run_before_normalization_atomic_rename_hook(request.slot_path);
        if !append_atomic_rename_step(
            &mut report,
            "atomically move staged replacement into the canonical pool slot",
            &replacement_path,
            request.slot_path,
        ) {
            return Ok(report);
        }
        if let Err(detail) = mutation_lock.verify_pool_root(request.pool_root) {
            append_verification_failure(
                &mut report,
                "revalidate pool root after installing canonical replacement",
                detail,
            );
            return Ok(report);
        }
        let repair_replacement_report = run_git_sequence(
            "quarantine target-equivalent LF normalization drift",
            vec![GitCommandStep::new(
                "repair canonical replacement worktree registration",
                [
                    "-C",
                    git_repo_root.as_str(),
                    "worktree",
                    "repair",
                    git_slot_path.as_str(),
                ],
            )],
        );
        report.steps.extend(repair_replacement_report.steps);
        if !report.succeeded() {
            return Ok(report);
        }

        let completed_validation =
            mutation_lock
                .verify_pool_root(request.pool_root)
                .and_then(|()| {
                    verify_completed_recovery(
                        &git_repo_root,
                        request.slot_path,
                        quarantine_path,
                        &replacement_path,
                        request.source_oid,
                        &target_oid,
                    )
                });
        if let Err(detail) = completed_validation {
            append_verification_failure(
                &mut report,
                "verify quarantined legacy slot and clean replacement",
                detail,
            );
        }
        Ok(report)
    })();

    let report = result.unwrap_or_else(normalization_recovery_preflight_failure);
    let completed_quarantine_path = report.succeeded().then(|| quarantine_path.ok()).flatten();
    NormalizationRecoveryOutcome {
        report,
        quarantine_path: completed_quarantine_path,
    }
}

pub(in crate::application::service::parallel_mode) fn normalization_quarantine_path(
    pool_root: &Path,
    slot_id: &str,
    source_oid: &str,
) -> Option<PathBuf> {
    if slot_id.is_empty()
        || !slot_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || !matches!(source_oid.len(), 40 | 64)
        || !source_oid.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    Some(pool_root.join(format!(
        "{NORMALIZATION_QUARANTINE_PREFIX}{slot_id}-{}",
        source_oid.to_ascii_lowercase()
    )))
}

fn new_normalization_quarantine_path(
    pool_root: &Path,
    slot_id: &str,
    source_oid: &str,
) -> Result<PathBuf, String> {
    let base_path = normalization_quarantine_path(pool_root, slot_id, source_oid)
        .ok_or_else(|| "normalization recovery source identity is invalid".to_string())?;
    match base_path.symlink_metadata() {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(base_path),
        Ok(_) => Ok(base_path.with_file_name(format!(
            "{}-{}",
            base_path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    "normalization recovery quarantine name is not valid Unicode".to_string()
                })?,
            random_normalization_nonce()?
        ))),
        Err(error) => Err(format!(
            "normalization quarantine path could not be inspected at `{}`: {error}",
            base_path.display()
        )),
    }
}

fn new_normalization_replacement_path(
    pool_root: &Path,
    slot_id: &str,
    target_oid: &str,
) -> Result<PathBuf, String> {
    if slot_id.is_empty()
        || !slot_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || !matches!(target_oid.len(), 40 | 64)
        || !target_oid.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("normalization recovery target identity is invalid".to_string());
    }
    let nonce = random_normalization_nonce()?;
    Ok(pool_root.join(format!(
        "{NORMALIZATION_REPLACEMENT_PREFIX}{slot_id}-{}-{nonce}",
        target_oid.to_ascii_lowercase()
    )))
}

fn random_normalization_nonce() -> Result<String, String> {
    let mut nonce = [0_u8; 16];
    rand::rngs::OsRng
        .try_fill_bytes(&mut nonce)
        .map_err(|error| format!("normalization recovery requires OS randomness: {error}"))?;
    Ok(nonce.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(super) fn normalization_recovery_artifact_paths(
    pool_root: &Path,
) -> Result<Vec<PathBuf>, String> {
    let entries = match std::fs::read_dir(pool_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "normalization recovery artifacts could not be inspected under `{}`: {error}",
                pool_root.display()
            ));
        }
    };
    let mut artifacts = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "normalization recovery pool entry could not be inspected under `{}`: {error}",
                pool_root.display()
            )
        })?;
        let path = entry.path();
        if entry.file_name().to_str().is_some_and(|name| {
            normalization_quarantine_slot_id(name).is_some()
                || normalization_replacement_slot_id(name).is_some()
        }) {
            artifacts.push(path);
        }
    }
    artifacts.sort();
    Ok(artifacts)
}

fn normalization_quarantine_slot_id(name: &str) -> Option<&str> {
    let remainder = name.strip_prefix(NORMALIZATION_QUARANTINE_PREFIX)?;
    let (slot_and_oid, last_component) = remainder.rsplit_once('-')?;
    let (slot_id, source_oid) = if is_hex_identifier(last_component, &[40, 64]) {
        (slot_and_oid, last_component)
    } else if is_hex_identifier(last_component, &[32]) {
        slot_and_oid.rsplit_once('-')?
    } else {
        return None;
    };
    (is_valid_normalization_slot_id(slot_id) && is_hex_identifier(source_oid, &[40, 64]))
        .then_some(slot_id)
}

fn normalization_replacement_slot_id(name: &str) -> Option<&str> {
    let remainder = name.strip_prefix(NORMALIZATION_REPLACEMENT_PREFIX)?;
    let (slot_and_oid, nonce) = remainder.rsplit_once('-')?;
    let (slot_id, target_oid) = slot_and_oid.rsplit_once('-')?;
    (is_valid_normalization_slot_id(slot_id)
        && is_hex_identifier(target_oid, &[40, 64])
        && is_hex_identifier(nonce, &[32]))
    .then_some(slot_id)
}

fn is_valid_normalization_slot_id(slot_id: &str) -> bool {
    !slot_id.is_empty()
        && slot_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn is_hex_identifier(value: &str, allowed_lengths: &[usize]) -> bool {
    allowed_lengths.contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn normalization_replacement_artifacts_for_slot(
    pool_root: &Path,
    slot_id: &str,
) -> Result<Vec<PathBuf>, String> {
    normalization_recovery_artifact_paths(pool_root).map(|artifacts| {
        artifacts
            .into_iter()
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .and_then(normalization_replacement_slot_id)
                    == Some(slot_id)
            })
            .collect()
    })
}

pub(super) fn has_normalization_replacement_artifact_for_slot(
    artifacts: &[PathBuf],
    slot_id: &str,
) -> bool {
    artifacts.iter().any(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .and_then(normalization_replacement_slot_id)
            == Some(slot_id)
    })
}

fn ensure_path_absent(path: &Path, label: &str) -> Result<(), String> {
    match path.symlink_metadata() {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err(format!(
            "normalization {label} path already exists at `{}`",
            path.display()
        )),
        Err(error) => Err(format!(
            "normalization {label} path could not be inspected at `{}`: {error}",
            path.display()
        )),
    }
}

struct PinnedStagingDirectory {
    path: PathBuf,
    opened: File,
}

impl PinnedStagingDirectory {
    fn verify(&self) -> Result<(), String> {
        verify_pinned_staging_directory(&self.path, &self.opened)
    }
}

fn create_private_staging_directory(path: &Path) -> Result<PinnedStagingDirectory, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        builder.create(path).map_err(|error| {
            format!(
                "normalization replacement staging could not be claimed at `{}`: {error}",
                path.display()
            )
        })?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir(path).map_err(|error| {
        format!(
            "normalization replacement staging could not be claimed at `{}`: {error}",
            path.display()
        )
    })?;
    let opened = open_pinned_staging_directory(path)?;
    let staging = PinnedStagingDirectory {
        path: path.to_path_buf(),
        opened,
    };
    staging.verify()?;
    Ok(staging)
}

#[cfg(unix)]
fn open_pinned_staging_directory(path: &Path) -> Result<File, String> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| {
            format!(
                "normalization replacement staging could not be pinned at `{}`: {error}",
                path.display()
            )
        })
}

#[cfg(windows)]
fn open_pinned_staging_directory(path: &Path) -> Result<File, String> {
    use std::os::windows::fs::OpenOptionsExt;

    use crate::private_fs::{
        WINDOWS_FILE_FLAG_BACKUP_SEMANTICS, WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
        WINDOWS_GENERIC_READ, WINDOWS_READ_CONTROL,
    };

    const FILE_SHARE_READ_WRITE: u32 = 0x0000_0001 | 0x0000_0002;
    OpenOptions::new()
        .read(true)
        .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL)
        // Deliberately omit FILE_SHARE_DELETE so the staging identity cannot be renamed or
        // replaced while Git is populating it.
        .share_mode(FILE_SHARE_READ_WRITE)
        .custom_flags(WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT | WINDOWS_FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .map_err(|error| {
            format!(
                "normalization replacement staging could not be pinned at `{}`: {error}",
                path.display()
            )
        })
}

#[cfg(not(any(unix, windows)))]
fn open_pinned_staging_directory(path: &Path) -> Result<File, String> {
    Err(format!(
        "normalization replacement staging identity pinning is unsupported at `{}`",
        path.display()
    ))
}

#[cfg(unix)]
fn verify_pinned_staging_directory(path: &Path, opened: &File) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;

    let opened_metadata = opened.metadata().map_err(|error| {
        format!(
            "normalization replacement staging handle could not be inspected at `{}`: {error}",
            path.display()
        )
    })?;
    let path_metadata = path.symlink_metadata().map_err(|error| {
        format!(
            "normalization replacement staging path could not be reinspected at `{}`: {error}",
            path.display()
        )
    })?;
    if !opened_metadata.is_dir()
        || !path_metadata.is_dir()
        || path_metadata.file_type().is_symlink()
        || opened_metadata.dev() != path_metadata.dev()
        || opened_metadata.ino() != path_metadata.ino()
        || opened_metadata.uid() != unsafe { libc::geteuid() }
        || path_metadata.uid() != opened_metadata.uid()
        || opened_metadata.mode() & 0o077 != 0
        || path_metadata.mode() & 0o077 != 0
    {
        return Err(format!(
            "normalization replacement staging identity changed or is not private at `{}`",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn verify_pinned_staging_directory(path: &Path, opened: &File) -> Result<(), String> {
    crate::private_fs::validate_windows_path_identity_only(path, opened, true).map_err(|error| {
        format!(
            "normalization replacement staging identity changed at `{}`: {error}",
            path.display()
        )
    })
}

#[cfg(not(any(unix, windows)))]
fn verify_pinned_staging_directory(path: &Path, _opened: &File) -> Result<(), String> {
    Err(format!(
        "normalization replacement staging identity verification is unsupported at `{}`",
        path.display()
    ))
}

fn git_worktree_path(canonical_repo_root: &Path, worktree_path: &Path) -> Result<String, String> {
    git_worktree_destination(canonical_repo_root, worktree_path)?
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| {
            format!(
                "normalization recovery path is not valid Unicode for Git: `{}`",
                worktree_path.display()
            )
        })
}

fn append_atomic_rename_step(
    report: &mut GitCommandSequenceReport,
    label: &str,
    source: &Path,
    destination: &Path,
) -> bool {
    let result = atomic_rename_noreplace(source, destination);
    report.steps.push(GitCommandStepReport {
        label: label.to_string(),
        args: vec![
            source.display().to_string(),
            destination.display().to_string(),
        ],
        exit_code: Some(if result.is_ok() { 0 } else { 1 }),
        stdout: String::new(),
        stderr: result.err().map_or_else(String::new, |error| {
            format!(
                "atomic normalization worktree move from `{}` to `{}` failed: {error}",
                source.display(),
                destination.display()
            )
        }),
    });
    report.succeeded()
}

#[cfg(target_vendor = "apple")]
fn atomic_rename_noreplace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    // SAFETY: both C strings are NUL-terminated owned buffers valid for this syscall.
    let result =
        unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(any(all(target_os = "linux", target_env = "gnu"), target_os = "android"))]
fn atomic_rename_noreplace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    // SAFETY: both C strings are NUL-terminated owned buffers valid for this syscall.
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(all(target_os = "linux", not(target_env = "gnu")))]
fn atomic_rename_noreplace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    // SAFETY: both C strings are NUL-terminated owned buffers valid for this syscall.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn atomic_rename_noreplace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Storage::FileSystem::MoveFileW;

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both vectors are NUL-terminated and remain alive through the call. MoveFileW
    // fails when the destination already exists, unlike Rust's replacement-capable rename.
    if unsafe { MoveFileW(source.as_ptr(), destination.as_ptr()) } != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(any(
    target_vendor = "apple",
    target_os = "linux",
    target_os = "android",
    windows
)))]
fn atomic_rename_noreplace(_source: &Path, _destination: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "atomic no-replace worktree moves are unsupported on this platform",
    ))
}

fn verify_registered_detached_worktree(
    repo_root: &str,
    worktree_path: &Path,
    expected_oid: &str,
    require_clean: bool,
) -> Result<(), String> {
    if !worktree_paths_match(worktree_path, worktree_path) {
        return Err(format!(
            "normalization recovery worktree path is missing or link-aliased at `{}`",
            worktree_path.display()
        ));
    }
    let records = load_worktree_inventory(repo_root)?;
    if !records.iter().any(|record| {
        worktree_paths_match(&record.path, worktree_path)
            && record.detached
            && record.head_sha.eq_ignore_ascii_case(expected_oid)
    }) {
        return Err(format!(
            "normalization recovery worktree is not registered detached at `{expected_oid}`: `{}`",
            worktree_path.display()
        ));
    }
    let status = inspect_slot_git_status(worktree_path).map_err(|error| {
        format!(
            "normalization recovery status could not be inspected at `{}`: {error}",
            worktree_path.display()
        )
    })?;
    if status.has_pending_operation() {
        return Err(format!(
            "normalization recovery worktree has pending Git operation/lock metadata at `{}`",
            worktree_path.display()
        ));
    }
    if require_clean && !status.is_clean_baseline() {
        return Err(format!(
            "normalization replacement worktree is not clean at `{}`",
            worktree_path.display()
        ));
    }
    Ok(())
}

fn load_worktree_inventory(repo_root: &str) -> Result<Vec<super::GitWorktreeRecord>, String> {
    let output = run_git(
        Path::new(repo_root),
        ["worktree", "list", "--porcelain"],
        "inspect normalization recovery worktree inventory",
    )
    .ok_or_else(|| "normalization recovery worktree inventory command failed".to_string())?;
    if !output.status.success() {
        return Err("normalization recovery worktree inventory could not be read".to_string());
    }
    let output = std::str::from_utf8(&output.stdout)
        .map_err(|_| "normalization recovery worktree inventory is not UTF-8".to_string())?;
    Ok(parse_worktree_records(output))
}

fn verify_completed_recovery(
    repo_root: &str,
    slot_path: &Path,
    quarantine_path: &Path,
    replacement_path: &Path,
    source_oid: &str,
    target_oid: &str,
) -> Result<(), String> {
    verify_registered_detached_worktree(repo_root, quarantine_path, source_oid, false)?;
    verify_registered_detached_worktree(repo_root, slot_path, target_oid, true)?;
    ensure_path_absent(replacement_path, "replacement staging")?;
    let records = load_worktree_inventory(repo_root)?;
    if records.iter().any(|record| record.path == replacement_path) {
        return Err(format!(
            "normalization replacement staging remains registered at `{}`",
            replacement_path.display()
        ));
    }
    Ok(())
}

fn append_verification_failure(report: &mut GitCommandSequenceReport, label: &str, detail: String) {
    report.steps.push(GitCommandStepReport {
        label: label.to_string(),
        args: vec!["worktree/status/head verification".to_string()],
        exit_code: Some(1),
        stdout: String::new(),
        stderr: detail,
    });
}

fn normalization_recovery_preflight_failure(detail: String) -> GitCommandSequenceReport {
    GitCommandSequenceReport {
        label: "quarantine target-equivalent LF normalization drift".to_string(),
        steps: vec![GitCommandStepReport {
            label: "verify normalization recovery preconditions".to_string(),
            args: vec!["pool/worktree/target verification".to_string()],
            exit_code: None,
            stdout: String::new(),
            stderr: detail,
        }],
    }
}

fn resolve_target_oid(slot_path: &Path, target_ref: &str) -> Option<String> {
    let commit_ref = format!("{target_ref}^{{commit}}");
    let output = run_git(
        slot_path,
        ["rev-parse", "--verify", commit_ref.as_str()],
        "resolve normalization recovery target",
    )?;
    successful_ascii_stdout(output)
}

fn index_has_only_normal_entries(slot_path: &Path) -> bool {
    let Some(output) = run_git(
        slot_path,
        ["ls-files", "-v", "-z", "--"],
        "inspect normalization recovery index flags",
    ) else {
        return false;
    };
    output.status.success()
        && output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty())
            .all(|record| record.starts_with(b"H "))
}

fn index_has_unmerged_entries(slot_path: &Path) -> bool {
    run_git(
        slot_path,
        ["ls-files", "--unmerged", "-z", "--"],
        "inspect normalization recovery unmerged index",
    )
    .is_none_or(|output| !output.status.success() || !output.stdout.is_empty())
}

fn load_normalization_path_proofs(slot_path: &Path) -> Option<Vec<NormalizationPathProof>> {
    let output = run_git(
        slot_path,
        [
            "status",
            "--porcelain=v2",
            "-z",
            "--untracked-files=all",
            "--ignored=matching",
            "--ignore-submodules=none",
        ],
        "inspect normalization recovery status",
    )?;
    if !output.status.success() || output.stdout.len() > MAX_NORMALIZATION_STATUS_BYTES {
        return None;
    }

    let mut proofs = Vec::new();
    for record in output.stdout.split(|byte| *byte == 0) {
        if record.is_empty() {
            continue;
        }
        let record = std::str::from_utf8(record).ok()?;
        let fields = record.splitn(9, ' ').collect::<Vec<_>>();
        if fields.len() != 9
            || fields[0] != "1"
            || fields[1] != ".M"
            || fields[2] != "N..."
            || fields[3] != fields[4]
            || fields[4] != fields[5]
            || !matches!(fields[3], "100644" | "100755")
            || fields[6] != fields[7]
            || fields[8].is_empty()
        {
            return None;
        }
        proofs.push(NormalizationPathProof {
            path: fields[8].to_string(),
            mode: fields[4].to_string(),
            index_oid: fields[7].to_string(),
        });
        if proofs.len() > MAX_NORMALIZATION_PATHS {
            return None;
        }
    }
    Some(proofs)
}

fn differs_only_by_cr_at_eol(slot_path: &Path) -> bool {
    run_git(
        slot_path,
        [
            "diff",
            "--quiet",
            "--ignore-cr-at-eol",
            "--no-ext-diff",
            "--no-textconv",
            "--ignore-submodules=none",
            "--",
        ],
        "verify CR-at-EOL-only normalization drift",
    )
    .is_some_and(|output| output.status.success())
}

fn normalization_candidate_path(slot_path: &Path, path: &str) -> Option<PathBuf> {
    let relative_path = Path::new(path);
    if relative_path.as_os_str().is_empty()
        || relative_path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return None;
    }
    let candidate_path = slot_path.join(relative_path);
    if !worktree_paths_match(&candidate_path, &candidate_path) {
        return None;
    }
    Some(candidate_path)
}

fn read_bounded_unshared_regular_file(path: &Path) -> Option<Vec<u8>> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }

    let file = options.open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.file_type().is_file()
        || metadata.len() > MAX_NORMALIZATION_FILE_BYTES as u64
        || !opened_file_has_one_link_and_no_reparse(&file, &metadata)
    {
        return None;
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_NORMALIZATION_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= MAX_NORMALIZATION_FILE_BYTES).then_some(bytes)
}

#[cfg(unix)]
fn opened_file_has_one_link_and_no_reparse(_file: &File, metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    metadata.nlink() == 1
}

#[cfg(windows)]
fn opened_file_has_one_link_and_no_reparse(file: &File, metadata: &std::fs::Metadata) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    use std::os::windows::fs::MetadataExt;

    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
        && crate::private_fs::windows_file_link_count(file).is_ok_and(|links| links == 1)
}

#[cfg(not(any(unix, windows)))]
fn opened_file_has_one_link_and_no_reparse(_file: &File, _metadata: &std::fs::Metadata) -> bool {
    true
}

fn path_has_safe_lf_attributes(slot_path: &Path, path: &str, cached: bool) -> bool {
    let mut args = vec![OsString::from("check-attr")];
    if cached {
        args.push(OsString::from("--cached"));
    }
    args.extend([
        OsString::from("-z"),
        OsString::from("text"),
        OsString::from("eol"),
        OsString::from("filter"),
        OsString::from("working-tree-encoding"),
        OsString::from("ident"),
        OsString::from("diff"),
        OsString::from("--"),
        OsString::from(path),
    ]);
    let Some(output) = run_git_os(slot_path, args, "inspect normalization recovery attributes")
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let fields = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .collect::<Vec<_>>();
    fields.len() == SAFE_TEXT_ATTRIBUTES.len() * 3
        && fields.chunks_exact(3).zip(SAFE_TEXT_ATTRIBUTES).all(
            |(chunk, (expected_attribute, allowed_values))| {
                chunk[0] == path.as_bytes()
                    && chunk[1] == expected_attribute.as_bytes()
                    && allowed_values
                        .iter()
                        .any(|allowed| chunk[2] == allowed.as_bytes())
            },
        )
}

fn path_bytes_match_index_and_target(
    slot_path: &Path,
    target_oid: &str,
    proof: &NormalizationPathProof,
) -> bool {
    let Some(candidate_path) = normalization_candidate_path(slot_path, &proof.path) else {
        return false;
    };
    let Some(raw_bytes) = read_bounded_unshared_regular_file(&candidate_path) else {
        return false;
    };
    let Some(raw_oid) = hash_blob_bytes(slot_path, &raw_bytes) else {
        return false;
    };
    if raw_oid != proof.index_oid {
        return false;
    }
    let normalized_bytes = normalize_crlf(&raw_bytes);
    if normalized_bytes == raw_bytes {
        return false;
    }
    let Some(normalized_oid) = hash_blob_bytes(slot_path, &normalized_bytes) else {
        return false;
    };
    target_tree_matches_blob(slot_path, target_oid, proof, &normalized_oid)
}

fn normalize_crlf(bytes: &[u8]) -> Vec<u8> {
    let mut normalized = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
            normalized.push(b'\n');
            index += 2;
        } else {
            normalized.push(bytes[index]);
            index += 1;
        }
    }
    normalized
}

fn hash_blob_bytes(slot_path: &Path, bytes: &[u8]) -> Option<String> {
    let mut command_args = vec![OsString::from("-C"), slot_path.as_os_str().to_os_string()];
    command_args.extend([
        OsString::from("hash-object"),
        OsString::from("--no-filters"),
        OsString::from("--stdin"),
    ]);
    let mut command = crate::git_subprocess::command(command_args);
    let output = crate::subprocess::command_output_with_input(
        &mut command,
        "hash normalization recovery bytes",
        bytes,
    )
    .ok()?;
    successful_ascii_stdout(output)
}

fn target_tree_matches_blob(
    slot_path: &Path,
    target_oid: &str,
    proof: &NormalizationPathProof,
    normalized_oid: &str,
) -> bool {
    let Some(output) = run_git(
        slot_path,
        ["ls-tree", "-z", target_oid, "--", proof.path.as_str()],
        "inspect normalization recovery target tree",
    ) else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let records = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .collect::<Vec<_>>();
    if records.len() != 1 {
        return false;
    }
    let Ok(record) = std::str::from_utf8(records[0]) else {
        return false;
    };
    let Some((metadata, path)) = record.split_once('\t') else {
        return false;
    };
    let metadata = metadata.split_whitespace().collect::<Vec<_>>();
    metadata.len() == 3
        && metadata[0] == proof.mode
        && metadata[1] == "blob"
        && metadata[2] == normalized_oid
        && metadata[2] != proof.index_oid
        && path == proof.path
}

fn run_git<const N: usize>(
    slot_path: &Path,
    args: [&str; N],
    command_label: &str,
) -> Option<Output> {
    run_git_os(
        slot_path,
        args.into_iter().map(OsString::from).collect(),
        command_label,
    )
}

fn run_git_os(slot_path: &Path, args: Vec<OsString>, command_label: &str) -> Option<Output> {
    let mut command_args = vec![OsString::from("-C"), slot_path.as_os_str().to_os_string()];
    command_args.extend(args);
    let mut command = crate::git_subprocess::command(command_args);
    crate::subprocess::command_output(&mut command, command_label).ok()
}

fn successful_ascii_stdout(output: Output) -> Option<String> {
    if !output.status.success() {
        return None;
    }
    let value = std::str::from_utf8(&output.stdout).ok()?.trim();
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

#[cfg(test)]
type BeforeQuarantineHook = Box<dyn FnOnce(&Path) + Send + 'static>;

#[cfg(test)]
static BEFORE_NORMALIZATION_QUARANTINE_HOOK: std::sync::Mutex<
    Vec<(PathBuf, BeforeQuarantineHook)>,
> = std::sync::Mutex::new(Vec::new());

#[cfg(test)]
static AFTER_NORMALIZATION_QUARANTINE_MOVE_HOOK: std::sync::Mutex<
    Vec<(PathBuf, BeforeQuarantineHook)>,
> = std::sync::Mutex::new(Vec::new());

#[cfg(test)]
static BEFORE_NORMALIZATION_ATOMIC_RENAME_HOOK: std::sync::Mutex<
    Vec<(PathBuf, BeforeQuarantineHook)>,
> = std::sync::Mutex::new(Vec::new());

#[cfg(test)]
static BEFORE_NORMALIZATION_STAGING_PROVISION_HOOK: std::sync::Mutex<
    Vec<(PathBuf, BeforeQuarantineHook)>,
> = std::sync::Mutex::new(Vec::new());

#[cfg(test)]
pub(in crate::application::service::parallel_mode) fn install_before_normalization_quarantine_hook(
    slot_path: &Path,
    hook: impl FnOnce(&Path) + Send + 'static,
) {
    BEFORE_NORMALIZATION_QUARANTINE_HOOK
        .lock()
        .expect("normalization quarantine test hook should not be poisoned")
        .push((slot_path.to_path_buf(), Box::new(hook)));
}

#[cfg(test)]
pub(in crate::application::service::parallel_mode) fn install_after_normalization_quarantine_move_hook(
    slot_path: &Path,
    hook: impl FnOnce(&Path) + Send + 'static,
) {
    AFTER_NORMALIZATION_QUARANTINE_MOVE_HOOK
        .lock()
        .expect("normalization post-move test hook should not be poisoned")
        .push((slot_path.to_path_buf(), Box::new(hook)));
}

#[cfg(test)]
pub(in crate::application::service::parallel_mode) fn install_before_normalization_atomic_rename_hook(
    destination: &Path,
    hook: impl FnOnce(&Path) + Send + 'static,
) {
    BEFORE_NORMALIZATION_ATOMIC_RENAME_HOOK
        .lock()
        .expect("normalization atomic-rename test hook should not be poisoned")
        .push((destination.to_path_buf(), Box::new(hook)));
}

#[cfg(test)]
pub(in crate::application::service::parallel_mode) fn install_before_normalization_staging_provision_hook(
    slot_path: &Path,
    hook: impl FnOnce(&Path) + Send + 'static,
) {
    BEFORE_NORMALIZATION_STAGING_PROVISION_HOOK
        .lock()
        .expect("normalization staging test hook should not be poisoned")
        .push((slot_path.to_path_buf(), Box::new(hook)));
}

#[cfg(test)]
fn run_before_normalization_quarantine_hook(slot_path: &Path) {
    let hook = {
        let mut guard = BEFORE_NORMALIZATION_QUARANTINE_HOOK
            .lock()
            .expect("normalization quarantine test hook should not be poisoned");
        guard
            .iter()
            .position(|(expected_path, _)| worktree_paths_match(expected_path, slot_path))
            .map(|position| guard.swap_remove(position).1)
    };
    if let Some(hook) = hook {
        hook(slot_path);
    }
}

#[cfg(test)]
fn run_after_normalization_quarantine_move_hook(slot_path: &Path) {
    let hook = {
        let mut guard = AFTER_NORMALIZATION_QUARANTINE_MOVE_HOOK
            .lock()
            .expect("normalization post-move test hook should not be poisoned");
        guard
            .iter()
            .position(|(expected_path, _)| expected_path == slot_path)
            .map(|position| guard.swap_remove(position).1)
    };
    if let Some(hook) = hook {
        hook(slot_path);
    }
}

#[cfg(test)]
fn run_before_normalization_atomic_rename_hook(destination: &Path) {
    let hook = {
        let mut guard = BEFORE_NORMALIZATION_ATOMIC_RENAME_HOOK
            .lock()
            .expect("normalization atomic-rename test hook should not be poisoned");
        guard
            .iter()
            .position(|(expected_path, _)| expected_path == destination)
            .map(|position| guard.swap_remove(position).1)
    };
    if let Some(hook) = hook {
        hook(destination);
    }
}

#[cfg(test)]
fn run_before_normalization_staging_provision_hook(slot_path: &Path, replacement_path: &Path) {
    let hook = {
        let mut guard = BEFORE_NORMALIZATION_STAGING_PROVISION_HOOK
            .lock()
            .expect("normalization staging test hook should not be poisoned");
        guard
            .iter()
            .position(|(expected_path, _)| worktree_paths_match(expected_path, slot_path))
            .map(|position| guard.swap_remove(position).1)
    };
    if let Some(hook) = hook {
        hook(replacement_path);
    }
}

#[cfg(not(test))]
fn run_before_normalization_quarantine_hook(_slot_path: &Path) {}

#[cfg(not(test))]
fn run_after_normalization_quarantine_move_hook(_slot_path: &Path) {}

#[cfg(not(test))]
fn run_before_normalization_atomic_rename_hook(_destination: &Path) {}

#[cfg(not(test))]
fn run_before_normalization_staging_provision_hook(_slot_path: &Path, _replacement_path: &Path) {}
