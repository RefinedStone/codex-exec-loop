mod git_metadata;
mod tracked;

use super::index::StorageIndex;
use crate::domain::storage_analysis::*;
use anyhow::{Result, bail, ensure};
use rusqlite::{OptionalExtension, params};
use std::{
    ffi::OsStr,
    path::Path,
    process::Output,
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
                ensure!(output.len() < 4096, "storage_git_worktree_limit");
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
        ensure!(output.len() < 4096, "storage_git_worktree_limit");
        output.push(record);
    }
    Ok(output)
}
fn git_output(
    path: &Path,
    metadata: &git_metadata::GitMetadata,
    args: &[&str],
    cancelled: &AtomicBool,
) -> Result<Output> {
    ensure!(!cancelled.load(Ordering::Acquire), "storage_git_cancelled");
    let workspace = git_metadata::git_argument(path);
    let mut command = crate::git_subprocess::command([OsStr::new("-C"), workspace.as_os_str()]);
    command
        .arg("--git-dir")
        .arg(git_metadata::git_argument(&metadata.git_dir));
    if let Some(work_tree) = &metadata.work_tree {
        command
            .arg("--work-tree")
            .arg(git_metadata::git_argument(work_tree));
    }
    command
        .args(["--no-optional-locks", "-c", "core.fsmonitor=false"])
        .args(args);
    ensure!(!cancelled.load(Ordering::Acquire), "storage_git_cancelled");
    let output = crate::subprocess::command_output_with_input_timeout_and_limits(
        &mut command,
        "storage Git metadata",
        b"",
        Duration::from_secs(3),
        1024 * 1024,
        64 * 1024,
    )?;
    Ok(output)
}
fn git(
    path: &Path,
    metadata: &git_metadata::GitMetadata,
    args: &[&str],
    cancelled: &AtomicBool,
) -> Result<Vec<u8>> {
    let output = git_output(path, metadata, args, cancelled)?;
    ensure!(output.status.success(), "Git metadata query failed");
    Ok(output.stdout)
}
fn cached_index_changes(
    path: &Path,
    metadata: &git_metadata::GitMetadata,
    cancelled: &AtomicBool,
) -> Result<bool> {
    let output = git_output(
        path,
        metadata,
        &[
            "diff-index",
            "--cached",
            "--quiet",
            "--no-ext-diff",
            "--no-textconv",
            "--ignore-submodules=all",
            "--no-renames",
            "HEAD",
            "--",
        ],
        cancelled,
    )?;
    match output.status.code() {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => bail!("Git index state could not be verified"),
    }
}
fn cached_worktrees(
    db: &StorageIndex,
    path: &Path,
    metadata: &git_metadata::GitMetadata,
    cancelled: &AtomicBool,
) -> Result<Vec<WorktreeRecord>> {
    let common_key = git_metadata::key(&metadata.common_dir);
    let cached: Option<Option<Vec<u8>>> = db
        .conn
        .query_row(
            "SELECT records FROM git_worktree_cache WHERE common_key=?1",
            [&common_key],
            |row| row.get(0),
        )
        .optional()?;
    let bytes = match cached {
        Some(Some(bytes)) => bytes,
        Some(None) => bail!("Git worktree registration unavailable"),
        None => {
            let result = git(
                path,
                metadata,
                &["worktree", "list", "--porcelain", "-z"],
                cancelled,
            );
            db.conn.execute(
                "INSERT INTO git_worktree_cache(common_key,records) VALUES(?1,?2)",
                params![common_key, result.as_ref().ok().map(Vec::as_slice)],
            )?;
            result?
        }
    };
    parse_porcelain(&bytes)
}
/// Git lists a separate main Git directory (including absorbed submodules) as its
/// first worktree path. Its pinned local core.worktree must also match the observed
/// directory before that record can authorize a checkout classification.
fn registered_worktree<'a>(
    db: &StorageIndex,
    id: i64,
    path: &Path,
    metadata: &git_metadata::GitMetadata,
    records: &'a [WorktreeRecord],
    cancelled: &AtomicBool,
) -> Option<&'a WorktreeRecord> {
    if let Some(record) = records.iter().find(|record| {
        git_metadata::key(Path::new(
            &String::from_utf8_lossy(&record.path).into_owned(),
        )) == git_metadata::key(path)
    }) {
        return Some(record);
    }
    let main = records.first()?;
    if main.bare
        || metadata.work_tree.is_none()
        || git_metadata::key(&metadata.git_dir) != git_metadata::key(&metadata.common_dir)
        || git_metadata::key(Path::new(&String::from_utf8_lossy(&main.path).into_owned()))
            != git_metadata::key(&metadata.common_dir)
    {
        return None;
    }
    let configured = git(
        path,
        metadata,
        &["config", "--local", "--null", "--get", "core.worktree"],
        cancelled,
    )
    .ok()?;
    let configured = std::str::from_utf8(configured.strip_suffix(&[0])?).ok()?;
    if configured.is_empty() || configured.len() > 4096 || configured.contains('\0') {
        return None;
    }
    let configured = Path::new(configured);
    let resolved = if configured.is_absolute() {
        configured.to_path_buf()
    } else {
        metadata.git_dir.join(configured)
    };
    (git_metadata::key(&resolved) == git_metadata::key(path)
        && matches!(git_metadata::runtime_node(db, path), Ok(Some(observed)) if observed == id))
    .then_some(main)
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes)
        .trim_end_matches(['\r', '\n'])
        .to_string()
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
    runtime_probe: Option<&super::StorageRuntimeProbe>,
) -> Result<()> {
    let Some(runtime_complete) = prepare_candidates(db, runtime_probe)? else {
        return classify(db);
    };
    let mut candidates = db
        .conn
        .prepare("SELECT node,git_discovered,runtime_known FROM git_candidates ORDER BY node")?;
    for row in candidates.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, bool>(1)?,
            r.get::<_, bool>(2)?,
        ))
    })? {
        if cancelled.load(Ordering::Acquire) {
            break;
        }
        let (id, git_discovered, runtime_known) = row?;
        let path = db.path(&format!("n{id}"))?;
        let mut reasons = runtime_probe
            .map(|probe| (probe.observe)(&path))
            .unwrap_or_default();
        if reasons.is_empty() {
            reasons.push(evidence(
                "runtime_unknown",
                "Runtime ownership has not been verified; no session or kernel was started",
            ));
        }
        if runtime_known {
            reasons.push(evidence("akra_known_repository", "An existing kernel root matched an observed directory identity; no kernel was started"));
        }
        reasons.push(evidence(
            "status_unknown",
            "Working tree contents and ignore rules are not read; observed index/path evidence is not a complete or clean status",
        ));
        let metadata = if git_discovered {
            git_metadata::observe(db, &path)
        } else {
            Err(anyhow::anyhow!("storage_git_metadata_unobserved"))
        };
        match metadata {
            Err(_) => reasons.push(evidence(
                "git_metadata_unverified",
                "Git metadata is unobserved, partial, changed, external, or contains a placeholder/link; Git commands were skipped",
            )),
            Ok(metadata) => {
                let records = cached_worktrees(db, &path, &metadata, cancelled);
                match &records {
                    Ok(records) => {
                        let current =
                            registered_worktree(db, id, &path, &metadata, records, cancelled);
                        if let Some(current) = current {
                            if !current.bare {
                                db.conn.execute(
                                    "INSERT OR IGNORE INTO git_registered_nodes(node) VALUES(?1)",
                                    [id],
                                )?;
                            }
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
                if cancelled.load(Ordering::Acquire) { break; }
                match cached_index_changes(&path, &metadata, cancelled) {
                    Ok(true) => reasons.push(evidence(
                        "index_changes",
                        "Index entries differ from local HEAD; working tree contents were not read",
                    )),
                    Ok(false) => reasons.push(evidence(
                        "index_unchanged",
                        "Index entries match local HEAD; this is not a clean working tree determination",
                    )),
                    Err(_) => reasons.push(evidence(
                        "index_unknown",
                        "Index versus local HEAD could not be verified",
                    )),
                }
                if cancelled.load(Ordering::Acquire) { break; }
                reasons.extend(tracked::observe(db, id, &path, &metadata, cancelled));
                if cancelled.load(Ordering::Acquire) { break; }
                match git(&path, &metadata, &["rev-list", "--count", "@{upstream}..HEAD"], cancelled).map(text) {
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
    tracked::annotate_submodules(db, cancelled)?;
    if !runtime_complete {
        let raw: String =
            db.conn
                .query_row("SELECT protection FROM nodes WHERE id=1", [], |row| {
                    row.get(0)
                })?;
        let mut reasons: Vec<StorageEvidence> = serde_json::from_str(&raw)?;
        reasons.push(evidence("runtime_candidates_partial", "The bounded existing-kernel root inventory was incomplete; absence does not mean inactive"));
        db.conn.execute(
            "UPDATE nodes SET protection=?1 WHERE id=1",
            [serde_json::to_string(&reasons)?],
        )?;
    }
    classify(db)
}
fn prepare_candidates(
    db: &StorageIndex,
    probe: Option<&super::StorageRuntimeProbe>,
) -> Result<Option<bool>> {
    const LIMIT: usize = 256;
    let mut known =
        probe
            .map(|probe| (probe.known_roots)(LIMIT))
            .unwrap_or(super::StorageRuntimeRoots {
                paths: Vec::new(),
                complete: true,
            });
    if known.paths.len() > LIMIT {
        known.paths.truncate(LIMIT);
        known.complete = false;
    }
    let discovered: bool=db.conn.query_row("SELECT EXISTS(SELECT parent FROM nodes WHERE name='.git' UNION SELECT parent FROM nodes WHERE name IN ('HEAD','objects','refs') GROUP BY parent HAVING count(DISTINCT name)=3)",[],|row|row.get(0))?;
    if !discovered && known.paths.is_empty() && known.complete {
        return Ok(None);
    }
    git_metadata::prepare(db)?;
    db.conn.execute_batch("CREATE TABLE git_candidates(node INTEGER PRIMARY KEY,git_discovered INTEGER NOT NULL,runtime_known INTEGER NOT NULL);
        CREATE TABLE git_registered_nodes(node INTEGER PRIMARY KEY);
        INSERT INTO git_candidates SELECT id,1,0 FROM nodes WHERE kind='directory' AND id IN(
          SELECT parent FROM nodes WHERE name='.git' UNION
          SELECT parent FROM nodes WHERE name IN ('HEAD','objects','refs') GROUP BY parent HAVING count(DISTINCT name)=3);")?;
    for path in known.paths {
        match git_metadata::runtime_node(db, &path) {
            Ok(Some(id)) => {
                db.conn.execute("INSERT INTO git_candidates VALUES(?1,0,1) ON CONFLICT(node) DO UPDATE SET runtime_known=1",[id])?;
            }
            Ok(None) => {} // Outside this observed volume is never opened or exposed.
            Err(_) => known.complete = false,
        }
    }
    Ok(Some(known.complete))
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
                format!("candidate_{}", name.to_ascii_lowercase())
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
    #[test]
    fn worktree_record_count_is_bounded_even_for_tiny_records() {
        let bytes = b"worktree a\0\0".repeat(4097);
        assert!(parse_porcelain(&bytes).is_err());
    }
}
