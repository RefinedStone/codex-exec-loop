use super::*;
use crate::domain::planning::{
    PlanningMutationRefusal, TASK_AUTHORITY_FORMAT_VERSION, TaskAcceptanceChannel,
    TaskAcceptanceCriterion, normalize_task_acceptance,
};

fn criterion(id: &str, scenario: &str) -> TaskAcceptanceCriterion {
    TaskAcceptanceCriterion {
        id: id.into(),
        scenario: scenario.into(),
        channel: TaskAcceptanceChannel::Command,
        essential: true,
    }
}
fn create_input(acceptance: Option<Vec<TaskAcceptanceCriterion>>) -> PlanningTaskCreateInput {
    PlanningTaskCreateInput {
        direction_id: None,
        direction_relation_note: None,
        title: "Acceptance task".into(),
        description: Some("Synthetic stored acceptance criteria".into()),
        status: None,
        base_priority: None,
        dynamic_priority_delta: None,
        priority_reason: None,
        depends_on: Vec::new(),
        blocked_by: Vec::new(),
        acceptance,
    }
}
fn update_input(
    expected_planning_revision: Option<i64>,
    acceptance: Option<Vec<TaskAcceptanceCriterion>>,
) -> PlanningTaskUpdateInput {
    PlanningTaskUpdateInput {
        task_id: "task-1".into(),
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
        expected_planning_revision,
        acceptance,
    }
}
struct Fixture {
    repo: Arc<NoopPlanningTaskRepositoryPort>,
    workspace: String,
    service: PlanningTaskMutationService,
}
impl Fixture {
    fn new(label: &str, tasks: Vec<TaskDefinition>) -> Self {
        let repo = repo();
        let workspace = workspace(&format!("acceptance-{label}"));
        seed(
            repo.as_ref(),
            &workspace,
            TaskAuthorityDocument { version: 1, tasks },
        );
        let service = PlanningTaskMutationService::new(
            repo.clone(),
            crate::domain::planning::PriorityQueueService::new(),
        );
        Self {
            repo,
            workspace,
            service,
        }
    }
    fn snapshot(&self) -> (i64, TaskAuthorityDocument) {
        let value = self
            .repo
            .load_task_authority_snapshot(&self.workspace)
            .unwrap()
            .unwrap();
        (value.planning_revision, value.task_authority)
    }
    fn apply(
        &self,
        source: PlanningTaskMutationSource,
        commands: Vec<PlanningTaskMutationCommand>,
    ) -> anyhow::Result<()> {
        self.service
            .apply_commands(PlanningTaskMutationRequest {
                workspace_directory: self.workspace.clone(),
                source,
                legacy_source_turn_id: Some("acceptance-turn".into()),
                provenance: TaskMutationProvenance::new(OriginSessionKind::Main),
                commands,
            })
            .map(|_| ())
    }
    fn update(
        &self,
        source: PlanningTaskMutationSource,
        input: PlanningTaskUpdateInput,
    ) -> anyhow::Result<()> {
        self.apply(source, vec![PlanningTaskMutationCommand::UpdateTask(input)])
    }
}
fn task_with_acceptance(revision: u64) -> TaskDefinition {
    let mut task = task("task-1", TaskStatus::Ready);
    task.acceptance = vec![
        criterion("a", "first scenario"),
        criterion("b", "second scenario"),
    ];
    task.acceptance_revision = revision;
    task
}

#[test]
fn user_create_normalizes_criteria_and_host_assigns_initial_revision() {
    for (index, criteria) in [
        None,
        Some(Vec::new()),
        Some(vec![
            criterion(" b ", " second "),
            criterion("a", " first "),
        ]),
    ]
    .into_iter()
    .enumerate()
    {
        let fixture = Fixture::new(&format!("create-{index}"), Vec::new());
        fixture
            .apply(
                PlanningTaskMutationSource::User,
                vec![PlanningTaskMutationCommand::CreateTask(create_input(
                    criteria.clone(),
                ))],
            )
            .unwrap();
        let (_, document) = fixture.snapshot();
        assert_eq!(document.version, TASK_AUTHORITY_FORMAT_VERSION);
        let task = &document.tasks[0];
        let expected = normalize_task_acceptance(criteria.as_deref().unwrap_or_default()).unwrap();
        assert_eq!(task.acceptance, expected);
        assert_eq!(task.acceptance_revision, u64::from(!expected.is_empty()));
        assert_eq!(task.created_by, TaskActor::User);
    }
}

#[test]
fn only_meaningful_criteria_changes_and_clear_advance_the_host_revision() {
    let fixture = Fixture::new("semantic-revisions", vec![task_with_acceptance(1)]);
    let baseline = fixture.snapshot();
    let mut reordered = baseline.1.tasks[0].acceptance.clone();
    reordered.reverse();
    reordered[0].id = " b ".into();
    reordered[0].scenario = " second scenario ".into();
    fixture
        .update(
            PlanningTaskMutationSource::User,
            update_input(Some(baseline.0), Some(reordered)),
        )
        .unwrap();
    assert_eq!(fixture.snapshot(), baseline);

    let mut omitted = update_input(None, None);
    omitted.title = Some("A different title".into());
    fixture
        .update(PlanningTaskMutationSource::User, omitted)
        .unwrap();
    let title_change = fixture.snapshot();
    assert_eq!(
        title_change.1.tasks[0].acceptance,
        baseline.1.tasks[0].acceptance
    );
    assert_eq!(title_change.1.tasks[0].acceptance_revision, 1);

    let mut replacement = title_change.1.tasks[0].acceptance.clone();
    replacement[0].scenario = "new scenario".into();
    for expected_revision in 2..=4 {
        if expected_revision == 3 {
            replacement[0].channel = TaskAcceptanceChannel::Http;
        }
        if expected_revision == 4 {
            replacement[0].essential = false;
        }
        let current = fixture.snapshot();
        fixture
            .update(
                PlanningTaskMutationSource::User,
                update_input(Some(current.0), Some(replacement.clone())),
            )
            .unwrap();
        assert_eq!(
            fixture.snapshot().1.tasks[0].acceptance_revision,
            expected_revision
        );
    }
    let current = fixture.snapshot();
    fixture
        .update(
            PlanningTaskMutationSource::User,
            update_input(Some(current.0), Some(Vec::new())),
        )
        .unwrap();
    let cleared = fixture.snapshot();
    assert!(cleared.1.tasks[0].acceptance.is_empty());
    assert_eq!(cleared.1.tasks[0].acceptance_revision, 5);
    fixture
        .update(
            PlanningTaskMutationSource::User,
            update_input(Some(cleared.0), Some(Vec::new())),
        )
        .unwrap();
    assert_eq!(fixture.snapshot(), cleared);
}

#[test]
fn empty_acceptance_noop_does_not_rewrite_a_legacy_document_or_audit() {
    let fixture = Fixture::new("legacy-noop", vec![task("task-1", TaskStatus::Ready)]);
    let before = fixture.snapshot();
    fixture
        .update(
            PlanningTaskMutationSource::User,
            update_input(Some(before.0), Some(Vec::new())),
        )
        .unwrap();
    assert_eq!(fixture.snapshot(), before);
    assert_eq!(fixture.snapshot().1.version, 1);
}

#[test]
fn missing_or_stale_planning_revision_rejects_acceptance_without_writes() {
    let fixture = Fixture::new("cas-rejection", vec![task_with_acceptance(7)]);
    let before = fixture.snapshot();
    let missing = fixture
        .update(
            PlanningTaskMutationSource::User,
            update_input(None, Some(Vec::new())),
        )
        .unwrap_err();
    assert!(missing.to_string().contains("expected_planning_revision"));
    assert_eq!(fixture.snapshot(), before);
    let stale = fixture
        .update(
            PlanningTaskMutationSource::User,
            update_input(Some(before.0 - 1), Some(Vec::new())),
        )
        .unwrap_err();
    assert!(matches!(
        PlanningMutationRefusal::find_in(&stale),
        Some(PlanningMutationRefusal::RevisionStale { .. })
    ));
    assert_eq!(fixture.snapshot(), before);
}

#[test]
fn worker_and_system_typed_inputs_cannot_supply_even_empty_acceptance() {
    for (index, source) in [
        PlanningTaskMutationSource::Worker,
        PlanningTaskMutationSource::System,
    ]
    .into_iter()
    .enumerate()
    {
        let fixture = Fixture::new(&format!("source-{index}"), vec![task_with_acceptance(7)]);
        let before = fixture.snapshot();
        for criteria in [Vec::new(), vec![criterion("a", "worker-defined")]] {
            assert!(
                fixture
                    .apply(
                        source,
                        vec![PlanningTaskMutationCommand::CreateTask(create_input(Some(
                            criteria.clone()
                        )))]
                    )
                    .is_err()
            );
            assert_eq!(fixture.snapshot(), before);
            assert!(
                fixture
                    .update(source, update_input(Some(before.0), Some(criteria)))
                    .is_err()
            );
            assert_eq!(fixture.snapshot(), before);
        }
    }
}

#[test]
fn criteria_revision_overflow_and_in_progress_changes_are_no_write_refusals() {
    let fixture = Fixture::new("overflow", vec![task_with_acceptance(u64::MAX)]);
    let before = fixture.snapshot();
    let error = fixture
        .update(
            PlanningTaskMutationSource::User,
            update_input(Some(before.0), Some(Vec::new())),
        )
        .unwrap_err();
    assert!(error.to_string().contains("overflow"));
    assert_eq!(fixture.snapshot(), before);
    fixture
        .update(
            PlanningTaskMutationSource::User,
            update_input(Some(before.0), Some(before.1.tasks[0].acceptance.clone())),
        )
        .unwrap();
    assert_eq!(fixture.snapshot(), before);

    let mut running = task_with_acceptance(7);
    running.status = TaskStatus::InProgress;
    let fixture = Fixture::new("in-progress", vec![running]);
    let before = fixture.snapshot();
    let mut input = update_input(Some(before.0), Some(Vec::new()));
    input.status = Some(TaskStatus::Ready);
    let error = fixture
        .update(PlanningTaskMutationSource::User, input)
        .unwrap_err();
    assert!(matches!(
        PlanningMutationRefusal::find_in(&error),
        Some(PlanningMutationRefusal::Locked { .. })
    ));
    assert_eq!(fixture.snapshot(), before);
}

#[test]
fn invalid_criteria_reject_the_whole_mutation_batch_without_partial_create() {
    let fixture = Fixture::new("invalid-batch", vec![task_with_acceptance(7)]);
    let before = fixture.snapshot();
    let duplicate = vec![criterion("same", "first"), criterion(" same ", "second")];
    assert!(
        fixture
            .apply(
                PlanningTaskMutationSource::User,
                vec![
                    PlanningTaskMutationCommand::CreateTask(create_input(None)),
                    PlanningTaskMutationCommand::UpdateTask(update_input(
                        Some(before.0),
                        Some(duplicate)
                    )),
                ]
            )
            .is_err()
    );
    assert_eq!(fixture.snapshot(), before);
    assert!(
        fixture
            .apply(
                PlanningTaskMutationSource::User,
                vec![PlanningTaskMutationCommand::CreateTask(create_input(Some(
                    vec![criterion("id", " ")]
                )))]
            )
            .is_err()
    );
    assert_eq!(fixture.snapshot(), before);
}

#[test]
fn private_worker_json_rejects_acceptance_and_host_revision_fields() {
    for op in ["create_task", "update_task"] {
        for field in ["acceptance", "acceptance_revision"] {
            let mut command = serde_json::json!({"op":op,"title":"task"});
            if op == "update_task" {
                command["task_id"] = serde_json::json!("task-1");
            }
            command[field] = if field == "acceptance" {
                serde_json::json!([])
            } else {
                serde_json::json!(7)
            };
            let message =
                serde_json::json!({"planning_task_commands":{"version":1,"commands":[command]}})
                    .to_string();
            assert!(matches!(
                extract_planning_task_commands(&message),
                PlanningTaskCommandExtraction::InvalidCommands { .. }
            ));
        }
    }
}

#[test]
fn public_preview_cannot_forge_worker_criteria_or_user_host_revision() {
    for case in 0..4 {
        let fixture = Fixture::new(&format!("preview-tamper-{case}"), Vec::new());
        let source = if case < 2 {
            PlanningTaskMutationSource::Worker
        } else {
            PlanningTaskMutationSource::User
        };
        let criteria = if case < 2 {
            None
        } else {
            Some(vec![criterion("a", "user-agreed")])
        };
        let mut preview = fixture
            .service
            .preview_create_task(PlanningTaskCreatePreviewRequest {
                workspace_directory: fixture.workspace.clone(),
                source,
                legacy_source_turn_id: None,
                provenance: provenance(),
                input: create_input(criteria),
            })
            .unwrap();
        let before = fixture.snapshot();
        match case {
            0 => {
                preview.task.acceptance = vec![criterion("a", "forged")];
                preview.task.acceptance_revision = 1;
            }
            1 => {
                preview.request.input.acceptance = Some(vec![criterion("a", "forged")]);
                preview.task.acceptance = vec![criterion("a", "forged")];
                preview.task.acceptance_revision = 1;
            }
            2 => preview.task.acceptance_revision = 99,
            _ => preview.task.acceptance[0].scenario = "different user criteria".into(),
        }
        assert!(fixture.service.commit_create_preview(&preview).is_err());
        assert_eq!(fixture.snapshot(), before);
    }
}

#[test]
fn genuine_user_preview_and_cancellation_preserve_agreed_acceptance() {
    let fixture = Fixture::new("preview-commit", Vec::new());
    let preview = fixture
        .service
        .preview_create_task(PlanningTaskCreatePreviewRequest {
            workspace_directory: fixture.workspace.clone(),
            source: PlanningTaskMutationSource::User,
            legacy_source_turn_id: None,
            provenance: provenance(),
            input: create_input(Some(vec![criterion("a", "user-agreed")])),
        })
        .unwrap();
    fixture.service.commit_create_preview(&preview).unwrap();
    let before = fixture.snapshot();
    assert_eq!(before.1.version, TASK_AUTHORITY_FORMAT_VERSION);
    assert_eq!(before.1.tasks[0].acceptance, preview.task.acceptance);
    assert_eq!(before.1.tasks[0].acceptance_revision, 1);
    fixture
        .service
        .cancel_queue_tasks(PlanningQueueCancellationRequest {
            workspace_directory: fixture.workspace.clone(),
            expected_planning_revision: before.0,
            targets: vec![PlanningQueueCancellationTarget {
                task_id: before.1.tasks[0].id.clone(),
                expected_status: TaskStatus::Ready,
                expected_updated_at: before.1.tasks[0].updated_at.clone(),
            }],
        })
        .unwrap();
    let after = fixture.snapshot();
    assert_eq!(after.1.version, TASK_AUTHORITY_FORMAT_VERSION);
    assert_eq!(after.1.tasks[0].status, TaskStatus::Cancelled);
    assert_eq!(after.1.tasks[0].acceptance, before.1.tasks[0].acceptance);
    assert_eq!(after.1.tasks[0].acceptance_revision, 1);
}

#[test]
fn replayed_system_admission_cannot_smuggle_user_acceptance() {
    let fixture = Fixture::new("system-replay", Vec::new());
    fixture
        .service
        .admit_system_task_once(&fixture.workspace, "synthetic-key", create_input(None))
        .unwrap();
    let before = fixture.snapshot();
    assert!(
        fixture
            .service
            .admit_system_task_once(
                &fixture.workspace,
                "synthetic-key",
                create_input(Some(vec![criterion("a", "forged")]))
            )
            .is_err()
    );
    assert_eq!(fixture.snapshot(), before);
}

#[test]
fn original_in_progress_task_cannot_change_acceptance_through_reordered_or_mixed_batch() {
    for (case, label) in [
        "status-first",
        "acceptance-first",
        "mixed-middle",
        "mixed-first",
    ]
    .into_iter()
    .enumerate()
    {
        let mut running = task_with_acceptance(7);
        running.status = TaskStatus::InProgress;
        let fixture = Fixture::new(
            &format!("original-active-batch-{label}"),
            vec![running, task("task-2", TaskStatus::Ready)],
        );
        let before = fixture.snapshot();
        let before_audit = fixture
            .service
            .load_queue_authority_mutations(&fixture.workspace, 0, i64::MAX)
            .unwrap();
        let mut status = update_input(None, None);
        status.status = Some(TaskStatus::Ready);
        let mut acceptance = update_input(
            Some(before.0),
            Some(vec![criterion("changed", "new criteria")]),
        );
        if case == 3 {
            acceptance.task_id = " task-1 ".into();
        }
        let mut unrelated = update_input(None, None);
        unrelated.task_id = "task-2".into();
        unrelated.title = Some("Unrelated task edit".into());
        let update = PlanningTaskMutationCommand::UpdateTask;
        let commands = match case {
            0 => vec![update(status), update(acceptance)],
            1 => vec![update(acceptance), update(status)],
            2 => vec![update(status), update(unrelated), update(acceptance)],
            _ => vec![update(unrelated), update(status), update(acceptance)],
        };
        let result = fixture.apply(PlanningTaskMutationSource::User, commands);
        let after = fixture.snapshot();
        let after_audit = fixture
            .service
            .load_queue_authority_mutations(&fixture.workspace, 0, i64::MAX)
            .unwrap();
        assert!(
            result.is_err(),
            "{label} unexpectedly committed the batch (planning revision {} -> {})",
            before.0,
            after.0,
        );
        let error = result.unwrap_err();
        assert!(matches!(
            PlanningMutationRefusal::find_in(&error),
            Some(PlanningMutationRefusal::Locked { .. })
        ));
        assert_eq!(
            after, before,
            "{label} changed authority or planning revision"
        );
        assert_eq!(
            after_audit, before_audit,
            "{label} changed mutation audit records"
        );
    }
}
