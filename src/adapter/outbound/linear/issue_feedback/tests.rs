use std::sync::Mutex;
use std::sync::atomic::Ordering;

use serde_json::json;

use super::*;
use crate::adapter::outbound::linear::test_support::*;
use crate::domain::issue_work::IssueProjectScope;

const COMMENT: &str = "6f1c1a52-0000-4000-8000-00000000f001";

fn issue() -> IssueReference {
    IssueReference {
        provider: IssueProvider::Linear,
        server_id: LINEAR_SERVER_ID.into(),
        repository_id: TEAM.into(),
        issue_id: ISSUE.into(),
        project_scope: IssueProjectScope {
            workspace_dir: "/fixture/workspace".into(),
        },
    }
}

fn closing_binding() -> IssueAutomationLinearBinding {
    IssueAutomationLinearBinding {
        close_state: Some(IssueAutomationLinearCloseState {
            from_state_ids: vec![STARTED.into()],
            to_state_id: DONE.into(),
        }),
        ..binding()
    }
}

struct Fixture {
    adapter: LinearIssueFeedbackAdapter,
    credentials: Arc<Credentials>,
    api: Arc<Api>,
    send_policy: Arc<Mutex<bool>>,
}
impl Fixture {
    fn new() -> Self {
        let credentials = Arc::new(Credentials::new());
        let api = Arc::new(Api::new(credentials.clone()));
        let send_policy = Arc::new(Mutex::new(true));
        let allowed = send_policy.clone();
        let adapter = LinearIssueFeedbackAdapter::new(
            Arc::new(SharedCredentials(credentials.clone())),
            api.clone(),
            Arc::new(|| Ok(closing_binding())),
            Arc::new(move |_| {
                if *allowed.lock().unwrap() {
                    Ok(())
                } else {
                    bail!("policy changed")
                }
            }),
        );
        Self {
            adapter,
            credentials,
            api,
            send_policy,
        }
    }
    /// `context()`: the viewer and the bound issue.
    fn context(&self, state: (&str, &str)) {
        self.api
            .answer(json!({"viewer": {"id": VIEWER, "active": true}}));
        self.issue(state);
    }
    fn issue(&self, (id, kind): (&str, &str)) {
        self.api.answer(json!({"issue": {
            "id": ISSUE, "team": {"id": TEAM}, "state": {"id": id, "type": kind}}}));
    }
    fn comments(&self, nodes: Vec<Value>) {
        self.api.answer(json!({"issue": {"id": ISSUE, "comments": {
            "nodes": nodes, "pageInfo": {"hasNextPage": false, "endCursor": null}}}}));
    }
    fn history(&self, nodes: Vec<Value>) {
        self.api.answer(json!({"issue": {"id": ISSUE, "history": {
            "nodes": nodes, "pageInfo": {"hasNextPage": false, "endCursor": null}}}}));
    }
    fn comment_intent(&self, id: &str, content: &str) -> IssueFeedbackIntent {
        let body = format!("{content}\n\n{}", feedback_marker(id));
        let intent = IssueFeedbackIntent {
            intent_id: id.into(),
            issue: issue(),
            actor_id: VIEWER.into(),
            connection: Some(IssueFeedbackConnection {
                profile_id: "a".repeat(64),
                connection_id: "b".repeat(32),
                revision: 1,
            }),
            automation_policy_version: "a".repeat(64),
            disclosure_policy_version: "b".repeat(64),
            desired_revision: 1,
            operation: IssueFeedbackOperation::CreateComment {
                body_sha256: feedback_body_hash(&body),
                body,
                content_sha256: feedback_body_hash(content),
            },
            recorded_at: "2026-11-23T12:00:00Z".into(),
        };
        intent.validate().unwrap();
        intent
    }
    fn calls(&self) -> Vec<Call> {
        self.api.calls.lock().unwrap().clone()
    }
}

fn comment(body: &str, user: &str) -> Value {
    json!({"id": COMMENT, "body": body, "user": {"id": user}, "issue": {"id": ISSUE}})
}

fn body_of(intent: &IssueFeedbackIntent) -> &str {
    match &intent.operation {
        IssueFeedbackOperation::CreateComment { body, .. } => body,
        _ => unreachable!(),
    }
}

const OPEN: (&str, &str) = (STARTED, "started");
const CLOSED: (&str, &str) = (DONE, "completed");

#[test]
fn actor_context_names_the_exact_viewer_and_credential_generation() {
    let fixture = Fixture::new();
    fixture.context(OPEN);
    let actor = fixture.adapter.actor_context(&issue()).unwrap();
    assert_eq!(actor.actor_id, VIEWER);
    assert_eq!(actor.connection.unwrap().revision, 1);
    let fixture = Fixture::new();
    fixture
        .api
        .answer(json!({"viewer": {"id": "another-viewer", "active": true}}));
    assert!(fixture.adapter.actor_context(&issue()).is_err());
    assert!(
        fixture
            .adapter
            .read_label(&issue(), ISSUE_FEEDBACK_PROGRESS_LABEL)
            .is_err()
    );
}

#[test]
fn a_comment_is_sent_once_and_confirmed_only_by_its_exact_body() {
    let fixture = Fixture::new();
    let intent = fixture.comment_intent("intent-1", "상태: 작업 중");
    fixture.context(OPEN);
    fixture.comments(vec![]);
    fixture.issue(OPEN);
    fixture.api.answer(json!({"commentCreate": {"success": true,
        "comment": comment(body_of(&intent), VIEWER)}}));
    assert_eq!(
        fixture.adapter.write(&intent).unwrap(),
        IssueFeedbackWriteResult::Acknowledged {
            comment_id: Some(COMMENT.into())
        }
    );
    assert_eq!(fixture.credentials.sends.load(Ordering::SeqCst), 1);
    assert!(matches!(
        fixture.calls().last(),
        Some(Call::CreateComment(body)) if body == body_of(&intent)
    ));

    fixture.context(OPEN);
    fixture.comments(vec![comment(body_of(&intent), VIEWER)]);
    fixture.issue(OPEN);
    let IssueFeedbackReadResult::Confirmed(receipt) = fixture.adapter.reconcile(&intent).unwrap()
    else {
        panic!("the exact comment should confirm the intent");
    };
    assert_eq!(
        receipt.evidence,
        IssueFeedbackEvidence::Comment {
            comment_id: COMMENT.into(),
            body_sha256: feedback_body_hash(body_of(&intent)),
        }
    );
}

#[test]
fn a_lost_response_is_unconfirmed_and_readback_never_adopts_a_changed_body() {
    let fixture = Fixture::new();
    let intent = fixture.comment_intent("intent-2", "상태: 작업 중");
    fixture.context(OPEN);
    fixture.comments(vec![]);
    fixture.issue(OPEN);
    fixture.api.fail(ProviderCredentialError::Unavailable);
    assert_eq!(
        fixture.adapter.write(&intent).unwrap(),
        IssueFeedbackWriteResult::Unconfirmed
    );
    // Nothing found: a person confirms; nothing is sent again.
    fixture.context(OPEN);
    fixture.comments(vec![]);
    fixture.issue(OPEN);
    assert_eq!(
        fixture.adapter.reconcile(&intent).unwrap(),
        IssueFeedbackReadResult::NeedsConfirmation
    );
    // Linear stored a different body (e.g. without the marker) or another user wrote it.
    for (body, user) in [
        (format!("{} edited", body_of(&intent)), VIEWER),
        (body_of(&intent).to_owned(), "another-user"),
    ] {
        fixture.context(OPEN);
        fixture.comments(vec![comment(&body, user)]);
        fixture.issue(OPEN);
        assert_eq!(
            fixture.adapter.reconcile(&intent).unwrap(),
            IssueFeedbackReadResult::Contradictory
        );
    }
    assert_eq!(fixture.credentials.sends.load(Ordering::SeqCst), 1);
}

#[test]
fn the_send_boundary_rechecks_policy_and_sends_nothing_when_it_changed() {
    let fixture = Fixture::new();
    let intent = fixture.comment_intent("intent-3", "상태: 작업 중");
    *fixture.send_policy.lock().unwrap() = false;
    fixture.context(OPEN);
    fixture.comments(vec![]);
    fixture.issue(OPEN);
    assert_eq!(
        fixture.adapter.write(&intent).unwrap(),
        IssueFeedbackWriteResult::Unconfirmed
    );
    assert!(
        !fixture
            .calls()
            .iter()
            .any(|call| matches!(call, Call::CreateComment(_)))
    );
}

#[test]
fn an_existing_comment_of_the_same_intent_blocks_a_second_create() {
    let fixture = Fixture::new();
    let intent = fixture.comment_intent("intent-4", "상태: 작업 중");
    fixture.context(OPEN);
    fixture.comments(vec![comment(body_of(&intent), VIEWER)]);
    assert_eq!(
        fixture.adapter.write(&intent).unwrap(),
        IssueFeedbackWriteResult::Contradictory
    );
    assert_eq!(fixture.credentials.sends.load(Ordering::SeqCst), 0);
}

#[test]
fn the_close_state_change_uses_the_granted_completed_state_and_history() {
    let fixture = Fixture::new();
    let snapshot_reads = |fixture: &Fixture, previous: &[Value]| {
        fixture.issue(OPEN);
        fixture.api.answer(json!({"workflowState": {
            "id": DONE, "type": "completed", "team": {"id": TEAM}}}));
        fixture.history(previous.to_vec());
        fixture.issue(OPEN);
    };
    let previous = json!({"id": "6f1c1a52-0000-4000-8000-0000000a0001",
        "createdAt": "2026-11-20T10:00:00Z", "fromStateId": null,
        "toStateId": STARTED, "actorId": VIEWER});
    let actor = IssueFeedbackActorContext {
        actor_id: VIEWER.into(),
        connection: Some(IssueFeedbackConnection {
            profile_id: "a".repeat(64),
            connection_id: "b".repeat(32),
            revision: 1,
        }),
    };
    fixture.context(OPEN);
    snapshot_reads(&fixture, std::slice::from_ref(&previous));
    let snapshot = fixture
        .adapter
        .read_linear_close_state(
            &issue(),
            &actor,
            closing_binding().close_state.as_ref().unwrap(),
        )
        .unwrap();
    assert_eq!(
        snapshot.target,
        IssueFeedbackLinearState {
            from_state_id: STARTED.into(),
            to_state_id: DONE.into(),
        }
    );
    let mut intent = fixture.comment_intent("close-1", "Status");
    intent.operation = IssueFeedbackOperation::CloseIssue {
        integration_record_id: "integration-1".into(),
        integration_sha: "a".repeat(40),
        decision_id: "human-close-1".into(),
        previous_event_id: snapshot.previous_event_id.clone(),
        jira_transition: None,
        linear_state: Some(snapshot.target),
    };
    intent.validate().unwrap();

    fixture.context(OPEN);
    snapshot_reads(&fixture, std::slice::from_ref(&previous));
    fixture.api.answer(json!({"issueUpdate": {"success": true,
        "issue": {"id": ISSUE, "state": {"id": DONE, "type": "completed"}}}}));
    assert_eq!(
        fixture.adapter.write(&intent).unwrap(),
        IssueFeedbackWriteResult::Acknowledged { comment_id: None }
    );
    assert_eq!(
        fixture.calls().last(),
        Some(&Call::UpdateState(DONE.into()))
    );

    let moved = json!({"id": "6f1c1a52-0000-4000-8000-0000000a0002",
        "createdAt": "2026-11-20T11:00:00Z", "fromStateId": STARTED,
        "toStateId": DONE, "actorId": VIEWER});
    fixture.context(CLOSED);
    fixture.issue(CLOSED);
    fixture.history(vec![moved, previous]);
    fixture.issue(CLOSED);
    assert!(matches!(
        fixture.adapter.reconcile(&intent).unwrap(),
        IssueFeedbackReadResult::Confirmed(receipt)
            if receipt.evidence == IssueFeedbackEvidence::Closed {
                event_id: "6f1c1a52-0000-4000-8000-0000000a0002".into()
            }
    ));
}

#[test]
fn a_state_outside_the_grant_or_a_non_completed_target_is_never_sent() {
    let actor = IssueFeedbackActorContext {
        actor_id: VIEWER.into(),
        connection: Some(IssueFeedbackConnection {
            profile_id: "a".repeat(64),
            connection_id: "b".repeat(32),
            revision: 1,
        }),
    };
    let mapping = closing_binding().close_state.unwrap();
    let fixture = Fixture::new();
    fixture.context(("6f1c1a52-0000-4000-8000-00000000d009", "backlog"));
    fixture.issue(("6f1c1a52-0000-4000-8000-00000000d009", "backlog"));
    assert!(
        fixture
            .adapter
            .read_linear_close_state(&issue(), &actor, &mapping)
            .is_err()
    );
    let fixture = Fixture::new();
    fixture.context(OPEN);
    fixture.issue(OPEN);
    fixture.api.answer(json!({"workflowState": {
        "id": DONE, "type": "canceled", "team": {"id": TEAM}}}));
    assert!(
        fixture
            .adapter
            .read_linear_close_state(&issue(), &actor, &mapping)
            .is_err()
    );
    assert_eq!(fixture.credentials.sends.load(Ordering::SeqCst), 0);
}
