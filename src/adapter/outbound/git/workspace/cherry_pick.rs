use super::*;
use crate::domain::git_graph::is_git_commit_id;
use crate::domain::git_operation::cherry_pick::{PickPrediction, PickPreview};
use std::collections::HashSet;

pub(super) fn preview(
    repository: &Path,
    head: &str,
    commits: &[String],
) -> Result<Vec<PickPreview>> {
    let mut range = vec!["rev-list", "--topo-order", "--reverse"];
    range.extend(commits.iter().map(String::as_str));
    range.push("--");
    let selected: HashSet<_> = commits.iter().map(String::as_str).collect();
    let order = read(repository, &range)?;
    let dirs = GitDirs::read(repository)?;
    let mut tree = Some(
        read(repository, &["rev-parse", &format!("{head}^{{tree}}")])?
            .trim()
            .to_owned(),
    );
    let mut previews = Vec::new();
    for commit in order.lines().filter(|id| selected.contains(id)) {
        let parents = read(
            repository,
            &[
                "show",
                "-s",
                "--no-show-signature",
                "--format=%P",
                commit,
                "--",
            ],
        )?;
        let parents: Vec<_> = parents.split_whitespace().collect();
        if parents.len() > 1 {
            bail!("merge commit {commit} cannot be cherry-picked");
        }
        if let Some(reason) = filter_attribute_refusal(repository, &dirs, Some(commit))? {
            bail!(reason);
        }
        let mut args = vec!["cherry", head, commit];
        args.extend(parents.iter().copied());
        let cherries = read(repository, &args)?;
        let equivalent = !cherries.lines().any(|line| line == format!("+ {commit}"));
        let prediction = if equivalent {
            PickPrediction::AlreadyApplied
        } else if let Some(current) = tree.as_deref() {
            let base = match parents.first() {
                Some(parent) => (*parent).to_owned(),
                None => read(repository, &["mktree"])?.trim().to_owned(),
            };
            let merged = merge(repository, &base, (current, commit))?;
            match merged {
                Ok(next) if next == current => PickPrediction::AlreadyApplied,
                Ok(next) => {
                    tree = Some(next);
                    PickPrediction::Clean
                }
                Err(paths) => {
                    tree = None;
                    PickPrediction::Conflict { paths }
                }
            }
        } else {
            PickPrediction::AfterConflict
        };
        previews.push(PickPreview {
            commit: commit.to_owned(),
            prediction,
        });
    }
    if previews.len() != selected.len() {
        bail!("one or more selected objects are not commits");
    }
    Ok(previews)
}

fn merge(
    repo: &Path,
    base: &str,
    (ours, theirs): (&str, &str),
) -> Result<Result<String, Vec<String>>> {
    let mut command = git_subprocess::command([
        "merge-tree",
        "--write-tree",
        "--name-only",
        "--no-messages",
        "-z",
        &format!("--merge-base={base}"),
        ours,
        theirs,
    ]);
    command
        .current_dir(repo)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = subprocess::command_output(&mut command, "cherry-pick preview")?;
    let value = String::from_utf8_lossy(&output.stdout);
    let mut fields = value.split('\0');
    let tree = fields.next().unwrap_or("").trim();
    match output.status.code() {
        Some(0) if is_git_commit_id(tree) => Ok(Ok(tree.to_owned())),
        Some(1) if is_git_commit_id(tree) => Ok(Err(fields
            .take_while(|p| !p.is_empty())
            .map(str::to_owned)
            .collect())),
        _ => bail!(
            "cherry-pick preview failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ),
    }
}

pub(super) fn apply(repo: &Path, args: &[&str], receipt: &str) -> Result<()> {
    if let Some(reason) = configuration_refusal(repo) {
        bail!(reason);
    }
    let overrides = CommandOverrides::read(repo)?;
    let mut argv = vec![
        "-c",
        "commit.gpgSign=false",
        "-c",
        "rerere.enabled=false",
        "-c",
        "gc.auto=0",
    ];
    argv.extend(args);
    let mut command = git_subprocess::command(overrides.with(&argv));
    command
        .env("GIT_REFLOG_ACTION", receipt)
        .env("GIT_EDITOR", "true");
    output_of(repo, command)?
        .map(|_| ())
        .map_err(anyhow::Error::msg)
}
