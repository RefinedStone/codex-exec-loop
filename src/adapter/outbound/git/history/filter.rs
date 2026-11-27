use super::*;
use crate::domain::git_graph::{
    GitGraphRow,
    filter::{BranchScope, GraphFilter},
};
use std::ops::Range;

fn tips(filter: &GraphFilter, refs: &GitHistoryRefs) -> Result<Vec<String>> {
    let mut tips = Vec::new();
    match &filter.branch {
        BranchScope::All => {
            tips.extend(["--branches", "--remotes", "--tags", BACKUP_GLOB].map(String::from));
            tips.extend(refs.head.clone());
        }
        BranchScope::Local => tips.push("--branches".into()),
        BranchScope::Thread => tips.extend(refs.head.clone()),
        BranchScope::Branch { name } => {
            let target = refs
                .refs
                .iter()
                .find(|r| match r.kind {
                    GitGraphRefKind::LocalBranch => {
                        name.strip_prefix("refs/heads/") == Some(r.name.as_str())
                    }
                    GitGraphRefKind::RemoteBranch => {
                        name.strip_prefix("refs/remotes/") == Some(r.name.as_str())
                    }
                    GitGraphRefKind::Tag | GitGraphRefKind::Backup => false,
                })
                .context("branch filter is not in this repository")?;
            tips.push(target.target.clone());
        }
    }
    tips.sort();
    tips.dedup();
    Ok(tips)
}

fn query(
    cwd: &str,
    filter: &GraphFilter,
    revisions: &[String],
    page: Range<usize>,
    format: &str,
) -> Result<String> {
    if revisions.is_empty() {
        return Ok(String::new());
    }
    let mut args = vec![
        "log".to_owned(),
        "-z".into(),
        format.into(),
        "--date-order".into(),
        "--no-show-signature".into(),
        "--no-color".into(),
        "--encoding=UTF-8".into(),
        "--fixed-strings".into(),
        "--regexp-ignore-case".into(),
        format!("--skip={}", page.start),
        format!("--max-count={}", page.len()),
    ];
    if !filter.author.is_empty() {
        args.push(format!("--author={}", filter.author));
    }
    if !filter.message.is_empty() {
        args.push(format!("--grep={}", filter.message));
    }
    args.extend(revisions.iter().cloned());
    args.push("--".into());
    if !filter.path.is_empty() {
        args.push(format!(":(top,literal){}", filter.path));
    }
    required(
        git_output(
            Path::new(cwd),
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
        )?,
        "git log (filter)",
    )
}

pub(super) fn page(
    cwd: &str,
    filter: &GraphFilter,
    refs: &GitHistoryRefs,
    bounds: Range<usize>,
) -> Result<Vec<GitGraphCommit>> {
    query(cwd, filter, &tips(filter, refs)?, bounds, LOG_FORMAT)?
        .split('\0')
        .filter(|r| !r.is_empty())
        .map(parse_commit_record)
        .collect()
}

pub(super) fn matches(
    cwd: &str,
    filter: &GraphFilter,
    refs: &GitHistoryRefs,
    rows: &[GitGraphRow],
) -> Result<HashSet<String>> {
    if rows.is_empty() {
        return Ok(HashSet::new());
    }
    let wanted: HashSet<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    // Match with exactly the same revision walk as hide mode. A path-limited log can simplify merge ancestry,
    // so introducing page-local exclusions here would disagree about merge commits. Bound each read.
    let revisions = tips(filter, refs)?;
    let mut found = HashSet::new();
    let mut skip: usize = 0;
    const CHUNK: usize = 4096;
    loop {
        let ids = query(
            cwd,
            filter,
            &revisions,
            skip..skip.saturating_add(CHUNK),
            "--format=%H",
        )?;
        let mut count = 0;
        for id in ids.split('\0').filter(|s| !s.is_empty()) {
            count += 1;
            if wanted.contains(id) {
                found.insert(id.to_owned());
            }
        }
        if count < CHUNK || found.len() == wanted.len() {
            break;
        }
        skip = skip.saturating_add(count);
    }
    Ok(found)
}
