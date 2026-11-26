//! Evaluate observed paths against copied, bounded repository ignore controls only.
use super::super::index::StorageIndex;
use super::{git_metadata, git_output};
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

const MAX_BYTES: usize = 1024 * 1024;
const MAX_PATHS: usize = 10_000;
const MAX_RULE_FILES: usize = 256;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Summary {
    pub untracked: usize,
    pub ignored: usize,
}

struct Snapshot {
    root: PathBuf,
}
impl Snapshot {
    fn new(ignore_case: bool) -> Result<Self> {
        let root = std::env::temp_dir().join(format!(
            "akra-storage-ignore-{:032x}",
            rand::random::<u128>()
        ));
        fs::create_dir(&root)?;
        let snapshot = Self { root };
        fs::create_dir_all(snapshot.root.join(".git/objects"))?;
        fs::create_dir(snapshot.root.join(".git/refs"))?;
        fs::create_dir(snapshot.root.join(".git/info"))?;
        fs::write(
            snapshot.root.join(".git/HEAD"),
            "ref: refs/heads/observation\n",
        )?;
        fs::write(
            snapshot.root.join(".git/config"),
            format!(
                "[core]\nrepositoryformatversion = 0\nbare = false\nignorecase = {ignore_case}\n"
            ),
        )?;
        Ok(snapshot)
    }
}
impl Drop for Snapshot {
    fn drop(&mut self) {
        // Only this invocation's random, exclusively-created directory is removed.
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(super) fn observe(
    db: &StorageIndex,
    repository: i64,
    path: &Path,
    metadata: &git_metadata::GitMetadata,
    cancelled: &AtomicBool,
) -> Result<Summary> {
    let paths = untracked_paths(db, repository, path)?;
    if paths.is_empty() {
        return Ok(Summary {
            untracked: 0,
            ignored: 0,
        });
    }
    ensure!(!cancelled.load(Ordering::Acquire), "storage_git_cancelled");
    // Global config is disabled by the common Git boundary. A repository-specific
    // external exclude must not silently disappear from an allegedly complete result.
    let external = git_output(
        path,
        metadata,
        &["config", "--null", "--get", "core.excludesFile"],
        cancelled,
    )?;
    match external.status.code() {
        Some(1) => {}
        Some(0) if external.stdout == [0] => {}
        _ => anyhow::bail!("storage_git_external_excludes_unknown"),
    }
    let case = git_output(
        path,
        metadata,
        &["config", "--type=bool", "--get", "core.ignoreCase"],
        cancelled,
    )?;
    let ignore_case = match (case.status.code(), case.stdout.as_slice()) {
        (Some(0), b"true\n") => true,
        (Some(0), b"false\n") | (Some(1), b"") => false,
        _ => anyhow::bail!("storage_git_ignore_case_unknown"),
    };
    let snapshot = Snapshot::new(ignore_case)?;
    copy_rules(db, path, metadata, &paths, &snapshot, cancelled)?;
    ensure!(!cancelled.load(Ordering::Acquire), "storage_git_cancelled");
    let mut input = Vec::new();
    for path in &paths {
        input.extend_from_slice(path.as_bytes());
        input.push(0);
    }
    ensure!(input.len() <= MAX_BYTES, "storage_git_ignore_input_limit");
    let mut command = crate::git_subprocess::command(["-C"]);
    command
        .arg(&snapshot.root)
        .args([
            "-c",
            if cfg!(windows) {
                "core.excludesFile=NUL"
            } else {
                "core.excludesFile=/dev/null"
            },
        ])
        .args([
            "check-ignore",
            "--no-index",
            "--stdin",
            "-z",
            "--verbose",
            "--non-matching",
        ]);
    let output = crate::subprocess::command_output_with_input_timeout_and_limits(
        &mut command,
        "storage Git ignore snapshot",
        &input,
        Duration::from_secs(3),
        MAX_BYTES,
        64 * 1024,
    )?;
    ensure!(
        matches!(output.status.code(), Some(0 | 1)),
        "storage_git_ignore_query_failed"
    );
    parse(&paths, &output.stdout)
}

fn untracked_paths(db: &StorageIndex, repository: i64, root: &Path) -> Result<Vec<String>> {
    let separator = std::path::MAIN_SEPARATOR.to_string();
    let mut query = db.conn.prepare(
        "WITH RECURSIVE tree(id,excluded) AS(
             SELECT ?1,0 UNION ALL
             SELECT n.id,tree.excluded OR lower(n.name)='.git'
             FROM nodes n JOIN tree ON n.parent=tree.id WHERE n.kind IN ('file','directory'))
         SELECT n.path FROM tree JOIN nodes n ON n.id=tree.id
         JOIN git_observed_paths p ON p.node=n.id
         WHERE tree.excluded=0 AND n.kind='file' AND n.unknown=0
           AND NOT EXISTS(SELECT 1 FROM git_tracked_paths t WHERE t.repository=?1 AND t.path_key=p.path_key)
           AND NOT EXISTS(SELECT 1 FROM git_tracked_paths t WHERE t.repository=?1 AND t.excluded=1
                AND substr(p.path_key,1,length(t.path_key)+1)=t.path_key||?2)
         ORDER BY n.id LIMIT 10001"
    )?;
    let mut paths = Vec::new();
    let mut bytes = 0;
    for row in query.query_map(rusqlite::params![repository, separator], |row| {
        row.get::<_, String>(0)
    })? {
        ensure!(paths.len() < MAX_PATHS, "storage_git_ignore_path_limit");
        let observed = PathBuf::from(row?);
        let relative = observed
            .strip_prefix(root)?
            .to_str()
            .context("storage_git_ignore_path")?;
        #[cfg(windows)]
        let relative = relative.replace('\\', "/");
        #[cfg(not(windows))]
        let relative = relative.to_owned();
        git_metadata::validate_relative(&relative)?;
        bytes += relative.len() + 1;
        ensure!(bytes <= MAX_BYTES, "storage_git_ignore_input_limit");
        paths.push(relative);
    }
    Ok(paths)
}

fn copy_rules(
    db: &StorageIndex,
    root: &Path,
    metadata: &git_metadata::GitMetadata,
    paths: &[String],
    snapshot: &Snapshot,
    cancelled: &AtomicBool,
) -> Result<()> {
    let volume = fs::canonicalize(db.path("n1")?)?;
    let mut directories = BTreeSet::from([String::new()]);
    for path in paths {
        let mut parent = Path::new(path).parent();
        while let Some(directory) = parent {
            ensure!(
                directories.len() <= MAX_PATHS,
                "storage_git_ignore_directory_limit"
            );
            directories.insert(
                directory
                    .to_str()
                    .context("storage_git_ignore_path")?
                    .to_owned(),
            );
            parent = directory.parent();
        }
    }
    ensure!(
        directories.len() <= MAX_PATHS,
        "storage_git_ignore_directory_limit"
    );
    let started = Instant::now();
    let mut files = Vec::new();
    let mut bytes = 0;
    for relative in directories {
        ensure!(!cancelled.load(Ordering::Acquire), "storage_git_cancelled");
        ensure!(
            started.elapsed() < Duration::from_secs(3),
            "storage_git_ignore_time_limit"
        );
        let source = root.join(&relative).join(".gitignore");
        let destination = snapshot.root.join(&relative);
        fs::create_dir_all(&destination)?;
        if let Some(contents) =
            git_metadata::optional_file(db, &source, &volume, 64 * 1024, &mut files)?
        {
            bytes += contents.len();
            ensure!(
                bytes <= MAX_BYTES && files.len() <= MAX_RULE_FILES,
                "storage_git_ignore_rules_limit"
            );
            fs::write(destination.join(".gitignore"), contents)?;
        }
    }
    if let Some(contents) = git_metadata::optional_file(
        db,
        &metadata.common_dir.join("info/exclude"),
        &volume,
        64 * 1024,
        &mut files,
    )? {
        bytes += contents.len();
        ensure!(
            bytes <= MAX_BYTES && files.len() <= MAX_RULE_FILES,
            "storage_git_ignore_rules_limit"
        );
        fs::write(snapshot.root.join(".git/info/exclude"), contents)?;
    }
    Ok(())
}

fn parse(paths: &[String], bytes: &[u8]) -> Result<Summary> {
    ensure!(
        bytes.len() <= MAX_BYTES && bytes.last() == Some(&0),
        "storage_git_ignore_output_truncated"
    );
    let fields = bytes[..bytes.len() - 1]
        .split(|byte| *byte == 0)
        .collect::<Vec<_>>();
    ensure!(
        fields.len() == paths.len() * 4,
        "storage_git_ignore_output_incomplete"
    );
    let mut ignored = 0;
    for (path, record) in paths.iter().zip(fields.chunks_exact(4)) {
        ensure!(
            record[3] == path.as_bytes(),
            "storage_git_ignore_output_path"
        );
        if record[0].is_empty() {
            ensure!(
                record[1].is_empty() && record[2].is_empty(),
                "storage_git_ignore_output_record"
            );
        } else {
            ensure!(
                !record[2].is_empty() && std::str::from_utf8(record[1])?.parse::<u64>()? > 0,
                "storage_git_ignore_output_record"
            );
            ignored += usize::from(!record[2].starts_with(b"!"));
        }
    }
    Ok(Summary {
        untracked: paths.len() - ignored,
        ignored,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_analysis_ignore_parser_handles_negation_and_requires_every_path() {
        let paths = vec!["a b\n.txt".into(), "keep.txt".into(), "plain.txt".into()];
        let bytes = b".gitignore\x001\x00*.txt\x00a b\n.txt\x00.gitignore\x002\x00!keep.txt\x00keep.txt\x00\x00\x00\x00plain.txt\x00";
        assert_eq!(
            parse(&paths, bytes).unwrap(),
            Summary {
                untracked: 2,
                ignored: 1
            }
        );
        assert!(parse(&paths, &bytes[..bytes.len() - 1]).is_err());
        assert!(parse(&paths[..2], bytes).is_err());
        assert!(parse(&["other".into()], b"\x00\x00\x00plain.txt\x00").is_err());
    }
}
