use super::*;
use crate::application::port::inbound::issue_form_install_port::{
    IssueFormInstallRequest, IssueFormInstallStatus,
};
use crate::application::port::outbound::github_automation_port::{
    GithubAutomationCapabilities, GithubAutomationPort, GithubAutomationPullRequest,
};
use crate::application::service::issue_form_install::{ISSUE_FORM_ASSET, IssueFormInstallService};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

mod ref_contention;

struct Fixture {
    root: PathBuf,
    repo: PathBuf,
    remote: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-issue-form-{}-{suffix}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst),
        ));
        let repo = root.join("repo");
        let remote = root.join("remote.git");
        fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.name", "Form Fixture"]);
        git(&repo, &["config", "user.email", "fixture@example.invalid"]);
        fs::write(repo.join("README.md"), "base\n").unwrap();
        fs::write(repo.join("staged.txt"), "base staged file\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "base"]);
        git(&root, &["init", "--bare", "-q", remote.to_str().unwrap()]);
        git(
            &repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git(&repo, &["push", "-q", "origin", "main"]);
        Self { root, repo, remote }
    }

    fn request(&self) -> IssueFormInstallRequest {
        IssueFormInstallRequest {
            workspace_dir: self.repo.display().to_string(),
            base_branch: "main".to_owned(),
        }
    }

    fn target(&self) -> IssueFormTarget {
        IssueFormTarget {
            repository: "acme/widgets".to_owned(),
            push_remote: "origin".to_owned(),
            push_url: "https://github.com/acme/widgets.git".to_owned(),
            base_branch: "main".to_owned(),
            asset_sha256: format!("{:x}", Sha256::digest(ISSUE_FORM_ASSET.as_bytes())),
        }
    }

    fn github(&self) -> Arc<FakeGithub> {
        Arc::new(FakeGithub {
            remote: self.remote.clone(),
            target: self.target(),
            creates: AtomicUsize::new(0),
            pushes: AtomicUsize::new(0),
            response: AtomicUsize::new(0),
            push_response_lost: AtomicBool::new(false),
            insert_foreign_branch: AtomicBool::new(false),
            observed_sent: AtomicBool::new(false),
            pr: Mutex::new(None),
        })
    }

    fn service(&self, github: Arc<FakeGithub>) -> IssueFormInstallService {
        IssueFormInstallService::new(Arc::new(GitIssueFormRepositoryAdapter::new()), github)
    }

    fn stored(&self) -> StoredIssueFormInstall {
        let adapter = GitIssueFormRepositoryAdapter::new();
        let repository = adapter
            .inspect(self.repo.to_str().unwrap(), "main")
            .unwrap();
        adapter.load(&repository, &self.target()).unwrap().unwrap()
    }

    fn make_dirty(&self) {
        fs::write(self.repo.join("README.md"), "user unstaged bytes\r\n").unwrap();
        fs::write(self.repo.join("staged.txt"), "user staged bytes\n").unwrap();
        git(&self.repo, &["add", "staged.txt"]);
        fs::create_dir_all(self.repo.join("notes")).unwrap();
        fs::write(self.repo.join("notes/untracked.bin"), [0, 1, 255, 7]).unwrap();
        fs::write(self.repo.join(".git/FETCH_HEAD"), "user fetch evidence\n").unwrap();
    }

    fn commit_form(&self, contents: &str) {
        fs::create_dir_all(self.repo.join(".github/ISSUE_TEMPLATE")).unwrap();
        fs::write(self.repo.join(ISSUE_FORM_PATH), contents).unwrap();
        git(&self.repo, &["add", ISSUE_FORM_PATH]);
        git(&self.repo, &["commit", "-q", "-m", "existing form"]);
        git(&self.repo, &["push", "-q", "origin", "main"]);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn git_bytes(repo: &Path, args: &[&str]) -> Vec<u8> {
    let output = crate::test_utils::git_command()
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "fixture Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn git(repo: &Path, args: &[&str]) -> String {
    String::from_utf8(git_bytes(repo, args))
        .unwrap()
        .trim()
        .to_owned()
}

#[derive(Debug, PartialEq, Eq)]
struct CheckoutSnapshot {
    head: String,
    head_file: Vec<u8>,
    index: Vec<u8>,
    fetch_head: Option<Vec<u8>>,
    files: Vec<(PathBuf, Vec<u8>)>,
}

fn checkout_snapshot(repo: &Path) -> CheckoutSnapshot {
    fn visit(root: &Path, path: &Path, files: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name() == ".git" {
                continue;
            }
            if entry.file_type().unwrap().is_dir() {
                visit(root, &entry.path(), files);
            } else {
                files.push((
                    entry.path().strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(entry.path()).unwrap(),
                ));
            }
        }
    }
    let mut files = Vec::new();
    visit(repo, repo, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));
    CheckoutSnapshot {
        head: git(repo, &["rev-parse", "HEAD"]),
        head_file: fs::read(repo.join(".git/HEAD")).unwrap(),
        index: fs::read(repo.join(".git/index")).unwrap(),
        fetch_head: fs::read(repo.join(".git/FETCH_HEAD")).ok(),
        files,
    }
}

struct FakeGithub {
    remote: PathBuf,
    target: IssueFormTarget,
    creates: AtomicUsize,
    pushes: AtomicUsize,
    // 0 success, 1 lost before a PR was created, 2 created but response was lost.
    response: AtomicUsize,
    push_response_lost: AtomicBool,
    insert_foreign_branch: AtomicBool,
    observed_sent: AtomicBool,
    pr: Mutex<Option<GithubAutomationPullRequest>>,
}

impl FakeGithub {
    fn head(&self, branch: &str) -> Option<String> {
        let output = crate::test_utils::git_command()
            .arg("-C")
            .arg(&self.remote)
            .args([
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ])
            .output()
            .unwrap();
        if output.status.success() {
            Some(String::from_utf8(output.stdout).unwrap().trim().to_owned())
        } else {
            assert_eq!(output.status.code(), Some(1));
            None
        }
    }
}

impl GithubAutomationPort for FakeGithub {
    fn inspect_capabilities(&self, _: &str) -> GithubAutomationCapabilities {
        panic!("not needed")
    }
    fn push_branch(&self, _: &str, _: &str, _: bool) -> Result<()> {
        panic!("not needed")
    }
    fn ensure_pull_request(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<GithubAutomationPullRequest> {
        panic!("unfrozen write")
    }
    fn inspect_pull_request(&self, _: &str, _: u64) -> Result<GithubAutomationPullRequest> {
        panic!("unfrozen read")
    }
    fn push_integration_branch(&self, _: &str, _: &str, _: &str) -> Result<()> {
        panic!("not needed")
    }
    fn close_pull_request(&self, _: &str, _: u64) -> Result<()> {
        panic!("not needed")
    }

    fn credential_redacted_push_url_for_remote(&self, _: &str, remote: &str) -> Result<String> {
        assert_eq!(remote, "origin");
        Ok(self.target.push_url.clone())
    }

    fn remote_branch_head_for_delivery_target(
        &self,
        _: &str,
        _: &str,
        url: &str,
        branch: &str,
    ) -> Result<Option<String>> {
        assert_eq!(url, self.target.push_url);
        Ok(self.head(branch))
    }

    fn fetch_branch_to_tracking_ref_for_delivery_target(
        &self,
        repo: &str,
        _: &str,
        url: &str,
        branch: &str,
        tracking_ref: &str,
    ) -> Result<String> {
        assert_eq!(url, self.target.push_url);
        assert_eq!(tracking_ref, self.target.base_ref());
        let sha = self.head(branch).unwrap();
        // This local bare fixture was created from the same object store, so the immutable base is already available.
        git(
            Path::new(repo),
            &["cat-file", "-e", &format!("{sha}^{{commit}}")],
        );
        Ok(sha)
    }

    fn create_frozen_branch_for_delivery_target(
        &self,
        repo: &str,
        _: &str,
        url: &str,
        sha: &str,
        branch: &str,
    ) -> Result<()> {
        assert_eq!(url, self.target.push_url);
        self.pushes.fetch_add(1, Ordering::SeqCst);
        if self.insert_foreign_branch.swap(false, Ordering::SeqCst) {
            let base = self.head("main").unwrap();
            git(
                &self.remote,
                &["update-ref", &format!("refs/heads/{branch}"), &base],
            );
        }
        let lease = format!("--force-with-lease=refs/heads/{branch}:");
        let refspec = format!("{sha}:refs/heads/{branch}");
        let output = crate::test_utils::git_command()
            .arg("-C")
            .arg(repo)
            .args([
                "push",
                "--quiet",
                &lease,
                self.remote.to_str().unwrap(),
                &refspec,
            ])
            .output()
            .unwrap();
        if !output.status.success() {
            bail!("fixture remote refused an existing branch");
        }
        if self.push_response_lost.load(Ordering::SeqCst) {
            bail!("fixture push response lost");
        }
        Ok(())
    }

    fn find_pull_request_for_delivery_target(
        &self,
        _: &str,
        _: &str,
        url: &str,
        base: &str,
        head: &str,
    ) -> Result<Option<GithubAutomationPullRequest>> {
        assert_eq!(url, self.target.push_url);
        assert_eq!(base, "main");
        assert_eq!(head, self.target.head_branch());
        Ok(self.pr.lock().unwrap().clone())
    }

    fn ensure_pull_request_for_delivery_target(
        &self,
        repo: &str,
        _: &str,
        url: &str,
        base: &str,
        head: &str,
        _: &str,
        _: &str,
    ) -> Result<GithubAutomationPullRequest> {
        assert_eq!(url, self.target.push_url);
        let adapter = GitIssueFormRepositoryAdapter::new();
        let repository = adapter.inspect(repo, "main")?;
        let stored = adapter.load(&repository, &self.target)?.unwrap();
        assert_eq!(
            stored.record.phase,
            IssueFormInstallPhase::Sent,
            "remote mutation requires a durable Sent record"
        );
        self.observed_sent.store(true, Ordering::SeqCst);
        self.creates.fetch_add(1, Ordering::SeqCst);
        if self.response.load(Ordering::SeqCst) == 1 {
            bail!("PRIVATE_UNEXPECTED_PROVIDER_BODY");
        }
        let pr = GithubAutomationPullRequest::new(
            23,
            "https://github.com/acme/widgets/pull/23",
            "OPEN",
            base,
            head,
            false,
        )
        .with_head_commit_sha(self.head(head).unwrap());
        *self.pr.lock().unwrap() = Some(pr.clone());
        if self.response.load(Ordering::SeqCst) == 2 {
            bail!("PRIVATE_UNEXPECTED_PROVIDER_BODY");
        }
        Ok(pr)
    }
}

#[test]
fn installation_adds_exactly_one_frozen_file_and_preserves_dirty_index_head_and_untracked_bytes() {
    let fixture = Fixture::new();
    fixture.make_dirty();
    let before = checkout_snapshot(&fixture.repo);
    let github = fixture.github();
    let result = fixture
        .service(github.clone())
        .install(&fixture.request())
        .unwrap();
    assert_eq!(result.status, IssueFormInstallStatus::PullRequestCreated);
    assert_eq!(before, checkout_snapshot(&fixture.repo));
    assert!(github.observed_sent.load(Ordering::SeqCst));
    let plan = fixture.stored().record.plan;
    assert_eq!(
        git_bytes(
            &fixture.repo,
            &[
                "diff-tree",
                "--no-commit-id",
                "--name-only",
                "-r",
                "-z",
                &plan.base_commit_sha,
                &plan.commit_sha,
            ]
        ),
        format!("{ISSUE_FORM_PATH}\0").as_bytes()
    );
    assert_eq!(
        git_bytes(
            &fixture.repo,
            &["show", &format!("{}:{ISSUE_FORM_PATH}", plan.commit_sha)]
        ),
        ISSUE_FORM_ASSET.as_bytes()
    );
    assert_eq!(
        github.head(&plan.target.head_branch()).as_deref(),
        Some(plan.commit_sha.as_str())
    );
}

#[test]
fn identical_untracked_form_still_proposes_a_pr_while_identical_base_is_a_noop() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.repo.join(".github/ISSUE_TEMPLATE")).unwrap();
    fs::write(fixture.repo.join(ISSUE_FORM_PATH), ISSUE_FORM_ASSET).unwrap();
    let before = checkout_snapshot(&fixture.repo);
    let github = fixture.github();
    assert_eq!(
        fixture
            .service(github)
            .install(&fixture.request())
            .unwrap()
            .status,
        IssueFormInstallStatus::PullRequestCreated
    );
    assert_eq!(before, checkout_snapshot(&fixture.repo));

    let installed = Fixture::new();
    installed.commit_form(ISSUE_FORM_ASSET);
    let github = installed.github();
    let before = checkout_snapshot(&installed.repo);
    assert_eq!(
        installed
            .service(github.clone())
            .install(&installed.request())
            .unwrap()
            .status,
        IssueFormInstallStatus::AlreadyInstalled
    );
    assert_eq!(github.creates.load(Ordering::SeqCst), 0);
    assert_eq!(github.pushes.load(Ordering::SeqCst), 0);
    assert_eq!(before, checkout_snapshot(&installed.repo));
}

#[test]
fn a_different_existing_form_is_a_conflict_without_remote_writes() {
    let fixture = Fixture::new();
    fixture.commit_form("name: User form\nbody: []\n");
    let before = checkout_snapshot(&fixture.repo);
    let github = fixture.github();
    assert_eq!(
        fixture
            .service(github.clone())
            .install(&fixture.request())
            .unwrap()
            .status,
        IssueFormInstallStatus::Conflict
    );
    assert_eq!(github.pushes.load(Ordering::SeqCst), 0);
    assert_eq!(github.creates.load(Ordering::SeqCst), 0);
    assert_eq!(before, checkout_snapshot(&fixture.repo));
}

#[test]
fn lost_pr_response_is_never_resent_and_can_be_reconciled_by_reading_the_exact_pr() {
    for response in [1, 2] {
        let fixture = Fixture::new();
        let github = fixture.github();
        github.response.store(response, Ordering::SeqCst);
        let service = fixture.service(github.clone());
        let first = service.install(&fixture.request()).unwrap();
        assert_eq!(first.status, IssueFormInstallStatus::NeedsConfirmation);
        assert_eq!(fixture.stored().record.phase, IssueFormInstallPhase::Sent);
        assert!(
            !serde_json::to_string(&first)
                .unwrap()
                .contains("PRIVATE_UNEXPECTED_PROVIDER_BODY")
        );
        let replay = service.install(&fixture.request()).unwrap();
        assert_eq!(
            replay.status,
            if response == 1 {
                IssueFormInstallStatus::NeedsConfirmation
            } else {
                IssueFormInstallStatus::PullRequestFound
            }
        );
        assert_eq!(github.creates.load(Ordering::SeqCst), 1);
        assert_eq!(github.pushes.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn push_sent_without_an_observed_branch_is_not_retried() {
    let fixture = Fixture::new();
    let adapter = GitIssueFormRepositoryAdapter::new();
    let repository = adapter
        .inspect(fixture.repo.to_str().unwrap(), "main")
        .unwrap();
    let base = git(&fixture.repo, &["rev-parse", "HEAD"]);
    let IssueFormPreparation::Prepared(stored) = adapter
        .prepare(&repository, &fixture.target(), &base, ISSUE_FORM_ASSET)
        .unwrap()
    else {
        panic!("plan");
    };
    let mut sent = stored.record.clone();
    sent.phase = IssueFormInstallPhase::PushSent;
    assert!(matches!(
        adapter.save(&repository, &stored, &sent).unwrap(),
        IssueFormInstallSaveResult::Saved(_)
    ));
    let github = fixture.github();
    let result = fixture
        .service(github.clone())
        .install(&fixture.request())
        .unwrap();
    assert_eq!(result.status, IssueFormInstallStatus::NeedsConfirmation);
    assert_eq!(github.pushes.load(Ordering::SeqCst), 0);
    assert_eq!(github.creates.load(Ordering::SeqCst), 0);
}

#[test]
fn an_observed_exact_branch_recovers_a_lost_push_response_without_another_push() {
    let fixture = Fixture::new();
    let github = fixture.github();
    github.push_response_lost.store(true, Ordering::SeqCst);
    let service = fixture.service(github.clone());
    assert_eq!(
        service.install(&fixture.request()).unwrap().status,
        IssueFormInstallStatus::NeedsConfirmation
    );
    assert_eq!(
        fixture.stored().record.phase,
        IssueFormInstallPhase::PushSent
    );
    assert_eq!(
        service.install(&fixture.request()).unwrap().status,
        IssueFormInstallStatus::PullRequestCreated
    );
    assert_eq!(github.pushes.load(Ordering::SeqCst), 1);
    assert_eq!(github.creates.load(Ordering::SeqCst), 1);
}

#[test]
fn a_racing_different_remote_sha_is_never_overwritten() {
    let fixture = Fixture::new();
    let github = fixture.github();
    let base = github.head("main").unwrap();
    github.insert_foreign_branch.store(true, Ordering::SeqCst);
    let service = fixture.service(github.clone());
    assert_eq!(
        service.install(&fixture.request()).unwrap().status,
        IssueFormInstallStatus::NeedsConfirmation
    );
    assert_eq!(
        github.head(&fixture.target().head_branch()).as_deref(),
        Some(base.as_str())
    );
    assert_eq!(
        service.install(&fixture.request()).unwrap().status,
        IssueFormInstallStatus::Conflict
    );
    assert_eq!(github.pushes.load(Ordering::SeqCst), 1);
    assert_eq!(github.creates.load(Ordering::SeqCst), 0);
}

#[test]
fn concurrent_installers_share_the_cas_journal_and_send_each_remote_write_at_most_once() {
    let fixture = Fixture::new();
    let github = fixture.github();
    let service = Arc::new(fixture.service(github.clone()));
    let request = fixture.request();
    let barrier = Barrier::new(2);
    let results = std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            barrier.wait();
            service.install(&request)
        });
        let second = scope.spawn(|| {
            barrier.wait();
            service.install(&request)
        });
        [
            first.join().unwrap().unwrap(),
            second.join().unwrap().unwrap(),
        ]
    });
    for result in results {
        assert!(matches!(
            result.status,
            IssueFormInstallStatus::PullRequestCreated
                | IssueFormInstallStatus::PullRequestFound
                | IssueFormInstallStatus::NeedsConfirmation
        ));
    }
    assert_eq!(github.pushes.load(Ordering::SeqCst), 1);
    assert_eq!(github.creates.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.stored().record.phase,
        IssueFormInstallPhase::Confirmed
    );
}

#[test]
fn a_stale_base_or_pr_head_never_becomes_a_new_write_or_a_successful_confirmation() {
    let fixture = Fixture::new();
    let adapter = GitIssueFormRepositoryAdapter::new();
    let repository = adapter
        .inspect(fixture.repo.to_str().unwrap(), "main")
        .unwrap();
    let base = git(&fixture.repo, &["rev-parse", "HEAD"]);
    assert!(matches!(
        adapter
            .prepare(&repository, &fixture.target(), &base, ISSUE_FORM_ASSET)
            .unwrap(),
        IssueFormPreparation::Prepared(_)
    ));
    fs::write(fixture.repo.join("README.md"), "new base\n").unwrap();
    git(&fixture.repo, &["add", "README.md"]);
    git(&fixture.repo, &["commit", "-q", "-m", "base advanced"]);
    git(&fixture.repo, &["push", "-q", "origin", "main"]);
    let github = fixture.github();
    assert_eq!(
        fixture
            .service(github.clone())
            .install(&fixture.request())
            .unwrap()
            .status,
        IssueFormInstallStatus::Conflict
    );
    assert_eq!(github.pushes.load(Ordering::SeqCst), 0);

    let fixture = Fixture::new();
    let github = fixture.github();
    github.response.store(2, Ordering::SeqCst);
    let service = fixture.service(github.clone());
    assert_eq!(
        service.install(&fixture.request()).unwrap().status,
        IssueFormInstallStatus::NeedsConfirmation
    );
    github.pr.lock().unwrap().as_mut().unwrap().head_commit_sha = Some("f".repeat(40));
    assert_eq!(
        service.install(&fixture.request()).unwrap().status,
        IssueFormInstallStatus::NeedsConfirmation
    );
    assert_eq!(github.creates.load(Ordering::SeqCst), 1);
}

#[test]
fn stale_journal_cas_and_plan_replacement_are_rejected() {
    let fixture = Fixture::new();
    let adapter = GitIssueFormRepositoryAdapter::new();
    let repository = adapter
        .inspect(fixture.repo.to_str().unwrap(), "main")
        .unwrap();
    let base = git(&fixture.repo, &["rev-parse", "HEAD"]);
    let IssueFormPreparation::Prepared(stored) = adapter
        .prepare(&repository, &fixture.target(), &base, ISSUE_FORM_ASSET)
        .unwrap()
    else {
        panic!("plan");
    };
    let mut sent = stored.record.clone();
    sent.phase = IssueFormInstallPhase::PushSent;
    assert!(matches!(
        adapter.save(&repository, &stored, &sent).unwrap(),
        IssueFormInstallSaveResult::Saved(_)
    ));
    assert!(matches!(
        adapter.save(&repository, &stored, &sent).unwrap(),
        IssueFormInstallSaveResult::Conflict
    ));
    let current = adapter
        .load(&repository, &fixture.target())
        .unwrap()
        .unwrap();
    let mut replaced = current.record.clone();
    replaced.plan.commit_sha = base;
    replaced.phase = IssueFormInstallPhase::BranchReady;
    assert!(adapter.save(&repository, &current, &replaced).is_err());
    assert_eq!(
        adapter
            .load(&repository, &fixture.target())
            .unwrap()
            .unwrap(),
        current
    );
}

#[cfg(unix)]
#[test]
fn a_symlink_form_or_private_journal_ref_is_rejected_without_touching_the_target() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let sentinel = fixture.root.join("sentinel");
    fs::write(&sentinel, "unchanged").unwrap();
    fs::create_dir_all(fixture.repo.join(".github/ISSUE_TEMPLATE")).unwrap();
    symlink(&sentinel, fixture.repo.join(ISSUE_FORM_PATH)).unwrap();
    let github = fixture.github();
    assert_eq!(
        fixture
            .service(github.clone())
            .install(&fixture.request())
            .unwrap()
            .status,
        IssueFormInstallStatus::Conflict
    );
    assert_eq!(github.pushes.load(Ordering::SeqCst), 0);
    assert_eq!(fs::read_to_string(&sentinel).unwrap(), "unchanged");
    fs::remove_file(fixture.repo.join(ISSUE_FORM_PATH)).unwrap();
    let reference = fixture
        .repo
        .join(".git")
        .join(fixture.target().journal_ref());
    fs::create_dir_all(reference.parent().unwrap()).unwrap();
    symlink(&sentinel, reference).unwrap();
    assert!(
        fixture
            .service(github.clone())
            .install(&fixture.request())
            .is_err()
    );
    assert_eq!(github.pushes.load(Ordering::SeqCst), 0);
    assert_eq!(fs::read_to_string(&sentinel).unwrap(), "unchanged");
}

#[test]
fn git_version_gate_accepts_platform_suffixes_and_rejects_older_durability_contracts() {
    assert!(supported_git_version("git version 2.43.0"));
    assert!(supported_git_version("git version 2.53.0.windows.1"));
    assert!(!supported_git_version("git version 2.42.9"));
    assert!(!supported_git_version("unknown"));
}

#[cfg(windows)]
#[test]
fn git_arguments_keep_unc_roots_and_do_not_reinterpret_other_extended_paths() {
    assert_eq!(
        git_path_argument(r"\\?\UNC\server\share\project\.git"),
        r"\\server\share\project\.git"
    );
    assert_eq!(
        git_path_argument(r"\\?\C:\workspace\project\.git"),
        r"C:\workspace\project\.git"
    );
    let other_extended = r"\\?\Volume{synthetic-volume}\project\.git";
    assert_eq!(git_path_argument(other_extended), other_extended);
}
