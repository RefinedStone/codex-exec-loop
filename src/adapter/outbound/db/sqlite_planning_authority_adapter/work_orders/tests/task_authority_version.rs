use std::sync::Arc;

use super::*;
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkControlRepositoryPort, PlanningWorkInstruction,
    PlanningWorkOrderSupplementalAdmission, PlanningWorkOrderSupplementalAdmissionResult,
};
use crate::application::service::planning::task_mutation::PlanningTaskCreateInput;
use crate::application::service::planning::task_mutation::work_order::PlanningSupplementalWorkRequest;
use crate::application::service::planning::{PlanningWorkOrderRequest, PlanningWorkOrderService};
use crate::domain::planning::{
    PriorityQueueService, TASK_AUTHORITY_FORMAT_VERSION, TaskAcceptanceChannel,
    TaskAcceptanceCriterion,
};

const ORDER: &str = "version-order";
const ADMISSION: &str = "version-admission";

fn input() -> PlanningTaskCreateInput {
    PlanningTaskCreateInput {
        acceptance: None,
        direction_id: Some("direction-work-order".into()),
        direction_relation_note: Some("legacy authority version regression".into()),
        title: "New work-order task".into(),
        description: None,
        status: None,
        base_priority: None,
        dynamic_priority_delta: None,
        priority_reason: None,
        depends_on: Vec::new(),
        blocked_by: Vec::new(),
    }
}

fn service() -> PlanningWorkOrderService {
    PlanningWorkOrderService::new(
        Arc::new(SqlitePlanningAuthorityAdapter::new()),
        PriorityQueueService::new(),
    )
}

#[test]
fn work_order_task_authority_version_legacy_service_registration_promotes_v1() {
    let workspace = workspace("version-service-registration");
    let before = seed(&workspace, vec![task("retained")]);
    assert_eq!(before.task_authority.version, 1);
    let request = PlanningWorkOrderRequest {
        workspace_directory: workspace.clone(),
        work_order_id: ORDER.into(),
        origin: "synthetic-source".into(),
        link_reference: None,
        input: input(),
    };
    let service = service();
    let admission = service
        .register_work_order(request.clone())
        .expect("an ordinary work-order write must upgrade a supported legacy authority");
    assert!(admission.created);
    let after = snapshot(&workspace);
    assert_eq!(after.task_authority.version, TASK_AUTHORITY_FORMAT_VERSION);
    assert_eq!(after.planning_revision, before.planning_revision + 1);
    assert_eq!(after.task_authority.tasks.len(), 2);
    assert_eq!(
        after.task_authority.tasks[0],
        before.task_authority.tasks[0]
    );
    assert_eq!(after.task_authority.tasks[1].id, admission.task_id);
    assert!(after.task_authority.tasks[1].acceptance.is_empty());
    assert_eq!(after.task_authority.tasks[1].acceptance_revision, 0);
    assert_eq!(admission.work_order.task_ids, [admission.task_id.clone()]);

    let rows = authority_rows(&workspace);
    let replay = service.register_work_order(request).unwrap();
    assert!(!replay.created);
    assert_eq!(replay.work_order, admission.work_order);
    assert_eq!(snapshot(&workspace), after);
    assert_eq!(authority_rows(&workspace), rows);
}

fn historical_task() -> TaskDefinition {
    let mut historical = task("historical");
    historical.provenance = TaskMutationProvenance::new(OriginSessionKind::System)
        .with_thread_turn(None, Some(ADMISSION.into()));
    historical
}

fn assert_service_supplemental_upgrade(historical: bool) {
    let workspace = workspace("version-service-supplemental");
    let mut tasks = vec![task("retained")];
    if historical {
        tasks.push(historical_task());
    }
    seed(&workspace, tasks);
    register_next(&workspace, ORDER, "initial");
    let before = snapshot(&workspace);
    assert_eq!(before.task_authority.version, 1);
    let request = PlanningSupplementalWorkRequest {
        workspace_directory: workspace.clone(),
        work_order_id: ORDER.into(),
        admission_key: ADMISSION.into(),
        input: input(),
    };
    let service = service();
    let admission = service
        .admit_supplemental_task_once(request.clone())
        .expect("supplemental admission must preserve historical authority or upgrade a newly created task");
    assert_eq!(admission.created, !historical);
    let after = snapshot(&workspace);
    assert_eq!(
        after.task_authority.version,
        if historical {
            1
        } else {
            TASK_AUTHORITY_FORMAT_VERSION
        }
    );
    assert_eq!(after.planning_revision, before.planning_revision + 1);
    assert_eq!(
        &after.task_authority.tasks[..before.task_authority.tasks.len()],
        before.task_authority.tasks.as_slice()
    );
    if historical {
        assert_eq!(admission.task_id, "historical");
        assert_eq!(after.task_authority, before.task_authority);
    } else {
        assert_eq!(
            after.task_authority.tasks.len(),
            before.task_authority.tasks.len() + 1
        );
        assert_eq!(
            after.task_authority.tasks.last().unwrap().id,
            admission.task_id
        );
    }
    assert_eq!(
        admission.work_order.task_ids,
        ["initial", admission.task_id.as_str()]
    );
    assert_eq!(count(&workspace, "planning_work_order_task_admissions"), 1);

    let rows = authority_rows(&workspace);
    let replay = service.admit_supplemental_task_once(request).unwrap();
    assert!(!replay.created);
    assert_eq!(replay.task_id, admission.task_id);
    assert_eq!(snapshot(&workspace), after);
    assert_eq!(authority_rows(&workspace), rows);
}

#[test]
fn work_order_task_authority_version_legacy_service_supplemental_creation_promotes_v1() {
    assert_service_supplemental_upgrade(false);
}

#[test]
fn work_order_task_authority_version_legacy_service_historical_adoption_preserves_v1() {
    assert_service_supplemental_upgrade(true);
}

#[derive(Debug, Clone, Copy)]
enum WriteKind {
    Registration,
    SupplementalCreation,
    HistoricalAdoption,
}

impl WriteKind {
    const ALL: [Self; 3] = [
        Self::Registration,
        Self::SupplementalCreation,
        Self::HistoricalAdoption,
    ];

    fn task_id(self) -> &'static str {
        match self {
            Self::HistoricalAdoption => "historical",
            _ => "added",
        }
    }
}

fn version_fixture(kind: WriteKind, version: u32) -> (String, PlanningTaskAuthoritySnapshot) {
    let workspace = workspace("version-db-boundary");
    let mut tasks = vec![task("retained")];
    if matches!(kind, WriteKind::HistoricalAdoption) {
        tasks.push(historical_task());
    }
    seed(&workspace, tasks);
    if !matches!(kind, WriteKind::Registration) {
        register_next(&workspace, ORDER, "initial");
    }
    if version != 1 {
        let before = snapshot(&workspace);
        let mut document = before.task_authority;
        document.version = version;
        // The raw repository can retain future documents for diagnosis. The WorkOrder
        // mutation boundary must refuse to reinterpret them as supported authority.
        commit_document(&workspace, Some(before.planning_revision), &document);
    }
    ensure_work_order_tables(&connection(&workspace)).unwrap();
    let before = snapshot(&workspace);
    assert_eq!(before.task_authority.version, version);
    (workspace, before)
}

fn version_candidate(
    kind: WriteKind,
    before: &PlanningTaskAuthoritySnapshot,
    version: u32,
) -> TaskAuthorityDocument {
    let mut document = if matches!(kind, WriteKind::HistoricalAdoption) {
        before.task_authority.clone()
    } else {
        candidate(before, kind.task_id())
    };
    document.version = version;
    if matches!(kind, WriteKind::SupplementalCreation) {
        document.tasks.last_mut().unwrap().provenance =
            TaskMutationProvenance::new(OriginSessionKind::System)
                .with_thread_turn(None, Some(ADMISSION.into()));
    }
    document
}

fn write_candidate(
    kind: WriteKind,
    workspace: &str,
    before: &PlanningTaskAuthoritySnapshot,
    document: &TaskAuthorityDocument,
) -> Result<()> {
    if matches!(kind, WriteKind::Registration) {
        return match register_document(
            workspace,
            ORDER,
            "synthetic-source",
            kind.task_id(),
            before.planning_revision,
            document,
        )? {
            PlanningWorkOrderRegistrationResult::Registered { .. } => Ok(()),
            other => anyhow::bail!("expected a new registration, received {other:?}"),
        };
    }
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let expected = adapter
        .load_work_order_admission_snapshot(workspace, ORDER)?
        .unwrap();
    let ids = [kind.task_id().to_owned()];
    let provenance = TaskMutationProvenance::new(OriginSessionKind::System)
        .with_thread_turn(None, Some(ADMISSION.into()));
    match adapter.admit_supplemental_work_order_task(
        workspace,
        PlanningWorkOrderSupplementalAdmission {
            admission_key: ADMISSION,
            expected: &expected,
            observed_planning_revision: before.planning_revision,
            task_id: kind.task_id(),
            task_authority: document,
            queue_projection: &projection(document),
            audit: PlanningTaskAuthorityMutationAudit {
                task_ids: &ids,
                legacy_source_turn_id: None,
                provenance: &provenance,
            },
        },
    )? {
        PlanningWorkOrderSupplementalAdmissionResult::Admitted { .. } => Ok(()),
        other => anyhow::bail!("expected supplemental admission, received {other:?}"),
    }
}

/// A refusal must preserve authority, revision, audit, queue, lineage, permits and admission rows.
fn authority_rows(workspace: &str) -> BTreeMap<String, Vec<Vec<String>>> {
    let db = connection(workspace);
    let tables = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    tables
        .into_iter()
        .map(|name| {
            let mut query = db
                .prepare(&format!("SELECT * FROM \"{}\"", name.replace('"', "\"\"")))
                .unwrap();
            let columns = query.column_count();
            let mut rows = query
                .query_map([], |row| {
                    (0..columns)
                        .map(|column| {
                            row.get::<_, rusqlite::types::Value>(column)
                                .map(|value| format!("{value:?}"))
                        })
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            rows.sort();
            (name, rows)
        })
        .collect()
}

#[test]
fn work_order_task_authority_version_write_paths_enforce_their_version_contract() {
    for kind in WriteKind::ALL {
        let cases: &[(u32, u32, bool)] = match kind {
            WriteKind::HistoricalAdoption => &[
                (1, 1, true),
                (2, 2, true),
                (1, 2, false),
                (2, 1, false),
                (1, 3, false),
                (3, 2, false),
                (3, 3, false),
            ],
            _ => &[
                (1, 1, true),
                (1, 2, true),
                (2, 2, true),
                (2, 1, false),
                (1, 3, false),
                (3, 2, false),
                (3, 3, false),
            ],
        };
        for &(previous, next, allowed) in cases {
            let (workspace, before) = version_fixture(kind, previous);
            let document = version_candidate(kind, &before, next);
            let rows = authority_rows(&workspace);
            let result = write_candidate(kind, &workspace, &before, &document);
            assert_eq!(
                result.is_ok(),
                allowed,
                "{kind:?} {previous}->{next}: {result:?}"
            );
            if allowed {
                assert_eq!(snapshot(&workspace).task_authority, document);
            } else {
                assert_eq!(snapshot(&workspace), before);
                assert_eq!(authority_rows(&workspace), rows);
            }
        }
    }
}

#[test]
fn work_order_task_authority_version_upgrade_never_authorizes_existing_task_changes() {
    for kind in WriteKind::ALL {
        let (workspace, before) = version_fixture(kind, 1);
        let version = if matches!(kind, WriteKind::HistoricalAdoption) {
            1
        } else {
            TASK_AUTHORITY_FORMAT_VERSION
        };
        let mut document = version_candidate(kind, &before, version);
        document.tasks[0].acceptance = vec![TaskAcceptanceCriterion {
            id: "ac-1".into(),
            scenario: "An unrelated criterion must not be changed by admission.".into(),
            channel: TaskAcceptanceChannel::Command,
            essential: true,
        }];
        document.tasks[0].acceptance_revision = 1;
        let rows = authority_rows(&workspace);
        assert!(
            write_candidate(kind, &workspace, &before, &document).is_err(),
            "{kind:?}"
        );
        assert_eq!(snapshot(&workspace), before);
        assert_eq!(authority_rows(&workspace), rows);
    }
}

#[test]
fn work_order_task_authority_version_historical_service_refuses_future_authority() {
    let (workspace, before) = version_fixture(WriteKind::HistoricalAdoption, 3);
    let rows = authority_rows(&workspace);
    assert!(
        service()
            .admit_supplemental_task_once(PlanningSupplementalWorkRequest {
                workspace_directory: workspace.clone(),
                work_order_id: ORDER.into(),
                admission_key: ADMISSION.into(),
                input: input(),
            })
            .is_err()
    );
    assert_eq!(snapshot(&workspace), before);
    assert_eq!(authority_rows(&workspace), rows);
}

#[test]
fn work_order_task_authority_version_historical_instructions_preserve_acceptance() {
    let (workspace, initial) = version_fixture(WriteKind::HistoricalAdoption, 2);
    let mut document = initial.task_authority.clone();
    let historical = document
        .tasks
        .iter_mut()
        .find(|task| task.id == "historical")
        .unwrap();
    historical.acceptance = vec![TaskAcceptanceCriterion {
        id: "retain-criterion".into(),
        scenario: "The original reviewed criterion remains authoritative.".into(),
        channel: TaskAcceptanceChannel::Data,
        essential: true,
    }];
    historical.acceptance_revision = 7;
    commit_document(&workspace, Some(initial.planning_revision), &document);
    let instruction = PlanningWorkInstruction {
        command_id: "queued-before-adoption".into(),
        message: "Keep the operator's queued follow-up instruction.".into(),
    };
    SqlitePlanningAuthorityAdapter::new()
        .append_work_order_instruction(&workspace, ORDER, &instruction)
        .unwrap();
    let before = snapshot(&workspace);
    let request = PlanningSupplementalWorkRequest {
        workspace_directory: workspace.clone(),
        work_order_id: ORDER.into(),
        admission_key: ADMISSION.into(),
        input: input(),
    };
    let service = service();
    let admission = service
        .admit_supplemental_task_once(request.clone())
        .unwrap();
    assert!(!admission.created);
    assert_eq!(admission.task_id, "historical");
    let after = snapshot(&workspace);
    let mut expected = before.task_authority.clone();
    let historical = expected
        .tasks
        .iter_mut()
        .find(|task| task.id == "historical")
        .unwrap();
    historical.description.push_str(&format!(
        "\n\nOperator instruction [{}]:\n{}",
        instruction.command_id, instruction.message
    ));
    let preserved_acceptance = historical.acceptance.clone();
    // Only the previously queued instruction changes the historical task. Its criteria,
    // criterion revision, provenance and document version remain exactly as stored.
    assert_eq!(after.task_authority, expected);
    let updated = after
        .task_authority
        .tasks
        .iter()
        .find(|task| task.id == "historical")
        .unwrap();
    assert_eq!(updated.acceptance_revision, 7);
    assert_eq!(updated.acceptance, preserved_acceptance);
    assert_eq!(updated.description.matches(&instruction.message).count(), 1);

    let rows = authority_rows(&workspace);
    assert!(
        !service
            .admit_supplemental_task_once(request)
            .unwrap()
            .created
    );
    assert_eq!(snapshot(&workspace), after);
    assert_eq!(authority_rows(&workspace), rows);
}
