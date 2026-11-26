//! 승인 → 영속 이슈 checkpoint → 실제 SQLite task 접수. provider/모델은 호출하지 않는다.
use super::*;
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::application::port::inbound::stored_command_port::{
    StoredCommandContext,
    issue_confirmation::{IssueConfirmationError, VerifiedHumanConfirmation},
};
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningDirectionAuthorityCommit, PlanningTaskAuthorityCommit, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_work_order_repository_port::PlanningWorkOrderRepositoryPort;
use crate::application::service::planning::task_mutation::{
    PlanningTaskMutationCommand, PlanningTaskMutationRequest, PlanningTaskMutationService,
    PlanningTaskMutationSource, PlanningTaskUpdateInput,
};
use crate::domain::issue_automation::criteria::{
    IssueWorkCriterion, IssueWorkCriterionChannel, IssueWorkVerificationRecipe,
};
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, OriginSessionKind,
    PriorityQueueService, QueueIdleConfig, QueueIdlePolicy, TASK_AUTHORITY_FORMAT_VERSION,
    TaskActor, TaskAuthorityDocument, TaskMutationProvenance,
};

fn criteria() -> Vec<IssueWorkCriterion> {
    vec![IssueWorkCriterion {
        id: "issue-criterion-1".into(),
        scenario: "기능이 요구한 결과를 반환한다.".into(),
        channel: IssueWorkCriterionChannel::Command,
        essential: true,
        verification_recipe: Some(IssueWorkVerificationRecipe::GitDiffCheckV1),
    }]
}

fn confirmation(h: &Harness, criteria: &[IssueWorkCriterion]) -> StoredCommandContext {
    let preview = h
        .decisions
        .prepare_confirmation_with_criteria(
            h.decision().request.id().as_str(),
            "start",
            Some(criteria),
        )
        .unwrap();
    assert!(preview.display.detail.contains(&criteria[0].scenario));
    assert!(preview.display.detail.contains("명령"));
    StoredCommandContext {
        human_confirmation: Some(
            VerifiedHumanConfirmation::from_verified(
                preview.target,
                format!("desktop-confirmation:{}", "c".repeat(32)),
            )
            .unwrap(),
        ),
        caller: Some("Desktop".into()),
        channel: "gateway".into(),
        at: "2026-11-25T12:00:00Z".into(),
    }
}

fn confirmed(h: &Harness) -> IssueWorkOrder {
    h.tick(false).unwrap();
    let context = confirmation(h, &criteria());
    let id = h.decision().request.id().clone();
    h.decisions
        .answer_with_criteria(id.as_str(), "start", None, Some(&criteria()), &context)
        .unwrap();
    h.tick(false).unwrap();
    let checkpoint = h.work().unwrap();
    checkpoint.validate().unwrap();
    assert_eq!(
        checkpoint
            .order
            .criteria_adoption
            .as_ref()
            .unwrap()
            .criteria,
        criteria()
    );
    checkpoint.order
}

fn seed_tasks(workspace: &str) -> Arc<SqlitePlanningAuthorityAdapter> {
    let adapter = Arc::new(SqlitePlanningAuthorityAdapter::new());
    let directions = DirectionCatalogDocument {
        version: 1,
        queue_idle: QueueIdleConfig {
            policy: QueueIdlePolicy::Stop,
            prompt_path: String::new(),
        },
        directions: vec![DirectionDefinition {
            id: "issue-criteria-direction".into(),
            title: "Confirmed issue".into(),
            summary: "Synthetic deterministic test".into(),
            success_criteria: vec!["retain approval".into()],
            scope_hints: Vec::new(),
            detail_doc_path: String::new(),
            state: DirectionState::Active,
        }],
    };
    adapter
        .commit_direction_authority_snapshot(
            workspace,
            PlanningDirectionAuthorityCommit {
                observed_planning_revision: None,
                directions: &directions,
                authority_mutation_owner_token: None,
            },
        )
        .unwrap();
    let document = TaskAuthorityDocument {
        version: TASK_AUTHORITY_FORMAT_VERSION,
        tasks: Vec::new(),
    };
    adapter
        .commit_task_authority_snapshot(
            workspace,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &document,
                queue_projection: &PriorityQueueService::new()
                    .build_projection(&directions, &document)
                    .unwrap(),
            },
        )
        .unwrap();
    adapter
}

fn execution(
    h: &Harness,
    authority: Arc<SqlitePlanningAuthorityAdapter>,
) -> crate::composition::issue_work_execution::AkraIssueWorkExecutionAdapter {
    let parallel = crate::application::service::parallel_mode::ParallelModeService::new(
        authority.clone(),
        Arc::new(crate::adapter::outbound::github::automation::GithubAutomationAdapter::new()),
        Arc::new(crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter::new()),
    );
    crate::composition::issue_work_execution::AkraIssueWorkExecutionAdapter::new(
        authority.clone(),
        authority,
        parallel,
    )
    .with_criteria_store(h.fixture.store())
}

#[test]
fn criteria_adoption_requires_the_exact_native_payload_and_persists_with_the_human_answer() {
    let h = Harness::new();
    h.tick(false).unwrap();
    let id = h.decision().request.id().clone();
    let context = confirmation(&h, &criteria());
    let mut agent = context.clone();
    agent.human_confirmation = None;
    let error = h
        .decisions
        .answer_with_criteria(id.as_str(), "start", None, Some(&criteria()), &agent)
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<IssueConfirmationError>(),
        Some(&IssueConfirmationError::Required)
    );
    let mut altered = criteria();
    altered[0].essential = false;
    assert!(
        h.decisions
            .answer_with_criteria(id.as_str(), "start", None, Some(&altered), &context)
            .is_err()
    );
    assert!(
        h.decisions
            .answer(id.as_str(), "start", None, &context)
            .is_err()
    );
    assert!(h.decision().request.answer().is_none());
    let result = h
        .decisions
        .answer_with_criteria(id.as_str(), "start", None, Some(&criteria()), &context)
        .unwrap();
    assert_eq!(
        result.commit.record.request.answer().unwrap().by,
        IssueDecisionAnswerBy::Human
    );
    assert_eq!(
        result.commit.record.request.answer().unwrap().criteria,
        Some(criteria())
    );
    let stored = h
        .fixture
        .store()
        .get_issue_decision(&h.fixture.workspace, &id)
        .unwrap()
        .unwrap();
    assert_eq!(stored, result.commit.record);
}

#[test]
fn criteria_adoption_real_sqlite_replays_preserve_user_edits_and_removed_task_identity() {
    let h = Harness::new();
    let order = confirmed(&h);
    let authority = seed_tasks(&h.fixture.workspace);
    let adapter = execution(&h, authority.clone());
    let ids = adapter.submit(&h.fixture.workspace, &order).unwrap();
    let before = authority
        .load_task_authority_snapshot(&h.fixture.workspace)
        .unwrap()
        .unwrap();
    let task = &before.task_authority.tasks[0];
    assert_eq!(ids, vec![task.id.clone()]);
    assert_eq!(task.acceptance_revision, 1);
    assert_eq!(task.acceptance[0].scenario, criteria()[0].scenario);
    assert_eq!(
        task.acceptance[0].verification_recipe,
        Some(crate::domain::planning::verification::TaskVerificationRecipe::GitDiffCheckV1)
    );
    assert_eq!(task.created_by, TaskActor::System);
    assert_eq!(
        task.provenance.origin_session_kind,
        Some(OriginSessionKind::System)
    );
    let receipt = authority
        .load_work_order(&h.fixture.workspace, &order.work_order_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        receipt.acceptance_receipt.as_deref(),
        Some(
            order
                .criteria_adoption
                .as_ref()
                .unwrap()
                .receipt_digest(&order.issue, &order.work_order_id)
                .as_str()
        )
    );

    let mut updated = task.acceptance.clone();
    updated[0].scenario = "사용자가 승인 후 수정한 완료 기준".into();
    PlanningTaskMutationService::new(authority.clone(), PriorityQueueService::new())
        .apply_commands(PlanningTaskMutationRequest {
            workspace_directory: h.fixture.workspace.clone(),
            source: PlanningTaskMutationSource::User,
            legacy_source_turn_id: None,
            provenance: TaskMutationProvenance::default(),
            commands: vec![PlanningTaskMutationCommand::UpdateTask(
                PlanningTaskUpdateInput {
                    task_id: ids[0].clone(),
                    acceptance: Some(updated.clone()),
                    expected_planning_revision: Some(before.planning_revision),
                    direction_id: None,
                    direction_relation_note: None,
                    title: None,
                    description: None,
                    status: None,
                    base_priority: None,
                    dynamic_priority_delta: None,
                    priority_reason: None,
                    depends_on: None,
                    blocked_by: None,
                },
            )],
        })
        .unwrap();
    let after_edit = authority
        .load_task_authority_snapshot(&h.fixture.workspace)
        .unwrap()
        .unwrap();
    assert_eq!(after_edit.task_authority.tasks[0].acceptance, updated);
    assert_eq!(after_edit.task_authority.tasks[0].acceptance_revision, 2);
    assert_eq!(
        execution(&h, authority.clone())
            .submit(&h.fixture.workspace, &order)
            .unwrap(),
        ids
    );
    assert_eq!(
        authority
            .load_task_authority_snapshot(&h.fixture.workspace)
            .unwrap()
            .unwrap(),
        after_edit
    );
    authority
        .clear_task_authority_snapshot(&h.fixture.workspace)
        .unwrap();
    assert_eq!(
        execution(&h, authority.clone())
            .submit(&h.fixture.workspace, &order)
            .unwrap(),
        ids
    );
    assert!(
        authority
            .load_task_authority_snapshot(&h.fixture.workspace)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        authority
            .load_work_order(&h.fixture.workspace, &order.work_order_id)
            .unwrap()
            .unwrap(),
        receipt
    );
}

#[test]
fn criteria_adoption_rejects_forged_scope_spec_and_receipts_before_creating_a_task() {
    let h = Harness::new();
    let order = confirmed(&h);
    let authority = seed_tasks(&h.fixture.workspace);
    let adapter = execution(&h, authority.clone());
    for change in [
        "omitted",
        "criterion",
        "receipt",
        "decision_revision",
        "hash",
        "work_order",
        "handoff",
        "workspace",
    ] {
        let mut altered = order.clone();
        match change {
            "omitted" => altered.criteria_adoption = None,
            "criterion" => {
                altered.criteria_adoption.as_mut().unwrap().criteria[0].essential = false
            }
            "receipt" => {
                altered.criteria_adoption.as_mut().unwrap().confirmation_ref =
                    format!("desktop-confirmation:{}", "d".repeat(32))
            }
            "decision_revision" => {
                altered
                    .criteria_adoption
                    .as_mut()
                    .unwrap()
                    .decision_revision += 1
            }
            "hash" => altered.field_hash.sha256 = "b".repeat(64),
            "work_order" => altered.work_order_id.push_str("-other"),
            "handoff" => altered.handoff.body.push_str("unapproved changed source"),
            "workspace" => altered.scope.push_str("-other"),
            _ => unreachable!(),
        }
        assert!(
            adapter.submit(&h.fixture.workspace, &altered).is_err(),
            "{change}"
        );
        assert!(
            authority
                .load_task_authority_snapshot(&h.fixture.workspace)
                .unwrap()
                .unwrap()
                .task_authority
                .tasks
                .is_empty(),
            "{change}"
        );
    }
}

#[test]
fn criteria_adoption_does_not_apply_a_native_approval_after_the_issue_spec_changes() {
    let h = Harness::new();
    h.tick(false).unwrap();
    let id = h.decision().request.id().clone();
    let context = confirmation(&h, &criteria());
    let mut observation = h.fixture.observation("I_001");
    if let IssueSourceContent::Shared(snapshot) = &mut observation.content {
        snapshot
            .shared_copy
            .body
            .push_str("- 승인 이후 추가된 다른 완료 기준\n");
        snapshot.basis.field_hash =
            IssueFieldHash::of_fields(&snapshot.shared_copy.title, &snapshot.shared_copy.body);
    }
    h.fixture.seed(vec![observation], &disclosure_version());
    let result = h
        .decisions
        .answer_with_criteria(id.as_str(), "start", None, Some(&criteria()), &context)
        .unwrap();
    assert_eq!(
        result.outcome,
        crate::domain::issue_automation::decision::IssueDecisionOutcome::Superseded
    );
    assert!(result.commit.record.request.answer().is_none());
    assert!(h.work().is_none());
}
