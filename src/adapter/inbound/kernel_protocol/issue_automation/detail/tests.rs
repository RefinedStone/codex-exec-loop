use super::*;
use crate::adapter::inbound::kernel_protocol::{ERROR_INVALID_PARAMS, KernelQueryAnswer, describe};
use crate::domain::issue_automation::collection::{IssueCollectionItem, IssueTriageObserveReport};
use crate::domain::issue_automation::source::{IssueSourceFailure, IssueSourceRead};
use crate::domain::issue_automation::{IssueAutomationAutonomy, IssueAutomationBindingId};
use crate::domain::issue_work::{IssueProjectScope, IssueProvider};
use serde_json::{Value, json};

fn reference() -> IssueReference {
    IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".to_owned(),
        repository_id: "R_stable".to_owned(),
        issue_id: "I_stable".to_owned(),
        project_scope: IssueProjectScope {
            workspace_dir: "/workspace".to_owned(),
        },
    }
}

fn query(reference: &IssueReference) -> KernelQuery {
    KernelQuery::Issue {
        issue: reference.into(),
    }
}

fn report() -> IssueTriageObserveReport {
    IssueTriageObserveReport {
        revision: 31,
        binding_id: Some(IssueAutomationBindingId::new("binding").unwrap()),
        policy_version: Some("a".repeat(64)),
        assumed_mode: Some(IssueAutomationAutonomy::Auto),
        policy_blocked: false,
        total: 55,
        automatic_start: 1,
        human_approval: 2,
        deferred: 3,
        excluded: 4,
        needs_info: 5,
        unknown: 6,
        observation_only: 7,
        stale: 8,
        untriaged: 9,
        blocked: 10,
    }
}

fn detail(item: Option<IssueCollectionItem>) -> IssueCollectionDetail {
    IssueCollectionDetail {
        comment_requests: Vec::new(),
        arbiter_waits: Vec::new(),
        timeline: None,
        revision: 31,
        item,
        report: Some(report()),
        interaction: None,
        execution: None,
        feedback: None,
    }
}

fn assert_wire_matches_spec(value: &Value) {
    let spec = describe();
    let mut schema = spec
        .queries
        .iter()
        .find(|entry| entry.name == "issue")
        .unwrap()
        .output
        .clone();
    schema
        .as_object_mut()
        .unwrap()
        .insert("$defs".to_owned(), Value::Object(spec.definitions));
    let validator = jsonschema::draft202012::options()
        .offline()
        .build(&schema)
        .unwrap();
    let errors: Vec<_> = validator
        .iter_errors(value)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn issue_query_uses_full_stable_identity_and_the_attached_primary_workspace() {
    let issue = reference();
    let request = query(&issue);
    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json["type"], "issue");
    assert_eq!(
        request.issue_reference("/workspace").unwrap(),
        Some(issue.clone())
    );
    let decoded: KernelQuery = serde_json::from_value(json).unwrap();
    assert_eq!(
        decoded.issue_reference("/workspace").unwrap(),
        Some(issue.clone())
    );
    assert_eq!(
        request
            .issue_reference("/workspace/subdir")
            .unwrap_err()
            .code,
        ERROR_INVALID_PARAMS
    );
    assert_eq!(
        request
            .issue_reference("/other-repository")
            .unwrap_err()
            .code,
        ERROR_INVALID_PARAMS
    );
    assert_eq!(
        KernelQuery::Issues {
            after: None,
            limit: None
        }
        .issue_reference("/workspace")
        .unwrap(),
        None
    );

    // 같은 번호를 가진 다른 저장소도 전체 안정 좌표로 구분한다.
    let mut other = issue;
    other.repository_id = "R_other".to_owned();
    assert_eq!(
        query(&other).issue_reference("/workspace").unwrap(),
        Some(other)
    );
    for invalid in [
        json!({"type":"issue","number":17}),
        json!({"type":"issue","url":"https://github.com/sample/widgets/issues/17"}),
        json!({"type":"issue","issue":reference(),"number":17}),
    ] {
        assert!(serde_json::from_value::<KernelQuery>(invalid).is_err());
    }
}

#[test]
fn issue_query_rejects_empty_padded_control_and_oversized_coordinates() {
    for invalid in [
        "".to_owned(),
        " padded".to_owned(),
        "trailing ".to_owned(),
        "line\nbreak".to_owned(),
        "x".repeat(4097),
    ] {
        for field in 0..4 {
            let mut issue = reference();
            match field {
                0 => issue.server_id = invalid.clone(),
                1 => issue.repository_id = invalid.clone(),
                2 => issue.issue_id = invalid.clone(),
                _ => issue.project_scope.workspace_dir = invalid.clone(),
            }
            let error = query(&issue)
                .issue_reference(&issue.project_scope.workspace_dir)
                .unwrap_err();
            assert_eq!(error.code, ERROR_INVALID_PARAMS);
            assert_eq!(error.message, "invalid issue reference");
        }
    }
}

#[test]
fn issue_detail_distinguishes_not_found_from_unavailable_and_keeps_global_report() {
    let absent = serde_json::to_value(KernelQueryAnswer::Issue(IssueState::from_domain(&detail(
        None,
    ))))
    .unwrap();
    assert_eq!(absent["status"], "not_found");
    assert_eq!(absent["revision"], 31);
    assert_eq!(absent["report"]["total"], 55);
    assert_eq!(absent["report"]["assumed_mode"], "auto");
    assert!(absent.get("item").is_none());
    assert_wire_matches_spec(&absent);

    let unavailable =
        serde_json::to_value(KernelQueryAnswer::Issue(IssueState::unavailable())).unwrap();
    assert_eq!(unavailable, json!({"type":"issue","status":"unavailable"}));
    assert_wire_matches_spec(&unavailable);
}

#[test]
fn issue_detail_unknown_has_no_content_fallback_and_uses_the_catalog_revision() {
    let issue = reference();
    let item = IssueCollectionItem {
        binding_run_revision: 0,
        binding_run_blocked: false,
        start_budget_wait: None,
        work: None,
        open_decisions: Vec::new(),
        issue: issue.clone(),
        binding_id: IssueAutomationBindingId::new("binding").unwrap(),
        tracked: None,
        current: Some(IssueSourceRead::Unknown {
            issue,
            reason: IssueSourceFailure::ProviderUnavailable,
        }),
        observation_revision: Some(9),
        tracked_revision: None,
        triage: None,
    };
    let ready = serde_json::to_value(KernelQueryAnswer::Issue(IssueState::from_domain(&detail(
        Some(item),
    ))))
    .unwrap();
    assert_eq!(ready["status"], "ready");
    assert_eq!(ready["revision"], 31);
    assert_eq!(ready["report"]["revision"], 31);
    assert_eq!(ready["report"]["total"], 55);
    assert_eq!(ready["item"]["current"]["status"], "unknown");
    assert!(ready["item"]["presentation"]["title"].is_null());
    assert!(ready["item"]["presentation"]["url"].is_null());
    assert!(ready["item"]["presentation"]["number"].is_null());
    assert!(ready["item"].get("snapshot").is_none());
    assert_wire_matches_spec(&ready);
}

#[test]
fn issue_detail_query_time_preserves_the_snapshot_and_legacy_wire_contract() {
    let snapshot = detail(Some(IssueCollectionItem {
        binding_run_revision: 0,
        binding_run_blocked: false,
        start_budget_wait: None,
        work: None,
        open_decisions: Vec::new(),
        issue: reference(),
        binding_id: IssueAutomationBindingId::new("binding").unwrap(),
        tracked: None,
        current: None,
        observation_revision: None,
        tracked_revision: None,
        triage: None,
    }));
    let legacy = IssueState::from_domain(&snapshot);
    assert!(
        serde_json::to_value(&legacy)
            .unwrap()
            .get("as_of_unix_ms")
            .is_none()
    );
    for at in [1_795_000_000_000, 1_795_000_120_000] {
        let queried = legacy.clone().with_as_of_unix_ms(Some(at));
        let wire = serde_json::to_value(KernelQueryAnswer::Issue(queried.clone())).unwrap();
        assert_eq!(wire["as_of_unix_ms"], at);
        assert_eq!(wire["revision"], snapshot.revision);
        assert_wire_matches_spec(&wire);
        assert_eq!(queried.with_as_of_unix_ms(None), legacy);
    }
    let restored: IssueState =
        serde_json::from_value(serde_json::to_value(&legacy).unwrap()).unwrap();
    assert_eq!(restored, legacy);
    let missing = IssueState::from_domain(&detail(None));
    assert_eq!(missing.clone().with_as_of_unix_ms(Some(123)), missing);
}

#[test]
fn issue_detail_exposes_server_actions_and_only_the_selected_safe_conversation_target() {
    use crate::domain::issue_automation::interaction::{
        IssueActionCommand, IssueActionProjection, IssueControlProjection, IssueConversationTarget,
        IssueInteractionProjection,
    };
    use crate::domain::issue_work::IssueSharedCopy;

    let item = IssueCollectionItem {
        binding_run_revision: 0,
        binding_run_blocked: false,
        start_budget_wait: None,
        issue: reference(),
        binding_id: IssueAutomationBindingId::new("binding").unwrap(),
        tracked: None,
        current: None,
        observation_revision: None,
        tracked_revision: None,
        triage: None,
        work: None,
        open_decisions: Vec::new(),
    };
    let mut detail = detail(Some(item));
    let legacy = serde_json::to_value(IssueState::from_domain(&detail)).unwrap();
    assert!(legacy.get("control").is_none());
    assert_eq!(legacy["actions"], json!([]));
    assert_eq!(legacy["decision_history"], json!([]));
    assert_eq!(legacy["conversation"]["kind"], "unavailable");
    detail.interaction = Some(IssueInteractionProjection {
        control: Some(IssueControlProjection {
            tracked: true,
            held: true,
            taken_over: false,
            cancelled: false,
            authorization_epoch: 7,
        }),
        actions: IssueActionCommand::ALL
            .into_iter()
            .map(|command| IssueActionProjection {
                command,
                enabled: command == IssueActionCommand::ReleaseIssue,
                reason: (command != IssueActionCommand::ReleaseIssue)
                    .then(|| "이슈가 보류 중입니다.".into()),
            })
            .collect(),
        decision_history: Vec::new(),
        conversation: IssueConversationTarget::Draft {
            shared_copy: IssueSharedCopy {
                title: "공유 제목".into(),
                body: "공유 본문".into(),
            },
        },
    });
    let draft =
        serde_json::to_value(KernelQueryAnswer::Issue(IssueState::from_domain(&detail))).unwrap();
    assert_eq!(draft["control"]["held"], true);
    assert_eq!(draft["control"]["authorization_epoch"], 7);
    assert_eq!(draft["actions"].as_array().unwrap().len(), 8);
    assert_eq!(draft["actions"][1]["command"], "release_issue");
    assert_eq!(draft["actions"][1]["enabled"], true);
    assert!(draft["actions"][1]["reason"].is_null());
    assert!(draft["actions"][0]["reason"].is_string());
    assert_eq!(
        draft["conversation"],
        json!({
            "kind":"draft", "shared_copy":{"title":"공유 제목","body":"공유 본문"}
        })
    );
    assert_wire_matches_spec(&draft);

    detail.interaction.as_mut().unwrap().conversation = IssueConversationTarget::Thread {
        thread_id: "thread-1".into(),
    };
    let thread =
        serde_json::to_value(KernelQueryAnswer::Issue(IssueState::from_domain(&detail))).unwrap();
    assert_eq!(
        thread["conversation"],
        json!({"kind":"thread","thread_id":"thread-1"})
    );
    assert_wire_matches_spec(&thread);

    detail.interaction.as_mut().unwrap().conversation = IssueConversationTarget::Unavailable;
    let unavailable =
        serde_json::to_value(KernelQueryAnswer::Issue(IssueState::from_domain(&detail))).unwrap();
    assert_eq!(unavailable["conversation"]["kind"], "unavailable");
    assert!(unavailable["conversation"]["reason"].is_string());
    assert!(unavailable["conversation"].get("shared_copy").is_none());
    assert!(unavailable["conversation"].get("thread_id").is_none());
    assert_wire_matches_spec(&unavailable);
}

#[test]
fn issue_detail_feedback_projects_only_persisted_metadata_and_keeps_legacy_compatible() {
    let item = IssueCollectionItem {
        binding_run_revision: 0,
        binding_run_blocked: false,
        start_budget_wait: None,
        issue: reference(),
        binding_id: IssueAutomationBindingId::new("binding").unwrap(),
        tracked: None,
        current: None,
        observation_revision: None,
        tracked_revision: None,
        triage: None,
        work: None,
        open_decisions: Vec::new(),
    };
    let mut detail = detail(Some(item));
    let legacy = serde_json::to_value(IssueState::from_domain(&detail)).unwrap();
    assert!(legacy.get("feedback").is_none());
    assert!(serde_json::from_value::<IssueState>(legacy).is_ok());

    detail.feedback = Some(IssueFeedbackDetail {
        revision: 9,
        publication_attempts: 2,
        comment_id: Some("IC_confirmed".into()),
        reason: Some(IssueFeedbackDetailReason::FeedbackUnconfirmed),
        integration: None,
        external_state: IssueExternalState::Unknown,
    });
    let pending =
        serde_json::to_value(KernelQueryAnswer::Issue(IssueState::from_domain(&detail))).unwrap();
    assert_eq!(
        pending["feedback"],
        json!({
            "revision": 9,
            "publication_attempts": 2,
            "comment_id": "IC_confirmed",
            "reason": "feedback_unconfirmed",
            "external_state": "unknown"
        })
    );
    assert_wire_matches_spec(&pending);

    let feedback = detail.feedback.as_mut().unwrap();
    feedback.revision = 10;
    feedback.reason = None;
    let confirmed =
        serde_json::to_value(KernelQueryAnswer::Issue(IssueState::from_domain(&detail))).unwrap();
    assert_eq!(
        confirmed["feedback"],
        json!({"revision": 10, "publication_attempts": 2, "comment_id": "IC_confirmed", "external_state": "unknown"})
    );
    assert_wire_matches_spec(&confirmed);

    let feedback = detail.feedback.as_mut().unwrap();
    feedback.comment_id = None;
    feedback.reason = Some(IssueFeedbackDetailReason::FeedbackUnconfirmed);
    let missing_ack =
        serde_json::to_value(KernelQueryAnswer::Issue(IssueState::from_domain(&detail))).unwrap();
    assert!(missing_ack["feedback"].get("comment_id").is_none());
    assert_eq!(missing_ack["feedback"]["reason"], "feedback_unconfirmed");
    assert_wire_matches_spec(&missing_ack);

    detail.item = None;
    let missing_issue = serde_json::to_value(IssueState::from_domain(&detail)).unwrap();
    assert_eq!(missing_issue["status"], "not_found");
    assert!(missing_issue.get("feedback").is_none());
}

#[test]
fn issue_feedback_detail_rejects_raw_provider_fields_and_open_ended_reasons() {
    let value = json!({
        "revision": 5,
        "publication_attempts": 1,
        "reason": "feedback_unconfirmed"
    });
    assert!(serde_json::from_value::<IssueFeedbackDetailState>(value.clone()).is_ok());
    for field in ["body", "actor_id", "intent", "response", "token"] {
        let mut invalid = value.clone();
        invalid[field] = json!("private-canary");
        assert!(serde_json::from_value::<IssueFeedbackDetailState>(invalid).is_err());
    }
    let mut invalid = value;
    invalid["reason"] = json!("raw-provider-error-canary");
    assert!(serde_json::from_value::<IssueFeedbackDetailState>(invalid).is_err());
}

#[test]
fn issue_detail_keeps_integration_independent_from_open_closed_and_unknown_provider_state() {
    let integration = IssueExecutionIntegration {
        record_id: "integration-confirmed".into(),
        source_sha: "a".repeat(40),
        integration_sha: "b".repeat(40),
        pull_request_url: Some("https://github.com/owner/repo/pull/10".into()),
    };
    for (external_state, expected) in [
        (IssueExternalState::Open, "open"),
        (IssueExternalState::Closed, "closed"),
        (IssueExternalState::Unknown, "unknown"),
    ] {
        let feedback = IssueFeedbackDetail {
            revision: 0,
            publication_attempts: 0,
            comment_id: None,
            reason: None,
            integration: Some(integration.clone()),
            external_state,
        };
        let state = IssueFeedbackDetailState::from(&feedback);
        let wire = serde_json::to_value(&state).unwrap();
        assert_eq!(wire["external_state"], expected);
        assert_eq!(
            wire["integration"]["integration_sha"],
            integration.integration_sha
        );
        assert_eq!(wire["integration"]["record_id"], integration.record_id);
        assert!(wire.get("comment_id").is_none());
        assert_eq!(
            serde_json::from_value::<IssueFeedbackDetailState>(wire).unwrap(),
            state
        );
    }
}

#[test]
fn legacy_feedback_detail_defaults_external_state_to_unknown() {
    let legacy = json!({"revision": 1, "publication_attempts": 0});
    let state = serde_json::from_value::<IssueFeedbackDetailState>(legacy).unwrap();
    assert_eq!(state.external_state, IssueExternalStateRecord::Unknown);
    assert!(state.integration.is_none());
}

mod execution;
