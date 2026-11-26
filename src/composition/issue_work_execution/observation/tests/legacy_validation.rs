use super::*;
use std::sync::Arc;

use crate::adapter::outbound::db::sqlite_planning_authority_adapter::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
use crate::adapter::outbound::github::automation::GithubAutomationAdapter;
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDocumentCommit, PlanningAuthorityPort,
};
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityMutationAudit, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkOrderRegistration, PlanningWorkOrderRegistrationResult,
    PlanningWorkOrderRepositoryPort,
};
use crate::application::service::parallel_mode::ParallelModeService;
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, OriginSessionKind,
    PriorityQueueService, QueueIdleConfig, QueueIdlePolicy,
};

const WORK_ORDER_ID: &str = "issue-work-1";

struct LegacyValidationFixture {
    workspace: String,
    authority: Arc<SqlitePlanningAuthorityAdapter>,
    database_path: String,
    original_queue: PlanningAuthorityDistributorQueueRecord,
}

impl LegacyValidationFixture {
    fn new(label: &str, record: &PrValidationRecord) -> Self {
        let path = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-observation-legacy-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        let workspace = path.display().to_string();
        let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let directions = DirectionCatalogDocument {
            version: PLANNING_FORMAT_VERSION,
            queue_idle: QueueIdleConfig {
                policy: QueueIdlePolicy::Stop,
                prompt_path: String::new(),
            },
            directions: vec![DirectionDefinition {
                id: "direction".into(),
                title: "Issue execution".into(),
                summary: "Legacy observation fixture".into(),
                success_criteria: vec!["Observe the original validation".into()],
                scope_hints: Vec::new(),
                detail_doc_path: String::new(),
                state: DirectionState::Active,
            }],
        };
        let empty_tasks = tasks(Vec::new());
        authority
            .commit_planning_authority_documents(
                &workspace,
                PlanningAuthorityDocumentCommit {
                    observed_planning_revision: None,
                    directions: &directions,
                    task_authority: &empty_tasks,
                    queue_projection: &PriorityQueueService::new()
                        .build_projection(&directions, &empty_tasks)
                        .unwrap(),
                    result_output_markdown: Some("# Result Output\n\nLegacy validation fixture"),
                    active_document_mutations: &[],
                    retired_task_ids: &[],
                    authority_mutation_owner_token: None,
                },
            )
            .unwrap();
        let before = authority
            .load_task_authority_snapshot(&workspace)
            .unwrap()
            .unwrap();
        let owned_tasks = tasks(vec![task("initial", TaskStatus::Done)]);
        let result = authority
            .register_work_order(
                &workspace,
                PlanningWorkOrderRegistration {
                    acceptance_receipt: None,
                    link_reference: None,
                    work_order_id: WORK_ORDER_ID,
                    origin: "issue_automation",
                    initial_task_id: "initial",
                    observed_planning_revision: before.planning_revision,
                    task_authority: &owned_tasks,
                    queue_projection: &PriorityQueueService::new()
                        .build_projection(&directions, &owned_tasks)
                        .unwrap(),
                    audit: PlanningTaskAuthorityMutationAudit {
                        task_ids: &["initial".into()],
                        legacy_source_turn_id: None,
                        provenance: &TaskMutationProvenance::new(OriginSessionKind::System),
                    },
                },
            )
            .unwrap();
        assert!(matches!(
            result,
            PlanningWorkOrderRegistrationResult::Registered { .. }
        ));
        let mut original_queue = queue(&lease("initial", 'a'), Some(2551));
        original_queue.queue_item_id = record.key().as_str().to_owned();
        original_queue.queue_state = ParallelModeQueueItemState::Done;
        authority
            .upsert_runtime_distributor_queue_record(&workspace, &original_queue)
            .unwrap();
        assert!(
            authority
                .compare_and_swap_runtime_pr_validation_record(
                    &workspace,
                    record.key(),
                    None,
                    Some(record),
                )
                .unwrap()
        );
        let database_path = authority
            .resolve_authority_location(&workspace)
            .unwrap()
            .authority_store_path;
        let fixture = Self {
            workspace,
            authority,
            database_path,
            original_queue,
        };
        // Preserve the old queue, ownership, and validation record, but remove the table
        // that did not exist in the parent version. No source/key lookup repairs it here.
        fixture
            .database()
            .execute_batch("DROP TABLE planning_work_order_validation_links;")
            .unwrap();
        fixture
    }

    fn database(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(&self.database_path).unwrap()
    }

    fn event_count(&self) -> i64 {
        self.database()
            .query_row("SELECT COUNT(*) FROM runtime_events", [], |row| row.get(0))
            .unwrap()
    }

    fn observe(&self) -> IssueWorkExecutionObservation {
        // Construction only: observation::load uses SQLite and never invokes delivery
        // or worker methods on the otherwise unused parallel service.
        let parallel = ParallelModeService::new(
            self.authority.clone(),
            Arc::new(GithubAutomationAdapter::new()),
            Arc::new(GitParallelModeRuntimeAdapter::new()),
        );
        let adapter = AkraIssueWorkExecutionAdapter::new(
            self.authority.clone(),
            self.authority.clone(),
            parallel,
        );
        load(&adapter, &self.workspace, WORK_ORDER_ID)
            .unwrap()
            .unwrap()
            .observation
    }

    fn add_validation(&self, record: &PrValidationRecord, enqueued_at: &str) {
        let mut queued = self.original_queue.clone();
        queued.queue_item_id = record.key().as_str().to_owned();
        queued.enqueued_at = enqueued_at.into();
        queued.updated_at = enqueued_at.into();
        self.authority
            .upsert_runtime_distributor_queue_record(&self.workspace, &queued)
            .unwrap();
        assert!(
            self.authority
                .compare_and_swap_runtime_pr_validation_record(
                    &self.workspace,
                    record.key(),
                    None,
                    Some(record),
                )
                .unwrap()
        );
    }

    fn content_snapshot(&self, record: &PrValidationRecord) -> (String, String) {
        let database = self.database();
        let queue = database
            .query_row(
                "SELECT content FROM runtime_distributor_queue WHERE queue_item_id=?1",
                [record.key().as_str()],
                |row| row.get(0),
            )
            .unwrap();
        let validation = database
            .query_row(
                "SELECT content FROM runtime_pr_validation_records WHERE record_key=?1",
                [record.key().as_str()],
                |row| row.get(0),
            )
            .unwrap();
        (queue, validation)
    }

    fn assert_original_records(&self, record: &PrValidationRecord, content: &(String, String)) {
        assert_eq!(&self.content_snapshot(record), content);
        assert_eq!(
            self.authority
                .load_runtime_pr_validation_record(&self.workspace, record.key())
                .unwrap(),
            Some(record.clone())
        );
        assert_eq!(
            self.authority
                .load_runtime_projections(&self.workspace)
                .unwrap()
                .distributor_queue_records,
            vec![self.original_queue.clone()]
        );
        assert_eq!(
            self.authority
                .work_order_validation_record_keys(&self.workspace, WORK_ORDER_ID)
                .unwrap(),
            [record.key().as_str()]
        );
    }
}

#[test]
fn preupgrade_first_observation_recovers_pr_integration_without_a_queue_update() {
    let record = integrated("legacy-pr-integrated");
    let fixture = LegacyValidationFixture::new("integrated", &record);
    let events_before = fixture.event_count();
    let content_before = fixture.content_snapshot(&record);
    let authority_before = fixture
        .authority
        .load_planning_authority_documents(&fixture.workspace)
        .unwrap();

    let observed = fixture.observe();
    assert_eq!(observed.phase, IssueExecutionPhase::Integrated);
    assert_eq!(observed.active_task_id.as_deref(), Some("initial"));
    let integration = observed.integration.as_ref().unwrap();
    assert_eq!(integration.source_sha, "a".repeat(40));
    assert_eq!(integration.integration_sha, "c".repeat(40));
    assert_eq!(
        integration.pull_request_url,
        Some(record.target().canonical_url())
    );
    assert_eq!(observed.execution_ids, vec!["initial".to_owned()]);
    assert_eq!(fixture.event_count(), events_before + 1);
    fixture.assert_original_records(&record, &content_before);
    assert_eq!(fixture.observe(), observed);
    assert_eq!(fixture.event_count(), events_before + 1);
    assert_eq!(
        fixture
            .authority
            .load_planning_authority_documents(&fixture.workspace)
            .unwrap(),
        authority_before
    );
}

#[test]
fn preupgrade_first_observation_recovers_pr_terminal_without_a_queue_update() {
    let record = checkpoint(&validation("legacy-pr-terminal"), 3)
        .transition(PrValidationEvent::Fail(
            PrValidationTerminalReason::ObservationFailed,
        ))
        .unwrap();
    let fixture = LegacyValidationFixture::new("terminal", &record);
    let events_before = fixture.event_count();
    let content_before = fixture.content_snapshot(&record);
    let authority_before = fixture
        .authority
        .load_planning_authority_documents(&fixture.workspace)
        .unwrap();

    let observed = fixture.observe();
    assert_eq!(observed.phase, IssueExecutionPhase::Terminal);
    assert!(observed.integration.is_none());
    let expected = validation_terminal(&record).unwrap();
    assert_eq!(
        expected.classification,
        IssueExecutionTerminalClass::ObserveAgain
    );
    assert_eq!(
        observed.current_terminal_id,
        Some(expected.record_id.clone())
    );
    assert_eq!(observed.terminals, vec![expected]);
    assert_eq!(observed.execution_ids, vec!["initial".to_owned()]);
    assert_eq!(fixture.event_count(), events_before + 1);
    fixture.assert_original_records(&record, &content_before);
    assert_eq!(fixture.observe(), observed);
    assert_eq!(fixture.event_count(), events_before + 1);
    assert_eq!(
        fixture
            .authority
            .load_planning_authority_documents(&fixture.workspace)
            .unwrap(),
        authority_before
    );
}

fn older_terminal(key: &str) -> PrValidationRecord {
    checkpoint(&validation(key), 3)
        .transition(PrValidationEvent::Fail(
            PrValidationTerminalReason::ObservationFailed,
        ))
        .unwrap()
}

fn assert_current_review(
    fixture: &LegacyValidationFixture,
    older: &PrValidationRecord,
    newer: &PrValidationRecord,
    imported_sources: i64,
) {
    let old_content = fixture.content_snapshot(older);
    let new_content = fixture.content_snapshot(newer);
    let authority_before = fixture
        .authority
        .load_planning_authority_documents(&fixture.workspace)
        .unwrap();
    let events_before = fixture.event_count();
    let observed = fixture.observe();
    assert_eq!(observed.phase, IssueExecutionPhase::InReview);
    assert!(observed.current_terminal_id.is_none());
    assert!(observed.integration.is_none());
    assert_eq!(
        observed.terminals,
        vec![validation_terminal(older).unwrap()]
    );
    assert_eq!(observed.execution_ids, vec!["initial".to_owned()]);
    assert_eq!(
        fixture
            .authority
            .work_order_validation_record_keys(&fixture.workspace, WORK_ORDER_ID)
            .unwrap(),
        [older.key().as_str(), newer.key().as_str()]
    );
    assert_eq!(fixture.event_count(), events_before + imported_sources);
    assert_eq!(fixture.observe(), observed);
    assert_eq!(fixture.event_count(), events_before + imported_sources);
    assert_eq!(fixture.content_snapshot(older), old_content);
    assert_eq!(fixture.content_snapshot(newer), new_content);
    for original in [older, newer] {
        assert_eq!(
            fixture
                .authority
                .load_runtime_pr_validation_record(&fixture.workspace, original.key())
                .unwrap()
                .as_ref(),
            Some(original)
        );
    }
    assert_eq!(
        fixture
            .authority
            .load_planning_authority_documents(&fixture.workspace)
            .unwrap(),
        authority_before
    );
}

#[test]
fn preupgrade_new_queue_imports_older_terminal_before_first_observation() {
    let older = older_terminal("older-before-upsert");
    let fixture = LegacyValidationFixture::new("before-new-queue", &older);
    let newer = validation("newer-after-upgrade");
    // A normal first post-upgrade upsert must import the old queue before assigning
    // the new link, even though no observation or explicit source lookup has run.
    fixture.add_validation(&newer, "2026-11-21T10:00:00Z");
    assert_current_review(&fixture, &older, &newer, 0);
}

#[test]
fn preupgrade_direct_newest_source_lookup_imports_the_whole_legacy_order() {
    let older = older_terminal("older-before-direct-read");
    let fixture = LegacyValidationFixture::new("direct-newest-source", &older);
    let newer = validation("newer-before-direct-read");
    fixture.add_validation(&newer, "2026-11-21T10:00:00Z");
    fixture
        .database()
        .execute_batch("DROP TABLE planning_work_order_validation_links;")
        .unwrap();
    let events_before = fixture.event_count();

    let source = fixture
        .authority
        .load_work_order_validation_source(&fixture.workspace, newer.key().as_str())
        .unwrap()
        .unwrap();
    assert_eq!(source.source_task_id, "initial");
    assert_eq!(source.work_order_id.as_deref(), Some(WORK_ORDER_ID));
    assert_eq!(fixture.event_count(), events_before + 2);
    assert_current_review(&fixture, &older, &newer, 0);
}

#[test]
fn preupgrade_stored_mixed_links_restore_current_review_without_rewriting_records() {
    let older = older_terminal("older-missing-link");
    let fixture = LegacyValidationFixture::new("stored-mixed-links", &older);
    let newer = validation("newer-retained-link");
    fixture.add_validation(&newer, "2026-11-21T10:00:00Z");
    let db = fixture.database();
    // Reproduce the preview state: the newer queue had acquired ordinal zero while
    // the old queue still had no link. Its source identity remains authoritative.
    db.execute(
        "DELETE FROM planning_work_order_validation_links WHERE validation_record_key=?1",
        [older.key().as_str()],
    )
    .unwrap();
    db.execute(
        "UPDATE planning_work_order_validation_links SET ordinal=0 WHERE validation_record_key=?1",
        [newer.key().as_str()],
    )
    .unwrap();
    let retained: (String, String) = db.query_row(
        "SELECT work_order_id,source_task_id FROM planning_work_order_validation_links WHERE validation_record_key=?1",
        [newer.key().as_str()], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    drop(db);

    assert_current_review(&fixture, &older, &newer, 1);
    let repaired: (String, String, i64) = fixture.database().query_row(
        "SELECT work_order_id,source_task_id,ordinal FROM planning_work_order_validation_links WHERE validation_record_key=?1",
        [newer.key().as_str()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!((repaired.0, repaired.1), retained);
    assert_eq!(repaired.2, 1);
}
