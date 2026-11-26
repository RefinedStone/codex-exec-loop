use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};

use super::{ConversationWorkspaceStoreAdapter, ConversationWorkspaceStoreLocation};
use crate::adapter::outbound::git::{history::GitHistoryAdapter, workspace::GitWorkspaceAdapter};
use crate::application::port::inbound::conversation_workspace_port::ConversationWorkspacePort;
use crate::application::port::outbound::conversation_workspace_store_port::ConversationWorkspaceStorePort;
use crate::application::port::outbound::git_workspace_port::GitWorkspacePort;
use crate::application::service::conversation_workspace::ConversationWorkspaceService;
use crate::domain::conversation_workspace::{
    ConversationWorkspaceQuery, ConversationWorkspaceRecord, ConversationWorkspaceState,
    PrepareConversationWorkspaceRequest, request_key,
};

struct Fixture {
    root: PathBuf,
    repo: PathBuf,
    runtime: PathBuf,
    base: String,
}

impl Fixture {
    fn new() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-conversation-workspace-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>(),
        ));
        crate::private_directory::ensure_private_directory(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let repo = root.join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]);
        git(&repo, &["config", "user.name", "Workspace Fixture"]);
        git(&repo, &["config", "user.email", "workspace@example.test"]);
        std::fs::write(repo.join("tracked.txt"), "initial\n").unwrap();
        git(&repo, &["add", "tracked.txt"]);
        git(&repo, &["commit", "-m", "base"]);
        let base = git(&repo, &["rev-parse", "HEAD"]);
        Self {
            runtime: root.join("runtime"),
            root,
            repo,
            base,
        }
    }

    fn cwd(&self) -> String {
        self.repo.display().to_string()
    }

    fn store(&self) -> Arc<ConversationWorkspaceStoreAdapter> {
        let repository_dir = self.cwd();
        let runtime_dir = self.runtime.clone();
        Arc::new(ConversationWorkspaceStoreAdapter::new(move |_, _| {
            Ok(Some(ConversationWorkspaceStoreLocation {
                repository_dir: repository_dir.clone(),
                runtime_dir: runtime_dir.clone(),
            }))
        }))
    }

    fn service(&self) -> ConversationWorkspaceService {
        ConversationWorkspaceService::new(
            Arc::new(GitHistoryAdapter),
            Arc::new(GitWorkspaceAdapter),
            self.store(),
        )
    }

    fn request(&self) -> PrepareConversationWorkspaceRequest {
        PrepareConversationWorkspaceRequest {
            cwd: self.cwd(),
            request_id: "new-conversation-1".into(),
            base_commit: self.base.clone(),
        }
    }

    fn query(&self) -> ConversationWorkspaceQuery {
        ConversationWorkspaceQuery {
            cwd: self.cwd(),
            request_id: self.request().request_id,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn git(path: &Path, args: &[&str]) -> String {
    let output = crate::test_utils::git_command()
        .current_dir(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

#[test]
fn conversation_workspace_retry_after_reconnect_preserves_dirty_files_and_new_commits() {
    let fixture = Fixture::new();
    let record = fixture.service().prepare(fixture.request()).unwrap();
    assert_eq!(record.state, ConversationWorkspaceState::Prepared);
    let workspace = Path::new(&record.workspace_dir);
    std::fs::write(workspace.join("tracked.txt"), "user commit\n").unwrap();
    git(workspace, &["add", "tracked.txt"]);
    git(workspace, &["commit", "-m", "user work"]);
    let user_head = git(workspace, &["rev-parse", "HEAD"]);
    std::fs::write(workspace.join("draft.txt"), "unsaved user work\n").unwrap();
    // 새 service/store는 Gateway 재연결·재시작 뒤에도 같은 준비 공간을 돌려준다.
    assert_eq!(
        fixture.service().get(fixture.query()).unwrap(),
        Some(record.clone())
    );
    assert_eq!(
        fixture.service().prepare(fixture.request()).unwrap(),
        record
    );
    assert_eq!(git(workspace, &["rev-parse", "HEAD"]), user_head);
    assert_eq!(
        std::fs::read_to_string(workspace.join("draft.txt")).unwrap(),
        "unsaved user work\n"
    );
    assert_eq!(git(&fixture.repo, &["rev-parse", "HEAD"]), fixture.base);
    assert_eq!(
        git(&fixture.repo, &["worktree", "list", "--porcelain"])
            .lines()
            .filter(|line| line.starts_with("worktree "))
            .count(),
        2
    );
}

#[test]
fn conversation_workspace_query_is_read_only_and_invalid_requests_do_not_create_storage() {
    let fixture = Fixture::new();
    assert!(fixture.service().get(fixture.query()).unwrap().is_none());
    let mut request = fixture.request();
    request.request_id = "../escape".into();
    assert!(fixture.service().prepare(request).is_err());
    let mut request = fixture.request();
    request.base_commit = "HEAD".into();
    assert!(fixture.service().prepare(request).is_err());
    assert!(!fixture.runtime.exists());
}

#[test]
fn conversation_workspace_preparing_record_is_durable_before_git_and_recovers_after_creation() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let request = fixture.request();
    let key = request_key(&request.request_id);
    let mut transaction = store.begin(&fixture.cwd()).unwrap();
    let pending = ConversationWorkspaceRecord {
        request_id: request.request_id.clone(),
        repository_dir: fixture.cwd(),
        workspace_dir: transaction
            .workspaces_dir()
            .join(&key)
            .display()
            .to_string(),
        branch: format!("akra-conversation/{key}"),
        base_commit: fixture.base.clone(),
        state: ConversationWorkspaceState::Preparing,
        reason: None,
    };
    transaction.save(&pending).unwrap();
    // 별도 connection이 lease 유지 중에도 commit된 준비 의도를 읽는다.
    assert_eq!(
        fixture
            .store()
            .get(&fixture.cwd(), &request.request_id)
            .unwrap(),
        Some(pending.clone())
    );
    assert!(!Path::new(&pending.workspace_dir).exists());
    GitWorkspaceAdapter
        .create_conversation_worktree(
            &pending.repository_dir,
            &pending.workspace_dir,
            &pending.branch,
            &pending.base_commit,
        )
        .unwrap();
    drop(transaction); // 완료 기록을 쓰기 전에 프로세스가 끝난 경우와 같은 durable 상태.
    let recovered = fixture.service().prepare(request).unwrap();
    assert_eq!(recovered.workspace_dir, pending.workspace_dir);
    assert_eq!(recovered.state, ConversationWorkspaceState::Prepared);
}

#[test]
fn conversation_workspace_independent_adapters_serialize_duplicate_requests() {
    let fixture = Fixture::new();
    let barrier = Arc::new(Barrier::new(2));
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let service = fixture.service();
            let request = fixture.request();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                service.prepare(request).unwrap()
            })
        })
        .collect();
    let records: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(records[0], records[1]);
    assert_eq!(records[0].state, ConversationWorkspaceState::Prepared);
    assert_eq!(
        std::fs::read_dir(fixture.runtime.join("conversation-workspaces"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn conversation_workspace_same_request_cannot_change_base_or_reclaim_another_branch() {
    let fixture = Fixture::new();
    let record = fixture.service().prepare(fixture.request()).unwrap();
    git(&fixture.repo, &["commit", "--allow-empty", "-m", "next"]);
    let mut changed = fixture.request();
    changed.base_commit = git(&fixture.repo, &["rev-parse", "HEAD"]);
    assert!(fixture.service().prepare(changed).is_err());
    git(Path::new(&record.workspace_dir), &["checkout", "--detach"]);
    let preserved = fixture.service().prepare(fixture.request()).unwrap();
    assert_eq!(preserved.state, ConversationWorkspaceState::Preserved);
    assert_eq!(
        git(Path::new(&record.workspace_dir), &["rev-parse", "HEAD"]),
        fixture.base
    );
    assert_eq!(
        fixture.service().prepare(fixture.request()).unwrap(),
        preserved
    );
}

#[test]
fn conversation_workspace_existing_unowned_directory_is_never_adopted_or_deleted() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let transaction = store.begin(&fixture.cwd()).unwrap();
    let destination = transaction
        .workspaces_dir()
        .join(request_key(&fixture.request().request_id));
    std::fs::create_dir(&destination).unwrap();
    std::fs::write(destination.join("keep"), "foreign content").unwrap();
    drop(transaction);
    let record = fixture.service().prepare(fixture.request()).unwrap();
    assert_eq!(record.state, ConversationWorkspaceState::Preserved);
    assert_eq!(
        std::fs::read_to_string(destination.join("keep")).unwrap(),
        "foreign content"
    );
    assert!(!destination.join(".git").exists());
}

#[test]
fn conversation_workspace_existing_branch_without_ownership_is_not_reused() {
    let fixture = Fixture::new();
    let branch = format!(
        "akra-conversation/{}",
        request_key(&fixture.request().request_id)
    );
    git(&fixture.repo, &["branch", &branch, &fixture.base]);
    assert!(fixture.service().prepare(fixture.request()).is_err());
    assert_eq!(git(&fixture.repo, &["rev-parse", &branch]), fixture.base);
    assert!(fixture.service().get(fixture.query()).unwrap().is_none());
    assert_eq!(
        std::fs::read_dir(fixture.runtime.join("conversation-workspaces"))
            .unwrap()
            .count(),
        0
    );
}

#[cfg(any(unix, windows))]
#[test]
fn conversation_workspace_replaced_directory_link_never_adopts_the_target() {
    let fixture = Fixture::new();
    let record = fixture.service().prepare(fixture.request()).unwrap();
    let workspace = Path::new(&record.workspace_dir);
    std::fs::rename(workspace, fixture.root.join("retained-worktree")).unwrap();
    let target = fixture.root.join("foreign-target");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("sentinel"), "foreign content").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, workspace).unwrap();
    #[cfg(windows)]
    {
        let source = workspace.to_string_lossy();
        let destination = target.to_string_lossy();
        let output = std::process::Command::new("cmd")
            .args([
                "/C",
                "mklink",
                "/J",
                source.strip_prefix(r"\\?\").unwrap_or(&source),
                destination.strip_prefix(r"\\?\").unwrap_or(&destination),
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let preserved = fixture.service().prepare(fixture.request()).unwrap();
    assert_eq!(preserved.state, ConversationWorkspaceState::Preserved);
    assert_eq!(
        std::fs::read_to_string(target.join("sentinel")).unwrap(),
        "foreign content"
    );
    assert!(!target.join(".git").exists());
    #[cfg(unix)]
    std::fs::remove_file(workspace).unwrap();
    #[cfg(windows)]
    std::fs::remove_dir(workspace).unwrap();
}

#[test]
fn conversation_workspace_unknown_store_schema_stops_before_git_effects() {
    let fixture = Fixture::new();
    drop(fixture.store().begin(&fixture.cwd()).unwrap());
    let connection =
        rusqlite::Connection::open(fixture.runtime.join("conversation-workspaces.db")).unwrap();
    connection
        .pragma_update(None, "user_version", 99_i64)
        .unwrap();
    drop(connection);
    assert!(fixture.service().prepare(fixture.request()).is_err());
    assert_eq!(
        std::fs::read_dir(fixture.runtime.join("conversation-workspaces"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        git(&fixture.repo, &["worktree", "list", "--porcelain"])
            .lines()
            .filter(|line| line.starts_with("worktree "))
            .count(),
        1
    );
}

#[test]
fn conversation_workspace_target_filter_and_worktree_config_use_existing_write_guard() {
    let fixture = Fixture::new();
    std::fs::write(fixture.repo.join(".gitattributes"), "*.bin filter=lfs\n").unwrap();
    git(&fixture.repo, &["add", ".gitattributes"]);
    git(&fixture.repo, &["commit", "-m", "filtered target"]);
    let mut request = fixture.request();
    request.base_commit = git(&fixture.repo, &["rev-parse", "HEAD"]);
    git(&fixture.repo, &["checkout", "--detach", &fixture.base]);
    let record = fixture.service().prepare(request).unwrap();
    assert_eq!(record.state, ConversationWorkspaceState::Preserved);
    assert!(!Path::new(&record.workspace_dir).exists());
    git(&fixture.repo, &["config", "core.worktree", "../outside"]);
    let mut request = fixture.request();
    request.request_id = "unsafe-config".into();
    assert!(fixture.service().prepare(request).is_err());
    assert!(!fixture.root.join("outside").exists());
}
