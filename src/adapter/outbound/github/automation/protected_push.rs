//! The expected OID comes only from an explicit fetch of the frozen push destination.
use super::*;
use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::adapter::outbound::git::protection::PolicyStore;
use crate::application::port::outbound::parallel_mode_runtime_port::ParallelModeRuntimePort;
use crate::domain::git_protection::{ProtectedBranchPolicy, validate_rule};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(crate) struct ProtectedDestination {
    pub remote: String,
    pub url: String,
    pub automatic: Vec<String>,
    pub policy: ProtectedBranchPolicy,
}
#[derive(Debug, Serialize)]
pub(crate) struct LeasePreview {
    pub destination: ProtectedDestination,
    pub branch: String,
    pub local_branch: String,
    pub local_sha: String,
    pub remote_sha: String,
    pub lost: String,
}

fn automatic(cwd: &str, url: &str) -> Result<Vec<String>> {
    let listing = run_git_network_command_stdout(
        cwd,
        url,
        &["ls-remote", "--symref", FROZEN_GITHUB_REMOTE_NAME, "HEAD"],
    )?;
    let default = listing
        .lines()
        .find_map(|l| l.strip_prefix("ref: refs/heads/")?.strip_suffix("\tHEAD"))
        .context("remote default branch cannot be verified")?;
    let integration = GitParallelModeRuntimeAdapter::new()
        .integration_branch_for_workspace(cwd)
        .map_err(anyhow::Error::msg)?;
    let mut names = vec![default.to_owned(), integration];
    names.sort();
    names.dedup();
    Ok(names)
}
fn checked_remote(cwd: &str, remote: &str) -> Result<String> {
    if remote.is_empty()
        || remote.starts_with('-')
        || !remote
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        bail!("invalid remote name");
    }
    let all = run_git_stdout(cwd, &["remote", "get-url", "--push", "--all", remote])?;
    if all.lines().count() != 1 {
        bail!("push destination is ambiguous");
    }
    GithubAutomationAdapter::new().credential_redacted_push_url_for_remote(cwd, remote)
}
pub(crate) fn destination(cwd: &str, remote: &str) -> Result<ProtectedDestination> {
    let url = checked_remote(cwd, remote)?;
    let automatic = automatic(cwd, &url)?;
    let policy = PolicyStore::open(cwd)?.policy(&url)?;
    Ok(ProtectedDestination {
        remote: remote.to_owned(),
        url,
        automatic,
        policy,
    })
}
pub(crate) fn edit(
    cwd: &str,
    seen: &ProtectedDestination,
    verb: &str,
    rule: Option<&str>,
) -> Result<()> {
    if checked_remote(cwd, &seen.remote)? != seen.url
        || automatic(cwd, &seen.url)? != seen.automatic
    {
        bail!("remote or automatic protection changed; review again");
    }
    let mut next = seen.policy.clone();
    match (verb, rule) {
        ("review", None) => next.reviewed = true,
        ("add", Some(r)) => {
            validate_rule(r)?;
            if !next.rules.iter().any(|s| s == r) {
                next.rules.push(r.to_owned());
            }
            next.disabled_automatic.retain(|n| n != r);
        }
        ("remove", Some(r)) => {
            validate_rule(r)?;
            if !next.rules.iter().any(|s| s == r) && !seen.automatic.iter().any(|n| n == r) {
                bail!("rule does not exist");
            }
            next.rules.retain(|s| s != r);
            if seen.automatic.iter().any(|n| n == r)
                && !next.disabled_automatic.iter().any(|n| n == r)
            {
                next.disabled_automatic.push(r.to_owned());
            }
        }
        _ => bail!("invalid protection change"),
    }
    PolicyStore::open(cwd)?.save(&seen.url, &seen.policy, &next)
}
pub(crate) fn plan(cwd: &str, remote: &str, branch: &str) -> Result<LeasePreview> {
    run_git(cwd, &["check-ref-format", "--branch", branch])?;
    let destination = destination(cwd, remote)?;
    destination.policy.check(branch, &destination.automatic)?;
    let local_branch = run_git_stdout(cwd, &["symbolic-ref", "--quiet", "HEAD"])?;
    let local_sha = run_git_stdout(cwd, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    let remote_sha = fetch_branch_oid_isolated(cwd, &destination.url, branch)?;
    let mut command = git_subprocess::command([
        "log",
        "-100",
        "--no-show-signature",
        "--format=%h %an <%ae> %s",
        &format!("{local_sha}..{remote_sha}"),
        "--",
    ]);
    command
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = subprocess::command_output(&mut command, "git lost commits preview")?;
    if !output.status.success() {
        bail!("cannot inspect commits lost from the destination");
    }
    Ok(LeasePreview {
        destination,
        branch: branch.to_owned(),
        local_branch,
        local_sha,
        remote_sha,
        lost: String::from_utf8_lossy(&output.stdout).into_owned(),
    })
}
pub(crate) fn execute(cwd: &str, p: &LeasePreview) -> Result<()> {
    let url = checked_remote(cwd, &p.destination.remote)?;
    if url != p.destination.url
        || run_git_stdout(cwd, &["symbolic-ref", "--quiet", "HEAD"])? != p.local_branch
        || run_git_stdout(cwd, &["rev-parse", "HEAD^{commit}"])? != p.local_sha
    {
        bail!("source or destination changed after review");
    }
    let names = automatic(cwd, &url)?;
    PolicyStore::open(cwd)?.guarded(&url, |policy| {
        policy.check(&p.branch, &names)?;
        if *policy != p.destination.policy || names != p.destination.automatic {
            bail!("policy changed after review");
        }
        let reference = format!("refs/heads/{}", p.branch);
        let lease = format!("--force-with-lease={reference}:{}", p.remote_sha);
        let spec = format!("{}:{reference}", p.local_sha);
        run_git_to_delivery_target(
            cwd,
            &url,
            &[
                "push",
                "--no-follow-tags",
                &lease,
                FROZEN_GITHUB_REMOTE_NAME,
                &spec,
            ],
        )
    })
}
pub(super) fn guarded<T>(
    cwd: &str,
    url: &str,
    branch: &str,
    action: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let names = automatic(cwd, url)?;
    PolicyStore::open(cwd)?.guarded(url, |policy| {
        policy.check(branch, &names)?;
        action()
    })
}
