//! Git plumbing으로 폼 하나만 추가한다. index/worktree/HEAD를 쓰는 명령은 없다.
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::application::port::outbound::github_automation_port::{
    AKRA_GITHUB_PUSH_REMOTE_CONFIG_KEY, AKRA_GITHUB_PUSH_REMOTE_ENV_VAR,
    credential_redacted_canonical_github_push_url, parse_github_repository_identity,
    resolve_github_push_remote_name_strict,
};
use crate::application::port::outbound::issue_form_repository_port::{
    ISSUE_FORM_PATH, IssueFormInstallPhase, IssueFormInstallPlan, IssueFormInstallRecord,
    IssueFormInstallSaveResult, IssueFormPreparation, IssueFormRepository, IssueFormRepositoryPort,
    IssueFormTarget, StoredIssueFormInstall,
};
use crate::{git_subprocess, subprocess};

const MAX_BLOB_BYTES: usize = 64 * 1024;
const MAX_TREE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Default)]
pub struct GitIssueFormRepositoryAdapter;

impl GitIssueFormRepositoryAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl IssueFormRepositoryPort for GitIssueFormRepositoryAdapter {
    fn inspect(&self, workspace_dir: &str, base_branch: &str) -> Result<IssueFormRepository> {
        if workspace_dir.is_empty()
            || workspace_dir.chars().any(char::is_control)
            || base_branch.is_empty()
            || base_branch.starts_with('-')
            || base_branch.chars().any(char::is_control)
        {
            bail!("issue form workspace or base branch is invalid");
        }
        let cwd = fs::canonicalize(workspace_dir).context("issue form workspace is unavailable")?;
        let cwd_text = path_text(&cwd)?;
        let version = raw_line(&cwd_text, &["version"])?;
        if !supported_git_version(&version) {
            bail!(
                "issue form installation requires Git 2.43 or newer for durable object/ref writes"
            );
        }
        raw_success(&cwd_text, &["check-ref-format", "--branch", base_branch])?;
        let root = fs::canonicalize(raw_line(&cwd_text, &["rev-parse", "--show-toplevel"])?)
            .context("issue form repository root is unavailable")?;
        let root_text = path_text(&root)?;
        let common = fs::canonicalize(raw_line(
            &root_text,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?)
        .context("issue form common Git directory is unavailable")?;
        let config = raw_output(
            &root_text,
            &["config", "--get", AKRA_GITHUB_PUSH_REMOTE_CONFIG_KEY],
            None,
        )?;
        let config_value = if config.status.success() {
            Some(output_line(config.stdout)?)
        } else if config.status.code() == Some(1) {
            None
        } else {
            bail!("issue form push remote configuration is unavailable");
        };
        let environment = match std::env::var(AKRA_GITHUB_PUSH_REMOTE_ENV_VAR) {
            Ok(value) => Some(value),
            Err(std::env::VarError::NotPresent) => None,
            Err(_) => bail!("issue form push remote configuration is invalid"),
        };
        let push_remote =
            resolve_github_push_remote_name_strict(environment.as_deref(), config_value.as_deref())
                .map_err(anyhow::Error::msg)?;
        let repository = IssueFormRepository {
            workspace_dir: root_text,
            common_git_dir: path_text(&common)?,
            push_remote,
        };
        verify_repository(&repository)?;
        Ok(repository)
    }

    fn load(
        &self,
        repository: &IssueFormRepository,
        target: &IssueFormTarget,
    ) -> Result<Option<StoredIssueFormInstall>> {
        validate_target(repository, target)?;
        validate_namespace(repository, target)?;
        let Some(object_id) = read_ref(repository, &target.journal_ref())? else {
            return Ok(None);
        };
        let bytes = read_blob(repository, &object_id)?;
        let record: IssueFormInstallRecord = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("issue form install journal is malformed"))?;
        if record.plan.target.intent_key() != target.intent_key() {
            bail!("issue form install journal target does not match");
        }
        validate_record(repository, &record)?;
        Ok(Some(StoredIssueFormInstall { object_id, record }))
    }

    fn prepare(
        &self,
        repository: &IssueFormRepository,
        target: &IssueFormTarget,
        base_commit_sha: &str,
        contents: &str,
    ) -> Result<IssueFormPreparation> {
        validate_target(repository, target)?;
        validate_namespace(repository, target)?;
        require_oid(base_commit_sha)?;
        if contents.len() > MAX_BLOB_BYTES
            || format!("{:x}", Sha256::digest(contents.as_bytes())) != target.asset_sha256
        {
            bail!("issue form asset does not match the frozen plan");
        }
        if local_form_conflicts(repository, contents.as_bytes())? {
            return Ok(IssueFormPreparation::Conflict);
        }
        match form_entry(repository, base_commit_sha)? {
            FormEntry::Regular(oid) => {
                return Ok(if read_blob(repository, &oid)? == contents.as_bytes() {
                    IssueFormPreparation::AlreadyInstalled
                } else {
                    IssueFormPreparation::Conflict
                });
            }
            FormEntry::Conflict => return Ok(IssueFormPreparation::Conflict),
            FormEntry::Missing => {}
        }
        if let Some(existing) = self.load(repository, target)? {
            return Ok(IssueFormPreparation::Prepared(Box::new(existing)));
        }
        let blob = git_line(
            repository,
            &["hash-object", "-w", "--no-filters", "--stdin"],
            Some(contents.as_bytes()),
        )?;
        require_oid(&blob)?;
        let root = git_line(
            repository,
            &[
                "rev-parse",
                "--verify",
                &format!("{base_commit_sha}^{{tree}}"),
            ],
            None,
        )?;
        let tree = insert_form_tree(
            repository,
            Some(&root),
            &[b".github", b"ISSUE_TEMPLATE", b"akra-work.yml"],
            &blob,
        )?;
        // Frozen parent time makes concurrent preparations deterministic; the journal chooses one immutable plan.
        let timestamp = git_line(
            repository,
            &["show", "-s", "--format=%ct", base_commit_sha],
            None,
        )?;
        if timestamp.is_empty()
            || timestamp.len() > 12
            || !timestamp.bytes().all(|byte| byte.is_ascii_digit())
        {
            bail!("issue form base timestamp is invalid");
        }
        let mut command =
            repository_command(repository, &["commit-tree", &tree, "-p", base_commit_sha]);
        let date = format!("@{timestamp} +0000");
        command
            .env("GIT_AUTHOR_NAME", "Akra")
            .env("GIT_AUTHOR_EMAIL", "akra@localhost")
            .env("GIT_COMMITTER_NAME", "Akra")
            .env("GIT_COMMITTER_EMAIL", "akra@localhost")
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date);
        let commit = checked_line(run_command(&mut command, Some(b"Add Akra issue form\n"))?)?;
        require_oid(&commit)?;
        let record = IssueFormInstallRecord {
            version: 1,
            plan: IssueFormInstallPlan {
                target: target.clone(),
                base_commit_sha: base_commit_sha.to_owned(),
                commit_sha: commit.clone(),
            },
            phase: IssueFormInstallPhase::Prepared,
            pull_request: None,
        };
        validate_plan(repository, &record.plan)?;
        // Retain the commit before publishing the JSON journal; a JSON OID alone is not a Git reachability edge.
        if !cas_ref(repository, &target.commit_ref(), &commit, None)?
            && read_ref(repository, &target.commit_ref())?.as_deref() != Some(commit.as_str())
        {
            return Ok(IssueFormPreparation::Conflict);
        }
        let object_id = write_record(repository, &record)?;
        if cas_ref(repository, &target.journal_ref(), &object_id, None)? {
            return Ok(IssueFormPreparation::Prepared(Box::new(
                StoredIssueFormInstall { object_id, record },
            )));
        }
        self.load(repository, target)?
            .map(|stored| IssueFormPreparation::Prepared(Box::new(stored)))
            .ok_or_else(|| anyhow::anyhow!("issue form install journal could not be committed"))
    }

    fn save(
        &self,
        repository: &IssueFormRepository,
        expected: &StoredIssueFormInstall,
        candidate: &IssueFormInstallRecord,
    ) -> Result<IssueFormInstallSaveResult> {
        let Some(current) = self.load(repository, &expected.record.plan.target)? else {
            bail!("issue form install journal disappeared");
        };
        if current.object_id != expected.object_id || current.record != expected.record {
            return Ok(IssueFormInstallSaveResult::Conflict);
        }
        if candidate == &current.record {
            return Ok(IssueFormInstallSaveResult::Saved(Box::new(current)));
        }
        if candidate.version != 1
            || candidate.plan != current.record.plan
            || !allowed_transition(current.record.phase, candidate.phase)
            || (candidate.phase == IssueFormInstallPhase::Confirmed)
                != candidate.pull_request.is_some()
        {
            bail!("issue form install journal transition is invalid");
        }
        validate_record(repository, candidate)?;
        let object_id = write_record(repository, candidate)?;
        if cas_ref(
            repository,
            &candidate.plan.target.journal_ref(),
            &object_id,
            Some(&expected.object_id),
        )? {
            Ok(IssueFormInstallSaveResult::Saved(Box::new(
                StoredIssueFormInstall {
                    object_id,
                    record: candidate.clone(),
                },
            )))
        } else {
            Ok(IssueFormInstallSaveResult::Conflict)
        }
    }
}

fn allowed_transition(from: IssueFormInstallPhase, to: IssueFormInstallPhase) -> bool {
    use IssueFormInstallPhase::*;
    matches!(
        (from, to),
        (Prepared, PushSent | BranchReady)
            | (PushSent, BranchReady)
            | (BranchReady, Sent | Confirmed)
            | (Sent, Confirmed)
    )
}

fn supported_git_version(value: &str) -> bool {
    let Some(version) = value.strip_prefix("git version ") else {
        return false;
    };
    let mut parts = version.split('.');
    let major = parts.next().and_then(|v| v.parse::<u32>().ok());
    let minor = parts.next().and_then(|v| v.parse::<u32>().ok());
    matches!((major, minor), (Some(major), Some(minor)) if (major, minor) >= (2, 43))
}

fn path_text(path: &Path) -> Result<String> {
    let text = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("issue form Git path is not UTF-8"))?;
    if text.chars().any(char::is_control) {
        bail!("issue form Git path is invalid");
    }
    Ok(text.to_owned())
}

fn verify_repository(repository: &IssueFormRepository) -> Result<()> {
    for value in [&repository.workspace_dir, &repository.common_git_dir] {
        let path = Path::new(value);
        if !path.is_absolute() || fs::canonicalize(path)? != path || !path.is_dir() {
            bail!("issue form repository identity changed");
        }
    }
    let common = raw_line(
        &repository.workspace_dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    if fs::canonicalize(common)? != Path::new(&repository.common_git_dir) {
        bail!("issue form common repository identity changed");
    }
    for path in [
        PathBuf::from(&repository.common_git_dir),
        Path::new(&repository.common_git_dir).join("objects"),
    ] {
        let metadata = fs::symlink_metadata(path)?;
        if is_link(&metadata) || !metadata.is_dir() {
            bail!("issue form Git storage is unsafe");
        }
    }
    Ok(())
}

fn validate_target(repository: &IssueFormRepository, target: &IssueFormTarget) -> Result<()> {
    verify_repository(repository)?;
    if target.asset_sha256.len() != 64
        || !target
            .asset_sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || target.base_branch.is_empty()
        || target.base_branch.starts_with('-')
        || target.base_branch.chars().any(char::is_control)
        || parse_github_repository_identity(&target.push_url)
            .is_none_or(|v| !v.eq_ignore_ascii_case(&target.repository))
        || credential_redacted_canonical_github_push_url(&target.push_url).as_deref()
            != Some(target.push_url.as_str())
    {
        bail!("issue form frozen target is invalid");
    }
    resolve_github_push_remote_name_strict(Some(&target.push_remote), None)
        .map_err(anyhow::Error::msg)?;
    git_success(
        repository,
        &["check-ref-format", "--branch", &target.base_branch],
        None,
    )
}

fn validate_namespace(repository: &IssueFormRepository, target: &IssueFormTarget) -> Result<()> {
    let mut directory = PathBuf::from(&repository.common_git_dir);
    for component in ["refs", "akra", "issue-form", &target.intent_key()] {
        directory.push(component);
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if is_link(&metadata) || !metadata.is_dir() => {
                bail!("issue form ref namespace is unsafe")
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => bail!("issue form ref namespace is unreadable"),
        }
    }
    for name in ["journal", "base", "commit"] {
        match fs::symlink_metadata(directory.join(name)) {
            Ok(metadata) if is_link(&metadata) || !metadata.is_file() => {
                bail!("issue form ref is unsafe")
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => bail!("issue form ref is unreadable"),
        }
        let reference = format!("refs/akra/issue-form/{}/{name}", target.intent_key());
        let output = git_output(repository, &["symbolic-ref", "-q", &reference], None)?;
        if output.status.success() {
            bail!("issue form refs must not be symbolic");
        }
        if output.status.code() != Some(1) {
            bail!("issue form ref identity is unavailable");
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_link(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_type().is_symlink() || metadata.file_attributes() & 0x0400 != 0
}

#[cfg(not(windows))]
fn is_link(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn local_form_conflicts(repository: &IssueFormRepository, contents: &[u8]) -> Result<bool> {
    let mut path = PathBuf::from(&repository.workspace_dir);
    for component in [".github", "ISSUE_TEMPLATE"] {
        path.push(component);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if is_link(&metadata) || !metadata.is_dir() => return Ok(true),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(_) => bail!("issue form checkout path is unreadable"),
        }
    }
    path.push("akra-work.yml");
    match fs::symlink_metadata(&path) {
        Ok(metadata)
            if is_link(&metadata)
                || !metadata.is_file()
                || metadata.len() > MAX_BLOB_BYTES as u64 =>
        {
            Ok(true)
        }
        Ok(_) => Ok(fs::read(path)? != contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => bail!("issue form checkout file is unreadable"),
    }
}

struct TreeEntry {
    mode: String,
    kind: String,
    oid: String,
    name: Vec<u8>,
}

fn read_tree(repository: &IssueFormRepository, tree: &str) -> Result<Vec<TreeEntry>> {
    require_oid(tree)?;
    let bytes = checked_output(git_output(repository, &["ls-tree", "-z", tree], None)?)?;
    let mut entries = Vec::new();
    let mut names = HashSet::new();
    for entry in bytes
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        let Some(tab) = entry.iter().position(|byte| *byte == b'\t') else {
            bail!("issue form Git tree is malformed");
        };
        let header = std::str::from_utf8(&entry[..tab])
            .map_err(|_| anyhow::anyhow!("issue form Git tree header is invalid"))?;
        let fields = header.split(' ').collect::<Vec<_>>();
        let name = entry[tab + 1..].to_vec();
        if fields.len() != 3
            || name.is_empty()
            || name.contains(&b'/')
            || !names.insert(name.clone())
        {
            bail!("issue form Git tree is malformed");
        }
        require_oid(fields[2])?;
        entries.push(TreeEntry {
            mode: fields[0].to_owned(),
            kind: fields[1].to_owned(),
            oid: fields[2].to_owned(),
            name,
        });
    }
    Ok(entries)
}

enum FormEntry {
    Missing,
    Regular(String),
    Conflict,
}

fn form_entry(repository: &IssueFormRepository, commit: &str) -> Result<FormEntry> {
    require_oid(commit)?;
    let mut tree = git_line(
        repository,
        &["rev-parse", "--verify", &format!("{commit}^{{tree}}")],
        None,
    )?;
    for component in [b".github".as_slice(), b"ISSUE_TEMPLATE".as_slice()] {
        let Some(entry) = read_tree(repository, &tree)?
            .into_iter()
            .find(|entry| entry.name == component)
        else {
            return Ok(FormEntry::Missing);
        };
        if entry.kind != "tree" || entry.mode != "040000" {
            return Ok(FormEntry::Conflict);
        }
        tree = entry.oid;
    }
    let Some(entry) = read_tree(repository, &tree)?
        .into_iter()
        .find(|entry| entry.name == b"akra-work.yml")
    else {
        return Ok(FormEntry::Missing);
    };
    Ok(if entry.kind == "blob" && entry.mode == "100644" {
        FormEntry::Regular(entry.oid)
    } else {
        FormEntry::Conflict
    })
}

fn insert_form_tree(
    repository: &IssueFormRepository,
    tree: Option<&str>,
    segments: &[&[u8]],
    blob: &str,
) -> Result<String> {
    let mut entries = match tree {
        Some(tree) => read_tree(repository, tree)?,
        None => Vec::new(),
    };
    let index = entries.iter().position(|entry| entry.name == segments[0]);
    let (mode, kind, oid) = if segments.len() == 1 {
        if index.is_some() {
            bail!("issue form path already exists in the frozen tree");
        }
        ("100644", "blob", blob.to_owned())
    } else {
        let existing = index.map(|index| &entries[index]);
        if existing.is_some_and(|entry| entry.kind != "tree" || entry.mode != "040000") {
            bail!("issue form parent path is not a directory");
        }
        let child = insert_form_tree(
            repository,
            existing.map(|entry| entry.oid.as_str()),
            &segments[1..],
            blob,
        )?;
        ("040000", "tree", child)
    };
    let next = TreeEntry {
        mode: mode.to_owned(),
        kind: kind.to_owned(),
        oid,
        name: segments[0].to_vec(),
    };
    if let Some(index) = index {
        entries[index] = next;
    } else {
        entries.push(next);
    }
    let mut input = Vec::new();
    for entry in entries {
        input
            .extend_from_slice(format!("{} {} {}\t", entry.mode, entry.kind, entry.oid).as_bytes());
        input.extend_from_slice(&entry.name);
        input.push(0);
    }
    git_line(repository, &["mktree", "-z"], Some(&input))
}

fn validate_record(
    repository: &IssueFormRepository,
    record: &IssueFormInstallRecord,
) -> Result<()> {
    if record.version != 1
        || (record.phase == IssueFormInstallPhase::Confirmed) != record.pull_request.is_some()
    {
        bail!("issue form install journal state is invalid");
    }
    validate_target(repository, &record.plan.target)?;
    if let Some(pr) = &record.pull_request
        && (pr.number == 0
            || !pr.url.eq_ignore_ascii_case(&format!(
                "https://github.com/{}/pull/{}",
                record.plan.target.repository, pr.number,
            )))
    {
        bail!("issue form install journal PR is invalid");
    }
    validate_plan(repository, &record.plan)?;
    if read_ref(repository, &record.plan.target.commit_ref())?.as_deref()
        != Some(record.plan.commit_sha.as_str())
    {
        bail!("issue form staged commit reference changed");
    }
    Ok(())
}

fn validate_plan(repository: &IssueFormRepository, plan: &IssueFormInstallPlan) -> Result<()> {
    require_oid(&plan.base_commit_sha)?;
    require_oid(&plan.commit_sha)?;
    let parents = git_line(
        repository,
        &["rev-list", "--parents", "-n", "1", &plan.commit_sha],
        None,
    )?;
    if parents != format!("{} {}", plan.commit_sha, plan.base_commit_sha) {
        bail!("issue form staged commit has a different parent");
    }
    let changed = checked_output(git_output(
        repository,
        &[
            "diff-tree",
            "--no-commit-id",
            "--name-only",
            "--no-renames",
            "-r",
            "-z",
            &plan.base_commit_sha,
            &plan.commit_sha,
        ],
        None,
    )?)?;
    if changed != format!("{ISSUE_FORM_PATH}\0").as_bytes()
        || !matches!(
            form_entry(repository, &plan.base_commit_sha)?,
            FormEntry::Missing
        )
    {
        bail!("issue form staged commit must only add the fixed form path");
    }
    let FormEntry::Regular(blob) = form_entry(repository, &plan.commit_sha)? else {
        bail!("issue form staged path is not a regular file");
    };
    if format!("{:x}", Sha256::digest(read_blob(repository, &blob)?)) != plan.target.asset_sha256 {
        bail!("issue form staged content changed");
    }
    Ok(())
}

fn read_blob(repository: &IssueFormRepository, oid: &str) -> Result<Vec<u8>> {
    require_oid(oid)?;
    if git_line(repository, &["cat-file", "-t", oid], None)? != "blob" {
        bail!("issue form object is not a blob");
    }
    let size = git_line(repository, &["cat-file", "-s", oid], None)?
        .parse::<usize>()
        .map_err(|_| anyhow::anyhow!("issue form object size is invalid"))?;
    if size > MAX_BLOB_BYTES {
        bail!("issue form object exceeds its limit");
    }
    let bytes = checked_output(git_output(repository, &["cat-file", "blob", oid], None)?)?;
    if bytes.len() != size {
        bail!("issue form object size changed");
    }
    Ok(bytes)
}

fn read_ref(repository: &IssueFormRepository, reference: &str) -> Result<Option<String>> {
    let output = git_output(
        repository,
        &["rev-parse", "--verify", "--quiet", reference],
        None,
    )?;
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    let oid = checked_line(output)?;
    require_oid(&oid)?;
    Ok(Some(oid))
}

fn cas_ref(
    repository: &IssueFormRepository,
    reference: &str,
    next: &str,
    expected: Option<&str>,
) -> Result<bool> {
    require_oid(next)?;
    if let Some(expected) = expected {
        require_oid(expected)?;
    }
    let absent = "0".repeat(next.len());
    // A competing writer can hold the lock before the ref value changes. Let Git
    // wait for that transaction while retaining the same expected-value guard.
    // This bounded lock wait does not retry a journal transition or remote write.
    let output = git_output(
        repository,
        &[
            "-c",
            "core.filesRefLockTimeout=5000",
            "update-ref",
            "--no-deref",
            reference,
            next,
            expected.unwrap_or(&absent),
        ],
        None,
    )?;
    if output.status.success() {
        return Ok(true);
    }
    let actual = read_ref(repository, reference)?;
    if actual.as_deref() != expected {
        return Ok(false);
    }
    bail!("issue form durable Git ref update failed")
}

fn write_record(
    repository: &IssueFormRepository,
    record: &IssueFormInstallRecord,
) -> Result<String> {
    let bytes = serde_json::to_vec(record)
        .map_err(|_| anyhow::anyhow!("issue form install journal cannot serialize"))?;
    if bytes.len() > MAX_BLOB_BYTES {
        bail!("issue form install journal exceeds its limit");
    }
    git_line(
        repository,
        &["hash-object", "-w", "--no-filters", "--stdin"],
        Some(&bytes),
    )
}

fn require_oid(oid: &str) -> Result<()> {
    if !matches!(oid.len(), 40 | 64)
        || !oid
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        bail!("issue form Git object identity is invalid");
    }
    Ok(())
}

// Preserve canonical identities for filesystem checks; only Git argv changes its path spelling.
fn git_path_argument(path: &str) -> String {
    #[cfg(windows)]
    {
        if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{rest}");
        }
        if let Some(rest) = path.strip_prefix(r"\\?\")
            && rest.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
            && rest.as_bytes().get(1) == Some(&b':')
            && rest.as_bytes().get(2) == Some(&b'\\')
        {
            return rest.to_owned();
        }
    }
    path.to_owned()
}

fn repository_command(repository: &IssueFormRepository, args: &[&str]) -> Command {
    let workspace_argument = git_path_argument(&repository.workspace_dir);
    let common_argument = git_path_argument(&repository.common_git_dir);
    let mut command = git_subprocess::command([
        "-C",
        workspace_argument.as_str(),
        "--git-dir",
        common_argument.as_str(),
        "-c",
        "core.fsync=loose-object,reference",
        "-c",
        "core.fsyncMethod=fsync",
    ]);
    command.args(args);
    command.current_dir(&repository.workspace_dir);
    command
}

fn git_output(
    repository: &IssueFormRepository,
    args: &[&str],
    input: Option<&[u8]>,
) -> Result<Output> {
    run_command(&mut repository_command(repository, args), input)
}

fn git_line(
    repository: &IssueFormRepository,
    args: &[&str],
    input: Option<&[u8]>,
) -> Result<String> {
    checked_line(git_output(repository, args, input)?)
}

fn git_success(
    repository: &IssueFormRepository,
    args: &[&str],
    input: Option<&[u8]>,
) -> Result<()> {
    checked_output(git_output(repository, args, input)?).map(|_| ())
}

fn raw_output(cwd: &str, args: &[&str], input: Option<&[u8]>) -> Result<Output> {
    let cwd_argument = git_path_argument(cwd);
    let mut command = git_subprocess::command(["-C", cwd_argument.as_str()]);
    command.args(args).current_dir(cwd);
    run_command(&mut command, input)
}

fn raw_line(cwd: &str, args: &[&str]) -> Result<String> {
    checked_line(raw_output(cwd, args, None)?)
}
fn raw_success(cwd: &str, args: &[&str]) -> Result<()> {
    checked_output(raw_output(cwd, args, None)?).map(|_| ())
}

fn run_command(command: &mut Command, input: Option<&[u8]>) -> Result<Output> {
    subprocess::command_output_with_input_timeout_and_limits(
        command,
        "issue form Git operation",
        input.unwrap_or_default(),
        Duration::from_secs(30),
        MAX_TREE_BYTES,
        16 * 1024,
    )
    .context("issue form Git operation could not finish")
}

fn checked_output(output: Output) -> Result<Vec<u8>> {
    if !output.status.success() {
        bail!("issue form Git operation failed");
    }
    // Never accept a silently ignored durability component.
    if String::from_utf8_lossy(&output.stderr).contains("ignoring unknown core.fsync component") {
        bail!("issue form Git durability settings are unsupported");
    }
    Ok(output.stdout)
}

fn checked_line(output: Output) -> Result<String> {
    output_line(checked_output(output)?)
}

fn output_line(bytes: Vec<u8>) -> Result<String> {
    let text = String::from_utf8(bytes)
        .map_err(|_| anyhow::anyhow!("issue form Git response is not UTF-8"))?;
    let text = text.strip_suffix('\n').unwrap_or(&text);
    let text = text.strip_suffix('\r').unwrap_or(text);
    if text.chars().any(char::is_control) {
        bail!("issue form Git response contains unexpected lines");
    }
    Ok(text.to_owned())
}

#[cfg(test)]
mod tests;
