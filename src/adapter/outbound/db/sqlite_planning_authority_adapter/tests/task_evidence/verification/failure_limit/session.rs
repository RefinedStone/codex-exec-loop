use super::*;
use crate::domain::planning::PostTurnContinuationGate;

fn session_rows(f: &HostFixture) -> Vec<(String, String, String, String)> {
    let connection = authority_connection(&f.base.workspace);
    let mut statement = connection
        .prepare(
            "SELECT session_key,slot_id,updated_at,content
             FROM runtime_session_details ORDER BY session_key",
        )
        .unwrap();
    statement
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

#[test]
fn failure_limit_commits_an_answerable_session_before_caller_cancellation_or_reopen() {
    let f = fixture("host-failure-session-durable");
    let receipts = three_failures(&f);
    let original = session_detail(&f);
    let counters = f.base.counters();
    let events = f
        .base
        .adapter
        .pending_events(&f.base.workspace, 100)
        .unwrap();
    let mut commit = request(&f, &receipts[2]);
    commit.at = "2026-11-23T12:00:03Z".into();
    let gate = PostTurnContinuationGate::default();
    let permit = gate.capture();
    let outcome = permit
        .with_current(|| {
            f.base
                .adapter
                .apply_host_verification_failure_limit(&f.base.workspace, &commit)
        })
        .unwrap()
        .unwrap();
    assert!(outcome.changed);
    // No completion/preservation callback runs after this cancellation. The DB
    // must already contain every condition used to construct the answer target.
    assert!(permit.invalidate_if_current());
    assert!(
        permit
            .with_current(|| panic!("a cancelled completion must not repair the session"))
            .is_none()
    );

    let reopened = SqlitePlanningAuthorityAdapter::new();
    let current_task = reopened
        .load_task_authority_snapshot(&f.base.workspace)
        .unwrap()
        .unwrap()
        .task_authority
        .tasks
        .remove(0);
    let runtime =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&f.base.workspace).unwrap();
    assert_eq!(current_task.status, TaskStatus::AwaitingUser);
    assert_eq!(current_task.updated_at, commit.at);
    assert_eq!(
        runtime.slot_leases.get(&f.base.lease.slot_id),
        Some(&f.base.lease)
    );
    let [waiting] = runtime.session_details.as_slice() else {
        panic!("one exact worker session must remain answerable")
    };
    assert_eq!(waiting.state_label, "awaiting_user");
    assert_eq!(waiting.completion_state_label, "awaiting_user");
    assert_eq!(waiting.session_key, f.base.lease.session_key());
    assert_eq!(waiting.task_id, current_task.id);
    assert_eq!(waiting.slot_id, f.base.lease.slot_id);
    assert_eq!(waiting.agent_id, f.base.lease.agent_id);
    assert_eq!(waiting.branch_name, f.base.lease.branch_name);
    assert_eq!(waiting.worktree_path, f.base.lease.worktree_path);
    assert_eq!(waiting.lease_started_at, f.base.lease.leased_at);
    assert_eq!(waiting.thread_id, original.thread_id);
    assert!(waiting.thread_id.as_ref().is_some_and(|id| {
        !id.is_empty() && !id.chars().any(|ch| ch.is_whitespace() || ch.is_control())
    }));
    assert_eq!(waiting.updated_at, commit.at);
    assert_eq!(waiting.validation_summary, original.validation_summary);
    assert_eq!(
        waiting.authority_refresh_outcome,
        original.authority_refresh_outcome
    );
    assert_eq!(waiting.distributor_outcome, original.distributor_outcome);
    assert_eq!(waiting.history.len(), original.history.len() + 1);
    assert_eq!(
        &waiting.history[..original.history.len()],
        original.history.as_slice()
    );
    assert_eq!(f.base.counters(), counters);
    assert_eq!(
        reopened.pending_events(&f.base.workspace, 100).unwrap(),
        events
    );
    assert_eq!(
        reopened
            .load_host_verification(&f.base.workspace, &receipts[2].attempt.attempt_id)
            .unwrap()
            .unwrap()
            .receipt
            .as_ref(),
        Some(&receipts[2])
    );
    let persisted = session_rows(&f);
    let replay = reopened
        .apply_host_verification_failure_limit(&f.base.workspace, &commit)
        .unwrap();
    assert!(!replay.changed);
    assert_eq!(session_rows(&f), persisted);
}

#[test]
fn failure_limit_rejects_missing_ambiguous_or_invalid_session_without_pausing_the_task() {
    for invalid in [
        "missing",
        "ambiguous",
        "row-slot",
        "row-key",
        "missing-thread",
        "blank-thread",
        "whitespace-thread",
        "control-thread",
        "oversized-thread",
        "changed-agent",
        "changed-lease-time",
        "changed-json-key",
    ] {
        let f = fixture(&format!("host-failure-session-{invalid}"));
        let receipts = three_failures(&f);
        let mut detail = session_detail(&f);
        let connection = authority_connection(&f.base.workspace);
        match invalid {
            "missing" => {
                connection
                    .execute("DELETE FROM runtime_session_details", [])
                    .unwrap();
            }
            "ambiguous" => {
                connection
                    .execute(
                        "INSERT INTO runtime_session_details(session_key,slot_id,updated_at,content)
                         SELECT 'aliased-session',slot_id,updated_at,content FROM runtime_session_details",
                        [],
                    )
                    .unwrap();
            }
            "row-slot" => {
                connection
                    .execute(
                        "UPDATE runtime_session_details SET slot_id='other-slot'",
                        [],
                    )
                    .unwrap();
            }
            "row-key" => {
                connection
                    .execute(
                        "UPDATE runtime_session_details SET session_key='other-session'",
                        [],
                    )
                    .unwrap();
            }
            _ => {
                match invalid {
                    "missing-thread" => detail.thread_id = None,
                    "blank-thread" => detail.thread_id = Some(String::new()),
                    "whitespace-thread" => detail.thread_id = Some("thread worker".into()),
                    "control-thread" => detail.thread_id = Some("thread\0worker".into()),
                    "oversized-thread" => detail.thread_id = Some("t".repeat(1025)),
                    "changed-agent" => detail.agent_id = "another-agent".into(),
                    "changed-lease-time" => detail.lease_started_at = "2026-11-23T11:59:59Z".into(),
                    "changed-json-key" => detail.session_key = "another-session".into(),
                    _ => unreachable!(),
                }
                connection
                    .execute(
                        "UPDATE runtime_session_details SET content=?1",
                        [serde_json::to_string(&detail).unwrap()],
                    )
                    .unwrap();
            }
        }
        drop(connection);
        let before_task = task(&f);
        let before_revision = planning_revision(&f);
        let before_rows = session_rows(&f);
        let before_counters = f.base.counters();
        let before_events = f
            .base
            .adapter
            .pending_events(&f.base.workspace, 100)
            .unwrap();
        assert!(
            f.base
                .adapter
                .apply_host_verification_failure_limit(
                    &f.base.workspace,
                    &request(&f, &receipts[2])
                )
                .is_err(),
            "{invalid}"
        );
        assert_eq!(task(&f), before_task, "{invalid}");
        assert_eq!(planning_revision(&f), before_revision, "{invalid}");
        assert_eq!(session_rows(&f), before_rows, "{invalid}");
        assert_eq!(f.base.counters(), before_counters, "{invalid}");
        assert_eq!(
            f.base
                .adapter
                .pending_events(&f.base.workspace, 100)
                .unwrap(),
            before_events,
            "{invalid}"
        );
        assert!(marker(&f, &receipts[2]).is_none(), "{invalid}");
    }
}

#[test]
fn failure_limit_preserves_other_lease_sessions_and_changes_only_the_exact_current_one() {
    let f = fixture("host-failure-session-history");
    let receipts = three_failures(&f);
    let mut prior = session_detail(&f);
    prior.session_key = "historical-session".into();
    prior.lease_started_at = "2026-11-23T11:00:00Z".into();
    prior.thread_id = Some("historical-thread".into());
    f.base
        .adapter
        .upsert_runtime_session_detail(&f.base.workspace, &prior)
        .unwrap();
    assert!(apply(&f, &receipts[2]).changed);
    let runtime =
        SqlitePlanningAuthorityAdapter::load_runtime_projections(&f.base.workspace).unwrap();
    assert_eq!(runtime.session_details.len(), 2);
    assert_eq!(
        runtime
            .session_details
            .iter()
            .find(|detail| detail.session_key == prior.session_key),
        Some(&prior)
    );
    let current = runtime
        .session_details
        .iter()
        .find(|detail| detail.session_key == f.base.lease.session_key())
        .unwrap();
    assert_eq!(current.state_label, "awaiting_user");
    assert_eq!(current.completion_state_label, "awaiting_user");
}
