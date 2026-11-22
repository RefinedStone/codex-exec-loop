//! 실제 이슈 SQLite 장부와 가짜 실행/전달 포트로 개입의 영수증·재시작 경계를 검증한다.
use super::*;
use crate::adapter::outbound::db::SqliteIssueAutomationStoreAdapter;
use crate::application::port::outbound::issue_automation_store_port::IssueCollectionApplyResult;
use crate::application::port::outbound::issue_policy_source_port::IssuePolicySourcePort;
use crate::application::port::outbound::issue_source_port::{
    IssueDiscoveryPage, IssueDiscoveryRequest, IssueDiscoveryTarget,
};
use crate::application::port::outbound::issue_work_execution_port::{
    IssueWorkExecutionSnapshot, IssueWorkPermitRequest,
};
use crate::application::service::issue_policy::{IssuePolicyService, issue_policy_version};
use crate::domain::issue_automation::collection::IssueCollectionMode;
use crate::domain::issue_automation::coordinator::IssueWorkOrder;
use crate::domain::issue_automation::source::*;
use crate::domain::issue_automation::work_control::IssueWorkControl;
use crate::domain::issue_automation::{IssueAutomationSnapshot, TrackedIssue};
use crate::domain::issue_policy::{IssueDisclosurePolicy, IssuePolicy};
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueFieldHash, IssueProjectScope, IssueProvider,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

#[derive(Default)]
struct ExecutionState {
    controls: BTreeMap<String, IssueWorkControl>,
    applied: BTreeMap<String, (String, IssueWorkControlAction, IssueWorkControl)>,
    instructions: Vec<(String, IssueWorkInstruction)>,
}

struct AdmissionGate {
    entered: Sender<()>,
    resume: Receiver<()>,
}

#[derive(Default)]
struct FakeExecution {
    state: Mutex<ExecutionState>,
    active: AtomicBool,
    fail_control: AtomicBool,
    failed_control_work: Mutex<Option<String>>,
    next_admission_gate: Mutex<Option<AdmissionGate>>,
    control_calls: AtomicUsize,
    instruction_calls: AtomicUsize,
    automatic_calls: AtomicUsize,
}

impl IssueWorkExecutionPort for FakeExecution {
    fn control(&self, _: &str, work: &str) -> Result<IssueWorkControl> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .controls
            .get(work)
            .cloned()
            .unwrap_or_default())
    }
    fn apply_control(
        &self,
        _: &str,
        work: &str,
        id: &str,
        action: IssueWorkControlAction,
    ) -> Result<IssueWorkControl> {
        self.control_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_control.swap(false, Ordering::SeqCst)
            || self.failed_control_work.lock().unwrap().as_deref() == Some(work)
        {
            bail!("injected control store failure");
        }
        let mut state = self.state.lock().unwrap();
        if let Some((saved_work, saved_action, saved_result)) = state.applied.get(id) {
            anyhow::ensure!(
                saved_work == work && *saved_action == action,
                "fake command identity changed"
            );
            return Ok(saved_result.clone());
        }
        let next = state
            .controls
            .get(work)
            .cloned()
            .unwrap_or_default()
            .apply(action)?;
        state.controls.insert(work.to_owned(), next.clone());
        state
            .applied
            .insert(id.to_owned(), (work.to_owned(), action, next.clone()));
        Ok(next)
    }
    fn queue_instruction(
        &self,
        _: &str,
        work: &str,
        instruction: &IssueWorkInstruction,
    ) -> Result<IssueWorkInstructionAdmission> {
        self.instruction_calls.fetch_add(1, Ordering::SeqCst);
        let gate = self.next_admission_gate.lock().unwrap().take();
        if let Some(gate) = gate {
            gate.entered.send(()).unwrap();
            gate.resume
                .recv_timeout(Duration::from_secs(10))
                .expect("the concurrent admission test must release its first caller");
        }
        if self.active.load(Ordering::SeqCst) {
            return Ok(IssueWorkInstructionAdmission::Active);
        }
        let mut state = self.state.lock().unwrap();
        if state
            .instructions
            .iter()
            .any(|(saved_work, saved)| saved_work == work && saved == instruction)
        {
            return Ok(IssueWorkInstructionAdmission::Queued { changed: false });
        }
        state
            .instructions
            .push((work.to_owned(), instruction.clone()));
        Ok(IssueWorkInstructionAdmission::Queued { changed: true })
    }
    fn has_active_lease(&self, _: &str, _: &str) -> Result<bool> {
        Ok(self.active.load(Ordering::SeqCst))
    }
    fn snapshot(&self, _: &str, _: &str) -> Result<IssueWorkExecutionSnapshot> {
        Ok(IssueWorkExecutionSnapshot {
            task_ids: vec!["task-1".to_owned()],
            ..Default::default()
        })
    }
    fn submit(&self, _: &str, _: &IssueWorkOrder) -> Result<Vec<String>> {
        self.automatic_calls.fetch_add(1, Ordering::SeqCst);
        bail!("an intervention must not start an automatic task")
    }
    fn grant(&self, _: &str, _: &IssueWorkPermitRequest) -> Result<()> {
        self.automatic_calls.fetch_add(1, Ordering::SeqCst);
        bail!("an intervention must not restore a work permit")
    }
    fn revoke(&self, _: &str, _: &str) -> Result<()> {
        Ok(())
    }
}

struct FakeSteering {
    store: Arc<SqliteIssueAutomationStoreAdapter>,
    issue: IssueReference,
    attempts: Mutex<Vec<IssueWorkSteerRequest>>,
    outcome: Mutex<Option<IssueSteerDelivery>>,
}

impl IssueWorkSteeringPort for FakeSteering {
    fn steer(&self, request: &IssueWorkSteerRequest) -> Result<IssueSteerDelivery> {
        // 실제 송신 포트에 도달하기 전에 Unknown 영수증이 SQLite에 있어야 한다.
        let stored = self.store.begin_issue_intervention(
            &request.workspace_dir,
            &IssueInterventionRequest {
                issue: self.issue.clone(),
                command_id: request.command_id.clone(),
                action: IssueInterventionAction::Steer {
                    message: request.message.clone(),
                },
            },
        )?;
        assert_eq!(
            stored.receipt.unwrap().delivery,
            Some(IssueSteerDelivery::Unknown)
        );
        self.attempts.lock().unwrap().push(request.clone());
        match *self.outcome.lock().unwrap() {
            Some(delivery) => Ok(delivery),
            None => bail!("injected acknowledgement loss"),
        }
    }
}

struct BuiltinPolicy;
impl IssuePolicySourcePort for BuiltinPolicy {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        Ok(None)
    }
    fn load_project_policy_toml(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
}

struct Fixture {
    root: PathBuf,
    workspace: String,
    issue: IssueReference,
    store: Arc<SqliteIssueAutomationStoreAdapter>,
    execution: Arc<FakeExecution>,
    steering: Arc<FakeSteering>,
}

impl Fixture {
    fn new() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-intervention-{}-{:016x}",
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
            repository_id: "R_intervention".to_owned(),
            issue_id: "I_intervention".to_owned(),
            project_scope: IssueProjectScope {
                workspace_dir: workspace.clone(),
            },
        };
        let execution = Arc::new(FakeExecution::default());
        let steering = Arc::new(FakeSteering {
            store: store.clone(),
            issue: issue.clone(),
            attempts: Mutex::new(Vec::new()),
            outcome: Mutex::new(Some(IssueSteerDelivery::Sent)),
        });
        let fixture = Self {
            root,
            workspace,
            issue,
            store,
            execution,
            steering,
        };
        fixture.seed(&fixture.issue);
        fixture
    }

    fn service(&self) -> IssueInterventionService {
        IssueInterventionService::new(
            self.workspace.clone(),
            self.store.clone(),
            self.execution.clone(),
            self.steering.clone(),
        )
        .with_policy(IssuePolicyService::new(Arc::new(BuiltinPolicy)))
    }

    fn request(&self, id: &str, action: IssueInterventionAction) -> IssueInterventionRequest {
        IssueInterventionRequest {
            command_id: id.to_owned(),
            issue: self.issue.clone(),
            action,
        }
    }

    fn work(&self) -> String {
        issue_work_order_id(&self.issue)
    }

    fn control(&self) -> IssueWorkControl {
        self.execution
            .control(&self.workspace, &self.work())
            .unwrap()
    }

    fn observation(&self, issue: &IssueReference) -> IssueSourceObservation {
        let shared_copy = IssueSharedCopy {
            title: "Shared intervention issue".to_owned(),
            body: "## 완료 기준\n- 개입 뒤 다른 작업은 계속 진행한다.\n".to_owned(),
        };
        IssueSourceObservation {
            issue: issue.clone(),
            web: None,
            remote_state: IssueRemoteState::Open,
            labels: IssueMetadataList {
                items: Vec::new(),
                completeness: IssueMetadataCompleteness::Complete,
            },
            assignees: IssueMetadataList {
                items: Vec::new(),
                completeness: IssueMetadataCompleteness::Complete,
            },
            author: None,
            label_actors: IssueMetadataList {
                items: Vec::new(),
                completeness: IssueMetadataCompleteness::Complete,
            },
            linked_pull_request: IssuePrPresence::Absent,
            creation_provenance: IssueCreationProvenance::Unattributed,
            content: IssueSourceContent::Shared(IssueAutomationSnapshot {
                basis: IssueComparisonBasis {
                    field_hash: IssueFieldHash::of_fields(&shared_copy.title, &shared_copy.body),
                    provider_updated_at: "2026-11-21T00:00:00Z".to_owned(),
                },
                shared_copy,
                policy_version: issue_policy_version(&IssuePolicy::builtin()),
            }),
        }
    }

    fn rediscover(&self, issue: &IssueReference) {
        let state = self.store.ensure_collection(&self.workspace).unwrap();
        let repository = IssueSourceRepository::from_reference(issue);
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
            repository,
            observations: vec![self.observation(issue)],
            next_cursor: None,
            scanned_count: 1,
        };
        assert!(matches!(
            self.store
                .apply_discovery_page(&self.workspace, state.fence(), &request, &page,)
                .unwrap(),
            IssueCollectionApplyResult::Applied(_)
        ));
    }

    fn seed(&self, issue: &IssueReference) {
        self.rediscover(issue);
        if self
            .store
            .get_issue(&self.workspace, issue)
            .unwrap()
            .is_none()
        {
            let projection = self
                .store
                .get_collection_projection(&self.workspace)
                .unwrap()
                .unwrap();
            let binding = projection.binding.unwrap();
            let IssueSourceContent::Shared(snapshot) = self.observation(issue).content else {
                unreachable!()
            };
            let tracked = TrackedIssue::discovered(issue.clone(), &binding, snapshot).unwrap();
            self.store.insert_issue(&self.workspace, &tracked).unwrap();
        }
    }

    fn stored(
        &self,
        request: &IssueInterventionRequest,
    ) -> crate::domain::issue_automation::intervention::StoredIssueIntervention {
        self.store
            .begin_issue_intervention(&self.workspace, request)
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn context() -> StoredCommandContext {
    StoredCommandContext {
        human_confirmation: None,
        caller: Some("intervention-test".to_owned()),
        channel: "kernel".to_owned(),
        at: "2026-11-21T00:00:00Z".to_owned(),
    }
}

#[test]
fn issue_intervention_cancel_without_confirm_has_no_durable_or_execution_side_effects() {
    let fixture = Fixture::new();
    let before = fixture
        .store
        .get_issue(&fixture.workspace, &fixture.issue)
        .unwrap()
        .unwrap();
    let collection = fixture
        .store
        .get_collection_projection(&fixture.workspace)
        .unwrap()
        .unwrap();
    let result = fixture.service().intervene(
        fixture.request(
            "cancel-1",
            IssueInterventionAction::Cancel { confirm: false },
        ),
        &context(),
    );
    assert!(result.unwrap_err().to_string().contains("requires confirm"));
    assert_eq!(
        fixture
            .store
            .get_issue(&fixture.workspace, &fixture.issue)
            .unwrap()
            .unwrap(),
        before
    );
    assert_eq!(
        fixture
            .store
            .get_collection_projection(&fixture.workspace)
            .unwrap()
            .unwrap(),
        collection
    );
    assert!(
        fixture
            .store
            .pending_issue_interventions(&fixture.workspace)
            .unwrap()
            .is_empty()
    );
    assert_eq!(fixture.execution.control_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture.execution.instruction_calls.load(Ordering::SeqCst),
        0
    );
    assert_eq!(fixture.execution.automatic_calls.load(Ordering::SeqCst), 0);
    assert!(fixture.steering.attempts.lock().unwrap().is_empty());
}

#[test]
fn issue_intervention_hold_release_fences_old_approval_without_changing_another_issue() {
    let fixture = Fixture::new();
    let mut other = fixture.issue.clone();
    other.issue_id = "I_unrelated".to_owned();
    fixture.seed(&other);
    let service = fixture.service();
    let held = service
        .intervene(
            fixture.request("hold-1", IssueInterventionAction::Hold),
            &context(),
        )
        .unwrap();
    assert_eq!(held.authorization_epoch, 1);
    assert!(held.changed);
    assert!(!fixture.control().allows_automation());
    assert!(
        fixture
            .store
            .get_issue(&fixture.workspace, &fixture.issue)
            .unwrap()
            .unwrap()
            .flags()
            .held
    );
    assert!(
        !fixture
            .store
            .get_issue(&fixture.workspace, &other)
            .unwrap()
            .unwrap()
            .flags()
            .held
    );
    assert_eq!(
        fixture
            .execution
            .control(&fixture.workspace, &issue_work_order_id(&other))
            .unwrap(),
        IssueWorkControl::default()
    );
    let released = service
        .intervene(
            fixture.request("release-1", IssueInterventionAction::Release),
            &context(),
        )
        .unwrap();
    assert_eq!(released.authorization_epoch, 2);
    assert!(released.changed);
    assert!(fixture.control().allows_automation());
    assert!(
        fixture.control().require_epoch(0).is_err(),
        "old approval is not restored"
    );
    assert_eq!(
        fixture
            .store
            .issue_authorization_epoch(&fixture.workspace, &fixture.issue)
            .unwrap(),
        2
    );
    assert!(
        !fixture
            .store
            .get_issue(&fixture.workspace, &fixture.issue)
            .unwrap()
            .unwrap()
            .flags()
            .held
    );
    assert_eq!(fixture.execution.automatic_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn issue_intervention_replay_after_reopening_keeps_receipt_and_rejects_command_id_reuse() {
    let fixture = Fixture::new();
    let request = fixture.request("hold-once", IssueInterventionAction::Hold);
    let first = fixture
        .service()
        .intervene(request.clone(), &context())
        .unwrap();
    let replay = fixture.service().intervene(request, &context()).unwrap();
    assert!(!replay.changed);
    assert_eq!(replay.authorization_epoch, first.authorization_epoch);
    assert_eq!(replay.command_id, first.command_id);
    assert_eq!(fixture.execution.control_calls.load(Ordering::SeqCst), 1);
    assert!(
        fixture
            .service()
            .intervene(
                fixture.request("hold-once", IssueInterventionAction::Release),
                &context(),
            )
            .unwrap_err()
            .to_string()
            .contains("reused")
    );
    assert_eq!(fixture.control().authorization_epoch, 1);
}

#[test]
fn issue_intervention_queued_guidance_is_appended_once_across_service_restart() {
    let fixture = Fixture::new();
    let request = fixture.request(
        "queued-steer",
        IssueInterventionAction::Steer {
            message: "실행 전 지시".to_owned(),
        },
    );
    let first = fixture
        .service()
        .intervene(request.clone(), &context())
        .unwrap();
    assert_eq!(first.delivery, Some(IssueSteerDelivery::Queued));
    assert!(first.changed);
    let replay = fixture.service().intervene(request, &context()).unwrap();
    assert_eq!(replay.delivery, Some(IssueSteerDelivery::Queued));
    assert!(!replay.changed);
    assert_eq!(
        fixture.execution.instruction_calls.load(Ordering::SeqCst),
        1
    );
    assert_eq!(
        fixture.execution.state.lock().unwrap().instructions.len(),
        1
    );
    assert!(fixture.steering.attempts.lock().unwrap().is_empty());
}

#[test]
fn issue_intervention_running_guidance_records_unknown_before_sending_and_sends_only_once() {
    let fixture = Fixture::new();
    fixture.execution.active.store(true, Ordering::SeqCst);
    let request = fixture.request(
        "live-steer",
        IssueInterventionAction::Steer {
            message: "실행 중 지시".to_owned(),
        },
    );
    let first = fixture
        .service()
        .intervene(request.clone(), &context())
        .unwrap();
    assert_eq!(first.delivery, Some(IssueSteerDelivery::Sent));
    assert_eq!(
        fixture.stored(&request).receipt.unwrap().delivery,
        Some(IssueSteerDelivery::Sent)
    );
    let replay = fixture.service().intervene(request, &context()).unwrap();
    assert_eq!(replay.delivery, Some(IssueSteerDelivery::Sent));
    assert!(!replay.changed);
    let attempts = fixture.steering.attempts.lock().unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].work_order_id, fixture.work());
    assert_eq!(attempts[0].task_ids, vec!["task-1"]);
    assert_eq!(attempts[0].message, "실행 중 지시");
    assert_eq!(
        fixture.execution.instruction_calls.load(Ordering::SeqCst),
        1
    );
}

#[test]
fn issue_intervention_unknown_delivery_is_durable_and_never_retried() {
    for outcome in [None, Some(IssueSteerDelivery::Unknown)] {
        let fixture = Fixture::new();
        fixture.execution.active.store(true, Ordering::SeqCst);
        *fixture.steering.outcome.lock().unwrap() = outcome;
        let request = fixture.request(
            "unknown-steer",
            IssueInterventionAction::Steer {
                message: "응답 유실 지시".to_owned(),
            },
        );
        let first = fixture
            .service()
            .intervene(request.clone(), &context())
            .unwrap();
        assert_eq!(first.delivery, Some(IssueSteerDelivery::Unknown));
        let replay = fixture.service().intervene(request, &context()).unwrap();
        assert_eq!(replay.delivery, Some(IssueSteerDelivery::Unknown));
        assert!(!replay.changed);
        assert_eq!(fixture.steering.attempts.lock().unwrap().len(), 1);
        assert_eq!(
            fixture.execution.instruction_calls.load(Ordering::SeqCst),
            1
        );
    }
}

#[test]
fn issue_intervention_pending_control_recovers_once_and_never_replays_pending_steering() {
    let fixture = Fixture::new();
    let hold = fixture.request("pending-hold", IssueInterventionAction::Hold);
    fixture
        .store
        .begin_issue_intervention(&fixture.workspace, &hold)
        .unwrap();
    // planning commit 뒤 이슈 영수증 전에 프로세스가 사라진 경우다.
    fixture
        .execution
        .apply_control(
            &fixture.workspace,
            &fixture.work(),
            &hold.command_id,
            IssueWorkControlAction::Hold,
        )
        .unwrap();
    let mut other = fixture.issue.clone();
    other.issue_id = "I_pending_steer".to_owned();
    fixture.seed(&other);
    let steer = IssueInterventionRequest {
        command_id: "pending-steer".to_owned(),
        issue: other,
        action: IssueInterventionAction::Steer {
            message: "자동 재발송 금지".to_owned(),
        },
    };
    fixture
        .store
        .begin_issue_intervention(&fixture.workspace, &steer)
        .unwrap();
    recover_pending_controls(
        &fixture.workspace,
        fixture.store.as_ref(),
        fixture.execution.as_ref(),
    )
    .unwrap();
    let saved = fixture.stored(&hold).receipt.unwrap();
    assert_eq!(saved.authorization_epoch, 1);
    assert!(fixture.control().held);
    let pending = fixture
        .store
        .pending_issue_interventions(&fixture.workspace)
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].request.command_id, "pending-steer");
    recover_pending_controls(
        &fixture.workspace,
        fixture.store.as_ref(),
        fixture.execution.as_ref(),
    )
    .unwrap();
    assert_eq!(fixture.execution.control_calls.load(Ordering::SeqCst), 2);
    assert!(fixture.steering.attempts.lock().unwrap().is_empty());
}

#[test]
fn issue_intervention_failed_control_recovers_before_a_later_release() {
    let fixture = Fixture::new();
    fixture.execution.fail_control.store(true, Ordering::SeqCst);
    assert!(
        fixture
            .service()
            .intervene(
                fixture.request("failed-hold", IssueInterventionAction::Hold),
                &context()
            )
            .is_err()
    );
    assert_eq!(
        fixture
            .store
            .pending_issue_interventions(&fixture.workspace)
            .unwrap()
            .len(),
        1
    );
    assert!(
        !fixture
            .store
            .get_issue(&fixture.workspace, &fixture.issue)
            .unwrap()
            .unwrap()
            .flags()
            .held
    );
    let release = fixture
        .service()
        .intervene(
            fixture.request("after-failure-release", IssueInterventionAction::Release),
            &context(),
        )
        .unwrap();
    assert_eq!(release.authorization_epoch, 2);
    assert!(!fixture.control().held);
    assert!(
        fixture
            .store
            .pending_issue_interventions(&fixture.workspace)
            .unwrap()
            .is_empty()
    );
    assert_eq!(fixture.execution.automatic_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn issue_intervention_untrack_survives_rediscovery_until_explicit_tracking() {
    let fixture = Fixture::new();
    fixture
        .service()
        .intervene(
            fixture.request("untrack", IssueInterventionAction::Untrack),
            &context(),
        )
        .unwrap();
    fixture.rediscover(&fixture.issue);
    let batch = fixture
        .store
        .read_triage_batch(&fixture.workspace, None, 100)
        .unwrap();
    assert!(
        batch
            .items
            .iter()
            .filter(|item| item.issue == fixture.issue)
            .all(|item| item.tracked.is_none())
    );
    assert!(!fixture.control().tracked);
    assert!(!fixture.control().allows_automation());
    assert!(
        fixture
            .store
            .get_issue(&fixture.workspace, &fixture.issue)
            .unwrap()
            .is_some(),
        "history is preserved"
    );
    let track = fixture
        .service()
        .intervene(
            fixture.request("track", IssueInterventionAction::Track),
            &context(),
        )
        .unwrap();
    assert_eq!(track.authorization_epoch, 2);
    let batch = fixture
        .store
        .read_triage_batch(&fixture.workspace, None, 100)
        .unwrap();
    assert!(
        batch
            .items
            .iter()
            .any(|item| item.issue == fixture.issue && item.tracked.is_some())
    );
    assert_eq!(fixture.execution.automatic_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn issue_intervention_takeover_before_execution_returns_only_current_shared_handoff() {
    let fixture = Fixture::new();
    let result = fixture
        .service()
        .intervene(
            fixture.request("take-over", IssueInterventionAction::TakeOver),
            &context(),
        )
        .unwrap();
    assert!(fixture.control().taken_over);
    assert!(!fixture.control().allows_automation());
    let handoff = result.handoff.unwrap();
    assert_eq!(handoff.title, "Shared intervention issue");
    assert!(handoff.body.contains("개입 뒤 다른 작업은 계속 진행한다."));
    assert!(fixture.steering.attempts.lock().unwrap().is_empty());
    assert_eq!(fixture.execution.automatic_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn issue_intervention_independent_services_claim_the_same_live_guidance_only_once() {
    let fixture = Fixture::new();
    fixture.execution.active.store(true, Ordering::SeqCst);
    let request = fixture.request(
        "concurrent-live-steer",
        IssueInterventionAction::Steer {
            message: "독립 서비스에서도 한 번만 전달한다".to_owned(),
        },
    );
    let first_service = fixture.service();
    // 서비스별 Mutex뿐 아니라 SQLite claim이 중복을 막는지 별도 adapter로 확인한다.
    let runtime = fixture.root.join("runtime");
    let second_store = Arc::new(SqliteIssueAutomationStoreAdapter::new(move |_| {
        Ok(runtime.clone())
    }));
    let second_service = IssueInterventionService::new(
        fixture.workspace.clone(),
        second_store,
        fixture.execution.clone(),
        fixture.steering.clone(),
    );
    let (entered, admission_started) = std::sync::mpsc::channel();
    let (resume, admission_resume) = std::sync::mpsc::channel();
    *fixture.execution.next_admission_gate.lock().unwrap() = Some(AdmissionGate {
        entered,
        resume: admission_resume,
    });
    let first_request = request.clone();
    let (first, replay) = std::thread::scope(|scope| {
        let first = scope.spawn(move || first_service.intervene(first_request, &context()));
        admission_started
            .recv_timeout(Duration::from_secs(10))
            .expect("the first caller must reach instruction admission");
        // 최초 호출이 admission에서 멈춘 동안 두 번째 호출이 영속 Unknown을 읽는다.
        let replay = second_service.intervene(request.clone(), &context());
        resume.send(()).unwrap();
        (first.join().unwrap(), replay)
    });
    let first = first.unwrap();
    let replay = replay.unwrap();
    assert_eq!(first.delivery, Some(IssueSteerDelivery::Sent));
    assert!(first.changed);
    assert_eq!(replay.delivery, Some(IssueSteerDelivery::Unknown));
    assert!(!replay.changed);
    assert_eq!(
        fixture.execution.instruction_calls.load(Ordering::SeqCst),
        1
    );
    assert_eq!(fixture.steering.attempts.lock().unwrap().len(), 1);
    assert_eq!(
        fixture.stored(&request).receipt.unwrap().delivery,
        Some(IssueSteerDelivery::Sent)
    );
    assert!(
        fixture
            .store
            .pending_issue_interventions(&fixture.workspace)
            .unwrap()
            .is_empty()
    );
    assert_eq!(fixture.execution.automatic_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn issue_intervention_a_failed_pending_control_does_not_block_an_unrelated_issue() {
    let fixture = Fixture::new();
    let pending = fixture.request("a-pending-hold", IssueInterventionAction::Hold);
    assert!(fixture.stored(&pending).receipt.is_none());
    *fixture.execution.failed_control_work.lock().unwrap() = Some(fixture.work());
    let mut other = fixture.issue.clone();
    other.issue_id = "I_recovery_independent".to_owned();
    fixture.seed(&other);
    let other_request = IssueInterventionRequest {
        command_id: "b-independent-hold".to_owned(),
        issue: other.clone(),
        action: IssueInterventionAction::Hold,
    };
    let result = fixture
        .service()
        .intervene(other_request, &context())
        .unwrap();
    assert_eq!(result.issue, other);
    assert_eq!(result.authorization_epoch, 1);
    assert!(
        fixture
            .execution
            .control(&fixture.workspace, &issue_work_order_id(&other))
            .unwrap()
            .held
    );
    assert!(
        fixture
            .store
            .get_issue(&fixture.workspace, &other)
            .unwrap()
            .unwrap()
            .flags()
            .held
    );
    assert_eq!(fixture.control(), IssueWorkControl::default());
    assert!(fixture.stored(&pending).receipt.is_none());
    let remaining = fixture
        .store
        .pending_issue_interventions(&fixture.workspace)
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].request, pending);
    // 같은 이슈의 실패한 선행 제어는 새 명령으로 우회하지 못한다.
    assert!(
        fixture
            .service()
            .intervene(
                fixture.request("a-cannot-bypass-pending", IssueInterventionAction::Release),
                &context(),
            )
            .is_err()
    );
    assert_eq!(
        fixture.execution.instruction_calls.load(Ordering::SeqCst),
        0
    );
    assert!(fixture.steering.attempts.lock().unwrap().is_empty());
    assert_eq!(fixture.execution.automatic_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn issue_intervention_startup_recovery_fences_off_mode_before_runtime_or_returns_an_error() {
    for planning_fails in [false, true] {
        let fixture = Fixture::new();
        let hold = fixture.request("startup-pending-hold", IssueInterventionAction::Hold);
        assert!(fixture.stored(&hold).receipt.is_none());
        let mut other = fixture.issue.clone();
        other.issue_id = "I_startup_pending_steer".to_owned();
        fixture.seed(&other);
        let steer = IssueInterventionRequest {
            command_id: "startup-pending-steer".to_owned(),
            issue: other,
            action: IssueInterventionAction::Steer {
                message: "시작 복구에서 외부 송신하지 않는다".to_owned(),
            },
        };
        // 기존 버전의 pending steer가 있어도 시작 복구에서 재전송하지 않는다.
        assert!(fixture.stored(&steer).receipt.is_none());
        let collection = fixture
            .store
            .get_collection(&fixture.workspace)
            .unwrap()
            .unwrap();
        fixture
            .store
            .set_collection_mode(
                &fixture.workspace,
                Some(collection.epoch),
                IssueCollectionMode::Off,
            )
            .unwrap();
        if planning_fails {
            *fixture.execution.failed_control_work.lock().unwrap() = Some(fixture.work());
        }
        let result = fixture.service().recover_before_runtime();
        assert_eq!(result.is_err(), planning_fails);
        assert_eq!(
            fixture
                .store
                .get_collection(&fixture.workspace)
                .unwrap()
                .unwrap()
                .config
                .mode,
            IssueCollectionMode::Off
        );
        if planning_fails {
            assert_eq!(fixture.control(), IssueWorkControl::default());
            assert!(fixture.stored(&hold).receipt.is_none());
            assert!(
                !fixture
                    .store
                    .get_issue(&fixture.workspace, &fixture.issue)
                    .unwrap()
                    .unwrap()
                    .flags()
                    .held
            );
        } else {
            assert!(fixture.control().held);
            assert!(!fixture.control().allows_automation());
            assert!(fixture.control().require_epoch(0).is_err());
            assert_eq!(fixture.control().authorization_epoch, 1);
            assert_eq!(
                fixture.stored(&hold).receipt.unwrap().authorization_epoch,
                1
            );
            assert!(
                fixture
                    .store
                    .get_issue(&fixture.workspace, &fixture.issue)
                    .unwrap()
                    .unwrap()
                    .flags()
                    .held
            );
            fixture.service().recover_before_runtime().unwrap();
            assert_eq!(fixture.control().authorization_epoch, 1);
        }
        assert!(fixture.stored(&steer).receipt.is_none());
        assert_eq!(
            fixture.execution.instruction_calls.load(Ordering::SeqCst),
            0
        );
        assert!(fixture.steering.attempts.lock().unwrap().is_empty());
        assert_eq!(fixture.execution.automatic_calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn issue_intervention_rejected_guidance_after_takeover_does_not_block_cancellation() {
    let fixture = Fixture::new();
    fixture
        .service()
        .intervene(
            fixture.request("takeover-before-steer", IssueInterventionAction::TakeOver),
            &context(),
        )
        .unwrap();
    let rejected = fixture.request(
        "steer-after-takeover",
        IssueInterventionAction::Steer {
            message: "자동 소유권이 끝난 뒤에는 보내지 않는다".to_owned(),
        },
    );
    assert!(
        fixture
            .service()
            .intervene(rejected.clone(), &context())
            .is_err()
    );
    assert!(
        fixture
            .store
            .get_issue_intervention(&fixture.workspace, &rejected.command_id,)
            .unwrap()
            .is_none()
    );
    assert!(
        fixture
            .store
            .pending_issue_interventions(&fixture.workspace)
            .unwrap()
            .is_empty()
    );
    let cancelled = fixture
        .service()
        .intervene(
            fixture.request(
                "cancel-after-rejected-steer",
                IssueInterventionAction::Cancel { confirm: true },
            ),
            &context(),
        )
        .unwrap();
    assert_eq!(cancelled.action, "cancel_issue_work");
    assert!(fixture.control().cancelled);
    assert_eq!(
        fixture.execution.instruction_calls.load(Ordering::SeqCst),
        0
    );
    assert!(fixture.steering.attempts.lock().unwrap().is_empty());
    assert_eq!(fixture.execution.automatic_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn issue_intervention_reserves_internal_autonomy_command_ids() {
    let fixture = Fixture::new();
    for action in [
        IssueInterventionAction::Retriage,
        IssueInterventionAction::Hold,
    ] {
        let request = fixture.request("autonomy-reserved-fixture", action);
        assert!(
            fixture
                .service()
                .intervene(request.clone(), &context())
                .is_err()
        );
        assert!(
            fixture
                .store
                .get_issue_intervention(&fixture.workspace, &request.command_id)
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(fixture.execution.control_calls.load(Ordering::SeqCst), 0);
    assert!(
        fixture
            .store
            .pending_issue_interventions(&fixture.workspace)
            .unwrap()
            .is_empty()
    );
}
