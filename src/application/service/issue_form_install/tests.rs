use super::{ISSUE_FORM_ASSET, valid_pull_request};
use crate::application::port::outbound::github_automation_port::GithubAutomationPullRequest;
use crate::application::port::outbound::issue_form_repository_port::{
    ISSUE_FORM_PATH, IssueFormInstallPhase, IssueFormInstallPlan, IssueFormInstallRecord,
    IssueFormTarget, StoredIssueFormInstall,
};
use crate::application::service::issue_template::parse_issue_template;
use crate::domain::issue_automation::policy::IssueAutomationPolicy;
use crate::domain::issue_automation::triage::{
    IssueReadinessInput, IssueTriageReadiness, inspect_issue_readiness,
};
use crate::domain::issue_template::IssueTemplateResolution;
use crate::domain::issue_validation::{IssueFieldRequirement, IssueRequirements, validate_issue};
use crate::domain::issue_work::IssueSharedCopy;
use serde_yaml_ng::Value;

fn form() -> Value {
    serde_yaml_ng::from_str(ISSUE_FORM_ASSET).unwrap()
}

fn requirements() -> IssueRequirements {
    let IssueTemplateResolution::Resolved(convention) =
        parse_issue_template(ISSUE_FORM_PATH, ISSUE_FORM_ASSET)
    else {
        panic!("embedded form must parse through the production template parser");
    };
    assert!(convention.unsupported_required_inputs.is_empty());
    assert_eq!(convention.required_inputs.len(), 1);
    assert_eq!(convention.required_inputs[0].label, "완료 기준");
    IssueRequirements {
        template_fields: convention
            .required_inputs
            .into_iter()
            .map(|field| IssueFieldRequirement {
                label: field.label,
                required: true,
                format: None,
            })
            .collect(),
        required_sections: Vec::new(),
    }
}

fn rendered_body(answers: &[&str]) -> String {
    // GitHub turns each answered form label into an h3 section and empty textareas into _No response_.
    form()["body"]
        .as_sequence()
        .unwrap()
        .iter()
        .zip(answers)
        .map(|(field, answer)| {
            let label = field["attributes"]["label"].as_str().unwrap();
            let answer = if answer.is_empty() {
                "_No response_"
            } else {
                answer
            };
            format!("### {label}\n\n{answer}\n\n")
        })
        .collect()
}

#[test]
fn embedded_form_has_no_default_answers_or_automatic_selection_labels() {
    let form = form();
    assert!(form.get("labels").is_none());
    assert!(form.get("title").is_none());
    let fields = form["body"].as_sequence().unwrap();
    assert_eq!(fields.len(), 3);
    assert_eq!(
        fields
            .iter()
            .map(|field| field["attributes"]["label"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["완료 기준", "범위", "하지 않는 것"]
    );
    assert_eq!(fields[0]["validations"]["required"].as_bool(), Some(true));
    for field in fields {
        assert!(field["attributes"].get("value").is_none());
        assert!(
            !field["attributes"]["placeholder"]
                .as_str()
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn actual_rendered_form_answers_pass_parser_validator_and_shared_readiness() {
    let copy = IssueSharedCopy {
        title: "상태 표시 문구 수정".to_owned(),
        body: rendered_body(&[
            "- 상태 문구가 서버 응답과 일치한다.\n- 해당 화면의 회귀 테스트가 통과한다.",
            "상태 표시와 관련 회귀 테스트",
            "새로운 실행 동작 추가",
        ]),
    };
    let requirements = requirements();
    validate_issue(&copy, &requirements).unwrap();
    let result = inspect_issue_readiness(IssueReadinessInput {
        shared_copy: &copy,
        requirements: &requirements,
        policy: &IssueAutomationPolicy::builtin(),
        labels: &[],
        labels_complete: true,
        held: false,
    })
    .unwrap();
    assert_eq!(result.readiness, IssueTriageReadiness::Ready);
    assert!(result.validation_errors.is_empty());
    assert!(result.missing_sections.is_empty());
}

#[test]
fn empty_rendered_answers_never_turn_the_placeholder_into_readiness() {
    let copy = IssueSharedCopy {
        title: "아직 기준 없는 작업".to_owned(),
        body: rendered_body(&["", "", ""]),
    };
    let requirements = requirements();
    assert!(validate_issue(&copy, &requirements).is_err());
    let result = inspect_issue_readiness(IssueReadinessInput {
        shared_copy: &copy,
        requirements: &requirements,
        policy: &IssueAutomationPolicy::builtin(),
        labels: &[],
        labels_complete: true,
        held: false,
    })
    .unwrap();
    assert_eq!(result.readiness, IssueTriageReadiness::NeedsInfo);
}

#[test]
fn found_pr_must_match_the_frozen_target_base_branch_and_exact_commit() {
    let target = IssueFormTarget {
        repository: "acme/widgets".to_owned(),
        push_remote: "origin".to_owned(),
        push_url: "https://github.com/acme/widgets.git".to_owned(),
        base_branch: "main".to_owned(),
        asset_sha256: "a".repeat(64),
    };
    let stored = StoredIssueFormInstall {
        object_id: "c".repeat(40),
        record: IssueFormInstallRecord {
            version: 1,
            phase: IssueFormInstallPhase::Sent,
            pull_request: None,
            plan: IssueFormInstallPlan {
                target: target.clone(),
                base_commit_sha: "b".repeat(40),
                commit_sha: "d".repeat(40),
            },
        },
    };
    let matching = GithubAutomationPullRequest::new(
        7,
        "https://github.com/acme/widgets/pull/7",
        "OPEN",
        "main",
        target.head_branch(),
        false,
    )
    .with_head_commit_sha("d".repeat(40));
    assert!(valid_pull_request(&stored, &matching));
    let mut cases = Vec::new();
    let mut changed = matching.clone();
    changed.head_commit_sha = Some("e".repeat(40));
    cases.push(changed);
    let mut changed = matching.clone();
    changed.base_branch = "other".to_owned();
    cases.push(changed);
    let mut changed = matching.clone();
    changed.head_branch = "other".to_owned();
    cases.push(changed);
    let mut changed = matching.clone();
    changed.url = "https://github.com/other/widgets/pull/7".to_owned();
    cases.push(changed);
    let mut changed = matching.clone();
    changed.state = "CLOSED".to_owned();
    cases.push(changed);
    for changed in cases {
        assert!(!valid_pull_request(&stored, &changed));
    }
}
