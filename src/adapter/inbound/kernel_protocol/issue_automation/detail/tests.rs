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
        revision: 31,
        item,
        report: Some(report()),
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
