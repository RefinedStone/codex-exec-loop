use super::*;
use crate::domain::issue_automation::IssueAutomationSnapshot;
use crate::domain::issue_automation::policy::{
    IssueAutomationAdditionalHumanRule, IssueAutomationRiskRule, IssueAutomationSelectionMode,
    IssueAutomationTrustedActor,
};
use crate::domain::issue_automation::source::{
    IssueLabelAttribution, IssueMetadataCompleteness, IssueMetadataList, IssuePrPresence,
    IssueRemoteState, IssueRepositoryVisibility, IssueSourceActor, IssueSourceContent,
    IssueSourceFailure, IssueSourceLabel, IssueSourceObservation,
};
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueProjectScope, IssueProvider, IssueSharedCopy,
};

mod arbiter;
mod auto;
mod jira;
mod linear;

const AUTOMATION_VERSION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DISCLOSURE_VERSION: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const VALID_BODY: &str = "## 완료 기준\n- 결과가 저장된다\n- [ ] 다시 읽을 수 있다\n";

fn issue() -> IssueReference {
    IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".into(),
        repository_id: "R_1".into(),
        issue_id: "I_1".into(),
        project_scope: IssueProjectScope {
            workspace_dir: "/workspace/repo".into(),
        },
    }
}
fn actor(id: &str) -> IssueSourceActor {
    IssueSourceActor {
        kind: crate::domain::issue_automation::source::IssueSourceActorKind::UserAccount,
        id: id.into(),
        login: "display-name".into(),
    }
}
fn list<T>(items: Vec<T>) -> IssueMetadataList<T> {
    IssueMetadataList {
        items,
        completeness: IssueMetadataCompleteness::Complete,
    }
}
fn source() -> IssueSourceRead {
    IssueSourceRead::Observed(Box::new(IssueSourceObservation {
        content_problems: vec![],
        jira: None,
        linear: None,
        repository_visibility: IssueRepositoryVisibility::Private,
        repository_name_with_owner: Some("refinedstone/codex-exec-loop".into()),
        creation_provenance:
            crate::domain::issue_automation::source::IssueCreationProvenance::Unattributed,
        issue: issue(),
        web: None,
        remote_state: IssueRemoteState::Open,
        labels: list(vec![IssueSourceLabel {
            id: "L_auto".into(),
            name: "akra:auto".into(),
        }]),
        assignees: list(Vec::new()),
        author: Some(actor("U_trusted")),
        label_actors: list(Vec::new()),
        linked_pull_request: IssuePrPresence::Absent,
        content: IssueSourceContent::Shared(IssueAutomationSnapshot {
            basis: IssueComparisonBasis {
                field_hash: IssueFieldHash::of_fields("공유 제목", VALID_BODY),
                provider_updated_at: "2026-11-20T00:00:00Z".into(),
            },
            shared_copy: IssueSharedCopy {
                title: "공유 제목".into(),
                body: VALID_BODY.into(),
            },
            policy_version: DISCLOSURE_VERSION.into(),
        }),
    }))
}
fn policy() -> IssueAutomationPolicy {
    let mut policy = IssueAutomationPolicy::builtin();
    policy.binding.github.mode = IssueAutomationAutonomy::Auto;
    policy.authority.autonomy_cap = IssueAutomationAutonomy::Auto;
    policy
        .select
        .trusted_actors
        .push(IssueAutomationTrustedActor {
            provider: IssueProvider::GitHub,
            server_id: "github.com".into(),
            actor_id: "U_trusted".into(),
        });
    for gates in [
        &mut policy.gates.auto.low,
        &mut policy.gates.auto.normal,
        &mut policy.gates.auto.high,
    ] {
        gates.start = IssueAutomationGateAction::Auto;
        gates.delivery = IssueAutomationGateAction::Auto;
        gates.merge = IssueAutomationGateAction::Auto;
    }
    policy
}
fn observed(source: &mut IssueSourceRead) -> &mut IssueSourceObservation {
    match source {
        IssueSourceRead::Observed(observation) => observation,
        _ => panic!("observed fixture"),
    }
}
fn snapshot(source: &mut IssueSourceRead) -> &mut IssueAutomationSnapshot {
    match &mut observed(source).content {
        IssueSourceContent::Shared(snapshot) => snapshot,
        _ => panic!("shared fixture"),
    }
}
fn set_body(source: &mut IssueSourceRead, body: impl Into<String>) {
    let snapshot = snapshot(source);
    snapshot.shared_copy.body = body.into();
    snapshot.basis.field_hash =
        IssueFieldHash::of_fields(&snapshot.shared_copy.title, &snapshot.shared_copy.body);
}
fn input<'a>(
    issue: &'a IssueReference,
    current: Option<&'a IssueSourceRead>,
    policy: &'a IssueAutomationPolicy,
    requirements: &'a IssueRequirements,
    previous: Option<&'a IssueTriageAssessment>,
) -> IssueTriageInput<'a> {
    IssueTriageInput {
        runtime_selection_revision: 0,
        issue,
        current,
        policy,
        requirements,
        previous,
        automation_policy_version: AUTOMATION_VERSION,
        validation_policy_version: DISCLOSURE_VERSION,
        held: false,
        pinned_source_commit: None,
    }
}
fn run(
    current: Option<&IssueSourceRead>,
    policy: &IssueAutomationPolicy,
    previous: Option<&IssueTriageAssessment>,
) -> Result<IssueTriageAssessment, IssueTriageError> {
    triage_issue(input(
        &issue(),
        current,
        policy,
        &IssueRequirements::default(),
        previous,
    ))
}

#[test]
fn identical_inputs_and_repeated_observations_keep_assessment_and_gate_revisions() {
    let source = source();
    let policy = policy();
    let first = run(Some(&source), &policy, None).expect("first");
    assert_eq!(
        first,
        run(Some(&source), &policy, None).expect("deterministic")
    );
    assert_eq!(
        first,
        run(Some(&source), &policy, Some(&first)).expect("same poll")
    );
    assert_eq!(first.eligibility(), IssueTriageEligibility::Eligible);
    assert_eq!(first.readiness(), IssueTriageReadiness::Ready);
    assert_eq!(first.risk_class(), IssueAutomationRiskClass::Normal);
    assert_eq!(
        first.observation().disposition,
        IssueObservationDisposition::AutomaticStart
    );
    assert!(first.gates().iter().all(|gate| gate.gate_revision == 1));
    first
        .validate_current(&issue(), Some(&source), false)
        .expect("bound observation");
}

#[test]
fn source_identity_and_policy_versions_are_checked_before_evaluation() {
    let policy = policy();
    let mut current = source();
    observed(&mut current).issue.issue_id = "I_foreign".into();
    assert_eq!(
        run(Some(&current), &policy, None),
        Err(IssueTriageError::InvalidIdentity)
    );
    let current = source();
    let issue = issue();
    let requirements = IssueRequirements::default();
    let mut request = input(&issue, Some(&current), &policy, &requirements, None);
    request.automation_policy_version = "not-a-policy-hash";
    assert_eq!(triage_issue(request), Err(IssueTriageError::InvalidBasis));
}

#[test]
fn eligibility_distinguishes_confirmed_exclusions_from_unavailable_evidence() {
    let policy = policy();
    for (mutation, expected, reason) in [
        (
            0,
            IssueTriageEligibility::Ineligible,
            IssueTriageReasonCode::AlreadyClaimed,
        ),
        (
            1,
            IssueTriageEligibility::Ineligible,
            IssueTriageReasonCode::AlreadyClaimed,
        ),
        (
            2,
            IssueTriageEligibility::Unknown,
            IssueTriageReasonCode::MetadataIncomplete,
        ),
        (
            3,
            IssueTriageEligibility::Unknown,
            IssueTriageReasonCode::MetadataIncomplete,
        ),
        (
            4,
            IssueTriageEligibility::Unknown,
            IssueTriageReasonCode::MetadataIncomplete,
        ),
        (
            5,
            IssueTriageEligibility::Ineligible,
            IssueTriageReasonCode::IssueClosed,
        ),
        (
            6,
            IssueTriageEligibility::Ineligible,
            IssueTriageReasonCode::SelectionNotMatched,
        ),
        (
            7,
            IssueTriageEligibility::Ineligible,
            IssueTriageReasonCode::UntrustedActor,
        ),
    ] {
        let mut current = source();
        let observation = observed(&mut current);
        match mutation {
            0 => observation.assignees.items.push(actor("U_trusted")),
            1 => observation.linked_pull_request = IssuePrPresence::Present,
            2 => observation.assignees.completeness = IssueMetadataCompleteness::Truncated,
            3 => observation.assignees.completeness = IssueMetadataCompleteness::Unavailable,
            4 => observation.linked_pull_request = IssuePrPresence::Unknown,
            5 => observation.remote_state = IssueRemoteState::Closed,
            6 => observation.labels.items.clear(),
            7 => observation.author = Some(actor("U_untrusted")),
            _ => unreachable!(),
        }
        let result = run(Some(&current), &policy, None).expect("assessment");
        assert_eq!(result.eligibility(), expected, "case {mutation}");
        assert!(result.reasons().contains(&reason));
        assert!(
            result
                .gates()
                .iter()
                .all(|gate| !gate.enabled && gate.action == IssueAutomationGateAction::Human)
        );
    }
}

fn trusted_event(observation: &IssueSourceObservation) -> IssueLabelAttribution {
    IssueLabelAttribution {
        label: observation.labels.items[0].clone(),
        event_id: "E_current".into(),
        actor: Some(actor("U_trusted")),
        created_at: "2026-11-20T00:00:00Z".into(),
    }
}

#[test]
fn trusted_label_requires_current_selection_and_exact_provider_server_actor_ids() {
    let policy = policy();
    let mut current = source();
    let observation = observed(&mut current);
    observation.author = Some(actor("U_untrusted"));
    observation
        .label_actors
        .items
        .push(trusted_event(observation));
    assert_eq!(
        run(Some(&current), &policy, None)
            .expect("trusted event")
            .eligibility(),
        IssueTriageEligibility::Eligible
    );
    for mutation in 0..6 {
        let mut changed = current.clone();
        let observation = observed(&mut changed);
        match mutation {
            0 => observation.label_actors.items[0].label.id = "L_removed".into(),
            1 => observation.label_actors.items[0].label.name = "old-name".into(),
            2 => observation.label_actors.items[0].actor = Some(actor("U_untrusted")),
            3 => observation.label_actors.items[0].actor = None,
            4 => {
                let unrelated = IssueSourceLabel {
                    id: "L_bug".into(),
                    name: "bug".into(),
                };
                observation.labels.items.push(unrelated.clone());
                observation.label_actors.items[0].label = unrelated;
            }
            5 => observation.label_actors.completeness = IssueMetadataCompleteness::Unavailable,
            _ => unreachable!(),
        }
        assert_ne!(
            run(Some(&changed), &policy, None)
                .expect("evidence")
                .eligibility(),
            IssueTriageEligibility::Eligible,
            "case {mutation}"
        );
    }
    for wrong_actor in [
        IssueAutomationTrustedActor {
            provider: IssueProvider::Jira,
            server_id: "github.com".into(),
            actor_id: "U_trusted".into(),
        },
        IssueAutomationTrustedActor {
            provider: IssueProvider::GitHub,
            server_id: "enterprise.example".into(),
            actor_id: "U_trusted".into(),
        },
        IssueAutomationTrustedActor {
            provider: IssueProvider::GitHub,
            server_id: "github.com".into(),
            actor_id: "display-name".into(),
        },
    ] {
        let mut wrong = policy.clone();
        wrong.select.trusted_actors = vec![wrong_actor];
        assert_eq!(
            run(Some(&current), &wrong, None)
                .expect("identity")
                .eligibility(),
            IssueTriageEligibility::Ineligible
        );
    }
}

#[test]
fn all_open_human_authors_can_use_current_labels_but_conflicting_events_are_not_trust() {
    let mut policy = policy();
    policy.select.mode = IssueAutomationSelectionMode::AllOpen;
    let mut current = source();
    let observation = observed(&mut current);
    observation.author = Some(actor("U_untrusted"));
    observation.labels.items[0].name = "bug".into();
    let event = trusted_event(observation);
    observation.label_actors.items.push(event.clone());
    assert_eq!(
        run(Some(&current), &policy, None)
            .expect("current label")
            .eligibility(),
        IssueTriageEligibility::Eligible
    );
    observed(&mut current)
        .label_actors
        .items
        .push(IssueLabelAttribution {
            event_id: "E_other".into(),
            actor: Some(actor("U_untrusted")),
            ..event
        });
    let result = run(Some(&current), &policy, None).expect("ambiguous evidence");
    assert_eq!(result.eligibility(), IssueTriageEligibility::Unknown);
    assert!(
        result
            .reasons()
            .contains(&IssueTriageReasonCode::MetadataIncomplete)
    );
}

#[test]
fn incomplete_labels_cannot_produce_low_risk_or_automatic_start() {
    let mut policy = policy();
    policy.classify.rule.push(IssueAutomationRiskRule {
        class: IssueAutomationRiskClass::Low,
        labels_any: vec!["akra:auto".into()],
        paths_any: Vec::new(),
    });
    for completeness in [
        IssueMetadataCompleteness::Truncated,
        IssueMetadataCompleteness::Unavailable,
    ] {
        let mut current = source();
        observed(&mut current).labels.completeness = completeness;
        let result = run(Some(&current), &policy, None).expect("incomplete labels");
        assert_eq!(result.risk_class(), IssueAutomationRiskClass::High);
        assert!(
            result
                .gates()
                .iter()
                .all(|gate| gate.action == IssueAutomationGateAction::Human)
        );
        assert_ne!(
            result.observation().disposition,
            IssueObservationDisposition::AutomaticStart
        );
    }
}

#[test]
fn missing_criteria_and_required_sections_reuse_validation_without_copying_field_names() {
    let mut current = source();
    set_body(&mut current, "## 배경\n공유 내용");
    let requirements = IssueRequirements {
        required_sections: vec!["private-section-name".into()],
        ..IssueRequirements::default()
    };
    let result = triage_issue(input(
        &issue(),
        Some(&current),
        &policy(),
        &requirements,
        None,
    ))
    .expect("needs info");
    assert_eq!(result.readiness(), IssueTriageReadiness::NeedsInfo);
    assert!(
        result
            .reasons()
            .contains(&IssueTriageReasonCode::CriteriaMissing)
    );
    assert!(
        result
            .reasons()
            .contains(&IssueTriageReasonCode::RequiredSectionMissing)
    );
    assert!(
        !serde_json::to_string(&result)
            .expect("projection")
            .contains("private-section-name")
    );
}

#[test]
fn checked_and_unchecked_bullets_are_syntax_not_approval_or_pass() {
    let mut policy = policy();
    // A checked Markdown item cannot replace the policy's human Delivery/Merge conditions.
    policy.gates.auto.normal.delivery = IssueAutomationGateAction::Human;
    policy.gates.auto.normal.merge = IssueAutomationGateAction::Human;
    for item in [
        "- criterion",
        "+ criterion",
        "* criterion",
        "1. criterion",
        "2) criterion",
        "- [ ] criterion",
        "- [x] criterion",
        "- [X] criterion",
    ] {
        let mut current = source();
        set_body(&mut current, format!("## 완료 기준\n{item}\n"));
        let result = run(Some(&current), &policy, None).expect("list syntax");
        assert_eq!(result.readiness(), IssueTriageReadiness::Ready, "{item}");
        for gate in result.gates().iter().filter(|gate| {
            matches!(
                gate.gate,
                IssueAutomationGate::Spec
                    | IssueAutomationGate::Delivery
                    | IssueAutomationGate::Merge
            )
        }) {
            assert_eq!(gate.action, IssueAutomationGateAction::Human);
        }
    }
    for (body, reason) in [
        (
            "## 완료 기준\n설명 문장만 있다",
            IssueTriageReasonCode::CriteriaNotList,
        ),
        (
            "## 완료 기준\n- [x]",
            IssueTriageReasonCode::CriteriaMissing,
        ),
        (
            "## 완료 기준\n- _No response_",
            IssueTriageReasonCode::CriteriaMissing,
        ),
        (
            "## 완료 기준\n- ![screen](image.png)",
            IssueTriageReasonCode::UnsupportedContent,
        ),
        (
            "## 완료 기준\n- <img src='x'>",
            IssueTriageReasonCode::UnsupportedContent,
        ),
        (
            "## 완료 기준\n- one\n## 완료 기준\n- other",
            IssueTriageReasonCode::UnsupportedContent,
        ),
        (
            "\x60\x60\x60\n## 완료 기준\n- example\n\x60\x60\x60",
            IssueTriageReasonCode::CriteriaMissing,
        ),
        (
            "## 완료 기준\n\x60\x60\x60\n- example\n\x60\x60\x60",
            IssueTriageReasonCode::CriteriaMissing,
        ),
        (
            "## 완료 기준\n    - code example",
            IssueTriageReasonCode::CriteriaMissing,
        ),
        (
            "## 완료 기준\n> - quoted example",
            IssueTriageReasonCode::CriteriaMissing,
        ),
        (
            "<!--\n## 완료 기준\n- hidden example\n-->",
            IssueTriageReasonCode::CriteriaMissing,
        ),
    ] {
        let mut current = source();
        set_body(&mut current, body);
        let result = run(Some(&current), &policy, None).expect("invalid criterion syntax");
        assert_eq!(
            result.readiness(),
            IssueTriageReadiness::NeedsInfo,
            "{body}"
        );
        assert!(result.reasons().contains(&reason), "{body}");
    }
}

#[test]
fn criteria_and_input_size_limits_are_bounded_in_utf8_bytes() {
    assert_eq!(
        criteria::count_criteria(&format!(
            "## 완료 기준\n- {}",
            "x".repeat(MAX_ISSUE_CRITERION_BYTES)
        )),
        Ok(1)
    );
    assert_eq!(
        criteria::count_criteria(&format!(
            "## 완료 기준\n- {}",
            "x".repeat(MAX_ISSUE_CRITERION_BYTES + 1)
        )),
        Err(criteria::CriteriaProblem::TooLarge)
    );
    assert_eq!(
        criteria::count_criteria(&format!(
            "## 완료 기준\n{}",
            "- item\n".repeat(MAX_ISSUE_CRITERIA)
        )),
        Ok(MAX_ISSUE_CRITERIA)
    );
    assert_eq!(
        criteria::count_criteria(&format!(
            "## 완료 기준\n{}",
            "- item\n".repeat(MAX_ISSUE_CRITERIA + 1)
        )),
        Err(criteria::CriteriaProblem::TooLarge)
    );
    let policy = policy();
    for title_bytes in [
        MAX_ISSUE_TRIAGE_TITLE_BYTES,
        MAX_ISSUE_TRIAGE_TITLE_BYTES + 1,
    ] {
        let mut current = source();
        snapshot(&mut current).shared_copy.title = "x".repeat(title_bytes);
        let result = run(Some(&current), &policy, None).expect("title bound");
        assert_eq!(
            result
                .reasons()
                .contains(&IssueTriageReasonCode::IssueTooLarge),
            title_bytes > MAX_ISSUE_TRIAGE_TITLE_BYTES
        );
    }
    for body_bytes in [MAX_ISSUE_TRIAGE_BODY_BYTES, MAX_ISSUE_TRIAGE_BODY_BYTES + 1] {
        let mut current = source();
        set_body(
            &mut current,
            format!("{VALID_BODY}{}", "x".repeat(body_bytes - VALID_BODY.len())),
        );
        let result = run(Some(&current), &policy, None).expect("body bound");
        assert_eq!(
            result
                .reasons()
                .contains(&IssueTriageReasonCode::IssueTooLarge),
            body_bytes > MAX_ISSUE_TRIAGE_BODY_BYTES
        );
    }
}

#[test]
fn only_explicit_inline_relative_paths_contribute_and_globs_have_segment_semantics() {
    let paths = paths::mentioned_paths(
        "본문 src/raw.rs [link](src/link.rs) https://example/src/url.rs\n\x60src/one.rs\x60 \x60\x60src/two.rs\x60\x60 \x60AGENTS.md\x60\n\x60https://example/a.rs\x60 \x60/absolute/a.rs\x60 \x60C:/absolute/a.rs\x60\n\x60\x60\x60\n\x60src/code.rs\x60\n\x60\x60\x60\n",
    );
    assert_eq!(paths.paths, ["AGENTS.md", "src/one.rs", "src/two.rs"]);
    assert!(!paths.incomplete);
    for (pattern, path, expected) in [
        ("**/AGENTS.md", "AGENTS.md", true),
        ("**/AGENTS.md", "a/b/AGENTS.md", true),
        ("src/*/file?.rs", "src/a/file한.rs", true),
        ("src/*/file?.rs", "src/a/b/file1.rs", false),
        ("src/**", "src/a/b.rs", true),
        ("src/**/b.rs", "src/b.rs", true),
        ("*.rs", "src/file.rs", false),
        ("src/a?.rs", "src/a.rs", false),
    ] {
        assert_eq!(
            paths::matches_glob(pattern, path),
            Some(expected),
            "{pattern}/{path}"
        );
    }
    for body in [
        "\x60../outside.rs\x60",
        "\x60src/**\x60",
        "\x60git show src/a.rs\x60",
        "\x60src/unclosed.rs",
    ] {
        assert!(paths::mentioned_paths(body).incomplete, "{body}");
    }
}

#[test]
fn extensionless_root_files_match_builtin_and_added_protection_patterns() {
    let mut policy = policy();
    policy.protection.paths.push("Makefile".into());
    for path in [
        "credentials",
        "credentials-prod",
        "credentials-prod@v1",
        "Makefile",
    ] {
        let mut current = source();
        set_body(
            &mut current,
            format!("{VALID_BODY}\n변경 파일: \x60{path}\x60"),
        );
        let result = run(Some(&current), &policy, None).expect("protected root file");
        assert_eq!(
            result.eligibility(),
            IssueTriageEligibility::Eligible,
            "{path}"
        );
        assert_eq!(result.readiness(), IssueTriageReadiness::Ready, "{path}");
        assert_eq!(
            result.risk_class(),
            IssueAutomationRiskClass::Normal,
            "{path}"
        );
        assert!(
            result
                .reasons()
                .contains(&IssueTriageReasonCode::ProtectedPath),
            "{path}"
        );
        assert!(
            !result
                .reasons()
                .contains(&IssueTriageReasonCode::UnsupportedContent),
            "{path}"
        );
        assert!(
            result
                .gates()
                .iter()
                .all(|gate| gate.action == IssueAutomationGateAction::Human),
            "{path}"
        );
        assert_eq!(
            result.observation().disposition,
            IssueObservationDisposition::HumanApproval,
            "{path}"
        );
        assert_eq!(
            result,
            run(Some(&current), &policy, Some(&result)).expect("same root file")
        );
    }
}

#[test]
fn unsupported_extensionless_root_files_require_human_approval() {
    for path in [
        "credentials!",
        "credentials*",
        "credentials?",
        "credentials[prod]",
        "credentials{prod}",
        "credentials:prod",
    ] {
        let mut current = source();
        set_body(
            &mut current,
            format!("{VALID_BODY}\n변경 파일: \x60{path}\x60"),
        );
        let result = run(Some(&current), &policy(), None).expect("unsupported root file");
        assert_eq!(
            result.eligibility(),
            IssueTriageEligibility::Eligible,
            "{path}"
        );
        assert_eq!(result.readiness(), IssueTriageReadiness::Ready, "{path}");
        assert_eq!(
            result.risk_class(),
            IssueAutomationRiskClass::High,
            "{path}"
        );
        assert!(
            result
                .reasons()
                .contains(&IssueTriageReasonCode::UnsupportedContent),
            "{path}"
        );
        assert!(
            result
                .gates()
                .iter()
                .all(|gate| gate.action == IssueAutomationGateAction::Human),
            "{path}"
        );
        assert_eq!(
            result.observation().disposition,
            IssueObservationDisposition::HumanApproval,
            "{path}"
        );
    }
}

#[test]
fn unprotected_root_names_and_inline_literals_do_not_require_human_approval() {
    let mut current = source();
    set_body(
        &mut current,
        format!(
            "{VALID_BODY}\n\x60README\x60 \x60retries\x60 \x60true\x60 \x60cargo test\x60 \x60<!--\x60 \x60\x60<!-- \x60 note \x60 -->\x60\x60"
        ),
    );
    let result = run(Some(&current), &policy(), None).expect("ordinary inline content");
    assert_eq!(result.eligibility(), IssueTriageEligibility::Eligible);
    assert_eq!(result.readiness(), IssueTriageReadiness::Ready);
    assert_eq!(result.risk_class(), IssueAutomationRiskClass::Normal);
    assert!(
        !result
            .reasons()
            .contains(&IssueTriageReasonCode::ProtectedPath)
    );
    assert!(
        !result
            .reasons()
            .contains(&IssueTriageReasonCode::UnsupportedContent)
    );
    assert_eq!(result.gates()[0].action, IssueAutomationGateAction::Auto);
    assert_eq!(
        result.observation().disposition,
        IssueObservationDisposition::AutomaticStart
    );
}

#[test]
fn extensionless_root_file_limits_remain_fail_closed() {
    let mut current = source();
    let body = format!("{VALID_BODY}\n\x60credentials{}\x60", "x".repeat(1024));
    set_body(&mut current, body);
    let result = run(Some(&current), &policy(), None).expect("oversized root file");
    assert_eq!(result.eligibility(), IssueTriageEligibility::Eligible);
    assert_eq!(result.readiness(), IssueTriageReadiness::Ready);
    assert_eq!(result.risk_class(), IssueAutomationRiskClass::High);
    assert!(
        result
            .reasons()
            .contains(&IssueTriageReasonCode::UnsupportedContent)
    );
    assert!(
        result
            .gates()
            .iter()
            .all(|gate| gate.action == IssueAutomationGateAction::Human)
    );
    assert_eq!(
        result.observation().disposition,
        IssueObservationDisposition::HumanApproval
    );
}

#[test]
fn reopened_comments_cannot_supply_completion_criteria() {
    for body in [
        "<!-- note --> <!--\n## 완료 기준\n- hidden criterion\n-->",
        "<!--\n--> <!-- note --> <!--\n## 완료 기준\n- hidden criterion\n-->",
        "## 완료 기준\n<!-- note --> <!--\n- hidden criterion\n-->",
    ] {
        assert_eq!(
            criteria::count_criteria(body),
            Err(criteria::CriteriaProblem::Missing),
            "{body}"
        );
        let mut current = source();
        set_body(&mut current, body);
        let result = run(Some(&current), &policy(), None).expect("hidden criteria are absent");
        assert_eq!(result.readiness(), IssueTriageReadiness::NeedsInfo);
        assert!(
            result
                .reasons()
                .contains(&IssueTriageReasonCode::CriteriaMissing)
        );
        assert!(result.gates().iter().all(|gate| !gate.enabled));
        assert_ne!(
            result.observation().disposition,
            IssueObservationDisposition::AutomaticStart
        );
    }
}

#[test]
fn comment_mixed_path_lines_require_human_even_with_valid_criteria() {
    let mut policy = policy();
    policy.protection.paths.push("src/protected/**".into());
    for line in [
        "변경 파일: \x60src/protected/config.rs\x60 <!-- note -->",
        "<!-- note --> 변경 파일: \x60src/protected/config.rs\x60",
        "<!--\n--> 변경 파일: \x60src/protected/config.rs\x60 <!-- note -->",
        "변경 파일: \x60src/protected/config.rs\x60 <!-- note --> <!--\nhidden\n-->",
    ] {
        let body = format!("{VALID_BODY}\n{line}");
        let paths = paths::mentioned_paths(&body);
        assert!(paths.incomplete, "{line}");
        let mut current = source();
        set_body(&mut current, body);
        let result = run(Some(&current), &policy, None).expect("mixed line is uncertain");
        assert_eq!(result.eligibility(), IssueTriageEligibility::Eligible);
        assert_eq!(result.readiness(), IssueTriageReadiness::Ready);
        assert_eq!(result.risk_class(), IssueAutomationRiskClass::High);
        assert!(
            result
                .reasons()
                .contains(&IssueTriageReasonCode::UnsupportedContent)
        );
        assert!(
            result
                .gates()
                .iter()
                .all(|gate| gate.action == IssueAutomationGateAction::Human)
        );
        assert_eq!(
            result.observation().disposition,
            IssueObservationDisposition::HumanApproval
        );
    }
}

#[test]
fn comment_only_and_fenced_path_examples_leave_visible_paths_complete() {
    let mut policy = policy();
    policy.protection.paths.push("src/protected/**".into());
    for example in [
        "<!-- \x60src/protected/config.rs\x60 -->",
        "<!-- note --> <!--\n\x60src/protected/config.rs\x60\n-->",
        "<!--\n--> <!-- \x60src/protected/config.rs\x60 -->",
        "\x60\x60\x60\n변경 파일: \x60src/protected/config.rs\x60 <!-- note -->\n\x60\x60\x60",
    ] {
        let body = format!("{VALID_BODY}\n{example}\n변경 파일: \x60src/plain.rs\x60");
        let paths = paths::mentioned_paths(&body);
        assert_eq!(paths.paths, ["src/plain.rs"], "{example}");
        assert!(!paths.incomplete, "{example}");
        let mut current = source();
        set_body(&mut current, body);
        let result = run(Some(&current), &policy, None).expect("only the visible path counts");
        assert_eq!(result.readiness(), IssueTriageReadiness::Ready);
        assert_eq!(result.risk_class(), IssueAutomationRiskClass::Normal);
        assert_eq!(
            result.observation().disposition,
            IssueObservationDisposition::AutomaticStart
        );
    }
}

#[test]
fn inline_code_comment_text_preserves_criteria_and_visible_protected_paths() {
    let mut policy = policy();
    policy.protection.paths.push("src/protected/**".into());
    for body in [
        "## 완료 기준\n- 문자열 \x60<!--\x60를 출력한다\n변경 파일: \x60src/protected/config.rs\x60",
        "## 완료 기준\n- 문자열 \x60\x60<!-- \x60 note \x60 -->\x60\x60를 출력한다\n변경 파일: \x60src/protected/config.rs\x60",
        "## 완료 기준\n- 문자열 \x60<!--\x60와 파일 \x60src/protected/config.rs\x60를 표시한다",
    ] {
        assert_eq!(criteria::count_criteria(body), Ok(1), "{body}");
        let paths = paths::mentioned_paths(body);
        assert_eq!(paths.paths, ["src/protected/config.rs"], "{body}");
        assert!(!paths.incomplete, "{body}");
        let mut current = source();
        set_body(&mut current, body);
        let result = run(Some(&current), &policy, None).expect("inline code is literal text");
        assert_eq!(result.readiness(), IssueTriageReadiness::Ready);
        assert!(
            result
                .reasons()
                .contains(&IssueTriageReasonCode::ProtectedPath)
        );
        assert!(
            result
                .gates()
                .iter()
                .all(|gate| gate.action == IssueAutomationGateAction::Human)
        );
        assert_eq!(
            result.observation().disposition,
            IssueObservationDisposition::HumanApproval
        );
    }
}

#[test]
fn unmatched_inline_code_and_comment_mixtures_remain_incomplete() {
    let policy = policy();
    for line in [
        "변경 파일: \x60src/protected/config.rs",
        "변경 파일: \x60src/protected/config.rs <!-- note -->",
        "변경 파일: \x60src/protected/config.rs <!--\n\x60src/hidden.rs\x60\n-->",
    ] {
        let body = format!("{VALID_BODY}\n{line}");
        let paths = paths::mentioned_paths(&body);
        assert!(paths.incomplete, "{line}");
        assert!(paths.paths.is_empty(), "{line}");
        let mut current = source();
        set_body(&mut current, body);
        let result = run(Some(&current), &policy, None).expect("unclosed span is not safe");
        assert_eq!(result.readiness(), IssueTriageReadiness::Ready);
        assert_eq!(result.risk_class(), IssueAutomationRiskClass::High);
        assert!(
            result
                .gates()
                .iter()
                .all(|gate| gate.action == IssueAutomationGateAction::Human)
        );
        assert_eq!(
            result.observation().disposition,
            IssueObservationDisposition::HumanApproval
        );
    }
}

#[test]
fn risk_rule_order_is_preserved_and_unmatched_rules_default_to_normal() {
    let current = source();
    let mut policy = policy();
    policy.classify.rule = vec![
        IssueAutomationRiskRule {
            class: IssueAutomationRiskClass::Low,
            labels_any: vec!["akra:auto".into()],
            paths_any: Vec::new(),
        },
        IssueAutomationRiskRule {
            class: IssueAutomationRiskClass::High,
            labels_any: vec!["akra:auto".into()],
            paths_any: Vec::new(),
        },
    ];
    assert_eq!(
        run(Some(&current), &policy, None)
            .expect("first match")
            .risk_class(),
        IssueAutomationRiskClass::Low
    );
    policy.classify.rule.reverse();
    assert_eq!(
        run(Some(&current), &policy, None)
            .expect("first match")
            .risk_class(),
        IssueAutomationRiskClass::High
    );
    policy.classify.rule.clear();
    assert_eq!(
        run(Some(&current), &policy, None)
            .expect("no match")
            .risk_class(),
        IssueAutomationRiskClass::Normal
    );
}

#[test]
fn high_risk_held_manual_protected_paths_and_added_rules_force_human() {
    let mut policy = policy();
    policy.classify.rule = vec![IssueAutomationRiskRule {
        class: IssueAutomationRiskClass::High,
        labels_any: vec!["security".into()],
        paths_any: Vec::new(),
    }];
    for condition in 0..5 {
        let mut current = source();
        let issue = issue();
        let requirements = IssueRequirements::default();
        if condition == 0 {
            observed(&mut current).labels.items.push(IssueSourceLabel {
                id: "L_security".into(),
                name: "security".into(),
            });
        }
        if condition == 1 {
            observed(&mut current).labels.items.push(IssueSourceLabel {
                id: "L_hold".into(),
                name: "akra:hold".into(),
            });
        }
        if condition == 2 {
            observed(&mut current).labels.items.push(IssueSourceLabel {
                id: "L_manual".into(),
                name: "akra:manual".into(),
            });
        }
        if condition == 3 {
            set_body(
                &mut current,
                format!("{VALID_BODY}\n범위: \x60AGENTS.md\x60"),
            );
        }
        let mut request = input(&issue, Some(&current), &policy, &requirements, None);
        request.held = condition == 4;
        let result = triage_issue(request).expect("human condition");
        assert!(
            result
                .gates()
                .iter()
                .all(|gate| gate.action == IssueAutomationGateAction::Human),
            "condition {condition}"
        );
        assert_eq!(
            result.observation().disposition,
            IssueObservationDisposition::HumanApproval
        );
    }
    policy
        .human_required
        .additional
        .push(IssueAutomationAdditionalHumanRule {
            labels_any: vec!["akra:auto".into()],
            paths_any: Vec::new(),
            gates: vec![IssueAutomationGate::Start],
        });
    let result = run(Some(&source()), &policy, None).expect("additional rule");
    assert!(
        result
            .reasons()
            .contains(&IssueTriageReasonCode::PolicyHumanRequired)
    );
    assert_eq!(result.gates()[0].action, IssueAutomationGateAction::Human);
}

#[test]
fn actual_gates_and_counterfactual_observation_stay_separate_under_the_user_cap() {
    let current = source();
    let mut policy = policy();
    policy.binding.github.mode = IssueAutomationAutonomy::Observe;
    let result = run(Some(&current), &policy, None).expect("observe");
    assert!(
        result
            .gates()
            .iter()
            .all(|gate| !gate.enabled && gate.action == IssueAutomationGateAction::Human)
    );
    assert!(result.observation().observation_only);
    assert_eq!(
        result.observation().assumed_mode,
        IssueAutomationAutonomy::Auto
    );
    assert_eq!(
        result.observation().disposition,
        IssueObservationDisposition::AutomaticStart
    );
    for (cap, disposition) in [
        (
            IssueAutomationAutonomy::Off,
            IssueObservationDisposition::ObservationOnly,
        ),
        (
            IssueAutomationAutonomy::Observe,
            IssueObservationDisposition::ObservationOnly,
        ),
        (
            IssueAutomationAutonomy::Propose,
            IssueObservationDisposition::HumanApproval,
        ),
    ] {
        policy.authority.autonomy_cap = cap;
        policy.binding.github.mode = IssueAutomationAutonomy::Off;
        let result = run(Some(&current), &policy, None).expect("capped report");
        assert_eq!(result.observation().assumed_mode, cap);
        assert_eq!(result.observation().disposition, disposition);
    }
    policy.authority.autonomy_cap = IssueAutomationAutonomy::Gated;
    policy.gates.gated.normal.start = IssueAutomationGateAction::TimeoutArbiter;
    assert_eq!(
        run(Some(&current), &policy, None)
            .expect("timed report")
            .observation()
            .disposition,
        IssueObservationDisposition::Deferred
    );
}

#[test]
fn auto_delivery_and_merge_are_policy_results_while_close_remains_disabled() {
    let result = run(Some(&source()), &policy(), None).expect("auto policy");
    for gate in result.gates() {
        if matches!(
            gate.gate,
            IssueAutomationGate::Delivery | IssueAutomationGate::Merge
        ) {
            assert_eq!(gate.action, IssueAutomationGateAction::Auto);
        }
        if gate.gate == IssueAutomationGate::Close {
            assert_eq!(gate.action, IssueAutomationGateAction::Human);
            assert!(!gate.enabled);
        }
    }
    assert!(
        result
            .reasons()
            .contains(&IssueTriageReasonCode::ExecutionGrantRequired)
    );
    assert!(
        result
            .reasons()
            .contains(&IssueTriageReasonCode::CloseDisabled)
    );
}

#[test]
fn withheld_unknown_and_legacy_never_reuse_an_old_shared_snapshot() {
    let policy = policy();
    let mut current = source();
    let first = run(Some(&current), &policy, None).expect("shared");
    let snapshot = snapshot(&mut current).clone();
    observed(&mut current).content = IssueSourceContent::Withheld {
        basis: snapshot.basis, policy_version: snapshot.policy_version,
        reason: crate::domain::issue_automation::source::IssueContentWithheldReason::UserConfirmationRequired,
    };
    let withheld = run(Some(&current), &policy, Some(&first)).expect("withheld");
    assert_eq!(withheld.readiness(), IssueTriageReadiness::NeedsInfo);
    assert!(
        withheld
            .reasons()
            .contains(&IssueTriageReasonCode::DisclosureConfirmationRequired)
    );
    assert!(
        withheld
            .gates()
            .iter()
            .all(|gate| gate.action == IssueAutomationGateAction::Human)
    );
    for current in [
        Some(IssueSourceRead::Unknown {
            issue: issue(),
            reason: IssueSourceFailure::ProviderUnavailable,
        }),
        None,
    ] {
        let result = run(current.as_ref(), &policy, Some(&first)).expect("unknown");
        assert_eq!(result.eligibility(), IssueTriageEligibility::Unknown);
        assert_eq!(result.readiness(), IssueTriageReadiness::Unknown);
        assert!(result.basis().field_hash.is_none());
        assert!(result.basis().disclosure_policy_version.is_none());
        assert!(result.gates().iter().all(|gate| gate.gate_revision == 2));
        assert!(
            !serde_json::to_string(&result)
                .expect("projection")
                .contains("공유 제목")
        );
    }
}

#[test]
fn stale_disclosure_and_masking_do_not_turn_hidden_content_into_ready_auto_work() {
    let mut current = source();
    snapshot(&mut current).policy_version = "c".repeat(64);
    let result = run(Some(&current), &policy(), None).expect("old disclosure");
    assert_eq!(result.readiness(), IssueTriageReadiness::Unknown);
    assert!(
        result
            .reasons()
            .contains(&IssueTriageReasonCode::DisclosurePolicyStale)
    );
    let mut current = source();
    set_body(&mut current, "## 완료 기준\n- [[akra-mask:1]]\n");
    let result = run(Some(&current), &policy(), None).expect("masked");
    assert_eq!(result.readiness(), IssueTriageReadiness::NeedsInfo);
    assert_eq!(result.risk_class(), IssueAutomationRiskClass::High);
    assert!(
        result
            .reasons()
            .contains(&IssueTriageReasonCode::MaskedContent)
    );
}

#[test]
fn metadata_changes_with_the_same_field_hash_invalidate_old_gate_revisions() {
    let current = source();
    let policy = policy();
    let first = run(Some(&current), &policy, None).expect("first");
    for mutation in 0..5 {
        let mut changed = current.clone();
        let observation = observed(&mut changed);
        match mutation {
            0 => observation.labels.items.push(IssueSourceLabel {
                id: "L_extra".into(),
                name: "extra".into(),
            }),
            1 => observation.assignees.items.push(actor("U_other")),
            2 => observation.linked_pull_request = IssuePrPresence::Present,
            3 => observation.author = Some(actor("U_other")),
            4 => observation.labels.completeness = IssueMetadataCompleteness::Truncated,
            _ => unreachable!(),
        }
        let result = run(Some(&changed), &policy, Some(&first)).expect("new evidence");
        assert_eq!(result.basis().field_hash, first.basis().field_hash);
        assert_ne!(result.input_fingerprint(), first.input_fingerprint());
        assert!(result.gates().iter().all(|gate| gate.gate_revision == 2));
        assert!(
            first
                .validate_current(&issue(), Some(&changed), false)
                .is_err()
        );
    }
}

#[test]
fn display_names_provider_timestamps_and_set_order_do_not_change_semantic_input() {
    let mut current = source();
    observed(&mut current).labels.items.push(IssueSourceLabel {
        id: "L_extra".into(),
        name: "extra".into(),
    });
    let policy = policy();
    let first = run(Some(&current), &policy, None).expect("first");
    let observation = observed(&mut current);
    observation.labels.items.reverse();
    observation
        .labels
        .items
        .push(observation.labels.items[0].clone());
    observation.author.as_mut().expect("author").login = "new-display-name".into();
    snapshot(&mut current).basis.provider_updated_at = "2026-11-21T00:00:00Z".into();
    assert_eq!(
        run(Some(&current), &policy, Some(&first)).expect("same meaning"),
        first
    );
}

#[test]
fn body_policy_requirements_pinned_commit_and_held_state_are_all_freshness_inputs() {
    let current = source();
    let policy = policy();
    let first = run(Some(&current), &policy, None).expect("first");
    let issue = issue();
    for mutation in 0..6 {
        let mut changed = current.clone();
        let mut requirements = IssueRequirements::default();
        let other_version = "b".repeat(64);
        let pinned = "f".repeat(40);
        if mutation == 0 {
            set_body(&mut changed, format!("{VALID_BODY}\n추가 본문"));
        }
        if mutation == 2 {
            requirements.required_sections.push("새 섹션".into());
        }
        if mutation == 3 {
            snapshot(&mut changed).policy_version = other_version.clone();
        }
        let mut request = input(&issue, Some(&changed), &policy, &requirements, Some(&first));
        if mutation == 1 {
            request.automation_policy_version = &other_version;
        }
        if mutation == 3 {
            request.validation_policy_version = &other_version;
        }
        if mutation == 4 {
            request.pinned_source_commit = Some(&pinned);
        }
        if mutation == 5 {
            request.held = true;
        }
        let result = triage_issue(request).expect("freshness input changed");
        assert_ne!(
            result.input_fingerprint(),
            first.input_fingerprint(),
            "case {mutation}"
        );
        assert!(result.gates().iter().all(|gate| gate.gate_revision == 2));
    }
}

#[test]
fn hydration_rejects_invalid_hashes_identity_gate_order_revisions_and_execution_claims() {
    let first = run(Some(&source()), &policy(), None).expect("first");
    for mutation in 0..11 {
        let mut record = first.to_record();
        match mutation {
            0 => record.basis.issue.issue_id.clear(),
            1 => record.basis.evidence_sha256 = "invalid".into(),
            2 => record.basis.field_hash = None,
            3 => record.basis.source = IssueTriageSourceState::Unknown,
            4 => record.gates[0].gate_revision = 0,
            5 => record.gates[0].gate_revision = MAX_ISSUE_AUTOMATION_REVISION + 1,
            6 => record.gates.swap(0, 1),
            7 => record.gates[5].enabled = true,
            8 => record.gates[4].action = IssueAutomationGateAction::TimeoutArbiter,
            9 => record.observation.observation_only = false,
            10 => record.risk_class = IssueAutomationRiskClass::High,
            _ => unreachable!(),
        }
        assert!(
            IssueTriageAssessment::hydrate(record).is_err(),
            "case {mutation}"
        );
    }
    let encoded = serde_json::to_value(&first).expect("encoded");
    assert_eq!(
        serde_json::from_value::<IssueTriageAssessment>(encoded.clone()).expect("checked decode"),
        first
    );
    let mut tampered = encoded;
    tampered["gates"][0]["gateRevision"] = serde_json::json!(0);
    assert!(serde_json::from_value::<IssueTriageAssessment>(tampered).is_err());
}

#[test]
fn successor_validation_rejects_forged_revisions_and_overflow_is_fail_closed() {
    let current = source();
    let policy = policy();
    let first = run(Some(&current), &policy, None).expect("first");
    let mut record = first.to_record();
    for gate in &mut record.gates {
        gate.gate_revision = 2;
    }
    let forged = IssueTriageAssessment::hydrate(record).expect("shape");
    assert!(forged.validate_successor(Some(&first)).is_err());
    assert!(forged.validate_successor(None).is_err());
    let mut record = first.to_record();
    for gate in &mut record.gates {
        gate.gate_revision = MAX_ISSUE_AUTOMATION_REVISION;
    }
    let exhausted = IssueTriageAssessment::hydrate(record).expect("last valid revision");
    assert_eq!(
        run(Some(&current), &policy, Some(&exhausted)).expect("unchanged"),
        exhausted
    );
    let mut changed = current;
    set_body(&mut changed, format!("{VALID_BODY}\n변경"));
    assert_eq!(
        run(Some(&changed), &policy, Some(&exhausted)),
        Err(IssueTriageError::RevisionExhausted)
    );
}

#[test]
fn assessment_keeps_original_field_hash_but_no_shared_or_private_body_text() {
    let mut current = source();
    let original_hash = IssueFieldHash::of_fields("private title marker", "private body marker");
    snapshot(&mut current).basis.field_hash = original_hash.clone();
    let result = run(Some(&current), &policy(), None).expect("shared assessment");
    assert_eq!(result.basis().field_hash, Some(original_hash));
    let json = serde_json::to_string(&result).expect("projection");
    assert!(!json.contains("private title marker"));
    assert!(!json.contains("private body marker"));
    assert!(!json.contains("공유 제목"));
    assert!(!json.contains("결과가 저장된다"));
}

fn assert_zero_budget_requires_human(mut policy: IssueAutomationPolicy) {
    for mode in [
        IssueAutomationAutonomy::Auto,
        IssueAutomationAutonomy::Observe,
    ] {
        for configured in [
            IssueAutomationGateAction::Auto,
            IssueAutomationGateAction::TimeoutDefault,
            IssueAutomationGateAction::TimeoutArbiter,
        ] {
            policy.binding.github.mode = mode;
            policy.gates.auto.normal.start = configured;
            let result =
                run(Some(&source()), &policy, None).expect("zero budget is a human prerequisite");
            assert_eq!(result.eligibility(), IssueTriageEligibility::Eligible);
            assert_eq!(result.readiness(), IssueTriageReadiness::Ready);
            assert!(
                result
                    .reasons()
                    .contains(&IssueTriageReasonCode::BudgetExhausted)
            );
            assert!(
                result
                    .gates()
                    .iter()
                    .all(|gate| gate.action == IssueAutomationGateAction::Human)
            );
            assert_eq!(
                result.gates()[0].enabled,
                mode == IssueAutomationAutonomy::Auto
            );
            assert_eq!(
                result.observation().disposition,
                IssueObservationDisposition::HumanApproval
            );
            if mode == IssueAutomationAutonomy::Auto {
                let mut forged = result.to_record();
                forged.gates[0].action = configured;
                assert!(IssueTriageAssessment::hydrate(forged).is_err());
            }
            for disposition in [
                IssueObservationDisposition::AutomaticStart,
                IssueObservationDisposition::Deferred,
            ] {
                let mut forged = result.to_record();
                forged.observation.disposition = disposition;
                assert!(IssueTriageAssessment::hydrate(forged).is_err());
            }
        }
    }
}

#[test]
fn zero_running_budget_requires_human_without_changing_readiness() {
    let mut policy = policy();
    policy.budget.max_running = 0;
    assert_zero_budget_requires_human(policy);
}

#[test]
fn zero_daily_start_budget_blocks_automatic_and_timed_starts() {
    let mut policy = policy();
    policy.budget.max_started_per_day = 0;
    assert_zero_budget_requires_human(policy);
}

#[test]
fn zero_attempt_budget_is_not_treated_as_unlimited_attempts() {
    let mut policy = policy();
    policy.budget.max_attempts = 0;
    assert_zero_budget_requires_human(policy);
}

#[test]
fn draft_readiness_and_live_triage_use_the_same_content_risk_and_gate_rules() {
    let cases = [
        (VALID_BODY, Vec::<String>::new()),
        ("## 완료 기준\n- \n", Vec::new()),
        ("## 완료 기준\n목록이 아닌 글\n", Vec::new()),
        ("## 완료 기준\n- 결과 확인\n## 범위\n", Vec::new()),
        (
            "## 완료 기준\n- 결과 확인\n범위: \x60src/protected/config.rs\x60\n",
            Vec::new(),
        ),
        ("## 완료 기준\n- 결과 확인\n[removed]\n", Vec::new()),
        (VALID_BODY, vec!["akra:gated".to_owned()]),
        (VALID_BODY, vec!["akra:manual".to_owned()]),
        (VALID_BODY, vec!["akra:hold".to_owned()]),
    ];
    for (body, extra_labels) in cases {
        let mut current = source();
        set_body(&mut current, body);
        for (index, name) in extra_labels.into_iter().enumerate() {
            observed(&mut current).labels.items.push(IssueSourceLabel {
                id: format!("L_extra_{index}"),
                name,
            });
        }
        let mut policy = policy();
        policy.protection.paths.push("src/protected/**".to_owned());
        let requirements = IssueRequirements {
            required_sections: if body.contains("## 범위") {
                vec!["범위".to_owned()]
            } else {
                Vec::new()
            },
            ..IssueRequirements::default()
        };
        let IssueSourceRead::Observed(observation) = &current else {
            unreachable!()
        };
        let IssueSourceContent::Shared(snapshot) = &observation.content else {
            unreachable!()
        };
        let labels: Vec<_> = observation
            .labels
            .items
            .iter()
            .map(|label| label.name.clone())
            .collect();
        let preview = inspect_issue_readiness(IssueReadinessInput {
            provider: crate::domain::issue_work::IssueProvider::GitHub,
            shared_copy: &snapshot.shared_copy,
            requirements: &requirements,
            policy: &policy,
            labels: &labels,
            labels_complete: true,
            held: false,
        })
        .unwrap();
        let live = triage_issue(input(
            &issue(),
            Some(&current),
            &policy,
            &requirements,
            None,
        ))
        .unwrap();
        assert_eq!(live.eligibility(), IssueTriageEligibility::Eligible);
        assert_eq!(preview.readiness, live.readiness(), "{body}");
        assert_eq!(preview.risk_class, live.risk_class(), "{body}");
        assert_eq!(preview.effective_mode, live.effective_mode());
        assert_eq!(preview.assumed_mode, live.observation().assumed_mode);
        assert_eq!(preview.reasons, live.reasons());
        assert_eq!(preview.expected_gates.len(), live.gates().len());
        for (expected, actual) in preview.expected_gates.iter().zip(live.gates()) {
            assert_eq!(
                (expected.gate, expected.action),
                (actual.gate, actual.action),
                "{body}"
            );
        }
    }
}

#[test]
fn readiness_reports_missing_sections_and_never_needs_a_remote_identity() {
    let copy = IssueSharedCopy {
        title: String::new(),
        body: "## 범위\n_No response_\n".into(),
    };
    let requirements = IssueRequirements {
        required_sections: vec!["범위".to_owned(), "하지 않는 것".to_owned()],
        ..IssueRequirements::default()
    };
    let policy = policy();
    let labels = vec![policy.labels.auto.clone()];
    let result = inspect_issue_readiness(IssueReadinessInput {
        provider: crate::domain::issue_work::IssueProvider::GitHub,
        shared_copy: &copy,
        requirements: &requirements,
        policy: &policy,
        labels: &labels,
        labels_complete: true,
        held: false,
    })
    .unwrap();
    assert_eq!(result.readiness, IssueTriageReadiness::NeedsInfo);
    assert_eq!(
        result.missing_sections,
        ["범위", "완료 기준", "하지 않는 것"]
    );
    assert!(
        result
            .reasons
            .contains(&IssueTriageReasonCode::TitleMissing)
    );
    assert!(
        result
            .reasons
            .contains(&IssueTriageReasonCode::CriteriaMissing)
    );
    assert!(!result.validation_errors.is_empty());
    assert!(
        result
            .expected_gates
            .iter()
            .all(|gate| gate.action == IssueAutomationGateAction::Human)
    );
}

#[test]
fn configurable_labels_lower_modes_and_old_names_no_longer_act_as_hidden_controls() {
    let mut policy = policy();
    policy.labels.auto = "work:auto".into();
    policy.labels.gated = "work:gated".into();
    policy.labels.manual = "work:manual".into();
    policy.labels.hold = "work:hold".into();
    policy.select.labels_any = policy.labels.selection_labels();
    for (labels, expected_mode, held) in [
        (vec!["work:auto"], IssueAutomationAutonomy::Auto, false),
        (
            vec!["work:auto", "work:gated"],
            IssueAutomationAutonomy::Gated,
            false,
        ),
        (
            vec!["work:auto", "work:manual"],
            IssueAutomationAutonomy::Propose,
            true,
        ),
        (
            vec!["work:auto", "work:hold"],
            IssueAutomationAutonomy::Auto,
            true,
        ),
        (
            vec!["work:auto", "akra:manual", "akra:hold"],
            IssueAutomationAutonomy::Auto,
            false,
        ),
    ] {
        let mut current = source();
        observed(&mut current).labels.items = labels
            .into_iter()
            .enumerate()
            .map(|(index, name)| IssueSourceLabel {
                id: format!("L_{index}"),
                name: name.into(),
            })
            .collect();
        let result = run(Some(&current), &policy, None).unwrap();
        assert_eq!(result.eligibility(), IssueTriageEligibility::Eligible);
        assert_eq!(result.effective_mode(), expected_mode);
        assert_eq!(
            result
                .reasons()
                .contains(&IssueTriageReasonCode::HeldOrManual),
            held
        );
        if held {
            assert!(
                result
                    .gates()
                    .iter()
                    .all(|gate| gate.action == IssueAutomationGateAction::Human)
            );
        }
    }
    policy.authority.autonomy_cap = IssueAutomationAutonomy::Propose;
    policy.binding.github.mode = IssueAutomationAutonomy::Propose;
    let copy = IssueSharedCopy {
        title: "제목".into(),
        body: VALID_BODY.into(),
    };
    for label in ["work:auto", "work:gated"] {
        let labels = vec![label.to_owned()];
        let result = inspect_issue_readiness(IssueReadinessInput {
            provider: crate::domain::issue_work::IssueProvider::GitHub,
            shared_copy: &copy,
            requirements: &IssueRequirements::default(),
            policy: &policy,
            labels: &labels,
            labels_complete: true,
            held: false,
        })
        .unwrap();
        assert_eq!(result.effective_mode, IssueAutomationAutonomy::Propose);
        assert_eq!(result.assumed_mode, IssueAutomationAutonomy::Propose);
    }
}

#[test]
fn bot_unknown_and_locally_created_issues_need_current_trusted_human_selection() {
    use crate::domain::issue_automation::source::{
        IssueCreationProvenance as Origin, IssueSourceActorKind as Kind,
    };
    for (kind, provenance) in [
        (Kind::UserAccount, Origin::AkraCreated),
        (Kind::UserAccount, Origin::Unresolved),
        (Kind::Bot, Origin::Unattributed),
        (Kind::Unknown, Origin::Unattributed),
    ] {
        let policy = policy();
        let mut current = source();
        let observation = observed(&mut current);
        observation.author.as_mut().unwrap().kind = kind;
        observation.creation_provenance = provenance;
        let blocked = run(Some(&current), &policy, None).unwrap();
        assert_eq!(
            blocked.eligibility(),
            IssueTriageEligibility::Ineligible,
            "{kind:?}/{provenance:?}"
        );
        assert!(
            blocked
                .reasons()
                .contains(&IssueTriageReasonCode::HumanSelectionRequired)
        );
        assert!(blocked.gates().iter().all(|gate| !gate.enabled));
        let observation = observed(&mut current);
        observation
            .label_actors
            .items
            .push(trusted_event(observation));
        let selected = run(Some(&current), &policy, None).unwrap();
        assert_eq!(selected.eligibility(), IssueTriageEligibility::Eligible);
        assert!(
            !selected
                .reasons()
                .contains(&IssueTriageReasonCode::HumanSelectionRequired)
        );
    }
}

#[test]
fn bot_self_label_and_ambiguous_or_stale_label_events_cannot_select_an_agent_issue() {
    use crate::domain::issue_automation::source::{
        IssueCreationProvenance as Origin, IssueSourceActorKind as Kind,
    };
    for mutation in 0..8 {
        let policy = policy();
        let mut current = source();
        let observation = observed(&mut current);
        observation.creation_provenance = Origin::AkraCreated;
        observation
            .label_actors
            .items
            .push(trusted_event(observation));
        match mutation {
            0 => {
                observation.label_actors.items[0]
                    .actor
                    .as_mut()
                    .unwrap()
                    .kind = Kind::Bot
            }
            1 => {
                observation.label_actors.items[0]
                    .actor
                    .as_mut()
                    .unwrap()
                    .kind = Kind::Unknown
            }
            2 => observation.label_actors.items[0].actor.as_mut().unwrap().id = "U_other".into(),
            3 => observation.label_actors.items[0].label.id = "L_old".into(),
            4 => observation.label_actors.items[0].label.name = "old-name".into(),
            5 => observation.label_actors.completeness = IssueMetadataCompleteness::Unavailable,
            6 => {
                let mut event = observation.label_actors.items[0].clone();
                event.event_id = "E_other".into();
                event.actor.as_mut().unwrap().kind = Kind::Bot;
                observation.label_actors.items.push(event);
            }
            7 => {
                let mut event = observation.label_actors.items[0].clone();
                event.actor.as_mut().unwrap().kind = Kind::Unknown;
                observation.label_actors.items.push(event);
            }
            _ => unreachable!(),
        }
        let result = run(Some(&current), &policy, None).unwrap();
        assert_eq!(
            result.eligibility(),
            IssueTriageEligibility::Ineligible,
            "case {mutation}"
        );
        assert!(
            result
                .reasons()
                .contains(&IssueTriageReasonCode::HumanSelectionRequired)
        );
    }
}

#[test]
fn all_open_does_not_turn_an_unrelated_human_label_into_agent_selection() {
    use crate::domain::issue_automation::source::IssueCreationProvenance;
    let mut policy = policy();
    policy.select.mode = IssueAutomationSelectionMode::AllOpen;
    let mut current = source();
    let observation = observed(&mut current);
    observation.creation_provenance = IssueCreationProvenance::AkraCreated;
    observation.labels.items[0].name = "unrelated".into();
    observation
        .label_actors
        .items
        .push(trusted_event(observation));
    let result = run(Some(&current), &policy, None).unwrap();
    assert_eq!(result.eligibility(), IssueTriageEligibility::Ineligible);
    assert!(
        result
            .reasons()
            .contains(&IssueTriageReasonCode::HumanSelectionRequired)
    );
    observed(&mut current).labels.items[0].name = policy.labels.auto.clone();
    let observation = observed(&mut current);
    observation.label_actors.items = vec![trusted_event(observation)];
    assert_eq!(
        run(Some(&current), &policy, None).unwrap().eligibility(),
        IssueTriageEligibility::Eligible
    );
    policy.select.labels_any.clear();
    assert_eq!(
        run(Some(&current), &policy, None).unwrap().eligibility(),
        IssueTriageEligibility::Ineligible
    );
}

#[test]
fn account_kind_and_creation_provenance_are_semantic_evidence_even_with_the_same_actor_ids() {
    use crate::domain::issue_automation::source::{
        IssueCreationProvenance as Origin, IssueSourceActorKind as Kind,
    };
    let mut current = source();
    let observation = observed(&mut current);
    observation
        .label_actors
        .items
        .push(trusted_event(observation));
    let policy = policy();
    let first = run(Some(&current), &policy, None).unwrap();
    for mutation in 0..5 {
        let mut changed = current.clone();
        let observation = observed(&mut changed);
        match mutation {
            0 => observation.author.as_mut().unwrap().kind = Kind::Bot,
            1 => observation.author.as_mut().unwrap().kind = Kind::Unknown,
            2 => {
                observation.label_actors.items[0]
                    .actor
                    .as_mut()
                    .unwrap()
                    .kind = Kind::Bot
            }
            3 => observation.creation_provenance = Origin::AkraCreated,
            4 => observation.creation_provenance = Origin::Unresolved,
            _ => unreachable!(),
        }
        let result = run(Some(&changed), &policy, Some(&first)).unwrap();
        assert_eq!(result.basis().field_hash, first.basis().field_hash);
        assert_ne!(result.input_fingerprint(), first.input_fingerprint());
        assert!(
            first
                .validate_current(&issue(), Some(&changed), false)
                .is_err()
        );
        assert!(result.gates().iter().all(|gate| gate.gate_revision == 2));
    }
}

#[test]
fn old_valid_assessments_remain_readable_but_are_replaced_when_new_evidence_is_hashed() {
    let current = source();
    let policy = policy();
    let first = run(Some(&current), &policy, None).unwrap();
    let mut old = first.to_record();
    old.basis.evidence_sha256 = "e".repeat(64);
    let old = IssueTriageAssessment::hydrate(old).unwrap();
    old.validate_for(&issue()).unwrap();
    assert!(
        old.validate_current(&issue(), Some(&current), false)
            .is_err()
    );
    let refreshed = run(Some(&current), &policy, Some(&old)).unwrap();
    assert!(refreshed.gates().iter().all(|gate| gate.gate_revision == 2));
}
