use super::super::protected_push;
use super::*;

#[test]
fn reviewed_rule_and_exact_lease_bind_the_disposable_destination() {
    let f = GitFixture::new("protected-lease-g5");
    let cwd = path_str(&f.repo);
    git(&f.repo, &["push", "origin", "main"]);
    git(&f.remote, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    let base = git_stdout(&f.repo, &["rev-parse", "HEAD"]);
    git(&f.repo, &["switch", "-c", "topic"]);
    fs::write(f.repo.join("other"), "remote").unwrap();
    git(&f.repo, &["add", "other"]);
    git(&f.repo, &["commit", "-m", "remote work"]);
    git(&f.repo, &["push", "origin", "topic"]);
    let remote_tip = git_stdout(&f.remote, &["rev-parse", "refs/heads/topic"]);
    git(&f.repo, &["switch", "main"]);
    fs::write(f.repo.join("local"), "local").unwrap();
    git(&f.repo, &["add", "local"]);
    git(&f.repo, &["commit", "-m", "local work"]);
    let local_tip = git_stdout(&f.repo, &["rev-parse", "HEAD"]);
    assert!(protected_push::plan(cwd, "origin", "topic").is_err());
    let seen = protected_push::destination(cwd, "origin").unwrap();
    protected_push::edit(cwd, &seen, "review", None).unwrap();
    assert!(protected_push::plan(cwd, "origin", "main").is_err());
    let plan = protected_push::plan(cwd, "origin", "topic").unwrap();
    assert_eq!(plan.remote_sha, remote_tip);
    assert!(plan.lost.contains("remote work"));
    let seen = protected_push::destination(cwd, "origin").unwrap();
    protected_push::edit(cwd, &seen, "add", Some("top*")).unwrap();
    assert!(protected_push::execute(cwd, &plan).is_err());
    let adapter = GithubAutomationAdapter::new();
    let url = path_str(&f.remote);
    assert!(
        adapter
            .delete_branch_if_unchanged_for_delivery_target(
                cwd,
                "origin",
                url,
                "topic",
                &remote_tip
            )
            .is_err()
    );
    assert_eq!(git_stdout(&f.remote, &["rev-parse", "topic"]), remote_tip);
    let seen = protected_push::destination(cwd, "origin").unwrap();
    protected_push::edit(cwd, &seen, "remove", Some("top*")).unwrap();
    let plan = protected_push::plan(cwd, "origin", "topic").unwrap();
    git(
        &f.remote,
        &["update-ref", "refs/heads/topic", &base, &remote_tip],
    );
    // A tracking fetch after review is not the reviewed lease, even if it updates the local observation.
    git(&f.repo, &["fetch", "origin"]);
    assert!(protected_push::execute(cwd, &plan).is_err());
    assert_eq!(git_stdout(&f.remote, &["rev-parse", "topic"]), base);
    git(
        &f.remote,
        &["update-ref", "refs/heads/topic", &remote_tip, &base],
    );
    let plan = protected_push::plan(cwd, "origin", "topic").unwrap();
    protected_push::execute(cwd, &plan).unwrap();
    assert_eq!(git_stdout(&f.remote, &["rev-parse", "topic"]), local_tip);
    assert_eq!(git_stdout(&f.remote, &["rev-parse", "main"]), base);
}
