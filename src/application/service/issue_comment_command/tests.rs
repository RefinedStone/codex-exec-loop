//! 실제 이슈 SQLite 장부, 가짜 댓글 출처와 가짜 개입 포트로 댓글 명령의 경계를 검증한다.
use super::*;
use crate::adapter::outbound::db::SqliteIssueAutomationStoreAdapter;
use crate::application::port::outbound::issue_automation_policy_source_port::IssueAutomationPolicySourcePort;
use crate::application::port::outbound::issue_automation_store_port::IssueCollectionApplyResult;
use crate::application::port::outbound::issue_source_port::{
    IssueDiscoveryPage, IssueDiscoveryRequest, IssueDiscoveryTarget,
};
use crate::application::service::issue_policy::issue_policy_version;
use crate::domain::issue_automation::comment_command::IssueCommentCommandOutcome;
use crate::domain::issue_automation::source::*;
use crate::domain::issue_automation::{IssueAutomationSnapshot, TrackedIssue};
use crate::domain::issue_policy::{IssueDisclosurePolicy, IssuePolicy};
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueFieldHash, IssueProjectScope, IssueReference, IssueSharedCopy,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

mod request;

const TRUSTED: &str = "U_trusted";
const ENABLED: &str = r#"
[select]
comment_commands = true
trusted_actors = [{provider="github",server_id="github.com",actor_id="U_trusted"}]
"#;

struct Policy(Mutex<Option<String>>);
impl IssueAutomationPolicySourcePort for Policy {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn load_project_policy_toml(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
}

#[derive(Default)]
struct Source {
    commands: Mutex<Vec<IssueCommentCommand>>,
    reads: AtomicUsize,
}
impl IssueCommentCommandSourcePort for Source {
    fn read_comment_commands(
        &self,
        _: &IssueReference,
        trusted: &[String],
    ) -> Result<Vec<IssueCommentCommand>> {
        assert_eq!(trusted, [TRUSTED.to_owned()]);
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(self.commands.lock().unwrap().clone())
    }
}

/// 같은 command_id 는 기존 영수증을 changed=false 로 돌려준다. 실제 개입 서비스의 계약과 같다.
#[derive(Default)]
struct Interventions {
    applied: Mutex<BTreeMap<String, IssueInterventionReceipt>>,
    calls: Mutex<Vec<(IssueInterventionRequest, StoredCommandContext)>>,
}
impl IssueInterventionPort for Interventions {
    fn intervene(
        &self,
        request: IssueInterventionRequest,
        context: &StoredCommandContext,
    ) -> Result<IssueInterventionReceipt> {
        self.calls
            .lock()
            .unwrap()
            .push((request.clone(), context.clone()));
        let mut applied = self.applied.lock().unwrap();
        if let Some(receipt) = applied.get(&request.command_id) {
            let mut receipt = receipt.clone();
            receipt.changed = false;
            return Ok(receipt);
        }
        let receipt = IssueInterventionReceipt {
            issue: request.issue.clone(),
            command_id: request.command_id.clone(),
            action: request.action.label().into(),
            work_order_id: None,
            authorization_epoch: 1,
            changed: true,
            delivery: None,
            handoff: None,
        };
        applied.insert(request.command_id, receipt.clone());
        Ok(receipt)
    }
}

/// 개입 적용 뒤 기록 전에 끝난 경우를 흉내 낸다.
struct Records {
    inner: Arc<SqliteIssueAutomationStoreAdapter>,
    fail_next_record: AtomicBool,
}
impl IssueCommentCommandStorePort for Records {
    fn recorded_requests(
        &self,
        workspace: &str,
        issue: &IssueReference,
    ) -> Result<Vec<IssueCommentCommandRecord>> {
        self.inner.recorded_requests(workspace, issue)
    }
    fn issue_comment_command(
        &self,
        workspace: &str,
        issue: &IssueReference,
        comment_id: &str,
    ) -> Result<Option<IssueCommentCommandRecord>> {
        self.inner
            .issue_comment_command(workspace, issue, comment_id)
    }
    fn record_issue_comment_command(
        &self,
        workspace: &str,
        record: &IssueCommentCommandRecord,
    ) -> Result<bool> {
        if self.fail_next_record.swap(false, Ordering::SeqCst) {
            anyhow::bail!("injected stop before the comment record");
        }
        self.inner.record_issue_comment_command(workspace, record)
    }
}

struct Fixture {
    root: PathBuf,
    workspace: String,
    issue: IssueReference,
    store: Arc<SqliteIssueAutomationStoreAdapter>,
    policy: Arc<Policy>,
    source: Arc<Source>,
    interventions: Arc<Interventions>,
    records: Arc<Records>,
}

impl Fixture {
    fn new() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-comment-command-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>(),
        ));
        crate::private_directory::ensure_private_directory(&root).unwrap();
        let root = std::fs::canonicalize(root).unwrap();
        let workspace = root.join("workspace");
        let runtime = root.join("runtime");
        crate::private_directory::ensure_private_directory(&workspace).unwrap();
        crate::private_directory::ensure_private_directory(&runtime).unwrap();
        let workspace = workspace.to_str().unwrap().to_owned();
        let store = Arc::new(SqliteIssueAutomationStoreAdapter::new(move |_| {
            Ok(runtime.clone())
        }));
        let issue = IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".to_owned(),
            repository_id: "R_comment".to_owned(),
            issue_id: "I_comment".to_owned(),
            project_scope: IssueProjectScope {
                workspace_dir: workspace.clone(),
            },
        };
        let fixture = Self {
            root,
            workspace,
            issue,
            records: Arc::new(Records {
                inner: store.clone(),
                fail_next_record: AtomicBool::new(false),
            }),
            store,
            policy: Arc::new(Policy(Mutex::new(Some(ENABLED.to_owned())))),
            source: Arc::default(),
            interventions: Arc::default(),
        };
        fixture.seed();
        fixture
    }

    fn service(&self) -> IssueCommentCommandService {
        IssueCommentCommandService::new(
            self.workspace.clone(),
            self.store.clone(),
            self.records.clone(),
            self.source.clone(),
            IssueAutomationPolicyService::new(self.policy.clone()),
            self.interventions.clone(),
        )
    }

    fn poll(&self) -> Result<Vec<IssueInterventionReceipt>> {
        let (result, receipts) = self.poll_delivering();
        result.map(|()| receipts)
    }

    /// 실패한 주기도 그 전에 commit gate 안에서 넘긴 영수증을 함께 돌려준다.
    fn poll_delivering(&self) -> (Result<()>, Vec<IssueInterventionReceipt>) {
        let delivered = Mutex::new(Vec::new());
        let result = self.service().poll(
            &IssueCollectionCancellation::new(),
            1_795_000_000_000,
            &|receipt| delivered.lock().unwrap().push(receipt),
        );
        (result, delivered.into_inner().unwrap())
    }

    fn comment(&self, id: &str, author: &str, kind: IssueCommentCommandKind) {
        self.source
            .commands
            .lock()
            .unwrap()
            .push(IssueCommentCommand {
                comment_id: id.to_owned(),
                author_id: author.to_owned(),
                kind,
            });
    }

    fn record(&self, id: &str) -> Option<IssueCommentCommandRecord> {
        self.store
            .issue_comment_command(&self.workspace, &self.issue, id)
            .unwrap()
    }

    fn intervention_calls(&self) -> usize {
        self.interventions.calls.lock().unwrap().len()
    }

    fn seed(&self) {
        let shared_copy = IssueSharedCopy {
            title: "Shared comment issue".to_owned(),
            body: "## 완료 기준\n- 댓글 명령은 한 번만 처리한다.\n".to_owned(),
        };
        let snapshot = IssueAutomationSnapshot {
            basis: IssueComparisonBasis {
                field_hash: IssueFieldHash::of_fields(&shared_copy.title, &shared_copy.body),
                provider_updated_at: "2026-11-25T00:00:00Z".to_owned(),
            },
            shared_copy,
            policy_version: issue_policy_version(&IssuePolicy::builtin()),
        };

        let observation = IssueSourceObservation {
            content_problems: vec![],
            jira: None,
            linear: None,
            board: None,
            repository_visibility: Default::default(),
            repository_name_with_owner: None,
            issue: self.issue.clone(),
            web: None,
            remote_state: IssueRemoteState::Open,
            labels: empty_list(),
            assignees: empty_list(),
            author: None,
            label_actors: empty_list(),
            linked_pull_request: IssuePrPresence::Absent,
            creation_provenance: IssueCreationProvenance::Unattributed,
            content: IssueSourceContent::Shared(snapshot.clone()),
        };
        let state = self.store.ensure_collection(&self.workspace).unwrap();
        let repository = IssueSourceRepository::from_reference(&self.issue);
        let request = IssueDiscoveryRequest {
            target: IssueDiscoveryTarget::Repository(repository.clone()),
            selection: IssueDiscoverySelection::AllOpen,
            cursor: state.discovery_cursor.clone(),
            page_size: 100,
            disclosure: IssueSourceDisclosure {
                policy: IssueDisclosurePolicy::builtin(),
                policy_version: issue_policy_version(&IssuePolicy::builtin()),
            },
        };
        let page = IssueDiscoveryPage {
            jira_connection: None,
            jira_binding: None,
            linear: None,
            repository,
            observations: vec![observation],
            next_cursor: None,
            scanned_count: 1,
        };
        assert!(matches!(
            self.store
                .apply_discovery_page(&self.workspace, state.fence(), &request, &page)
                .unwrap(),
            IssueCollectionApplyResult::Applied(_)
        ));
        if self
            .store
            .get_issue(&self.workspace, &self.issue)
            .unwrap()
            .is_none()
        {
            let binding = self
                .store
                .get_collection_projection(&self.workspace)
                .unwrap()
                .unwrap()
                .binding
                .unwrap();
            let tracked = TrackedIssue::discovered(self.issue.clone(), &binding, snapshot).unwrap();
            self.store.insert_issue(&self.workspace, &tracked).unwrap();
        }
    }
}

fn empty_list<T>() -> IssueMetadataList<T> {
    IssueMetadataList {
        items: Vec::new(),
        completeness: IssueMetadataCompleteness::Complete,
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn comments_are_not_read_unless_the_user_policy_enables_them() {
    let fixture = Fixture::new();
    *fixture.policy.0.lock().unwrap() = Some(
        "[select]\ntrusted_actors = [{provider=\"github\",server_id=\"github.com\",actor_id=\"U_trusted\"}]\n"
            .to_owned(),
    );
    fixture.comment("IC_1", TRUSTED, IssueCommentCommandKind::Hold);
    assert!(fixture.poll().unwrap().is_empty());
    assert_eq!(fixture.source.reads.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.intervention_calls(), 0);
}

#[test]
fn hold_applies_once_without_human_confirmation_and_repeated_polls_do_not_replay_it() {
    let fixture = Fixture::new();
    fixture.comment("IC_hold", TRUSTED, IssueCommentCommandKind::Hold);
    let receipts = fixture.poll().unwrap();
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].action, "hold_issue");
    let (request, context) = fixture.interventions.calls.lock().unwrap()[0].clone();
    assert_eq!(request.command_id, "issue-comment:IC_hold");
    assert_eq!(request.action, IssueInterventionAction::Hold);
    assert!(context.human_confirmation.is_none());
    assert_eq!(
        fixture.record("IC_hold").unwrap().intervention_command_id,
        Some("issue-comment:IC_hold".to_owned())
    );
    for _ in 0..2 {
        assert!(fixture.poll().unwrap().is_empty());
    }
    assert_eq!(fixture.intervention_calls(), 1);
}

#[test]
fn approve_release_and_retriage_are_recorded_but_never_applied() {
    let fixture = Fixture::new();
    fixture.comment("IC_approve", TRUSTED, IssueCommentCommandKind::Approve);
    fixture.comment("IC_release", TRUSTED, IssueCommentCommandKind::Release);
    fixture.comment("IC_retriage", TRUSTED, IssueCommentCommandKind::Retriage);
    assert!(fixture.poll().unwrap().is_empty());
    assert_eq!(fixture.intervention_calls(), 0);
    for id in ["IC_approve", "IC_release", "IC_retriage"] {
        let record = fixture.record(id).unwrap();
        assert_eq!(record.intervention_command_id, None);
        assert_ne!(record.kind.outcome(), IssueCommentCommandOutcome::Applied);
    }
}

#[test]
fn untrusted_or_revoked_authors_are_ignored_and_not_recorded() {
    let fixture = Fixture::new();
    fixture.comment("IC_other", "U_other", IssueCommentCommandKind::Hold);
    assert!(fixture.poll().unwrap().is_empty());
    assert_eq!(fixture.record("IC_other"), None);

    *fixture.policy.0.lock().unwrap() =
        Some("[select]\ncomment_commands = true\ntrusted_actors = []\n".to_owned());
    fixture.comment("IC_hold", TRUSTED, IssueCommentCommandKind::Hold);
    assert!(fixture.poll().unwrap().is_empty());
    assert_eq!(fixture.record("IC_hold"), None);
    assert_eq!(fixture.intervention_calls(), 0);
}

#[test]
fn a_stop_after_the_hold_reuses_the_same_intervention_without_a_second_change() {
    let fixture = Fixture::new();
    fixture.comment("IC_hold", TRUSTED, IssueCommentCommandKind::Hold);
    fixture
        .records
        .fail_next_record
        .store(true, Ordering::SeqCst);
    let (result, delivered) = fixture.poll_delivering();
    assert!(result.is_err());
    // 확정된 보류는 뒤의 기록 실패와 무관하게 이미 넘어갔다. 재시도는 changed=false 라 다시 넘기지 않는다.
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].action, "hold_issue");
    assert_eq!(fixture.record("IC_hold"), None);

    assert!(fixture.poll().unwrap().is_empty());
    let calls = fixture.interventions.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].0, calls[1].0);
    assert_eq!(fixture.interventions.applied.lock().unwrap().len(), 1);
    drop(calls);
    assert!(fixture.record("IC_hold").is_some());
}
