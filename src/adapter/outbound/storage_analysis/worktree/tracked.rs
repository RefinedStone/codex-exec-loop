//! Cached index paths and bounded repository ignore controls; working file contents stay closed.
use super::super::index::StorageIndex;
use super::{evidence, git, git_metadata};
use crate::domain::storage_analysis::StorageEvidence;
use anyhow::{Context, Result, ensure};
use rusqlite::params;
use std::path::Path;

const MAX_OUTPUT: usize = 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;

#[derive(Debug, PartialEq, Eq)]
struct PathSummary {
    untracked: u64,
    conflicts: u64,
}

pub(super) fn observe(
    db: &StorageIndex,
    repository: i64,
    path: &Path,
    metadata: &git_metadata::GitMetadata,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Vec<StorageEvidence> {
    let result = git(
        path,
        metadata,
        &["ls-files", "--cached", "--stage", "-z", "--sparse"],
        cancelled,
    )
    .and_then(|bytes| compare(db, repository, path, &bytes));
    let verified = result.is_ok();
    let mut observations = reasons(result);
    if verified {
        match super::ignore::observe(db, repository, path, metadata, cancelled) {
            Ok(summary) => {
                observations.retain(|reason| reason.code != "untracked_observed");
                if summary.untracked + summary.ignored > 0 {
                observations.push(evidence(
                    "ignore_rules_observed",
                    format!("관측한 미추적 후보 중 저장소 .gitignore와 info/exclude 규칙으로 {}개를 제외했습니다. 사용자 전역 exclude는 읽지 않았으며 전체 Git 상태 판정이 아닙니다.", summary.ignored),
                ));
                }
                if summary.untracked > 0 {
                    observations.push(evidence(
                        "untracked_files",
                        format!("저장소 ignore 규칙을 적용한 뒤 인덱스 밖 관측 파일 {}개가 남았습니다. 전역 exclude와 스캔 이후 파일은 포함해 검증한 것이 아니므로 검토가 필요합니다.", summary.untracked),
                    ));
                }
            }
            Err(_) => observations.push(evidence(
                "ignore_unknown",
                "저장소 ignore 규칙을 검증하지 못했습니다. 범위 밖 exclude 설정, 읽기 실패·변경·링크·placeholder·조회 한도를 확인하세요. 인덱스 밖 경로 관측은 무시된 파일을 포함할 수 있습니다.",
            )),
        }
    }
    observations
}
fn reasons(result: Result<PathSummary>) -> Vec<StorageEvidence> {
    match result {
        Ok(summary) => {
            let mut reasons = Vec::new();
            if summary.untracked > 0 {
                reasons.push(evidence(
                    "untracked_observed",
                    format!(
                        "{} observed file paths are absent from the index; ignore rules were not read or applied",
                        summary.untracked
                    ),
                ));
            }
            if summary.conflicts > 0 {
                reasons.push(evidence(
                    "index_conflicts",
                    format!("{} index paths have unresolved stages", summary.conflicts),
                ));
            }
            reasons
        }
        Err(_) => vec![evidence(
            "index_paths_unknown",
            "Index path observation failed, exceeded a bound, or contained an unsupported path; no working tree status was inferred",
        )],
    }
}

fn compare(db: &StorageIndex, repository: i64, root: &Path, bytes: &[u8]) -> Result<PathSummary> {
    ensure!(bytes.len() <= MAX_OUTPUT, "storage_git_index_output_limit");
    ensure!(
        bytes.is_empty() || bytes.last() == Some(&0),
        "storage_git_index_truncated"
    );
    db.conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS git_tracked_paths(
             repository INTEGER NOT NULL,path_key TEXT NOT NULL,relative_path TEXT NOT NULL,
             stage INTEGER NOT NULL,mode INTEGER NOT NULL,excluded INTEGER NOT NULL,
             PRIMARY KEY(repository,path_key,stage));
         CREATE INDEX IF NOT EXISTS git_tracked_excluded ON git_tracked_paths(repository,excluded);",
    )?;
    let tx = db.conn.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM git_tracked_paths WHERE repository=?1",
        [repository],
    )?;
    let mut entries = 0;
    for record in bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        entries += 1;
        ensure!(entries <= MAX_ENTRIES, "storage_git_index_entry_limit");
        let tab = record
            .iter()
            .position(|byte| *byte == b'\t')
            .context("storage_git_index_record")?;
        let header = std::str::from_utf8(&record[..tab])?;
        let mut fields = header.split(' ');
        let mode = u32::from_str_radix(fields.next().context("storage_git_index_mode")?, 8)?;
        ensure!(
            matches!(mode, 0o100644 | 0o100755 | 0o120000 | 0o160000 | 0o040000),
            "storage_git_index_mode"
        );
        let object = fields.next().context("storage_git_index_object")?;
        ensure!(
            matches!(object.len(), 40 | 64) && object.bytes().all(|b| b.is_ascii_hexdigit()),
            "storage_git_index_object"
        );
        let stage: u8 = fields.next().context("storage_git_index_stage")?.parse()?;
        ensure!(
            stage <= 3 && fields.next().is_none(),
            "storage_git_index_stage"
        );
        let relative = std::str::from_utf8(&record[tab + 1..])?;
        let relative = if mode == 0o040000 {
            relative.strip_suffix('/').unwrap_or(relative)
        } else {
            relative
        };
        ensure!(
            !relative.is_empty()
                && !relative.contains(['\\', ':'])
                && relative.split('/').all(|part| !part.is_empty()
                    && !matches!(part, "." | "..")
                    && !part.eq_ignore_ascii_case(".git")),
            "storage_git_index_path"
        );
        let path_key = git_metadata::key(&root.join(relative));
        let ambiguous: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM git_tracked_paths WHERE repository=?1 AND path_key=?2 AND relative_path<>?3)",
            params![repository,path_key,relative],
            |row| row.get(0),
        )?;
        ensure!(!ambiguous, "storage_git_index_ambiguous_case");
        tx.execute(
            "INSERT INTO git_tracked_paths(repository,path_key,relative_path,stage,mode,excluded) VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                repository,
                path_key,
                relative,
                stage,
                mode,
                i64::from(matches!(mode, 0o160000 | 0o040000))
            ],
        )?;
    }
    tx.commit()?;
    let separator = std::path::MAIN_SEPARATOR.to_string();
    let untracked: u64 = db.conn.query_row(
        "WITH RECURSIVE tree(id,excluded) AS(
             SELECT ?1,0 UNION ALL
             SELECT n.id,tree.excluded OR lower(n.name)='.git'
             FROM nodes n JOIN tree ON n.parent=tree.id WHERE n.kind IN ('file','directory'))
         SELECT count(*) FROM tree JOIN nodes n ON n.id=tree.id
         JOIN git_observed_paths p ON p.node=n.id
         WHERE tree.excluded=0 AND n.kind='file' AND n.unknown=0
           AND NOT EXISTS(SELECT 1 FROM git_tracked_paths t WHERE t.repository=?1 AND t.path_key=p.path_key)
           AND NOT EXISTS(SELECT 1 FROM git_tracked_paths t WHERE t.repository=?1 AND t.excluded=1
                AND substr(p.path_key,1,length(t.path_key)+1)=t.path_key||?2)",
        params![repository,separator],
        |row| row.get(0),
    )?;
    let conflicts = db.conn.query_row(
        "SELECT count(DISTINCT path_key) FROM git_tracked_paths WHERE repository=?1 AND stage>0",
        [repository],
        |row| row.get(0),
    )?;
    Ok(PathSummary {
        untracked,
        conflicts,
    })
}

/// Join only registrations and paths already observed by this job. Deferring this pass until
/// all candidates finish makes the relationship independent of candidate discovery order.
pub(super) fn annotate_submodules(
    db: &StorageIndex,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<()> {
    use std::sync::atomic::Ordering;

    let indexed: bool = db.conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='git_tracked_paths')",
        [],
        |row| row.get(0),
    )?;
    if !indexed {
        return Ok(());
    }
    let mut registered = db.conn.prepare(
        "SELECT t.repository,n.id,n.path,n.protection FROM git_tracked_paths t
         JOIN git_registered_nodes parent ON parent.node=t.repository
         JOIN git_observed_paths observed ON observed.path_key=t.path_key
         JOIN nodes n ON n.id=observed.node AND n.kind='directory'
         JOIN git_registered_nodes child ON child.node=n.id
         WHERE t.stage=0 AND t.mode=?1 AND n.id<>t.repository",
    )?;
    for row in registered.query_map([0o160000u32], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })? {
        if cancelled.load(Ordering::Acquire) {
            break;
        }
        let (parent, child, path, raw) = row?;
        if !db.is_descendant(&format!("n{child}"), &format!("n{parent}"))?
            || !matches!(git_metadata::runtime_node(db, Path::new(&path)), Ok(Some(id)) if id == child)
        {
            continue;
        }
        let mut reasons: Vec<StorageEvidence> = serde_json::from_str(&raw)?;
        if reasons.iter().any(|reason| reason.code == "submodule") {
            continue;
        }
        reasons.push(evidence(
            "submodule",
            "A verified parent cached index registers this observed repository path as a stage-zero gitlink",
        ));
        db.conn.execute(
            "UPDATE nodes SET protection=?1 WHERE id=?2",
            params![serde_json::to_string(&reasons)?, child],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::outbound::storage_analysis::{index::Observation, native};
    use std::fs;
    use std::path::PathBuf;

    struct Fixture {
        base: PathBuf,
        root: PathBuf,
        db: StorageIndex,
    }
    impl Fixture {
        fn new() -> Self {
            let base = crate::test_utils::platform_safe_temp_dir().join(format!(
                "storage-index-paths-{:032x}",
                rand::random::<u128>()
            ));
            let root = base.join("source");
            let index = base.join("index");
            fs::create_dir_all(&root).unwrap();
            fs::create_dir(&index).unwrap();
            let db = StorageIndex::create(&index, "paths", &native::fixture_volume(&root).unwrap())
                .unwrap();
            Self { base, root, db }
        }
        fn node(&self, parent: i64, relative: &str, directory: bool) -> i64 {
            self.db
                .record(Observation {
                    parent,
                    name: relative.rsplit('/').next().unwrap().into(),
                    path: self.root.join(relative),
                    directory,
                    logical: if directory { 0 } else { 1 },
                    allocated: Some(1),
                    identity: Some(relative.into()),
                    issue: None,
                })
                .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let connection = std::mem::replace(
                &mut self.db.conn,
                rusqlite::Connection::open_in_memory().unwrap(),
            );
            drop(connection);
            let _ = fs::remove_dir_all(&self.base);
        }
    }
    fn entry(mode: &str, stage: u8, path: &str) -> Vec<u8> {
        format!("{mode} {} {stage}\t{path}\0", "a".repeat(40)).into_bytes()
    }
    #[test]
    fn storage_analysis_cached_paths_exclude_gitlinks_sparse_and_git_metadata() {
        let f = Fixture::new();
        f.node(1, "tracked.txt", false);
        f.node(1, "conflict.txt", false);
        f.node(1, "untracked.txt", false);
        let metadata = f.node(1, ".git", true);
        f.node(metadata, ".git/config", false);
        let linked = f.node(1, "sub", true);
        f.node(linked, "sub/private", false);
        let sparse = f.node(1, "sparse", true);
        f.node(sparse, "sparse/hidden", false);
        let similarly_named = f.node(1, "submarine", true);
        f.node(similarly_named, "submarine/file", false);
        git_metadata::prepare(&f.db).unwrap();
        let bytes = [
            entry("100644", 0, "tracked.txt"),
            entry("100644", 1, "conflict.txt"),
            entry("100644", 2, "conflict.txt"),
            entry("160000", 0, "sub"),
            entry("040000", 0, "sparse/"),
        ]
        .concat();
        assert_eq!(
            compare(&f.db, 1, &f.root, &bytes).unwrap(),
            PathSummary {
                untracked: 2,
                conflicts: 1
            }
        );
    }
    #[test]
    fn storage_analysis_cached_paths_fail_closed_on_output_limits_and_bad_paths() {
        let f = Fixture::new();
        git_metadata::prepare(&f.db).unwrap();
        let oversized = compare(&f.db, 1, &f.root, &vec![0; MAX_OUTPUT + 1]);
        assert_eq!(
            oversized.as_ref().unwrap_err().to_string(),
            "storage_git_index_output_limit"
        );
        assert_eq!(reasons(oversized)[0].code, "index_paths_unknown");
        assert!(compare(&f.db, 1, &f.root, &entry("100644", 0, "../outside")).is_err());
        assert!(compare(&f.db, 1, &f.root, &entry("100644", 0, ".git/config")).is_err());
        let mut truncated = entry("100644", 0, "file");
        truncated.pop();
        assert!(compare(&f.db, 1, &f.root, &truncated).is_err());
        let many = (0..=MAX_ENTRIES)
            .flat_map(|i| entry("100644", 0, &format!("file-{i}")))
            .collect::<Vec<_>>();
        assert_eq!(
            compare(&f.db, 1, &f.root, &many).unwrap_err().to_string(),
            "storage_git_index_entry_limit"
        );
    }
    #[cfg(windows)]
    #[test]
    fn storage_analysis_cached_path_case_is_matched_but_ambiguous_index_case_is_unknown() {
        let f = Fixture::new();
        f.node(1, "Mixed.txt", false);
        git_metadata::prepare(&f.db).unwrap();
        assert_eq!(
            compare(&f.db, 1, &f.root, &entry("100644", 0, "mixed.txt"))
                .unwrap()
                .untracked,
            0
        );
        let ambiguous = [
            entry("100644", 1, "mixed.txt"),
            entry("100644", 2, "Mixed.txt"),
        ]
        .concat();
        assert!(compare(&f.db, 1, &f.root, &ambiguous).is_err());
    }
}
