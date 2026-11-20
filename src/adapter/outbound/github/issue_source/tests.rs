use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use serde_json::{Value, json};

use super::*;
use crate::domain::issue_automation::source::IssueDiscoverySelection;
use crate::domain::issue_policy::{IssueDisclosurePolicy, IssueSensitiveSpanAction};
use crate::domain::issue_work::IssueProjectScope;

type Calls = Arc<Mutex<Vec<Vec<String>>>>;
enum Reply {
    Json(Value),
    Bytes(Vec<u8>),
    Failure(&'static str),
}

fn fake(replies: Vec<Reply>) -> (GithubIssueSourceAdapter, Calls) {
    let calls: Calls = Arc::default();
    let captured = calls.clone();
    let replies = Mutex::new(VecDeque::from(replies));
    let adapter = GithubIssueSourceAdapter::with_runner(Box::new(move |arguments| {
        captured.lock().unwrap().push(arguments.to_vec());
        match replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected source call")
        {
            Reply::Json(value) => Ok(serde_json::to_vec(&value).unwrap()),
            Reply::Bytes(bytes) => Ok(bytes),
            Reply::Failure(reason) => Err(anyhow!(reason)),
        }
    }));
    (adapter, calls)
}

fn disclosure() -> IssueSourceDisclosure {
    IssueSourceDisclosure {
        policy: IssueDisclosurePolicy::builtin(),
        policy_version: "policy-v1".into(),
    }
}

fn scope() -> IssueProjectScope {
    IssueProjectScope {
        workspace_dir: "/work/project".into(),
    }
}

fn repository() -> IssueSourceRepository {
    IssueSourceRepository {
        provider: IssueProvider::GitHub,
        server_id: "github.com".into(),
        repository_id: "R_1".into(),
        project_scope: scope(),
    }
}

fn reference(id: &str) -> IssueReference {
    IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".into(),
        repository_id: "R_1".into(),
        issue_id: id.into(),
        project_scope: scope(),
    }
}

fn discovery_request() -> IssueDiscoveryRequest {
    IssueDiscoveryRequest {
        target: IssueDiscoveryTarget::GitHubLocator {
            owner: "sample".into(),
            name: "widgets".into(),
            project_scope: scope(),
            expected_repository_id: None,
        },
        selection: IssueDiscoverySelection::AllOpen,
        cursor: None,
        page_size: 1,
        disclosure: disclosure(),
    }
}

fn metadata_json(nodes: Vec<Value>) -> Value {
    json!({"nodes":nodes,"pageInfo":{"hasNextPage":false,"hasPreviousPage":false}})
}

fn label_json(id: &str, name: &str) -> Value {
    json!({"id":id,"name":name})
}

fn event(id: &str, actor_id: &str, actor_login: &str, kind: &str) -> Value {
    json!({"__typename":kind,"id":id,"createdAt":"2026-11-20T00:00:00Z",
        "actor":{"id":actor_id,"login":actor_login},"label":label_json("LA_1","akra:auto")})
}

fn issue(id: &str) -> Value {
    json!({
        "__typename":"Issue","id":id,"title":"작업 제목","body":"가릴 값이 없는 본문",
        "state":"OPEN","updatedAt":"2026-11-20T01:00:00Z","repository":{"id":"R_1"},
        "author":{"id":"U_writer","login":"writer"},
        "labels":metadata_json(vec![label_json("LA_1","akra:auto")]),
        "assignees":metadata_json(vec![json!({"id":"U_assignee","login":"assignee"})]),
        "timelineItems":metadata_json(vec![event("LE_1","U_labeler","labeler","LabeledEvent")]),
        "closedByPullRequestsReferences":{"totalCount":0}
    })
}

fn page(nodes: Vec<Value>, stable: bool, more: bool, cursor: Option<&str>) -> Value {
    let repository = json!({"__typename":"Repository","id":"R_1","issues":{
        "nodes":nodes,"pageInfo":{"endCursor":cursor,"hasNextPage":more}}});
    if stable {
        json!({"data":{"node":repository}})
    } else {
        json!({"data":{"repository":repository}})
    }
}

fn read_reply(node: Value) -> Value {
    json!({"data":{"node":node}})
}

fn observed(value: IssueSourceRead) -> IssueSourceObservation {
    match value {
        IssueSourceRead::Observed(value) => *value,
        other => panic!("expected an observation: {other:?}"),
    }
}

fn read(adapter: &GithubIssueSourceAdapter) -> IssueSourceRead {
    adapter.resync(IssueResyncRequest {
        issue: reference("I_1"),
        disclosure: disclosure(),
    })
}

fn argument<'a>(arguments: &'a [String], key: &str) -> Option<&'a str> {
    arguments
        .iter()
        .find_map(|value| value.strip_prefix(&format!("{key}=")))
}

#[test]
fn two_pages_bind_the_cursor_to_stable_repository_and_use_one_read_each() {
    let (adapter, calls) = fake(vec![
        Reply::Json(page(vec![issue("I_1")], false, true, Some("cursor-one"))),
        Reply::Json(page(vec![issue("I_2")], true, false, Some("cursor-two"))),
    ]);
    let first = adapter.discover(discovery_request()).unwrap();
    assert_eq!(first.repository, repository());
    assert_eq!(first.observations[0].issue, reference("I_1"));
    let second = adapter
        .discover(IssueDiscoveryRequest {
            target: IssueDiscoveryTarget::Repository(first.repository),
            cursor: first.next_cursor,
            ..discovery_request()
        })
        .unwrap();
    assert_eq!(second.observations[0].issue, reference("I_2"));
    assert!(second.next_cursor.is_none());

    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(argument(&calls[0], "owner"), Some("sample"));
    assert_eq!(argument(&calls[0], "first"), Some("1"));
    assert_eq!(argument(&calls[1], "id"), Some("R_1"));
    assert_eq!(argument(&calls[1], "after"), Some("cursor-one"));
    assert!(argument(&calls[1], "owner").is_none());
    for arguments in calls.iter() {
        let query = argument(arguments, "query").unwrap();
        assert!(query.contains("states: [OPEN]"));
        assert!(query.contains("field: UPDATED_AT, direction: ASC"));
        assert!(query.contains("labels(first: 100)"));
        assert!(query.contains("timelineItems(last: 100"));
        assert!(query.contains("includeClosedPrs: true"));
        assert!(!query.contains("mutation"));
        assert!(!query.contains("comments("));
    }
}

#[test]
fn labels_any_is_or_and_an_empty_filtered_page_still_advances() {
    let mut first = issue("I_1");
    first["labels"] = metadata_json(vec![label_json("LA_other", "other")]);
    let mut second = issue("I_2");
    second["labels"] = metadata_json(vec![label_json("LA_bug", "bug")]);
    let (adapter, calls) = fake(vec![
        Reply::Json(page(vec![first], false, true, Some("next"))),
        Reply::Json(page(vec![second], true, false, None)),
    ]);
    let request = IssueDiscoveryRequest {
        selection: IssueDiscoverySelection::LabelsAny(vec!["todo".into(), "bug".into()]),
        ..discovery_request()
    };
    let first = adapter.discover(request.clone()).unwrap();
    assert!(first.observations.is_empty());
    assert_eq!(first.scanned_count, 1);
    let second = adapter
        .discover(IssueDiscoveryRequest {
            target: IssueDiscoveryTarget::Repository(first.repository),
            cursor: first.next_cursor,
            ..request
        })
        .unwrap();
    assert_eq!(second.observations.len(), 1);
    assert_eq!(second.observations[0].issue.issue_id, "I_2");
    assert_eq!(calls.lock().unwrap().len(), 2);
}

#[test]
fn resync_reports_closed_and_removed_labels_without_the_discovery_filter() {
    let mut closed = issue("I_1");
    closed["state"] = json!("CLOSED");
    closed["labels"] = metadata_json(vec![]);
    closed["timelineItems"] = metadata_json(vec![]);
    closed["closedByPullRequestsReferences"] = json!({"totalCount":2});
    let mut reopened = closed.clone();
    reopened["state"] = json!("OPEN");
    let (adapter, calls) = fake(vec![
        Reply::Json(read_reply(closed)),
        Reply::Json(read_reply(reopened)),
    ]);
    let closed = observed(read(&adapter));
    assert_eq!(closed.remote_state, IssueRemoteState::Closed);
    assert!(closed.labels.items.is_empty());
    assert_eq!(
        closed.labels.completeness,
        IssueMetadataCompleteness::Complete
    );
    assert_eq!(closed.linked_pull_request, IssuePrPresence::Present);
    assert_eq!(
        observed(read(&adapter)).remote_state,
        IssueRemoteState::Open
    );
    for arguments in calls.lock().unwrap().iter() {
        assert_eq!(argument(arguments, "id"), Some("I_1"));
        let query = argument(arguments, "query").unwrap();
        assert!(!query.contains("states: [OPEN]"));
        assert!(!query.contains("issues(first:"));
    }
}

#[test]
fn read_failures_never_reuse_a_previous_success_or_expose_external_errors() {
    let mut partial = read_reply(issue("I_1"));
    partial["errors"] = json!([{"message":"provider-secret-sentinel"}]);
    let (adapter, calls) = fake(vec![
        Reply::Json(read_reply(issue("I_1"))),
        Reply::Failure("HTTP 403 provider-secret-sentinel"),
        Reply::Failure("HTTP 404 provider-secret-sentinel"),
        Reply::Failure("timeout provider-secret-sentinel"),
        Reply::Bytes(b"{invalid provider-secret-sentinel".to_vec()),
        Reply::Json(partial),
        Reply::Json(json!({"data":{"node":null}})),
    ]);
    assert!(matches!(read(&adapter), IssueSourceRead::Observed(_)));
    for reason in [
        IssueSourceFailure::ProviderUnavailable,
        IssueSourceFailure::ProviderUnavailable,
        IssueSourceFailure::ProviderUnavailable,
        IssueSourceFailure::MalformedResponse,
        IssueSourceFailure::ProviderUnavailable,
        IssueSourceFailure::ProviderUnavailable,
    ] {
        let result = read(&adapter);
        assert_eq!(
            result,
            IssueSourceRead::Unknown {
                issue: reference("I_1"),
                reason
            }
        );
        let wire = serde_json::to_string(&result).unwrap();
        assert!(!wire.contains("provider-secret-sentinel"));
        assert!(!wire.contains("작업 제목"));
    }
    assert_eq!(calls.lock().unwrap().len(), 7);
}

#[test]
fn source_identity_and_required_fields_fail_closed() {
    let mut cases = Vec::new();
    let mut changed_id = issue("I_other");
    cases.push((changed_id.clone(), IssueSourceFailure::IdentityMismatch));
    changed_id["id"] = json!("I_1");
    changed_id["repository"]["id"] = json!("R_other");
    cases.push((changed_id, IssueSourceFailure::IdentityMismatch));
    let mut pull_request = issue("I_1");
    pull_request["__typename"] = json!("PullRequest");
    cases.push((pull_request, IssueSourceFailure::IdentityMismatch));
    let mut invalid_state = issue("I_1");
    invalid_state["state"] = json!("secret-sentinel");
    cases.push((invalid_state, IssueSourceFailure::MalformedResponse));
    let mut invalid_time = issue("I_1");
    invalid_time["updatedAt"] = json!("2026-99-99T99:99:99Z");
    cases.push((invalid_time, IssueSourceFailure::MalformedResponse));
    let mut no_author_field = issue("I_1");
    no_author_field.as_object_mut().unwrap().remove("author");
    cases.push((no_author_field, IssueSourceFailure::MalformedResponse));
    for (node, reason) in cases {
        let (adapter, _) = fake(vec![Reply::Json(read_reply(node))]);
        let result = read(&adapter);
        assert_eq!(
            result,
            IssueSourceRead::Unknown {
                issue: reference("I_1"),
                reason
            }
        );
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("secret-sentinel")
        );
    }
}

#[test]
fn shared_copy_removes_secrets_while_hashing_exact_original_fields() {
    let raw_title = "장애 ghp_SyntheticCanary0000";
    let raw_body = "password: synthetic-password-AAAA1111\r\n한글";
    let mut first = issue("I_1");
    first["title"] = json!(raw_title);
    first["body"] = json!(raw_body);
    let mut second = first.clone();
    second["title"] = json!("장애 ghp_SyntheticCanary9999");
    let (adapter, _) = fake(vec![
        Reply::Json(read_reply(first)),
        Reply::Json(read_reply(second)),
    ]);
    let first = observed(read(&adapter));
    let second = observed(read(&adapter));
    let (IssueSourceContent::Shared(first), IssueSourceContent::Shared(second)) =
        (first.content, second.content)
    else {
        panic!("built-in disclosure must produce masked shared copies");
    };
    assert_eq!(first.shared_copy.title, "장애 [removed]");
    assert_eq!(first.shared_copy.body, "password: [removed]\r\n한글");
    assert_eq!(
        first.basis.field_hash,
        IssueFieldHash::of_fields(raw_title, raw_body)
    );
    assert_eq!(first.shared_copy, second.shared_copy);
    assert_ne!(first.basis.field_hash, second.basis.field_hash);
    let wire = serde_json::to_string(&first).unwrap();
    assert!(!wire.contains("SyntheticCanary"));
    assert!(!wire.contains("synthetic-password"));
}

#[test]
fn confirmation_required_content_is_withheld_without_losing_observed_state() {
    let mut node = issue("I_1");
    node["title"] = json!("ghp_SyntheticCanary0000");
    node["state"] = json!("CLOSED");
    let (adapter, _) = fake(vec![Reply::Json(read_reply(node))]);
    let mut disclosure = disclosure();
    disclosure.policy.secret = IssueSensitiveSpanAction::UserConfirm;
    let result = observed(adapter.resync(IssueResyncRequest {
        issue: reference("I_1"),
        disclosure,
    }));
    assert_eq!(result.remote_state, IssueRemoteState::Closed);
    assert!(matches!(
        &result.content,
        IssueSourceContent::Withheld {
            reason: IssueContentWithheldReason::UserConfirmationRequired,
            ..
        }
    ));
    let wire = serde_json::to_string(&result).unwrap();
    assert!(!wire.contains("SyntheticCanary"));
    assert!(!wire.contains("sharedCopy"));
}

#[test]
fn incomplete_metadata_is_distinct_from_a_complete_empty_result() {
    let mut node = issue("I_1");
    node["labels"]["pageInfo"]["hasNextPage"] = json!(true);
    node["assignees"] = Value::Null;
    node["author"] = Value::Null;
    node["timelineItems"] = metadata_json(vec![]);
    node["timelineItems"]["pageInfo"]["hasPreviousPage"] = json!(true);
    node["closedByPullRequestsReferences"] = Value::Null;
    let (adapter, _) = fake(vec![
        Reply::Json(read_reply(node.clone())),
        Reply::Json(page(vec![node], false, false, None)),
    ]);
    let item = observed(read(&adapter));
    assert_eq!(
        item.labels.completeness,
        IssueMetadataCompleteness::Truncated
    );
    assert_eq!(
        item.assignees.completeness,
        IssueMetadataCompleteness::Unavailable
    );
    assert_eq!(
        item.label_actors.completeness,
        IssueMetadataCompleteness::Truncated
    );
    assert!(item.label_actors.items.is_empty());
    assert!(item.author.is_none());
    assert_eq!(item.linked_pull_request, IssuePrPresence::Unknown);
    let result = adapter.discover(IssueDiscoveryRequest {
        selection: IssueDiscoverySelection::LabelsAny(vec!["not-in-page".into()]),
        ..discovery_request()
    });
    assert_eq!(result, Err(IssueSourceFailure::MetadataIncomplete));
}

#[test]
fn latest_label_actor_comes_only_from_actual_current_label_events() {
    let mut node = issue("I_1");
    node["timelineItems"] = metadata_json(vec![
        event("LE_old", "U_trusted", "trusted-maintainer", "LabeledEvent"),
        event("LE_remove", "U_other", "other", "UnlabeledEvent"),
        event("LE_latest", "U_new", "actual-labeler", "LabeledEvent"),
    ]);
    let mut contradictory = node.clone();
    contradictory["timelineItems"]["nodes"]
        .as_array_mut()
        .unwrap()
        .push(event("LE_final", "U_other", "other", "UnlabeledEvent"));
    let mut deleted_actor = node.clone();
    deleted_actor["timelineItems"]["nodes"][2]["actor"] = Value::Null;
    let mut missing_tail = node.clone();
    missing_tail["timelineItems"]["pageInfo"]["hasNextPage"] = json!(true);
    let mut unavailable_event = node.clone();
    unavailable_event["timelineItems"]["nodes"]
        .as_array_mut()
        .unwrap()
        .push(Value::Null);
    let (adapter, _) = fake(
        vec![
            node,
            contradictory,
            deleted_actor,
            missing_tail,
            unavailable_event,
        ]
        .into_iter()
        .map(|node| Reply::Json(read_reply(node)))
        .collect(),
    );
    let first = observed(read(&adapter));
    assert_eq!(first.label_actors.items.len(), 1);
    let attribution = &first.label_actors.items[0];
    assert_eq!(attribution.event_id, "LE_latest");
    assert_eq!(attribution.actor.as_ref().unwrap().login, "actual-labeler");
    assert_ne!(attribution.actor, first.author);
    let contradictory = observed(read(&adapter));
    assert_eq!(
        contradictory.label_actors.completeness,
        IssueMetadataCompleteness::Unavailable
    );
    assert!(contradictory.label_actors.items.is_empty());
    assert!(
        observed(read(&adapter)).label_actors.items[0]
            .actor
            .is_none()
    );
    for _ in 0..2 {
        let result = observed(read(&adapter));
        assert_eq!(
            result.label_actors.completeness,
            IssueMetadataCompleteness::Unavailable
        );
        assert!(result.label_actors.items.is_empty());
    }
}

#[test]
fn broken_pages_never_return_a_next_cursor_or_partial_success() {
    let request = discovery_request();
    let mut cases = vec![
        (page(vec![issue("I_1")], false, true, None), request.clone()),
        (page(vec![], false, true, Some("next")), request.clone()),
        (page(vec![Value::Null], false, false, None), request.clone()),
        (
            page(vec![issue("I_1"), issue("I_2")], false, false, None),
            request.clone(),
        ),
        (
            page(vec![issue("I_1")], false, true, Some("bad\ncursor")),
            request.clone(),
        ),
    ];
    let two = IssueDiscoveryRequest {
        page_size: 2,
        ..request.clone()
    };
    cases.push((
        page(vec![issue("I_1"), issue("I_1")], false, false, None),
        two.clone(),
    ));
    let mut older = issue("I_2");
    older["updatedAt"] = json!("2026-11-19T00:00:00Z");
    cases.push((page(vec![issue("I_1"), older], false, false, None), two));
    let same_cursor = IssueDiscoveryCursor::new(
        repository(),
        request.selection.clone(),
        1,
        &disclosure(),
        "same",
    )
    .unwrap();
    cases.push((
        page(vec![issue("I_1")], true, true, Some("same")),
        IssueDiscoveryRequest {
            target: IssueDiscoveryTarget::Repository(repository()),
            cursor: Some(same_cursor),
            ..request
        },
    ));
    for (value, request) in cases {
        let (adapter, _) = fake(vec![Reply::Json(value)]);
        assert_eq!(
            adapter.discover(request),
            Err(IssueSourceFailure::MalformedResponse)
        );
    }
}

#[test]
fn discovery_rejects_partial_graphql_errors_and_wrong_repository_identity() {
    let mut partial = page(vec![issue("I_1")], false, false, None);
    partial["errors"] = json!([{"message":"raw-error-canary"}]);
    let mut moved = page(vec![issue("I_1")], false, false, None);
    moved["data"]["repository"]["id"] = json!("R_other");
    let expected = IssueDiscoveryRequest {
        target: IssueDiscoveryTarget::GitHubLocator {
            owner: "sample".into(),
            name: "widgets".into(),
            project_scope: scope(),
            expected_repository_id: Some("R_1".into()),
        },
        ..discovery_request()
    };
    for (reply, reason) in [
        (partial, IssueSourceFailure::ProviderUnavailable),
        (
            json!({"data":{"repository":null}}),
            IssueSourceFailure::ProviderUnavailable,
        ),
        (moved, IssueSourceFailure::IdentityMismatch),
    ] {
        let (adapter, _) = fake(vec![Reply::Json(reply)]);
        let result = adapter.discover(expected.clone());
        assert_eq!(result, Err(reason));
        assert!(!format!("{result:?}").contains("raw-error-canary"));
    }
}

#[test]
fn invalid_or_rebound_requests_are_rejected_before_provider_access() {
    let (adapter, calls) = fake(vec![]);
    let original = discovery_request();
    for page_size in [0, 101] {
        assert_eq!(
            adapter.discover(IssueDiscoveryRequest {
                page_size,
                ..original.clone()
            }),
            Err(IssueSourceFailure::InvalidRequest)
        );
    }
    let cursor = IssueDiscoveryCursor::new(
        repository(),
        original.selection.clone(),
        1,
        &disclosure(),
        "after",
    )
    .unwrap();
    assert_eq!(
        adapter.discover(IssueDiscoveryRequest {
            cursor: Some(cursor.clone()),
            ..original.clone()
        }),
        Err(IssueSourceFailure::InvalidCursor)
    );
    let mut changed_disclosure = disclosure();
    changed_disclosure.policy.secret = IssueSensitiveSpanAction::UserConfirm;
    assert_eq!(
        adapter.discover(IssueDiscoveryRequest {
            target: IssueDiscoveryTarget::Repository(repository()),
            cursor: Some(cursor),
            disclosure: changed_disclosure,
            ..original
        }),
        Err(IssueSourceFailure::InvalidCursor)
    );
    let mut unsupported = reference("I_1");
    unsupported.provider = IssueProvider::Jira;
    assert_eq!(
        adapter.resync(IssueResyncRequest {
            issue: unsupported.clone(),
            disclosure: disclosure()
        }),
        IssueSourceRead::Unknown {
            issue: unsupported,
            reason: IssueSourceFailure::UnsupportedProvider
        }
    );
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn source_capabilities_do_not_advertise_writes_or_an_incremental_watermark() {
    let (adapter, calls) = fake(vec![]);
    let caps = adapter.capabilities();
    assert!(caps.discovery && caps.resync);
    assert!(
        !caps.incremental
            && !caps.comments
            && !caps.labels
            && !caps.assign
            && !caps.transition
            && !caps.link_pull_request
            && !caps.board_fields
    );
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
#[ignore = "read-only gh verification; explicit repository and scope environment required"]
fn live_read_only_source_probe() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let owner = std::env::var("AKRA_LIVE_SOURCE_OWNER").expect("explicit repository owner");
    let name = std::env::var("AKRA_LIVE_SOURCE_NAME").expect("explicit repository name");
    let workspace_dir =
        std::env::var("AKRA_LIVE_SOURCE_WORKSPACE").expect("explicit project scope");
    let count = Arc::new(AtomicUsize::new(0));
    let captured = count.clone();
    let working_directory = PathBuf::from(&workspace_dir);
    let adapter = GithubIssueSourceAdapter::with_runner(Box::new(move |arguments| {
        captured.fetch_add(1, Ordering::SeqCst);
        run_gh_api(&working_directory, arguments)
    }));
    let first = adapter
        .discover(IssueDiscoveryRequest {
            target: IssueDiscoveryTarget::GitHubLocator {
                owner,
                name,
                project_scope: IssueProjectScope { workspace_dir },
                expected_repository_id: None,
            },
            ..discovery_request()
        })
        .expect("first read-only discovery page");
    assert_eq!(
        first.observations.len(),
        1,
        "the explicit validation repository must have open issues"
    );
    let reference = first.observations[0].issue.clone();
    let second = adapter
        .discover(IssueDiscoveryRequest {
            target: IssueDiscoveryTarget::Repository(first.repository),
            cursor: Some(
                first
                    .next_cursor
                    .expect("the validation repository must have a second page"),
            ),
            ..discovery_request()
        })
        .expect("second read-only discovery page");
    assert_eq!(second.observations.len(), 1);
    assert_ne!(reference.issue_id, second.observations[0].issue.issue_id);
    let refreshed = observed(adapter.resync(IssueResyncRequest {
        issue: reference.clone(),
        disclosure: disclosure(),
    }));
    assert_eq!(refreshed.issue, reference);
    assert_eq!(count.load(Ordering::SeqCst), 3);
    println!(
        "live source probe: requests=3, first={}, second={}, resync=observed",
        reference.issue_id, second.observations[0].issue.issue_id
    );
}

#[test]
fn assigned_bots_are_preserved_and_unreadable_actors_are_not_a_complete_empty_list() {
    let mut bot = issue("I_1");
    bot["assignees"] = metadata_json(vec![json!({"id":"BOT_1","login":"copilot-synthetic"})]);
    let mut unknown_actor = issue("I_1");
    unknown_actor["assignees"] = metadata_json(vec![json!({"id":"MAN_1"})]);
    let (adapter, calls) = fake(vec![
        Reply::Json(read_reply(bot)),
        Reply::Json(read_reply(unknown_actor)),
    ]);
    let bot = observed(read(&adapter));
    assert_eq!(
        bot.assignees.completeness,
        IssueMetadataCompleteness::Complete
    );
    assert_eq!(bot.assignees.items[0].id, "BOT_1");
    assert_eq!(bot.assignees.items[0].login, "copilot-synthetic");
    let unknown_actor = observed(read(&adapter));
    assert!(unknown_actor.assignees.items.is_empty());
    assert_eq!(
        unknown_actor.assignees.completeness,
        IssueMetadataCompleteness::Unavailable
    );
    for args in calls.lock().unwrap().iter() {
        let query = argument(args, "query").unwrap();
        assert!(query.contains("assignees: assignedActors(first: 100)"));
        assert!(query.contains("... on Actor { login }"));
        assert!(!query.contains("assignees(first:"));
    }
}

#[test]
fn missing_label_actor_is_unavailable_instead_of_a_deleted_account() {
    let mut node = issue("I_1");
    node["timelineItems"]["nodes"][0]
        .as_object_mut()
        .unwrap()
        .remove("actor");
    let (adapter, _) = fake(vec![Reply::Json(read_reply(node))]);
    let value = observed(read(&adapter));
    assert_eq!(
        value.label_actors.completeness,
        IssueMetadataCompleteness::Unavailable
    );
    assert!(value.label_actors.items.is_empty());
}
