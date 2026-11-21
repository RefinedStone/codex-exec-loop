use super::super::tests::{assert_matches_spec, schema_errors};
use super::super::{
    CommandRequest, CommandResult, ERROR_INVALID_PARAMS, KernelCommand, KernelCommandInput,
    KernelEvent, KernelQueryAnswer, ReasonCode, StoredCommand, StoredCommandResult, describe,
    reason_next,
};
use super::*;
use crate::domain::issue_automation::collection::{
    IssueCollectionConfig, IssueCollectionMode, IssueCollectionStatus, IssueResyncCursor,
    IssueTrackedSummary, StoredIssueCollection,
};
use crate::domain::issue_automation::{
    IssueAutomationBindingId, IssueAutomationFlags, IssueAutomationSnapshot,
};
use crate::domain::issue_work::{
    IssueFieldHash, IssueProjectScope, IssueProvider, IssueSharedCopy,
};
use serde_json::{Value, json};

fn reference() -> IssueReference {
    IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".to_string(),
        repository_id: "R_stable".to_string(),
        issue_id: "I_stable".to_string(),
        project_scope: IssueProjectScope {
            workspace_dir: "/workspace".to_string(),
        },
    }
}

fn projection(mode: IssueCollectionMode) -> IssueCollectionProjection {
    let issue = reference();
    let binding = IssueAutomationBinding::new(
        IssueAutomationBindingId::new("origin-binding").unwrap(),
        issue.provider,
        issue.server_id.clone(),
        issue.repository_id.clone(),
        issue.project_scope.clone(),
    )
    .unwrap();
    IssueCollectionProjection {
        catalog_revision: 13,
        state: StoredIssueCollection {
            config: IssueCollectionConfig { mode },
            epoch: 3,
            revision: 7,
            binding_id: Some(binding.id().clone()),
            discovery_cursor: None,
            resync_cursor: Some(IssueResyncCursor {
                after: issue.clone(),
                through: issue,
            }),
            status: IssueCollectionStatus {
                last_attempt_at_ms: Some(100),
                last_success_at_ms: Some(90),
                last_failure: Some(IssueCollectionFailure::Source(
                    IssueSourceFailure::ProviderUnavailable,
                )),
                consecutive_failures: 2,
            },
        },
        binding: Some(binding),
    }
}

fn basis() -> IssueComparisonBasis {
    IssueComparisonBasis {
        field_hash: IssueFieldHash::of_fields("private original title", "private original body"),
        provider_updated_at: "2026-11-20T12:00:00Z".to_string(),
    }
}

fn shared() -> IssueSourceContent {
    IssueSourceContent::Shared(IssueAutomationSnapshot {
        basis: basis(),
        policy_version: "policy-2".to_string(),
        shared_copy: IssueSharedCopy {
            title: "shared title".to_string(),
            body: "shared body".to_string(),
        },
    })
}

fn observation(content: IssueSourceContent) -> IssueSourceRead {
    let label = IssueSourceLabel {
        id: "L_stable".to_string(),
        name: "ready".to_string(),
    };
    IssueSourceRead::Observed(Box::new(IssueSourceObservation {
        creation_provenance:
            crate::domain::issue_automation::source::IssueCreationProvenance::Unattributed,
        issue: reference(),
        web: None,
        remote_state: IssueRemoteState::Closed,
        labels: IssueMetadataList {
            items: vec![label.clone()],
            completeness: IssueMetadataCompleteness::Complete,
        },
        assignees: IssueMetadataList {
            items: vec![IssueSourceActor {
                kind: crate::domain::issue_automation::source::IssueSourceActorKind::UserAccount,
                id: "U_stable".to_string(),
                login: "assignee".to_string(),
            }],
            completeness: IssueMetadataCompleteness::Truncated,
        },
        author: None,
        label_actors: IssueMetadataList {
            items: vec![IssueLabelAttribution {
                label,
                event_id: "LE_stable".to_string(),
                actor: None,
                created_at: "2026-11-20T11:00:00Z".to_string(),
            }],
            completeness: IssueMetadataCompleteness::Complete,
        },
        linked_pull_request: IssuePrPresence::Unknown,
        content,
    }))
}

fn page(current: Option<IssueSourceRead>) -> IssueCollectionPage {
    IssueCollectionPage {
        report: None,
        revision: 7,
        items: vec![IssueCollectionItem {
            work: None,
            open_decisions: Vec::new(),
            observation_revision: None,
            tracked_revision: None,
            triage: None,
            issue: reference(),
            binding_id: IssueAutomationBindingId::new("origin-binding").unwrap(),
            tracked: Some(IssueTrackedSummary {
                state: DomainWorkflowState::Running,
                flags: IssueAutomationFlags {
                    held: true,
                    stale: true,
                    unknown: true,
                },
            }),
            current,
        }],
        next_after: Some(reference()),
    }
}

fn issue_answer(page: &IssueCollectionPage) -> Value {
    serde_json::to_value(KernelQueryAnswer::Issues(IssuesState::from_domain(page))).unwrap()
}

fn encoded_record(value: Value) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(&value).unwrap())
}

#[test]
fn issue_commands_keep_observation_mode_stored_and_sync_runtime_only() {
    for (wire, domain) in [
        ("off", IssueCollectionMode::Off),
        ("observe", IssueCollectionMode::Observe),
    ] {
        let command: KernelCommand = serde_json::from_value(json!({
            "type": "set_issue_automation_mode", "mode": wire,
        }))
        .unwrap();
        assert!(!command.confirmed());
        assert_eq!(
            command.clone().into_input("/workspace").unwrap(),
            KernelCommandInput::Stored(StoredCommand::SetIssueAutomationMode { mode: domain }),
        );
        assert_eq!(
            KernelCommand::from_stored(StoredCommand::SetIssueAutomationMode { mode: domain }),
            command
        );
        let descriptor = CommandRequest {
            command,
            idempotency_key: None,
        }
        .descriptor()
        .unwrap()
        .unwrap();
        assert_eq!(descriptor.command.name(), "set_issue_automation_mode");
        assert!(descriptor.thread_id.is_empty());
    }
    let sync = KernelCommand::SyncIssues {};
    assert_eq!(
        sync.clone().into_input("/workspace").unwrap(),
        KernelCommandInput::SyncIssues
    );
    assert!(
        CommandRequest {
            command: sync,
            idempotency_key: None
        }
        .descriptor()
        .unwrap()
        .is_none()
    );

    for invalid in [
        json!({"type":"set_issue_automation_mode","mode":"propose"}),
        json!({"type":"set_issue_automation_mode","mode":"gated"}),
        json!({"type":"set_issue_automation_mode","mode":"auto"}),
        json!({"type":"set_issue_automation_mode","mode":"observe","confirm":true}),
        json!({"type":"sync_issues","force":true}),
    ] {
        assert!(serde_json::from_value::<KernelCommand>(invalid).is_err());
    }
    assert_eq!(
        serde_json::to_value(reason_next(ReasonCode::IssueAutomationOff)).unwrap(),
        json!([{"step":"query","name":"issue_automation"},{"step":"ask_user"}]),
    );
}

#[test]
fn issue_spec_describes_storage_polling_and_bounded_query_contracts() {
    let spec = serde_json::to_value(describe()).unwrap();
    let entry = |section: &str, name: &str| {
        spec[section]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == name)
            .unwrap()
    };
    let mode = entry("commands", "set_issue_automation_mode");
    assert_eq!(mode["scope"], "issue");
    assert_eq!(mode["state"], "store");
    assert_eq!(mode["effect"], "write");
    assert_eq!(mode["idempotency"], "natural");
    assert_eq!(mode["completion"], "records");
    assert!(mode.get("result").is_some());
    let sync = entry("commands", "sync_issues");
    assert_eq!(sync["state"], "runtime");
    assert_eq!(sync["effect"], "read");
    assert_eq!(sync["idempotency"], "natural");
    assert_eq!(sync["completion"], "accepted");
    assert!(sync.get("result").is_none());
    assert_eq!(
        sync["rejections"],
        json!(["kernel_unreachable", "issue_automation_off"])
    );
    assert_eq!(
        spec["$defs"]["IssueAutomationModeRecord"]["enum"],
        json!(["off", "observe"])
    );
    for limit in [0, 101] {
        let wire = json!({"type":"issues","limit":limit});
        assert!(
            !schema_errors(
                &entry("queries", "issues")["input"],
                &wire,
                &spec["$defs"],
                "issues"
            )
            .is_empty()
        );
    }
}

#[test]
fn mode_result_exposes_the_committed_collection_revision() {
    let result = CommandResult::from_stored(StoredCommandResult::IssueAutomationModeChanged {
        previous: Some(IssueCollectionMode::Observe),
        projection: Box::new(projection(IssueCollectionMode::Off)),
        changed: true,
    });
    let wire = serde_json::to_value(result).unwrap();
    assert_eq!(
        wire,
        json!({
            "type": "issue_automation_mode_changed", "previous": "observe", "mode": "off",
            "epoch": 3, "revision": 7, "changed": true,
        })
    );
    assert!(!wire.to_string().contains("planning_revision"));
    assert!(!wire.to_string().contains("cursor"));
}

#[test]
fn issue_collection_status_uses_the_committed_mode_and_separates_unknown_runtime() {
    let projection = projection(IssueCollectionMode::Off);
    // The binding can still carry its default Observe. Only the committed collection mode is authoritative.
    let offline = IssueAutomationState::from_domain(&projection);
    let wire = serde_json::to_value(&offline).unwrap();
    assert_eq!(wire["status"], "ready");
    assert_eq!(wire["mode"], "off");
    assert_eq!(wire["binding"]["id"], "origin-binding");
    assert_eq!(
        wire["last_failure"],
        json!({"kind":"source","reason":"provider_unavailable"})
    );
    assert!(wire["binding"].get("autonomy").is_none());
    assert!(wire["collecting"].is_null());
    assert!(wire.get("discovery_cursor").is_none());
    assert!(wire.get("resync_cursor").is_none());
    let runtime = offline.with_runtime(false, Some(2500));
    let wire = serde_json::to_value(&runtime).unwrap();
    assert_eq!(wire["collecting"], false);
    assert_eq!(wire["next_poll_after_ms"], 2500);
    assert_eq!(
        serde_json::from_value::<IssueAutomationState>(wire).unwrap(),
        runtime
    );
    assert_eq!(
        IssueAutomationState::unavailable().with_runtime(false, None),
        IssueAutomationState::Unavailable {},
    );
    assert_ne!(
        serde_json::to_value(KernelQueryAnswer::Issues(IssuesState::unavailable())).unwrap(),
        issue_answer(&IssueCollectionPage {
            report: None,
            revision: 7,
            items: vec![],
            next_after: None
        }),
    );
}

#[test]
fn issue_list_cursor_preserves_the_full_stable_reference_and_workspace() {
    let mut issue = reference();
    issue.provider = IssueProvider::Jira;
    issue.server_id = "cloud-id".to_string();
    issue.repository_id = "project-id".to_string();
    issue.issue_id = "stable-id-with-/and-한글".to_string();
    issue.project_scope.workspace_dir = "C:/dev/project".to_string();
    let page = IssueCollectionPage {
        report: None,
        revision: 1,
        items: vec![],
        next_after: Some(issue.clone()),
    };
    let wire = issue_answer(&page);
    let cursor = wire["next_after"].as_str().unwrap();
    assert!(!cursor.contains('='));
    let query = KernelQuery::Issues {
        after: Some(cursor.to_string()),
        limit: None,
    };
    assert_eq!(
        query.issues_range("C:/dev/project").unwrap(),
        Some((Some(issue), 50))
    );
    assert_eq!(
        query.issues_range("/another-workspace").unwrap_err().code,
        ERROR_INVALID_PARAMS
    );
    assert_eq!(
        KernelQuery::Issues {
            after: None,
            limit: Some(100)
        }
        .issues_range("/workspace")
        .unwrap(),
        Some((None, 100))
    );
    assert_eq!(
        KernelQuery::IssueAutomation {}
            .issues_range("/workspace")
            .unwrap(),
        None
    );
    for limit in [0, 101] {
        assert_eq!(
            KernelQuery::Issues {
                after: None,
                limit: Some(limit)
            }
            .issues_range("/workspace")
            .unwrap_err()
            .code,
            ERROR_INVALID_PARAMS,
        );
    }
}

#[test]
fn issue_list_cursor_rejects_other_namespaces_versions_and_invalid_references() {
    let valid = serde_json::to_value(IssueListCursor {
        kind: IssueListCursorKind::IssueList,
        version: 1,
        after: (&reference()).into(),
    })
    .unwrap();
    let mut cases = vec![
        String::new(),
        "not-a-cursor".to_string(),
        "A".repeat(MAX_ISSUES_CURSOR_BYTES + 1),
        encoded_record(json!({"version":1,"repository":{},"opaque":"provider-cursor"})),
    ];
    let mut wrong_kind = valid.clone();
    wrong_kind["kind"] = json!("issue_source");
    let mut wrong_version = valid.clone();
    wrong_version["version"] = json!(2);
    let mut blank = valid.clone();
    blank["after"]["issue_id"] = json!(" ");
    let mut nul = valid.clone();
    nul["after"]["repository_id"] = json!("\u{0}");
    let mut extra = valid.clone();
    extra["after"]["issue_number"] = json!(1);
    let mut missing = valid.clone();
    missing["after"]
        .as_object_mut()
        .unwrap()
        .remove("server_id");
    cases.extend(
        [wrong_kind, wrong_version, blank, nul, extra, missing]
            .into_iter()
            .map(encoded_record),
    );
    for cursor in cases {
        let failure = KernelQuery::Issues {
            after: Some(cursor),
            limit: None,
        }
        .issues_range("/workspace")
        .unwrap_err();
        assert_eq!(failure.code, ERROR_INVALID_PARAMS);
        assert_eq!(failure.message, "invalid issues page cursor");
    }
}

#[test]
fn only_a_current_shared_observation_exposes_a_disclosed_title_and_body() {
    let shared = issue_answer(&page(Some(observation(shared()))));
    let item = &shared["items"][0];
    assert_eq!(item["current"]["status"], "observed");
    assert_eq!(
        item["current"]["content"]["shared_copy"],
        json!({"title":"shared title","body":"shared body"})
    );
    assert_eq!(item["current"]["remote_state"], "closed");
    assert_eq!(
        item["tracked"],
        json!({"state":"running","held":true,"stale":true,"unknown":true})
    );
    assert_eq!(item["current"]["assignees"]["completeness"], "truncated");
    assert_eq!(
        item["current"]["label_actors"]["items"][0]["event_id"],
        "LE_stable"
    );
    assert!(item["current"]["label_actors"]["items"][0]["actor"].is_null());
    assert_eq!(item["current"]["linked_pull_request"], "unknown");
    assert!(!shared.to_string().contains("private original"));

    let withheld = IssueSourceContent::Withheld {
        basis: basis(),
        policy_version: "policy-2".to_string(),
        reason: IssueContentWithheldReason::UserConfirmationRequired,
    };
    let mut first_withheld = page(Some(observation(withheld)));
    first_withheld.items[0].tracked = None;
    let wire = issue_answer(&first_withheld);
    assert!(wire["items"][0]["tracked"].is_null());
    assert_eq!(wire["items"][0]["current"]["content"]["kind"], "withheld");
    assert_eq!(
        wire["items"][0]["current"]["content"]["reason"],
        "user_confirmation_required"
    );
    assert!(!wire.to_string().contains("shared title"));
    assert!(
        wire["items"][0]["current"]["content"]
            .get("shared_copy")
            .is_none()
    );
    assert!(
        wire["items"][0]["current"]["content"]
            .get("title")
            .is_none()
    );

    let unknown = issue_answer(&page(Some(IssueSourceRead::Unknown {
        issue: reference(),
        reason: IssueSourceFailure::ProviderUnavailable,
    })));
    assert_eq!(
        unknown["items"][0]["current"],
        json!({"status":"unknown","reason":"provider_unavailable"})
    );
    assert!(unknown["items"][0]["current"].get("content").is_none());
    let legacy = issue_answer(&page(None));
    assert_eq!(
        legacy["items"][0]["current"],
        json!({"status":"not_yet_observed"})
    );
    assert_eq!(legacy["items"][0]["tracked"]["state"], "running");
    assert!(legacy["items"][0].get("snapshot").is_none());
}

#[test]
fn populated_issue_queries_and_events_match_the_generated_schema_and_round_trip() {
    let spec = serde_json::to_value(describe()).unwrap();
    for state in [
        IssueAutomationState::unavailable(),
        IssueAutomationState::from_domain(&projection(IssueCollectionMode::Observe))
            .with_runtime(true, None),
    ] {
        let query = KernelQueryAnswer::IssueAutomation(state.clone());
        let wire = serde_json::to_value(&query).unwrap();
        assert_matches_spec(&spec, "queries", "output", &wire);
        assert_eq!(
            serde_json::from_value::<KernelQueryAnswer>(wire).unwrap(),
            query
        );
        let event = KernelEvent::Issues(state);
        let wire = serde_json::to_value(&event).unwrap();
        assert_matches_spec(&spec, "events", "payload", &wire);
        assert_eq!(serde_json::from_value::<KernelEvent>(wire).unwrap(), event);
    }
    let withheld = IssueSourceContent::Withheld {
        basis: basis(),
        policy_version: "policy-2".to_string(),
        reason: IssueContentWithheldReason::UserConfirmationRequired,
    };
    for current in [
        None,
        Some(observation(shared())),
        Some(observation(withheld)),
        Some(IssueSourceRead::Unknown {
            issue: reference(),
            reason: IssueSourceFailure::MetadataIncomplete,
        }),
    ] {
        let answer = KernelQueryAnswer::Issues(IssuesState::from_domain(&page(current)));
        let wire = serde_json::to_value(&answer).unwrap();
        assert_matches_spec(&spec, "queries", "output", &wire);
        assert_eq!(
            serde_json::from_value::<KernelQueryAnswer>(wire).unwrap(),
            answer
        );
    }
}

#[test]
fn issue_status_and_event_use_catalog_revision_even_without_a_control_change() {
    let mut projection = projection(IssueCollectionMode::Observe);
    projection.catalog_revision = 105;
    let before = serde_json::to_value(IssueAutomationState::from_domain(&projection)).unwrap();
    assert_eq!(before["revision"], 105);
    assert_eq!(projection.state.revision, 7);

    // 판정만 저장해도 catalog가 진전한다. 수집 control CAS 판을 공개 목록 판으로 쓰면
    // Desktop은 새 이벤트를 이전 페이지보다 오래된 것으로 보고 재조회를 건너뛴다.
    projection.catalog_revision = 106;
    let event = serde_json::to_value(KernelEvent::Issues(IssueAutomationState::from_domain(
        &projection,
    )))
    .unwrap();
    let page = issue_answer(&IssueCollectionPage {
        revision: 106,
        items: vec![],
        next_after: None,
        report: None,
    });
    assert_eq!(event["revision"], page["revision"]);
    assert!(event["revision"].as_u64().unwrap() > before["revision"].as_u64().unwrap());
    assert_eq!(projection.state.revision, 7);
}
