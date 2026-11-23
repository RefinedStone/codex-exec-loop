//! Host-selected evidence from exact Git objects. The model never receives a workspace path.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::application::port::outbound::issue_assessment_evidence_port::{
    ISSUE_ASSESSMENT_EVIDENCE_VERSION, IssueAssessmentEvidenceEntry,
    IssueAssessmentEvidenceError as Error, IssueAssessmentEvidencePort,
    IssueAssessmentEvidenceRequest, IssueAssessmentRepositoryEvidence,
    MAX_ISSUE_ASSESSMENT_EVIDENCE_FILE_BYTES, MAX_ISSUE_ASSESSMENT_EVIDENCE_PATH_BYTES,
    MAX_ISSUE_ASSESSMENT_EVIDENCE_PATHS, MAX_ISSUE_ASSESSMENT_EVIDENCE_TOTAL_BYTES,
};
use crate::domain::issue_automation::policy::{
    BUILTIN_ISSUE_PROTECTION_PATHS, IssueAutomationPolicy,
};
use crate::domain::issue_automation::triage::matches_glob;
use crate::domain::issue_masking::IssueTextField;
use crate::domain::issue_secret_spans::secret_spans;
use crate::subprocess;

use super::parallel_commit_diff::{
    ensure_exact_commit, ensure_exact_worktree_root, safe_git_command,
};

const MAX_TREE_ROW_BYTES: usize = MAX_ISSUE_ASSESSMENT_EVIDENCE_PATH_BYTES + 128;
const MAX_STDERR_BYTES: usize = 4096;
const MAX_PATH_SEGMENTS: usize = 32;
const ROOT_FILES: &[&str] = &[
    "README.md",
    "README.rst",
    "README.txt",
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "settings.gradle",
    "settings.gradle.kts",
    "go.mod",
];
const DIRECTORY_PREFIXES: &[&str] = &[
    "src/",
    "tests/",
    "docs/",
    "desktop/src/",
    "desktop/test/",
    "desktop/scripts/",
];

#[derive(Default)]
pub(crate) struct GitIssueAssessmentEvidenceAdapter;

impl GitIssueAssessmentEvidenceAdapter {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl IssueAssessmentEvidencePort for GitIssueAssessmentEvidenceAdapter {
    fn current_commit_sha(&self, workspace: &str) -> Result<String, Error> {
        current_head(&checked_workspace(workspace)?)
    }

    fn read(
        &self,
        request: IssueAssessmentEvidenceRequest<'_>,
    ) -> Result<IssueAssessmentRepositoryEvidence, Error> {
        if !valid_oid(request.commit_sha) {
            return Err(Error::UnsupportedEntry);
        }
        let paths = permitted_paths(request.selected_paths, request.policy)?;
        let workspace = checked_workspace(request.workspace_directory)?;
        ensure_exact_commit(&workspace, request.commit_sha).map_err(|_| Error::Unavailable)?;
        if current_head(&workspace)? != request.commit_sha {
            return Err(Error::SourceChanged);
        }
        let tree = format!("{}^{{tree}}", request.commit_sha);
        let tree = oid_line(&git(&workspace, &["rev-parse", "--verify", &tree], 128)?)?;
        if tree.len() != request.commit_sha.len() {
            return Err(Error::Incomplete);
        }
        let mut entries = Vec::with_capacity(paths.len());
        let mut total_bytes = 0usize;
        for path in paths {
            let entry = read_path(&workspace, &tree, &path)?;
            if let IssueAssessmentEvidenceEntry::File { text, .. } = &entry {
                total_bytes = total_bytes.checked_add(text.len()).ok_or(Error::TooLarge)?;
                if total_bytes > MAX_ISSUE_ASSESSMENT_EVIDENCE_TOTAL_BYTES {
                    return Err(Error::TooLarge);
                }
            }
            entries.push(entry);
        }
        // A capture that overlapped a new commit is never represented as the current input.
        if current_head(&workspace)? != request.commit_sha {
            return Err(Error::SourceChanged);
        }
        let encoded = serde_json::to_vec(&(
            ISSUE_ASSESSMENT_EVIDENCE_VERSION,
            request.commit_sha,
            &entries,
        ))
        .map_err(|_| Error::Incomplete)?;
        Ok(IssueAssessmentRepositoryEvidence {
            commit_sha: request.commit_sha.to_owned(),
            selection_version: ISSUE_ASSESSMENT_EVIDENCE_VERSION.to_owned(),
            snapshot_sha256: format!("{:x}", Sha256::digest(encoded)),
            entries,
        })
    }
}

fn permitted_paths(paths: &[String], policy: &IssueAutomationPolicy) -> Result<Vec<String>, Error> {
    policy.validate().map_err(|_| Error::Unavailable)?;
    if paths.is_empty() {
        return Err(Error::NoEligiblePaths);
    }
    if paths.len() > MAX_ISSUE_ASSESSMENT_EVIDENCE_PATHS {
        return Err(Error::TooLarge);
    }
    let mut selected = BTreeSet::new();
    for path in paths {
        if path.is_empty()
            || path.len() > MAX_ISSUE_ASSESSMENT_EVIDENCE_PATH_BYTES
            || path.chars().any(|character| {
                character.is_control()
                    || character.is_whitespace()
                    || matches!(
                        character,
                        '\\' | ':' | '*' | '?' | '[' | ']' | '{' | '}' | '!'
                    )
            })
            || path
                .split('/')
                .any(|part| part.is_empty() || matches!(part, "." | ".."))
        {
            return Err(Error::UnsupportedEntry);
        }
        if path.split('/').count() > MAX_PATH_SEGMENTS {
            return Err(Error::TooLarge);
        }
        for pattern in BUILTIN_ISSUE_PROTECTION_PATHS
            .iter()
            .copied()
            .chain(policy.protection.paths.iter().map(String::as_str))
        {
            // Reading is also conservative on case-insensitive workstations. This uses
            // the same matcher, with additional exclusions rather than broader access.
            let protected = matches_glob(pattern, path).ok_or(Error::Unavailable)?
                || matches_glob(&pattern.to_ascii_lowercase(), &path.to_ascii_lowercase())
                    .ok_or(Error::Unavailable)?;
            if protected {
                return Err(Error::ProtectedPath);
            }
        }
        if path.split('/').any(|part| part.starts_with('.'))
            || !(ROOT_FILES.contains(&path.as_str())
                || DIRECTORY_PREFIXES
                    .iter()
                    .any(|prefix| path.starts_with(prefix)))
        {
            return Err(Error::NoEligiblePaths);
        }
        if !secret_spans(IssueTextField::Body, path).is_empty() {
            return Err(Error::InvalidText);
        }
        if !selected.insert(path.clone()) {
            return Err(Error::Incomplete);
        }
    }
    Ok(selected.into_iter().collect())
}

fn checked_workspace(workspace: &str) -> Result<PathBuf, Error> {
    if workspace.is_empty() || workspace.chars().any(char::is_control) {
        return Err(Error::Unavailable);
    }
    let workspace = std::fs::canonicalize(workspace).map_err(|_| Error::Unavailable)?;
    crate::git_execution_guard::ensure_host_git_execution_config_safe(&workspace)
        .map_err(|_| Error::Unavailable)?;
    ensure_exact_worktree_root(&workspace).map_err(|_| Error::Unavailable)?;
    Ok(workspace)
}

fn current_head(workspace: &Path) -> Result<String, Error> {
    oid_line(&git(
        workspace,
        &["rev-parse", "--verify", "HEAD^{commit}"],
        128,
    )?)
}

fn read_path(
    workspace: &Path,
    root_tree: &str,
    path: &str,
) -> Result<IssueAssessmentEvidenceEntry, Error> {
    let components: Vec<_> = path.split('/').collect();
    let mut tree = root_tree.to_owned();
    for (index, component) in components.iter().enumerate() {
        let bytes = git(
            workspace,
            &["ls-tree", "-z", &tree, "--", component],
            MAX_TREE_ROW_BYTES,
        )?;
        let Some(entry) = tree_entry(&bytes, component, root_tree.len())? else {
            return Ok(IssueAssessmentEvidenceEntry::Absent {
                path: path.to_owned(),
            });
        };
        if index + 1 < components.len() {
            if entry.mode != "040000" || entry.kind != "tree" {
                return Err(Error::UnsupportedEntry);
            }
            tree = entry.oid;
        } else {
            if !matches!(entry.mode.as_str(), "100644" | "100755") || entry.kind != "blob" {
                return Err(Error::UnsupportedEntry);
            }
            let text = read_blob(workspace, &entry.oid)?;
            return Ok(IssueAssessmentEvidenceEntry::File {
                path: path.to_owned(),
                blob_sha: entry.oid,
                text,
            });
        }
    }
    Err(Error::Incomplete)
}

struct TreeEntry {
    mode: String,
    kind: String,
    oid: String,
}

fn tree_entry(bytes: &[u8], name: &str, oid_length: usize) -> Result<Option<TreeEntry>, Error> {
    if bytes.is_empty() {
        return Ok(None);
    }
    let row = bytes.strip_suffix(&[0]).ok_or(Error::Incomplete)?;
    if row.contains(&0) {
        return Err(Error::Incomplete);
    }
    let tab = row
        .iter()
        .position(|byte| *byte == b'\t')
        .ok_or(Error::Incomplete)?;
    if &row[tab + 1..] != name.as_bytes() {
        return Err(Error::Incomplete);
    }
    let header = std::str::from_utf8(&row[..tab]).map_err(|_| Error::Incomplete)?;
    let fields: Vec<_> = header.split(' ').collect();
    if fields.len() != 3 || !valid_oid(fields[2]) || fields[2].len() != oid_length {
        return Err(Error::Incomplete);
    }
    Ok(Some(TreeEntry {
        mode: fields[0].to_owned(),
        kind: fields[1].to_owned(),
        oid: fields[2].to_owned(),
    }))
}

fn read_blob(workspace: &Path, oid: &str) -> Result<String, Error> {
    if git(workspace, &["cat-file", "-t", oid], 32)? != b"blob\n" {
        return Err(Error::UnsupportedEntry);
    }
    let size = git(workspace, &["cat-file", "-s", oid], 64)?;
    let size = std::str::from_utf8(&size).map_err(|_| Error::Incomplete)?;
    let size = size.strip_suffix('\n').ok_or(Error::Incomplete)?;
    if size.is_empty() || !size.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Error::Incomplete);
    }
    let size = size.parse::<usize>().map_err(|_| Error::TooLarge)?;
    if size > MAX_ISSUE_ASSESSMENT_EVIDENCE_FILE_BYTES {
        return Err(Error::TooLarge);
    }
    let bytes = git(
        workspace,
        &["cat-file", "blob", oid],
        MAX_ISSUE_ASSESSMENT_EVIDENCE_FILE_BYTES,
    )?;
    if bytes.len() != size {
        return Err(Error::Incomplete);
    }
    shareable_text(bytes)
}

fn shareable_text(bytes: Vec<u8>) -> Result<String, Error> {
    let text = String::from_utf8(bytes).map_err(|_| Error::InvalidText)?;
    if text
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
        || !secret_spans(IssueTextField::Body, &text).is_empty()
    {
        return Err(Error::InvalidText);
    }
    Ok(text)
}

fn valid_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn oid_line(bytes: &[u8]) -> Result<String, Error> {
    let text = std::str::from_utf8(bytes).map_err(|_| Error::Incomplete)?;
    let value = text.strip_suffix('\n').ok_or(Error::Incomplete)?;
    if !valid_oid(value) {
        return Err(Error::Incomplete);
    }
    Ok(value.to_owned())
}

fn git(workspace: &Path, args: &[&str], maximum: usize) -> Result<Vec<u8>, Error> {
    let mut command = safe_git_command(workspace);
    command.arg("--literal-pathspecs").args(args);
    let child = subprocess::spawn(&mut command).map_err(|_| Error::Unavailable)?;
    let output = subprocess::wait_with_output_timeout_and_limits(
        child,
        "git issue assessment evidence",
        subprocess::configured_subprocess_timeout(),
        maximum,
        MAX_STDERR_BYTES,
    )
    .map_err(|_| Error::Incomplete)?;
    if !output.status.success() {
        return Err(Error::Unavailable);
    }
    Ok(output.stdout)
}

#[cfg(test)]
pub(crate) mod fake;
#[cfg(test)]
mod tests;
