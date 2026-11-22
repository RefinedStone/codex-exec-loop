use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use serde_json::{Value, json};

use super::*;
use crate::domain::issue_automation::feedback::{
    ISSUE_FEEDBACK_PROGRESS_LABEL, IssueFeedbackCommentOwner, feedback_body_hash, feedback_marker,
};
use crate::domain::issue_automation::source::IssueSourceLabel;
use crate::domain::issue_work::{IssueProjectScope, IssueProvider};

mod comments;
mod labels;

type Calls = Arc<Mutex<Vec<Vec<String>>>>;

fn adapter_with(responses: Vec<Result<Value>>) -> (GithubIssueFeedbackAdapter, Calls) {
    let calls = Calls::default();
    let recorded = calls.clone();
    let responses = Mutex::new(VecDeque::from(responses));
    let adapter = GithubIssueFeedbackAdapter::with_runner(Box::new(move |arguments| {
        recorded.lock().unwrap().push(arguments.to_vec());
        responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("the fake GitHub has no more answers")
            .map(|value| serde_json::to_vec(&value).unwrap())
    }));
    (adapter, calls)
}

fn issue() -> IssueReference {
    IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".to_string(),
        repository_id: "R_1".to_string(),
        issue_id: "I_1".to_string(),
        project_scope: IssueProjectScope {
            workspace_dir: "/workspace".to_string(),
        },
    }
}

fn issue_node() -> Value {
    json!({"__typename":"Issue","id":"I_1","repository":{"id":"R_1"}})
}

fn actor(id: &str) -> Value {
    json!({"__typename":"User","id":id})
}

fn viewer(id: &str) -> Value {
    json!({"data":{"viewer":actor(id),"node":issue_node()}})
}

fn body(intent_id: &str) -> String {
    format!(
        "Shared status without private text.\n\n{}",
        feedback_marker(intent_id)
    )
}

fn create_intent() -> IssueFeedbackIntent {
    let body = body("intent-1");
    IssueFeedbackIntent {
        intent_id: "intent-1".to_string(),
        issue: issue(),
        actor_id: "U_1".to_string(),
        automation_policy_version: "a".repeat(64),
        disclosure_policy_version: "b".repeat(64),
        desired_revision: 1,
        operation: IssueFeedbackOperation::CreateComment {
            body_sha256: feedback_body_hash(&body),
            content_sha256: feedback_body_hash("Shared status without private text."),
            body,
        },
        recorded_at: "2026-11-21T14:00:00Z".to_string(),
    }
}

fn update_intent() -> IssueFeedbackIntent {
    let mut intent = create_intent();
    let old_body = body("intent-old");
    let IssueFeedbackOperation::CreateComment {
        body,
        body_sha256,
        content_sha256,
    } = intent.operation
    else {
        unreachable!()
    };
    intent.operation = IssueFeedbackOperation::UpdateComment {
        owner: Box::new(IssueFeedbackCommentOwner {
            issue: issue(),
            comment_id: "IC_1".to_string(),
            actor_id: "U_1".to_string(),
            latest_intent_id: "intent-old".to_string(),
            body_sha256: feedback_body_hash(&old_body),
            content_sha256: feedback_body_hash("Shared status without private text."),
        }),
        body,
        body_sha256,
        content_sha256,
    };
    intent
}

fn comment(id: &str, intent_id: &str, edited: bool) -> Value {
    json!({
        "__typename":"IssueComment","id":id,"body":body(intent_id),
        "author":actor("U_1"),"editor":if edited { actor("U_1") } else { Value::Null },
        "lastEditedAt":if edited { json!("2026-11-21T14:01:00Z") } else { Value::Null },
        "issue":issue_node(),"repository":{"id":"R_1"},"pullRequest":null
    })
}

fn node_response(node: Value) -> Value {
    json!({"data":{"node":node}})
}

fn comment_ack(comment: Value, updated: bool) -> Value {
    let mut payload = json!({"clientMutationId":"intent-1"});
    if updated {
        payload["issueComment"] = comment;
    } else {
        payload["commentEdge"] = json!({"node":comment});
    }
    json!({"data":{"feedback":payload}})
}

fn comment_page(comments: Vec<Value>, total: usize, next: Option<&str>) -> Value {
    let mut node = issue_node();
    node["comments"] = json!({
        "nodes":comments,"totalCount":total,
        "pageInfo":{"hasNextPage":next.is_some(),"endCursor":next}
    });
    node_response(node)
}

fn progress_label() -> IssueSourceLabel {
    IssueSourceLabel {
        id: "LA_1".to_string(),
        name: ISSUE_FEEDBACK_PROGRESS_LABEL.to_string(),
    }
}

fn remote_label() -> Value {
    json!({"id":"LA_1","name":ISSUE_FEEDBACK_PROGRESS_LABEL,"repository":{"id":"R_1"}})
}

fn resolve_label(label: Value) -> Value {
    let mut node = issue_node();
    node["repository"]["label"] = label;
    node_response(node)
}

fn label_event(id: &str, actor_id: &str, applied: bool) -> Value {
    json!({
        "__typename":if applied { "LabeledEvent" } else { "UnlabeledEvent" },
        "id":id,"actor":actor(actor_id),"label":remote_label()
    })
}

fn label_pages(present: bool, events: Vec<Value>) -> Vec<Result<Value>> {
    let mut labels = issue_node();
    labels["labels"] = json!({
        "nodes":if present { vec![remote_label()] } else { Vec::<Value>::new() },
        "totalCount":usize::from(present),
        "pageInfo":{"hasNextPage":false,"endCursor":null}
    });
    let mut timeline = issue_node();
    let count = events.len();
    timeline["timelineItems"] = json!({
        "nodes":events,"totalCount":count,
        "pageInfo":{"hasNextPage":false,"endCursor":null}
    });
    vec![
        Ok(resolve_label(remote_label())),
        Ok(node_response(labels)),
        Ok(node_response(timeline)),
    ]
}

fn label_intent(applied: bool, previous: Option<&str>) -> IssueFeedbackIntent {
    let mut intent = create_intent();
    intent.operation = if applied {
        IssueFeedbackOperation::AddLabel {
            label: progress_label(),
            previous_event_id: previous.map(str::to_string),
        }
    } else {
        IssueFeedbackOperation::RemoveLabel {
            label: progress_label(),
            owned_event_id: previous.expect("removal needs owned event").to_string(),
        }
    };
    intent
}

fn label_ack() -> Value {
    json!({"data":{"feedback":{"clientMutationId":"intent-1","labelable":issue_node()}}})
}

fn mutation_count(calls: &Calls) -> usize {
    calls
        .lock()
        .unwrap()
        .iter()
        .filter(|arguments| {
            arguments
                .iter()
                .any(|argument| argument.contains("mutation AkraFeedback"))
        })
        .count()
}

fn confirmed_receipt(result: IssueFeedbackReadResult) -> IssueFeedbackReceipt {
    match result {
        IssueFeedbackReadResult::Confirmed(receipt) => *receipt,
        other => panic!("expected confirmed receipt, got {other:?}"),
    }
}

#[test]
fn actor_lookup_requires_the_exact_github_issue() {
    let (adapter, calls) = adapter_with(vec![Ok(viewer("U_1"))]);
    assert_eq!(adapter.actor_id(&issue()).unwrap(), "U_1");
    assert_eq!(mutation_count(&calls), 0);
    let mut wrong = viewer("U_1");
    wrong["data"]["node"]["repository"]["id"] = json!("R_other");
    let (adapter, calls) = adapter_with(vec![Ok(wrong)]);
    assert!(adapter.actor_id(&issue()).is_err());
    assert_eq!(mutation_count(&calls), 0);
}

#[test]
fn unsupported_target_or_invalid_intent_never_reaches_github() {
    let (adapter, calls) = adapter_with(Vec::new());
    let mut intent = create_intent();
    intent.issue.server_id = "example.invalid".to_string();
    assert!(adapter.write(&intent).is_err());
    intent = label_intent(true, None);
    let IssueFeedbackOperation::AddLabel { label, .. } = &mut intent.operation else {
        unreachable!()
    };
    label.name = "akra:auto".to_string();
    assert!(adapter.write(&intent).is_err());
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn account_change_after_intent_blocks_all_mutations() {
    let (adapter, calls) = adapter_with(vec![Ok(viewer("U_other"))]);
    assert_eq!(
        adapter.write(&create_intent()).unwrap(),
        IssueFeedbackWriteResult::Contradictory
    );
    assert_eq!(mutation_count(&calls), 0);
}

#[test]
fn provider_errors_and_partial_graphql_data_do_not_expose_remote_text() {
    let mut partial = viewer("U_1");
    partial["errors"] = json!([{"message":"FAKE_GH_RESPONSE_SECRET"}]);
    for response in [Ok(partial), Err(anyhow!("FAKE_GH_RESPONSE_SECRET"))] {
        let (adapter, calls) = adapter_with(vec![response]);
        let error = adapter.actor_id(&issue()).unwrap_err().to_string();
        assert!(!error.contains("FAKE_GH_RESPONSE_SECRET"));
        assert_eq!(mutation_count(&calls), 0);
    }
}
