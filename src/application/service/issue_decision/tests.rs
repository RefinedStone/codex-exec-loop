use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::adapter::outbound::db::SqliteIssueAutomationStoreAdapter;
use crate::application::port::outbound::issue_automation_policy_source_port::IssueAutomationPolicySourcePort;
use crate::application::port::outbound::issue_policy_source_port::IssuePolicySourcePort;
use crate::application::port::outbound::issue_source_port::{
    IssueDiscoveryPage, IssueDiscoveryRequest, IssueDiscoveryTarget,
};
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::application::service::issue_collection::IssueCollectionCancellation;
use crate::application::service::issue_policy::issue_policy_version;
use crate::application::service::issue_triage::{IssueTriageRunOutcome, IssueTriagedFact};
use crate::domain::issue_automation::IssueAutomationSnapshot;
use crate::domain::issue_automation::collection::{
    IssueCollectionMode, IssueCollectionPage, IssueCollectionQuery,
};
use crate::domain::issue_automation::decision::*;
use crate::domain::issue_automation::policy::IssueAutomationGate;
use crate::domain::issue_automation::source::*;
use crate::domain::issue_policy::IssueDisclosurePolicy;
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueFieldHash, IssueProjectScope, IssueProvider, IssueSharedCopy,
};
const AUTO_POLICY: &str = r#"
[binding.github]
mode = "observe"
[authority]
autonomy_cap = "auto"
[select]
mode = "all_open"
trusted_actors = [{provider="github",server_id="github.com",actor_id="U_author"}]
[gates.auto.normal]
start = "auto"
delivery = "auto"
merge = "auto"
"#;

struct Fixture {
    root: PathBuf,
    workspace: String,
    runtime: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-triage-service-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        crate::private_directory::ensure_private_directory(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let workspace = root.join("workspace");
        let runtime = root.join("runtime");
        crate::private_directory::ensure_private_directory(&workspace).unwrap();
        crate::private_directory::ensure_private_directory(&runtime).unwrap();
        Self {
            root,
            workspace: workspace.to_str().unwrap().to_string(),
            runtime,
        }
    }

    fn store(&self) -> Arc<SqliteIssueAutomationStoreAdapter> {
        let runtime = self.runtime.clone();
        Arc::new(SqliteIssueAutomationStoreAdapter::new(move |_| {
            Ok(runtime.clone())
        }))
    }

    fn service(&self, policy: Arc<MutableAutomationPolicy>) -> IssueTriageService {
        IssueTriageService::new(
            self.workspace.clone(),
            self.store(),
            IssueAutomationPolicyService::new(policy),
            IssuePolicyService::new(Arc::new(ValidationPolicy)),
        )
    }

    fn reference(&self, id: &str) -> IssueReference {
        IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".into(),
            repository_id: "R_triage".into(),
            issue_id: id.into(),
            project_scope: IssueProjectScope {
                workspace_dir: self.workspace.clone(),
            },
        }
    }

    fn observation(&self, id: &str) -> IssueSourceObservation {
        let shared_copy = IssueSharedCopy {
            title: "공유 제목".into(),
            body: "## 완료 기준\n- 기능이 요구한 결과를 반환한다.\n".into(),
        };
        IssueSourceObservation {
            creation_provenance:
                crate::domain::issue_automation::source::IssueCreationProvenance::Unattributed,
            issue: self.reference(id),
            web: None,
            remote_state: IssueRemoteState::Open,
            labels: complete(Vec::new()),
            assignees: complete(Vec::new()),
            author: Some(IssueSourceActor {
                kind: crate::domain::issue_automation::source::IssueSourceActorKind::UserAccount,
                id: "U_author".into(),
                login: "author".into(),
            }),
            label_actors: complete(Vec::new()),
            linked_pull_request: IssuePrPresence::Absent,
            content: IssueSourceContent::Shared(IssueAutomationSnapshot {
                basis: IssueComparisonBasis {
                    field_hash: IssueFieldHash::of_fields(&shared_copy.title, &shared_copy.body),
                    provider_updated_at: "2026-11-20T00:00:00Z".into(),
                },
                shared_copy,
                policy_version: disclosure_version(),
            }),
        }
    }

    fn seed(&self, observations: Vec<IssueSourceObservation>, policy_version: &str) {
        for observations in observations.chunks(100) {
            let store = self.store();
            let state = store.ensure_collection(&self.workspace).unwrap();
            let repository = IssueSourceRepository::from_reference(&observations[0].issue);
            let request = IssueDiscoveryRequest {
                target: IssueDiscoveryTarget::Repository(repository.clone()),
                selection: IssueDiscoverySelection::AllOpen,
                cursor: state.discovery_cursor.clone(),
                page_size: 100,
                disclosure: IssueSourceDisclosure {
                    policy: IssueDisclosurePolicy::builtin(),
                    policy_version: policy_version.to_string(),
                },
            };
            let page = IssueDiscoveryPage {
                repository,
                observations: observations.to_vec(),
                next_cursor: None,
                scanned_count: u16::try_from(observations.len()).unwrap(),
            };
            store
                .apply_discovery_page(&self.workspace, state.fence(), &request, &page)
                .unwrap();
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn complete<T>(items: Vec<T>) -> IssueMetadataList<T> {
    IssueMetadataList {
        items,
        completeness: IssueMetadataCompleteness::Complete,
    }
}

fn disclosure_version() -> String {
    issue_policy_version(&IssuePolicy::builtin())
}

struct ValidationPolicy;

impl IssuePolicySourcePort for ValidationPolicy {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        Ok(None)
    }
    fn load_project_policy_toml(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
}

struct MutableAutomationPolicy {
    text: Mutex<String>,
    unavailable: AtomicBool,
}

impl MutableAutomationPolicy {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            text: Mutex::new(AUTO_POLICY.to_string()),
            unavailable: AtomicBool::new(false),
        })
    }
}

impl IssueAutomationPolicySourcePort for MutableAutomationPolicy {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        if self.unavailable.load(Ordering::Acquire) {
            bail!("UNSHARED_POLICY_SOURCE_ERROR")
        }
        Ok(Some(self.text.lock().unwrap().clone()))
    }
    fn load_project_policy_toml(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
}

fn run(service: &IssueTriageService) -> (IssueTriageRunOutcome, Vec<IssueTriagedFact>) {
    let facts = Mutex::new(Vec::new());
    let outcome = service
        .triage_all(&IssueCollectionCancellation::new(), &|committed| {
            facts.lock().unwrap().extend(committed);
        })
        .unwrap();
    (outcome, facts.into_inner().unwrap())
}

fn page(service: &IssueTriageService) -> IssueCollectionPage {
    service.issues(&IssueCollectionQuery::default()).unwrap()
}

fn shared(value: &str) -> IssueDecisionSharedText {
    IssueDecisionSharedText::new_shared(value).unwrap()
}

fn definition(triage: &IssueTriageService, fixture: &Fixture) -> IssueDecisionDefinition {
    let item = page(triage).items.pop().unwrap();
    let assessment = item.triage.unwrap().assessment;
    IssueDecisionDefinition {
        subject: IssueDecisionSubject::Issue(fixture.reference("I_001")),
        gate: IssueAutomationGate::Start,
        kind: IssueDecisionKind::StartApproval,
        question: shared("시작 결정을 기록할까요?"),
        options: vec![
            IssueDecisionOption {
                id: "accept".into(),
                label: shared("수락"),
                effect: shared("시작 결정을 기록한다."),
            },
            IssueDecisionOption {
                id: "reject".into(),
                label: shared("거절"),
                effect: shared("거절 결정을 기록한다."),
            },
        ],
        recommended: Some("accept".into()),
        basis: IssueDecisionBasis::from_assessment(&assessment, IssueAutomationGate::Start, None)
            .unwrap(),
        on_timeout: IssueDecisionTimeout::Wait,
        due_at: None,
        asked_by: "triage".into(),
        evidence: vec!["record:triage".into()],
    }
}

fn setup(
    fixture: &Fixture,
    source: Arc<MutableAutomationPolicy>,
) -> (IssueDecisionService, IssueDecisionCommit) {
    fixture.seed(vec![fixture.observation("I_001")], &disclosure_version());
    let triage = Arc::new(fixture.service(source));
    run(&triage);
    let definition = definition(&triage, fixture);
    let service = IssueDecisionService::new(
        fixture.workspace.clone(),
        fixture.store(),
        triage,
        IssuePolicyService::new(Arc::new(ValidationPolicy)),
    );
    let commit = service
        .ensure_request(IssueDecisionId::new("decision-001").unwrap(), definition)
        .unwrap();
    (service, commit)
}

fn caller() -> StoredCommandContext {
    StoredCommandContext {
        caller: Some("Desktop".into()),
        channel: "gateway".into(),
        at: "2026-11-20T20:00:00Z".into(),
    }
}

#[test]
fn body_changes_before_retriage_close_as_superseded_without_applying_the_answer() {
    let fixture = Fixture::new();
    let (service, initial) = setup(&fixture, MutableAutomationPolicy::new());
    let before = fixture
        .store()
        .read_triage_batch(&fixture.workspace, None, 100)
        .unwrap()
        .items[0]
        .triage
        .clone();
    let mut observation = fixture.observation("I_001");
    if let IssueSourceContent::Shared(snapshot) = &mut observation.content {
        snapshot.shared_copy.body.push_str("새 완료 기준\n");
        snapshot.basis.field_hash =
            IssueFieldHash::of_fields(&snapshot.shared_copy.title, &snapshot.shared_copy.body);
    }
    fixture.seed(vec![observation], &disclosure_version());
    let result = service
        .answer(
            initial.record.request.id().as_str(),
            "accept",
            None,
            &caller(),
        )
        .unwrap();
    assert_eq!(result.outcome, IssueDecisionOutcome::Superseded);
    assert_eq!(
        result.commit.record.request.state(),
        IssueDecisionState::Superseded
    );
    assert!(result.commit.record.request.answer().is_none());
    let current = fixture
        .store()
        .read_triage_batch(&fixture.workspace, None, 100)
        .unwrap()
        .items
        .remove(0);
    assert_eq!(
        current.triage.unwrap().assessment,
        before.unwrap().assessment
    );
    assert!(current.open_decisions.is_empty());
}

#[test]
fn metadata_only_changes_invalidate_old_answers_even_when_the_body_hash_matches() {
    for change in ["label", "assignee", "pull_request"] {
        let fixture = Fixture::new();
        let (service, initial) = setup(&fixture, MutableAutomationPolicy::new());
        let mut observation = fixture.observation("I_001");
        match change {
            "label" => observation.labels.items.push(IssueSourceLabel {
                id: "L_changed".into(),
                name: "new-metadata".into(),
            }),
            "assignee" => observation.assignees.items.push(IssueSourceActor {
                kind: IssueSourceActorKind::UserAccount,
                id: "U_other".into(),
                login: "other".into(),
            }),
            "pull_request" => observation.linked_pull_request = IssuePrPresence::Present,
            _ => unreachable!(),
        }
        fixture.seed(vec![observation], &disclosure_version());
        let result = service
            .answer(
                initial.record.request.id().as_str(),
                "accept",
                None,
                &caller(),
            )
            .unwrap();
        assert_eq!(result.outcome, IssueDecisionOutcome::Superseded, "{change}");
        assert!(result.commit.record.request.answer().is_none(), "{change}");
    }
}

#[test]
fn automation_policy_changes_invalidate_a_pending_answer_without_retriage() {
    let fixture = Fixture::new();
    let source = MutableAutomationPolicy::new();
    let (service, initial) = setup(&fixture, source.clone());
    *source.text.lock().unwrap() =
        AUTO_POLICY.replace("autonomy_cap = \"auto\"", "autonomy_cap = \"gated\"");
    let result = service
        .answer(
            initial.record.request.id().as_str(),
            "accept",
            None,
            &caller(),
        )
        .unwrap();
    assert_eq!(result.outcome, IssueDecisionOutcome::Superseded);
}

struct MutableValidationPolicy(Mutex<Option<String>>);
impl IssuePolicySourcePort for MutableValidationPolicy {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn load_project_policy_toml(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
}

#[test]
fn requirement_or_disclosure_policy_changes_invalidate_pending_answers() {
    for update in [
        "[convention]\nrequired_sections = [\"새 필수 칸\"]\n",
        "[ai_handoff.spans]\npath = \"remove\"\n",
    ] {
        let fixture = Fixture::new();
        fixture.seed(vec![fixture.observation("I_001")], &disclosure_version());
        let validation = Arc::new(MutableValidationPolicy(Mutex::new(None)));
        let triage = Arc::new(IssueTriageService::new(
            fixture.workspace.clone(),
            fixture.store(),
            IssueAutomationPolicyService::new(MutableAutomationPolicy::new()),
            IssuePolicyService::new(validation.clone()),
        ));
        run(&triage);
        let definition = definition(&triage, &fixture);
        let service = IssueDecisionService::new(
            fixture.workspace.clone(),
            fixture.store(),
            triage,
            IssuePolicyService::new(validation.clone()),
        );
        let initial = service
            .ensure_request(IssueDecisionId::new("decision-policy").unwrap(), definition)
            .unwrap();
        *validation.0.lock().unwrap() = Some(update.into());
        let result = service
            .answer(
                initial.record.request.id().as_str(),
                "accept",
                None,
                &caller(),
            )
            .unwrap();
        assert_eq!(result.outcome, IssueDecisionOutcome::Superseded, "{update}");
        assert!(result.commit.record.request.answer().is_none());
    }
}

#[test]
fn policy_read_failure_preserves_an_open_request_and_uses_a_safe_error() {
    let fixture = Fixture::new();
    let source = MutableAutomationPolicy::new();
    let (service, initial) = setup(&fixture, source.clone());
    source.unavailable.store(true, Ordering::Release);
    let error = service
        .answer(
            initial.record.request.id().as_str(),
            "accept",
            None,
            &caller(),
        )
        .unwrap_err();
    assert!(!format!("{error:?}").contains("UNSHARED_POLICY_SOURCE_ERROR"));
    assert_eq!(
        fixture
            .store()
            .get_issue_decision(&fixture.workspace, initial.record.request.id())
            .unwrap(),
        Some(initial.record)
    );
}

#[test]
fn a_terminal_request_returns_the_first_answer_without_policy_or_second_note_validation() {
    let fixture = Fixture::new();
    let source = MutableAutomationPolicy::new();
    let (service, initial) = setup(&fixture, source.clone());
    let first = service
        .answer(
            initial.record.request.id().as_str(),
            "accept",
            Some("검토함"),
            &caller(),
        )
        .unwrap();
    source.unavailable.store(true, Ordering::Release);
    let second = service
        .answer(
            initial.record.request.id().as_str(),
            "not-an-option",
            Some(&"x".repeat(100_000)),
            &caller(),
        )
        .unwrap();
    assert_eq!(second.outcome, IssueDecisionOutcome::AlreadyResolved);
    assert!(!second.commit.changed);
    assert_eq!(second.commit.record, first.commit.record);
    assert_eq!(
        second.commit.catalog_revision,
        first.commit.catalog_revision
    );
    let answer = first.commit.record.request.answer().unwrap();
    assert_eq!(answer.by, IssueDecisionAnswerBy::Agent);
    assert_eq!(answer.caller.as_deref(), Some("Desktop"));
    assert!(answer.confirmation.is_none());
}

#[test]
fn shared_question_and_note_remove_secret_canaries_before_storage_query_or_facts() {
    const CANARY: &str = "ghp_SYNTHETIC_NOT_REAL_SECRET_000000000";
    let fixture = Fixture::new();
    let source = MutableAutomationPolicy::new();
    fixture.seed(vec![fixture.observation("I_001")], &disclosure_version());
    let triage = Arc::new(fixture.service(source));
    run(&triage);
    let mut definition = definition(&triage, &fixture);
    definition.question = shared(&format!("이 토큰 {CANARY}와 관련한 결정을 기록할까요?"));
    definition.options[0].effect = shared(&format!("token={CANARY} 효과"));
    let service = IssueDecisionService::new(
        fixture.workspace.clone(),
        fixture.store(),
        triage.clone(),
        IssuePolicyService::new(Arc::new(ValidationPolicy)),
    );
    let initial = service
        .ensure_request(IssueDecisionId::new("decision-secret").unwrap(), definition)
        .unwrap();
    let detail = triage.issue(&fixture.reference("I_001")).unwrap();
    let request = &detail.item.unwrap().open_decisions[0].request;
    let public = serde_json::to_string(request).unwrap();
    assert!(!public.contains(CANARY));
    assert!(public.contains("[removed]"));
    assert!(
        !serde_json::to_string(&initial.record.request.fact())
            .unwrap()
            .contains("토큰")
    );
    let result = service
        .answer(
            initial.record.request.id().as_str(),
            "accept",
            Some(&format!("검토 token={CANARY}")),
            &caller(),
        )
        .unwrap();
    let stored = fixture
        .store()
        .get_issue_decision(&fixture.workspace, initial.record.request.id())
        .unwrap()
        .unwrap();
    assert!(
        !serde_json::to_string(&stored.request)
            .unwrap()
            .contains(CANARY)
    );
    assert!(
        !serde_json::to_string(&result.commit.record.request.fact())
            .unwrap()
            .contains("검토")
    );
    assert!(
        stored
            .request
            .answer()
            .unwrap()
            .note
            .as_ref()
            .unwrap()
            .as_str()
            .contains("[removed]")
    );
}

#[test]
fn collection_off_does_not_block_an_existing_decision_answer() {
    let fixture = Fixture::new();
    let (service, initial) = setup(&fixture, MutableAutomationPolicy::new());
    let store = fixture.store();
    let control = store.get_collection(&fixture.workspace).unwrap().unwrap();
    store
        .set_collection_mode(
            &fixture.workspace,
            Some(control.epoch),
            IssueCollectionMode::Off,
        )
        .unwrap();
    let result = service
        .answer(
            initial.record.request.id().as_str(),
            "accept",
            None,
            &caller(),
        )
        .unwrap();
    assert_eq!(result.outcome, IssueDecisionOutcome::Answered);
    assert!(result.commit.changed);
}
