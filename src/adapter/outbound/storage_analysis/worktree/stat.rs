//! Compare cached index stat fields with attributes, without opening working file contents.
use super::super::index::StorageIndex;
use super::{evidence, git, git_metadata};
use crate::domain::storage_analysis::StorageEvidence;
use anyhow::{Context, Result, ensure};
use std::{
    fs::Metadata,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MAX_ENTRIES: usize = 10_000;

#[derive(Debug)]
struct IndexStat {
    path: String,
    mode: u32,
    stage: u8,
    ctime: (u32, u32),
    mtime: (u32, u32),
    size: u64,
    flags: u32,
}

#[derive(Default, Debug, PartialEq, Eq)]
struct Summary {
    checked: usize,
    missing: usize,
    size: usize,
    mode: usize,
    time: usize,
    changed: usize,
    unknown: usize,
}

pub(super) fn observe(
    db: &StorageIndex,
    path: &Path,
    metadata: &git_metadata::GitMetadata,
    cancelled: &AtomicBool,
) -> Vec<StorageEvidence> {
    let result = git(
        path,
        metadata,
        &[
            "ls-files", "--cached", "--stage", "--debug", "-z", "--sparse",
        ],
        cancelled,
    )
    .and_then(|bytes| parse(&bytes))
    .map(|entries| inspect(db, path, &entries, cancelled));
    match result {
        Ok(summary) => reasons(summary),
        Err(_) => vec![evidence(
            "worktree_stat_unknown",
            "Git 인덱스의 stat 필드를 읽지 못했거나 조회 한도·지원 형식에 맞지 않습니다. 작업 파일 내용의 변경 여부는 비교하지 않았습니다.",
        )],
    }
}

fn inspect(
    db: &StorageIndex,
    root: &Path,
    entries: &[IndexStat],
    cancelled: &AtomicBool,
) -> Summary {
    let started = Instant::now();
    let mut summary = Summary::default();
    for (position, entry) in entries.iter().enumerate() {
        if cancelled.load(Ordering::Acquire) || started.elapsed() >= Duration::from_secs(3) {
            summary.unknown += entries.len() - position;
            break;
        }
        // Conflicts are reported independently. Gitlinks, sparse directories, symlinks,
        // assume-unchanged, skip-worktree and intent-to-add have no comparable regular stat.
        if entry.stage != 0 {
            continue;
        }
        if !matches!(entry.mode, 0o100644 | 0o100755)
            || entry.flags & ((1 << 15) | (1 << 29) | (1 << 30)) != 0
        {
            summary.unknown += 1;
            continue;
        }
        match git_metadata::worktree_entry(db, root, &entry.path) {
            Ok(None) => {
                summary.checked += 1;
                summary.missing += 1;
                summary.changed += 1;
            }
            Ok(Some(current)) => compare(entry, &current, &mut summary),
            Err(_) => summary.unknown += 1,
        }
    }
    summary
}

fn compare(entry: &IndexStat, current: &Metadata, summary: &mut Summary) {
    summary.checked += 1;
    // A missing timestamp must not erase independently observed size/type changes.
    // Git's index size is a 32-bit field, so a larger current size remains unknown.
    let mut unknown = current.len() > u64::from(u32::MAX);
    let size = !unknown && current.len() != entry.size;
    let mode = !current.is_file() || executable_changed(entry.mode, current);
    let time = match (current.modified().and_then(timestamp), change_time(current)) {
        (Ok(mtime), Ok(ctime)) => entry.mtime != mtime || entry.ctime != ctime,
        _ => {
            unknown = true;
            false
        }
    };
    summary.size += usize::from(size);
    summary.mode += usize::from(mode);
    summary.time += usize::from(time);
    summary.changed += usize::from(size || mode || time);
    summary.unknown += usize::from(unknown);
}

fn timestamp(time: SystemTime) -> std::io::Result<(u32, u32)> {
    let value = time
        .duration_since(UNIX_EPOCH)
        .map_err(std::io::Error::other)?;
    let seconds = u32::try_from(value.as_secs()).map_err(std::io::Error::other)?;
    Ok((seconds, value.subsec_nanos()))
}

#[cfg(windows)]
fn change_time(metadata: &Metadata) -> std::io::Result<(u32, u32)> {
    // Git for Windows stores the creation time in its emulated stat ctime.
    metadata.created().and_then(timestamp)
}

#[cfg(unix)]
fn change_time(metadata: &Metadata) -> std::io::Result<(u32, u32)> {
    use std::os::unix::fs::MetadataExt;
    Ok((
        u32::try_from(metadata.ctime()).map_err(std::io::Error::other)?,
        u32::try_from(metadata.ctime_nsec()).map_err(std::io::Error::other)?,
    ))
}

#[cfg(not(any(windows, unix)))]
fn change_time(_: &Metadata) -> std::io::Result<(u32, u32)> {
    Err(std::io::Error::other("unsupported file timestamps"))
}

#[cfg(unix)]
fn executable_changed(mode: u32, metadata: &Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    (mode & 0o100 != 0) != (metadata.permissions().mode() & 0o100 != 0)
}

#[cfg(not(unix))]
fn executable_changed(_: u32, _: &Metadata) -> bool {
    false
}

fn reasons(summary: Summary) -> Vec<StorageEvidence> {
    let mut reasons = vec![evidence(
        "worktree_stat_observed",
        format!(
            "추적 파일 {}개의 메타데이터를 인덱스와 대조했습니다. 파일 내용을 비교하지 않았으며 같은 stat도 내용 동일성이나 삭제 가능 여부를 증명하지 않습니다.",
            summary.checked
        ),
    )];
    if summary.changed > 0 {
        reasons.push(evidence(
            "worktree_stat_changes",
            format!(
                "추적 경로 {}개에서 검토할 차이가 있습니다: 없음 {}, 크기 {}, 파일 유형·실행 권한 {}, 수정·생성 시각 {}. 한 경로에 여러 차이가 있을 수 있으며 stat 차이는 본문 변경 확정이 아닙니다.",
                summary.changed, summary.missing, summary.size, summary.mode, summary.time
            ),
        ));
    }
    if summary.unknown > 0 {
        reasons.push(evidence(
            "worktree_stat_unknown",
            format!(
                "추적 경로 {}개는 외부·변경된 부모, 링크·placeholder, 접근 실패, 특수 인덱스 항목 또는 조회 한도로 메타데이터를 대조하지 못했습니다. 확인한 보호 사유는 유지됩니다.",
                summary.unknown
            ),
        ));
    }
    reasons
}

fn line<'a>(bytes: &mut &'a [u8]) -> Result<&'a str> {
    let end = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .context("storage_git_stat_line")?;
    let value = std::str::from_utf8(&bytes[..end])?;
    *bytes = &bytes[end + 1..];
    Ok(value)
}

fn pair(line: &str, prefix: &str, separator: &str) -> Result<(u32, u32)> {
    let (left, right) = line
        .strip_prefix(prefix)
        .context("storage_git_stat_field")?
        .split_once(separator)
        .context("storage_git_stat_pair")?;
    Ok((left.parse()?, right.parse()?))
}

fn parse(mut bytes: &[u8]) -> Result<Vec<IndexStat>> {
    let mut entries = Vec::new();
    while !bytes.is_empty() {
        ensure!(entries.len() < MAX_ENTRIES, "storage_git_stat_entry_limit");
        let end = bytes
            .iter()
            .position(|byte| *byte == 0)
            .context("storage_git_stat_path")?;
        let record = std::str::from_utf8(&bytes[..end])?;
        bytes = &bytes[end + 1..];
        let (header, path) = record.split_once('\t').context("storage_git_stat_header")?;
        let mut fields = header.split(' ');
        let mode = u32::from_str_radix(fields.next().context("storage_git_stat_mode")?, 8)?;
        let object = fields.next().context("storage_git_stat_object")?;
        ensure!(
            matches!(object.len(), 40 | 64) && object.bytes().all(|b| b.is_ascii_hexdigit()),
            "storage_git_stat_object"
        );
        let stage: u8 = fields.next().context("storage_git_stat_stage")?.parse()?;
        ensure!(
            stage <= 3 && fields.next().is_none(),
            "storage_git_stat_stage"
        );
        ensure!(
            matches!(mode, 0o100644 | 0o100755 | 0o120000 | 0o160000 | 0o040000),
            "storage_git_stat_mode"
        );
        let path = if mode == 0o040000 {
            path.strip_suffix('/').unwrap_or(path)
        } else {
            path
        };
        git_metadata::validate_relative(path)?;
        let ctime = pair(line(&mut bytes)?, "  ctime: ", ":")?;
        let mtime = pair(line(&mut bytes)?, "  mtime: ", ":")?;
        ensure!(
            ctime.1 < 1_000_000_000 && mtime.1 < 1_000_000_000,
            "storage_git_stat_nanoseconds"
        );
        pair(line(&mut bytes)?, "  dev: ", "\tino: ")?;
        pair(line(&mut bytes)?, "  uid: ", "\tgid: ")?;
        let final_line = line(&mut bytes)?;
        let (size, flags) = final_line
            .strip_prefix("  size: ")
            .context("storage_git_stat_size")?
            .split_once("\tflags: ")
            .context("storage_git_stat_flags")?;
        entries.push(IndexStat {
            path: path.into(),
            mode,
            stage,
            ctime,
            mtime,
            size: size.parse()?,
            flags: u32::from_str_radix(flags, 16)?,
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(path: &str) -> Vec<u8> {
        format!("100644 {} 0\t{path}\0  ctime: 10:20\n  mtime: 30:40\n  dev: 0\tino: 0\n  uid: 0\tgid: 0\n  size: 5\tflags: 0\n", "a".repeat(40)).into_bytes()
    }

    #[test]
    fn storage_analysis_stat_parser_preserves_nul_delimited_paths_and_rejects_partial_records() {
        let bytes = record("space tab\tline\nname");
        let entries = parse(&bytes).unwrap();
        assert_eq!(entries[0].path, "space tab\tline\nname");
        assert_eq!(entries[0].mtime, (30, 40));
        assert!(parse(&bytes[..bytes.len() - 1]).is_err());
        assert!(parse(&record("../outside")).is_err());
        assert!(parse(&record(".git/config")).is_err());
        assert!(parse(&record("path").repeat(MAX_ENTRIES + 1)).is_err());
    }

    #[test]
    fn storage_analysis_stat_uncertainty_never_erases_observed_changes() {
        let reasons = reasons(Summary {
            checked: 3,
            missing: 1,
            changed: 2,
            size: 1,
            unknown: 4,
            ..Summary::default()
        });
        assert!(
            reasons
                .iter()
                .any(|reason| reason.code == "worktree_stat_changes")
        );
        assert!(
            reasons
                .iter()
                .any(|reason| reason.code == "worktree_stat_unknown")
        );
        assert!(!reasons.iter().any(|reason| reason.code == "clean"));
    }
}
