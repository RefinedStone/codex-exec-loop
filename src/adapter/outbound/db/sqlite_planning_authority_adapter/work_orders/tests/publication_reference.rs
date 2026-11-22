use std::collections::BTreeMap;
use std::sync::Arc;

use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkControlRepositoryPort, PlanningWorkLinkReference, PlanningWorkPermit,
    PlanningWorkPermitBasis, PlanningWorkPermitRepositoryPort, PlanningWorkPermitStage,
};
use crate::application::service::planning::task_mutation::PlanningTaskCreateInput;
use crate::application::service::planning::{PlanningWorkOrderRequest, PlanningWorkOrderService};
use crate::domain::planning::PriorityQueueService;
use anyhow::Result;

use super::*;

const ORDER: &str = "legacy-order";
const ORIGIN: &str = "synthetic-source";
const URL: &str = "https://github.com/acme/project/issues/7";

fn reference() -> PlanningWorkLinkReference {
    PlanningWorkLinkReference {
        url: URL.into(),
        close_on_merge: false,
    }
}

fn legacy_workspace(label: &str, origin: &str) -> String {
    let workspace = workspace(label);
    let initial = seed(&workspace, vec![task("supplement")]);
    register_document(
        &workspace,
        ORDER,
        origin,
        "initial",
        initial.planning_revision,
        &candidate(&initial, "initial"),
    )
    .unwrap();
    let adapter = SqlitePlanningAuthorityAdapter::new();
    adapter
        .link_supplemental_task(&workspace, ORDER, "supplement")
        .unwrap();
    let basis = PlanningWorkPermitBasis {
        authorization_epoch: 0,
        policy_version: "legacy-policy".into(),
        field_hash: "v1:legacy-hash".into(),
        gate_revision: 1,
        source_sha: "a".repeat(40),
    };
    adapter
        .set_work_order_permit_basis(&workspace, ORDER, &basis)
        .unwrap();
    for stage in [
        PlanningWorkPermitStage::Dispatch,
        PlanningWorkPermitStage::Push,
    ] {
        adapter
            .grant_work_order_permit(
                &workspace,
                &PlanningWorkPermit {
                    work_order_id: ORDER.into(),
                    stage,
                    basis: basis.clone(),
                    requires_human_review: true,
                },
            )
            .unwrap();
    }
    // A source refresh intentionally leaves old grants stale. Re-submission is needed
    // before the coordinator can grant the new source; recovery is not a permit check.
    adapter
        .set_work_order_permit_basis(
            &workspace,
            ORDER,
            &PlanningWorkPermitBasis {
                gate_revision: 2,
                source_sha: "b".repeat(40),
                ..basis
            },
        )
        .unwrap();
    adapter
        .set_work_order_execution_limit(&workspace, ORDER, 3)
        .unwrap();
    connection(&workspace)
        .execute(
            "INSERT INTO planning_work_order_started_tasks
         (work_order_id, task_id, first_session_key) VALUES (?1, 'initial', 'legacy-session')",
            [ORDER],
        )
        .unwrap();
    // This table did not exist before publication references were introduced.
    connection(&workspace)
        .execute_batch("DROP TABLE planning_work_order_links")
        .unwrap();
    workspace
}

/// Compare all existing authority tables, including task/audit/queue, permit/basis,
/// control and execution-attempt budget, rather than only the in-memory return value.
fn rows_except_links(workspace: &str) -> BTreeMap<String, Vec<Vec<String>>> {
    let db = connection(workspace);
    let tables = db
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table'
         AND name NOT LIKE 'sqlite_%' AND name != 'planning_work_order_links' ORDER BY name",
        )
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

fn register_replay(
    workspace: &str,
    origin: &str,
    reference: Option<&PlanningWorkLinkReference>,
) -> Result<PlanningWorkOrderRecord> {
    // An existing order must win before these deliberately stale/invalid candidate fields.
    let document = TaskAuthorityDocument {
        version: 1,
        tasks: Vec::new(),
    };
    let result = SqlitePlanningAuthorityAdapter::new().register_work_order(
        workspace,
        PlanningWorkOrderRegistration {
            work_order_id: ORDER,
            origin,
            link_reference: reference,
            initial_task_id: "",
            observed_planning_revision: -1,
            task_authority: &document,
            queue_projection: &projection(&document),
            audit: PlanningTaskAuthorityMutationAudit {
                task_ids: &[],
                legacy_source_turn_id: None,
                provenance: &TaskMutationProvenance::new(OriginSessionKind::System),
            },
        },
    )?;
    match result {
        PlanningWorkOrderRegistrationResult::Existing { work_order } => Ok(work_order),
        other => anyhow::bail!("a replay must return the existing work order: {other:?}"),
    }
}

fn service_replay(
    workspace: &str,
    reference: Option<PlanningWorkLinkReference>,
) -> Result<PlanningWorkOrderRecord> {
    let service = PlanningWorkOrderService::new(
        Arc::new(SqlitePlanningAuthorityAdapter::new()),
        PriorityQueueService::new(),
    );
    let admission = service.register_work_order(PlanningWorkOrderRequest {
        workspace_directory: workspace.into(),
        work_order_id: ORDER.into(),
        origin: ORIGIN.into(),
        link_reference: reference,
        input: PlanningTaskCreateInput {
            direction_id: Some("missing-direction".into()),
            direction_relation_note: None,
            title: String::new(),
            description: None,
            status: None,
            base_priority: None,
            dynamic_priority_delta: None,
            priority_reason: None,
            depends_on: Vec::new(),
            blocked_by: Vec::new(),
        },
    })?;
    assert!(!admission.created);
    assert_eq!(admission.task_id, "initial");
    Ok(admission.work_order)
}

fn assert_legacy_replay(service: bool) {
    let workspace = legacy_workspace(
        if service {
            "legacy-fast-admission"
        } else {
            "legacy-atomic-registration"
        },
        ORIGIN,
    );
    let original = stored(&workspace, ORDER);
    let before = rows_except_links(&workspace);
    let expected = PlanningWorkOrderRecord {
        link_reference: Some(reference()),
        ..original
    };
    let replay = if service {
        service_replay(&workspace, Some(reference()))
    } else {
        register_replay(&workspace, ORIGIN, Some(&reference()))
    }
    .expect("legacy publication metadata should be recovered without recreating work");
    assert_eq!(replay, expected);
    assert_eq!(stored(&workspace, ORDER), expected);
    assert_eq!(rows_except_links(&workspace), before);
    assert_eq!(count(&workspace, "planning_work_order_links"), 1);
    // New adapter/service instances reopen the same SQLite authority on every replay.
    assert_eq!(
        service_replay(&workspace, Some(reference())).unwrap(),
        expected
    );
    assert_eq!(
        register_replay(&workspace, ORIGIN, Some(&reference())).unwrap(),
        expected
    );
    assert_eq!(rows_except_links(&workspace), before);
}

#[test]
fn legacy_publication_reference_recovers_atomic_registration_without_authority_changes() {
    assert_legacy_replay(false);
}

#[test]
fn legacy_publication_reference_recovers_fast_admission_before_replacement_validation() {
    assert_legacy_replay(true);
}

#[test]
fn legacy_publication_reference_rejects_identity_and_authority_changes() {
    let workspace = legacy_workspace("legacy-reference-rejections", ORIGIN);
    let original = stored(&workspace, ORDER);
    let before = rows_except_links(&workspace);
    let closing = PlanningWorkLinkReference {
        close_on_merge: true,
        ..reference()
    };
    let invalid = PlanningWorkLinkReference {
        url: "https://github.com/acme/project/issues/7\ninjected".into(),
        close_on_merge: false,
    };
    for rejected in [&closing, &invalid] {
        assert!(register_replay(&workspace, ORIGIN, Some(rejected)).is_err());
        assert!(service_replay(&workspace, Some(rejected.clone())).is_err());
    }
    let adapter = SqlitePlanningAuthorityAdapter::new();
    assert!(
        adapter
            .replay_work_order(&workspace, ORDER, "another-origin", Some(&reference()))
            .is_err()
    );
    assert!(register_replay(&workspace, "another-origin", Some(&reference())).is_err());
    assert_eq!(stored(&workspace, ORDER), original);
    assert_eq!(rows_except_links(&workspace), before);
    assert_eq!(count(&workspace, "planning_work_order_links"), 0);

    let recovered = service_replay(&workspace, Some(reference())).unwrap();
    let different_url = PlanningWorkLinkReference {
        url: "https://github.com/acme/project/issues/8".into(),
        ..reference()
    };
    for rejected in [None, Some(closing), Some(different_url)] {
        assert!(register_replay(&workspace, ORIGIN, rejected.as_ref()).is_err());
        assert!(service_replay(&workspace, rejected).is_err());
        assert_eq!(stored(&workspace, ORDER), recovered);
        assert_eq!(rows_except_links(&workspace), before);
        assert_eq!(count(&workspace, "planning_work_order_links"), 1);
    }
}

#[test]
fn legacy_publication_reference_preserves_existing_closing_policy_on_exact_replay() {
    let workspace = legacy_workspace("existing-closing-reference", ORIGIN);
    ensure_work_order_tables(&connection(&workspace)).unwrap();
    let closing = PlanningWorkLinkReference {
        close_on_merge: true,
        ..reference()
    };
    // This represents an already persisted closing reference, not newly granted authority.
    connection(&workspace)
        .execute(
            "INSERT INTO planning_work_order_links (work_order_id, reference_json) VALUES (?1, ?2)",
            rusqlite::params![ORDER, serde_json::to_string(&closing).unwrap()],
        )
        .unwrap();
    let original = stored(&workspace, ORDER);
    let before = rows_except_links(&workspace);
    assert_eq!(
        register_replay(&workspace, ORIGIN, Some(&closing)).unwrap(),
        original
    );
    assert_eq!(service_replay(&workspace, Some(closing)).unwrap(), original);
    assert!(register_replay(&workspace, ORIGIN, Some(&reference())).is_err());
    assert!(service_replay(&workspace, None).is_err());
    assert_eq!(stored(&workspace, ORDER), original);
    assert_eq!(rows_except_links(&workspace), before);
}

#[test]
fn legacy_publication_reference_validates_durable_lineage_and_permit_identity_before_insert() {
    for corrupt_permit in [false, true] {
        let workspace = legacy_workspace("legacy-reference-invalid-lineage", ORIGIN);
        ensure_work_order_tables(&connection(&workspace)).unwrap();
        if corrupt_permit {
            let mut permits = stored(&workspace, ORDER).permits;
            permits[0].work_order_id = "another-order".into();
            connection(&workspace)
                .execute(
                    "UPDATE planning_work_orders SET permits_json=?1 WHERE work_order_id=?2",
                    rusqlite::params![serde_json::to_string(&permits).unwrap(), ORDER],
                )
                .unwrap();
        } else {
            connection(&workspace)
                .execute(
                    "UPDATE planning_work_order_tasks SET ordinal=7 WHERE task_id='supplement'",
                    [],
                )
                .unwrap();
        }
        let before = rows_except_links(&workspace);
        assert!(register_replay(&workspace, ORIGIN, Some(&reference())).is_err());
        assert!(service_replay(&workspace, Some(reference())).is_err());
        assert_eq!(rows_except_links(&workspace), before);
        assert_eq!(count(&workspace, "planning_work_order_links"), 0);
    }
}

#[test]
fn legacy_publication_reference_concurrent_replays_insert_once_and_reject_conflicting_urls() {
    for same_url in [true, false] {
        let workspace = legacy_workspace("legacy-reference-concurrent", ORIGIN);
        stored(&workspace, ORDER); // Complete additive schema initialization before competing.
        let before = rows_except_links(&workspace);
        let start = Arc::new(std::sync::Barrier::new(3));
        let mut workers = Vec::new();
        for use_service in [false, true] {
            let workspace = workspace.clone();
            let start = start.clone();
            let mut link = reference();
            if use_service && !same_url {
                link.url = "https://github.com/acme/project/issues/8".into();
            }
            workers.push(std::thread::spawn(move || {
                start.wait();
                // The fast service path and atomic registration use independent SQLite connections.
                if use_service {
                    service_replay(&workspace, Some(link))
                } else {
                    register_replay(&workspace, ORIGIN, Some(&link))
                }
            }));
        }
        start.wait();
        let results = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            results.iter().filter(|result| result.is_ok()).count(),
            if same_url { 2 } else { 1 }
        );
        let winner = stored(&workspace, ORDER);
        for result in results {
            match result {
                Ok(record) => assert_eq!(record, winner),
                Err(error) => assert!(error.to_string().contains("publication reference changed")),
            }
        }
        assert_eq!(rows_except_links(&workspace), before);
        assert_eq!(count(&workspace, "planning_work_order_links"), 1);
        assert_eq!(
            service_replay(&workspace, winner.link_reference.clone()).unwrap(),
            winner
        );
        assert_eq!(rows_except_links(&workspace), before);
    }
}

#[test]
fn legacy_publication_reference_write_failure_rolls_back_and_remains_retryable() {
    let workspace = legacy_workspace("legacy-reference-write-failure", ORIGIN);
    let original = stored(&workspace, ORDER);
    let before = rows_except_links(&workspace);
    connection(&workspace)
        .execute_batch(
            "CREATE TRIGGER fail_reference BEFORE INSERT ON planning_work_order_links
         BEGIN SELECT RAISE(ABORT, 'injected reference write failure'); END;",
        )
        .unwrap();
    for result in [
        register_replay(&workspace, ORIGIN, Some(&reference())),
        service_replay(&workspace, Some(reference())),
    ] {
        assert!(format!("{:#}", result.unwrap_err()).contains("injected reference write failure"));
    }
    assert_eq!(stored(&workspace, ORDER), original);
    assert_eq!(rows_except_links(&workspace), before);
    assert_eq!(count(&workspace, "planning_work_order_links"), 0);
    connection(&workspace)
        .execute_batch("DROP TRIGGER fail_reference")
        .unwrap();
    assert_eq!(
        service_replay(&workspace, Some(reference()))
            .unwrap()
            .link_reference,
        Some(reference())
    );
    assert_eq!(rows_except_links(&workspace), before);
}

mod submission;
