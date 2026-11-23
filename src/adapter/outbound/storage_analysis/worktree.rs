use super::index::StorageIndex;
use crate::domain::storage_analysis::*;
use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, params};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct WorktreeRecord {
    pub path: Vec<u8>,
    pub branch: Option<Vec<u8>>,
    pub head: Option<Vec<u8>>,
    pub bare: bool,
    pub detached: bool,
    pub locked: Option<Vec<u8>>,
    pub prunable: Option<Vec<u8>>,
}
/// NUL is both the field delimiter and, as an empty field, the record delimiter.
pub(super) fn parse_porcelain(bytes: &[u8]) -> Result<Vec<WorktreeRecord>> {
    let mut output = Vec::new();
    let mut record = WorktreeRecord::default();
    for field in bytes.split(|v| *v == 0) {
        if field.is_empty() {
            if !record.path.is_empty() {
                output.push(std::mem::take(&mut record));
            }
            continue;
        }
        let split = field.iter().position(|v| *v == b' ').unwrap_or(field.len());
        let key = &field[..split];
        let value = if split < field.len() {
            &field[split + 1..]
        } else {
            &[]
        };
        match key {
            b"worktree" => record.path = value.to_vec(),
            b"HEAD" => record.head = Some(value.to_vec()),
            b"branch" => record.branch = Some(value.to_vec()),
            b"bare" => record.bare = true,
            b"detached" => record.detached = true,
            b"locked" => record.locked = Some(value.to_vec()),
            b"prunable" => record.prunable = Some(value.to_vec()),
            _ => {}
        }
    }
    if !record.path.is_empty() {
        output.push(record);
    }
    Ok(output)
}
fn git(path: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let mut command = crate::git_subprocess::command([OsStr::new("-C"), path.as_os_str()]);
    command
        .args(["--no-optional-locks", "-c", "core.fsmonitor=false"])
        .args(args);
    let output = crate::subprocess::command_output_with_input_timeout_and_limits(
        &mut command,
        "storage Git metadata",
        b"",
        Duration::from_secs(3),
        1024 * 1024,
        64 * 1024,
    )?;
    if !output.status.success() {
        bail!("Git metadata query failed");
    }
    Ok(output.stdout)
}
fn text(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes)
        .trim_end_matches(['\r', '\n'])
        .to_string()
}
fn key(path: &Path) -> String {
    let value = std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned();
    #[cfg(windows)]
    {
        value.to_lowercase()
    }
    #[cfg(not(windows))]
    {
        value
    }
}
fn evidence(code: &str, detail: impl Into<String>) -> StorageEvidence {
    StorageEvidence {
        code: code.into(),
        detail: detail.into(),
        observed_at: chrono::Utc::now().to_rfc3339(),
    }
}

pub(super) fn annotate(
    db: &StorageIndex,
    cancelled: &AtomicBool,
    runtime_probe: Option<&(dyn Fn(&Path) -> Vec<StorageEvidence> + Send + Sync)>,
) -> Result<()> {
    let mut cache: BTreeMap<String, Result<Vec<WorktreeRecord>, String>> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut candidates=db.conn.prepare("SELECT DISTINCT parent FROM nodes WHERE name='.git' UNION
        SELECT parent FROM nodes WHERE name IN ('HEAD','objects','refs') GROUP BY parent HAVING count(DISTINCT name)=3")?;
    for row in candidates.query_map([], |r| r.get::<_, Option<i64>>(0))? {
        if cancelled.load(Ordering::Acquire) {
            break;
        }
        let Some(id) = row? else {
            continue;
        };
        if !seen.insert(id) {
            continue;
        }
        let path = db.path(&format!("n{id}"))?;
        let mut reasons = runtime_probe.map(|probe| probe(&path)).unwrap_or_default();
        if reasons.is_empty() {
            reasons.push(evidence(
                "runtime_unknown",
                "Runtime ownership has not been verified; no session or kernel was started",
            ));
        }
        let common = git(
            &path,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )
        .map(text);
        match common {
            Err(_) => reasons.push(evidence(
                "git_unavailable",
                "Git metadata could not be verified",
            )),
            Ok(common) => {
                let common = PathBuf::from(common);
                let entry = cache.entry(key(&common)).or_insert_with(|| {
                    git(&path, &["worktree", "list", "--porcelain", "-z"])
                        .and_then(|bytes| parse_porcelain(&bytes))
                        .map_err(|e| e.to_string())
                });
                match entry {
                    Ok(records) => {
                        let current = records.iter().find(|r| {
                            key(Path::new(&String::from_utf8_lossy(&r.path).into_owned()))
                                == key(&path)
                        });
                        if let Some(current) = current {
                            if !current.bare
                                && records
                                    .first()
                                    .is_some_and(|first| first.path == current.path)
                            {
                                reasons.push(evidence(
                                    "main_checkout",
                                    "Git registers this as the main checkout",
                                ));
                            }
                            if !current.bare
                                && records
                                    .first()
                                    .is_some_and(|first| first.path != current.path)
                            {
                                reasons.push(evidence(
                                    "linked_worktree",
                                    "Git registers this as a linked worktree",
                                ));
                            }
                            if current.locked.is_some() {
                                reasons.push(evidence(
                                    "git_locked",
                                    "Git worktree registration is locked",
                                ));
                            }
                            if current.prunable.is_some() {
                                reasons.push(evidence("git_prunable","Registration is prunable; this does not establish reclaimable bytes"));
                            }
                            if current.detached {
                                reasons.push(evidence("detached_head", "HEAD is detached"));
                            }
                            if current.bare {
                                reasons
                                    .push(evidence("bare_repository", "This is a bare repository"));
                            }
                            if let Some(branch) = &current.branch {
                                reasons.push(evidence(
                                    "branch",
                                    String::from_utf8_lossy(branch).into_owned(),
                                ));
                            }
                        } else {
                            reasons.push(evidence(
                                "registration_unmatched",
                                "No matching worktree registration was verified",
                            ));
                        }
                    }
                    Err(_) => reasons.push(evidence(
                        "registration_unknown",
                        "Worktree registration query failed",
                    )),
                }
                if git(&path, &["rev-parse", "--show-superproject-working-tree"])
                    .is_ok_and(|v| !text(v).is_empty())
                {
                    reasons.push(evidence(
                        "submodule",
                        "Git identifies this checkout as a submodule",
                    ));
                }
                match git(
                    &path,
                    &["status", "--porcelain=v1", "-z", "--untracked-files=normal"],
                ) {
                    Ok(status) => {
                        if status.split(|b| *b == 0).any(|p| p.starts_with(b"?? ")) {
                            reasons
                                .push(evidence("untracked_files", "Untracked paths were observed"));
                        }
                        if status
                            .split(|b| *b == 0)
                            .any(|p| !p.is_empty() && !p.starts_with(b"?? "))
                        {
                            reasons.push(evidence(
                                "modified_files",
                                "Working tree or index changes were observed",
                            ));
                        }
                    }
                    Err(_) => reasons.push(evidence(
                        "status_unknown",
                        "Working tree status could not be verified",
                    )),
                }
                match git(&path, &["rev-list", "--count", "@{upstream}..HEAD"]).map(text) {
                    Ok(count) if count.parse::<u64>().is_ok_and(|v| v > 0) => {
                        reasons.push(evidence(
                            "local_commits",
                            format!(
                                "{count} commits ahead of the locally observed upstream reference"
                            ),
                        ))
                    }
                    Ok(count) if count == "0" => reasons.push(evidence(
                        "upstream_observed",
                        "No ahead commits against the local upstream ref; no fetch was performed",
                    )),
                    _ => reasons.push(evidence(
                        "upstream_unknown",
                        "Upstream is absent, missing or unreadable; publication status is unknown",
                    )),
                }
            }
        }
        db.conn.execute(
            "UPDATE nodes SET protection=?1 WHERE id=?2",
            params![serde_json::to_string(&reasons)?, id],
        )?;
    }
    classify(db)
}
fn classify(db: &StorageIndex) -> Result<()> {
    let mut stmt=db.conn.prepare("SELECT id,parent,name FROM nodes WHERE kind='directory' AND name IN ('target','node_modules','Library')")?;
    for row in stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<i64>>(1)?,
            r.get::<_, String>(2)?,
        ))
    })? {
        let (id, Some(parent), name) = row? else {
            continue;
        };
        let marker = match name.as_str() {
            "target" => "Cargo.toml",
            "node_modules" => "package.json",
            "Library" => "ProjectSettings",
            _ => continue,
        };
        let present = db
            .conn
            .query_row(
                "SELECT 1 FROM nodes WHERE parent=?1 AND name=?2 LIMIT 1",
                params![parent, marker],
                |r| r.get::<_, u8>(0),
            )
            .optional()?
            .is_some();
        let classification = StorageClassification {
            kind: if present {
                format!("candidate_{name}")
            } else {
                "unclassified".into()
            },
            evidence: if present {
                format!(
                    "Sibling marker {marker} and directory name {name}; this is classification, not deletion permission"
                )
            } else {
                format!("Directory name {name} without verified project marker")
            },
        };
        db.conn.execute(
            "UPDATE nodes SET classification=?1 WHERE id=?2",
            params![serde_json::to_string(&vec![classification])?, id],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nul_parser_preserves_paths_and_optional_values() {
        let data=b"worktree C:/a b/line\nnext\0HEAD abc\0branch refs/heads/main\0locked\0\0worktree C:/second\0detached\0prunable missing path\0\0";
        let rows = parse_porcelain(data).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].path, b"C:/a b/line\nnext");
        assert_eq!(rows[0].locked, Some(vec![]));
        assert!(rows[1].detached);
        assert_eq!(rows[1].prunable.as_deref(), Some(&b"missing path"[..]));
    }
}
