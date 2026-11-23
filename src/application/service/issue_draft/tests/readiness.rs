//! The two preview facades share the pure evaluator while Publish keeps its own approval boundary.
use super::*;
use crate::application::port::outbound::issue_automation_policy_source_port::IssueAutomationPolicySourcePort;
use crate::application::service::issue_automation_policy::{
    IssueAutomationPolicyService, resolve_issue_automation_policy,
};
use crate::domain::issue_automation::IssueAutomationAutonomy;
use crate::domain::issue_automation::policy::{
    IssueAutomationGate, IssueAutomationGateAction, IssueAutomationRiskClass,
};
use crate::domain::issue_automation::triage::{
    IssueReadinessInput, IssueTriageReadiness, inspect_issue_readiness,
};
use crate::domain::issue_masking::plan_issue_masking;
use crate::domain::issue_secret_spans::secret_spans;

pub(super) struct AutomationFiles {
    pub(super) policy: Mutex<String>,
    pub(super) workspaces: Mutex<Vec<String>>,
}

impl IssueAutomationPolicySourcePort for AutomationFiles {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        Ok(Some(self.policy.lock().unwrap().clone()))
    }

    fn load_project_policy_toml(&self, workspace: &str) -> Result<Option<String>> {
        self.workspaces.lock().unwrap().push(workspace.to_owned());
        Ok(None)
    }
}

pub(super) fn wire(
    service: IssueDraftService,
    policy: &str,
) -> (IssueDraftService, Arc<AutomationFiles>) {
    let source = Arc::new(AutomationFiles {
        policy: Mutex::new(policy.to_owned()),
        workspaces: Mutex::new(Vec::new()),
    });
    (
        service.with_automation_policy(Arc::new(IssueAutomationPolicyService::new(source.clone()))),
        source,
    )
}

pub(super) fn current_text(body: &str) -> IssueDraftText {
    IssueDraftText {
        title: "초안의 준비 검사".into(),
        body: body.into(),
        spans: Vec::new(),
    }
}

pub(super) fn value(preview: &IssueDraftPreview) -> &IssueDraftReadiness {
    let result = match preview {
        IssueDraftPreview::Ready { readiness, .. }
        | IssueDraftPreview::NeedsConfirmation { readiness, .. }
        | IssueDraftPreview::InvalidSpan { readiness, .. } => readiness,
    };
    result
        .as_deref()
        .expect("every preview has an independent readiness result")
}

pub(super) fn assessed(preview: &IssueDraftPreview) -> &IssueDraftReadinessAssessment {
    let IssueDraftReadiness::Assessed { assessment } = value(preview) else {
        panic!("expected an assessed draft");
    };
    assessment
}

#[test]
fn readiness_matches_the_pure_evaluator_for_the_current_ai_handoff_copy() {
    let mut fixture = fixture(false);
    *fixture.files.policy.lock().unwrap() = String::new();
    let policy = r#"
[authority]
autonomy_cap = "auto"
[binding.github]
mode = "auto"
[[classify.rule]]
class = "low"
labels_any = ["docs"]
"#;
    let (service, source) = wire(fixture.service, policy);
    fixture.service = service;
    let text =
        current_text("## 완료 기준\n- 문서를 다시 읽어 확인한다.\n\n## 범위\n`docs/guide.md`");
    let labels = vec!["docs".to_owned()];
    let preview = fixture
        .service
        .preview(IssueDraftPreviewRequest {
            target: target(),
            text: text.clone(),
            expected_labels: labels.clone(),
        })
        .unwrap();
    let shown = assessed(&preview);
    let conventions = fixture.service.conventions(CWD).unwrap();
    let copy = IssueSharedCopy {
        title: text.title,
        body: text.body,
    };
    let policy = resolve_issue_automation_policy(Some(policy), None).unwrap();
    let expected = inspect_issue_readiness(IssueReadinessInput {
        provider: crate::domain::issue_work::IssueProvider::GitHub,
        shared_copy: &copy,
        requirements: &requirements(&conventions).unwrap(),
        policy: &policy.policy,
        labels: &labels,
        labels_complete: true,
        held: false,
    })
    .unwrap();
    assert_eq!(shown.readiness, expected.readiness);
    assert_eq!(shown.risk_class, IssueAutomationRiskClass::Low);
    assert_eq!(shown.risk_class, expected.risk_class);
    assert_eq!(
        shown
            .reasons
            .iter()
            .map(|reason| reason.code)
            .collect::<Vec<_>>(),
        expected.reasons
    );
    assert_eq!(
        shown
            .expected_gates
            .iter()
            .map(|gate| (gate.gate, gate.action))
            .collect::<Vec<_>>(),
        expected
            .expected_gates
            .iter()
            .map(|gate| (gate.gate, gate.action))
            .collect::<Vec<_>>(),
    );
    assert!(shown.preview_only);
    assert_eq!(*fixture.remote.writes.lock().unwrap(), 0);
    assert_eq!(*source.workspaces.lock().unwrap(), vec![CWD]);
    let json = serde_json::to_string(shown).unwrap();
    for forbidden in [
        "gateRevision",
        "enabled",
        "issueId",
        "acceptance",
        "title",
        "body",
    ] {
        assert!(!json.contains(&format!("\"{forbidden}\"")));
    }
}

#[test]
fn observation_readiness_distinguishes_the_gate_basis_from_the_report_assumption() {
    let mut fixture = fixture(false);
    *fixture.files.policy.lock().unwrap() = String::new();
    fixture.service = wire(
        fixture.service,
        r#"
[authority]
autonomy_cap = "auto"
[binding.github]
mode = "observe"
[gates.auto.normal]
start = "auto"
"#,
    )
    .0;
    let preview = fixture
        .service
        .preview(IssueDraftPreviewRequest {
            target: target(),
            text: current_text("## 완료 기준\n- 현재 관찰 결과를 확인한다."),
            expected_labels: Vec::new(),
        })
        .unwrap();
    let shown = assessed(&preview);
    assert_eq!(shown.readiness, IssueTriageReadiness::Ready);
    assert_eq!(shown.risk_class, IssueAutomationRiskClass::Normal);
    assert_eq!(shown.effective_mode, IssueAutomationAutonomy::Observe);
    assert_eq!(shown.assumed_mode, IssueAutomationAutonomy::Auto);
    assert_eq!(
        shown.mode_summary,
        "관문 기준: 관찰 · 관찰 보고의 가정 수준: 자동"
    );
    assert!(
        shown
            .expected_gates
            .iter()
            .any(|gate| gate.gate == IssueAutomationGate::Start)
    );
    assert!(
        shown
            .expected_gates
            .iter()
            .all(|gate| gate.action == IssueAutomationGateAction::Human)
    );
    let start = shown
        .expected_gates
        .iter()
        .find(|gate| gate.gate == IssueAutomationGate::Start)
        .unwrap();
    assert_eq!(start.action_summary, "사람 확인");
    assert_eq!(*fixture.remote.writes.lock().unwrap(), 0);
}

#[test]
fn readiness_uses_ai_handoff_rules_and_finds_unselected_secrets() {
    let mut fixture = fixture(false);
    *fixture.files.policy.lock().unwrap() =
        "[publish.spans]\nsecret = \"restorable_mask\"\n[ai_handoff.spans]\nsecret = \"remove\"\n"
            .into();
    fixture.service = wire(fixture.service, "").0;
    let token = "ghp_SyntheticReadinessBoundary000000";
    let text = current_text(&format!(
        "## 완료 기준\n- 출력에 비밀을 남기지 않는다.\n\npassword={token}"
    ));
    let mut spans = secret_spans(IssueTextField::Title, &text.title);
    spans.extend(secret_spans(IssueTextField::Body, &text.body));
    assert!(!spans.is_empty());
    let ai_copy = plan_issue_masking(
        &text.title,
        &text.body,
        &spans,
        fixture
            .service
            .conventions(CWD)
            .unwrap()
            .policy
            .disclosure_for(IssueDisclosureTarget::AiHandoff),
    )
    .unwrap()
    .shared_copy;
    let preview = fixture
        .service
        .preview(IssueDraftPreviewRequest {
            target: target(),
            text,
            expected_labels: Vec::new(),
        })
        .unwrap();
    let shown = assessed(&preview);
    assert_eq!(
        shown.shared_copy_hash,
        IssueFieldHash::of_fields(&ai_copy.title, &ai_copy.body)
    );
    let IssueDraftPreview::Ready { approval, .. } = &preview else {
        panic!("publish preview");
    };
    assert_ne!(shown.shared_copy_hash, approval.shared_copy_hash);
    assert!(
        !serde_json::to_string(value(&preview))
            .unwrap()
            .contains(token)
    );
}

#[test]
fn ai_confirmation_blocks_only_readiness_and_does_not_infer_publish_consent() {
    let mut fixture = fixture(false);
    *fixture.files.policy.lock().unwrap() =
        "[ai_handoff.spans]\nsecret = \"user_confirm\"\n".into();
    fixture.service = wire(fixture.service, "").0;
    let token = "ghp_SyntheticAiConfirmation000000";
    let mut text = current_text(&format!("## 완료 기준\n- 작동한다.\n{token}"));
    let start = text.body.find(token).unwrap();
    text.spans.push(IssueDraftSpan {
        field: IssueDraftField::Body,
        start: text.body[..start].encode_utf16().count(),
        end: text.body[..start + token.len()].encode_utf16().count(),
        class: Some(IssueSensitiveSpanClass::Secret),
        decision: Some(crate::domain::issue_policy::IssueSensitiveSpanAction::Remove),
    });
    let preview = fixture
        .service
        .preview(IssueDraftPreviewRequest {
            target: target(),
            text,
            expected_labels: Vec::new(),
        })
        .unwrap();
    assert!(matches!(
        preview,
        IssueDraftPreview::Ready {
            publishable: true,
            ..
        }
    ));
    assert!(matches!(
        value(&preview),
        IssueDraftReadiness::CannotAssess {
            reason: IssueDraftReadinessBlocker::DisclosureConfirmationRequired,
            ..
        }
    ));
    assert!(!serde_json::to_string(&preview).unwrap().contains(token));
}

#[test]
fn broken_automation_policy_does_not_change_publish_approval_or_recovery() {
    let mut fixture = fixture(false);
    *fixture.files.policy.lock().unwrap() = String::new();
    let (service, source) = wire(fixture.service, "");
    fixture.service = service;
    let text = current_text("## 완료 기준\n- 기대한 결과를 확인한다.");
    let before = fixture
        .service
        .preview(IssueDraftPreviewRequest {
            target: target(),
            text: text.clone(),
            expected_labels: Vec::new(),
        })
        .unwrap();
    let (_, original_approval, _) = ready(before);
    *source.policy.lock().unwrap() = "private-policy-canary = [".into();
    let after = fixture
        .service
        .preview(IssueDraftPreviewRequest {
            target: target(),
            text: text.clone(),
            expected_labels: vec!["akra:auto".into()],
        })
        .unwrap();
    assert!(matches!(
        value(&after),
        IssueDraftReadiness::CannotAssess {
            reason: IssueDraftReadinessBlocker::PolicyBlocked,
            ..
        }
    ));
    assert!(
        !serde_json::to_string(value(&after))
            .unwrap()
            .contains("private-policy-canary")
    );
    let (_, approval, publishable) = ready(after);
    assert!(publishable);
    assert_eq!(approval, original_approval);
    assert!(matches!(
        fixture
            .service
            .publish(publish_request(&fixture, text, approval))
            .unwrap(),
        IssueDraftPublished::Confirmed { .. }
    ));
    assert_eq!(*fixture.remote.writes.lock().unwrap(), 1);
    assert!(fixture.service.recheck(target()).unwrap().is_some());
}

#[test]
fn changed_drafts_and_assumed_labels_are_rechecked_without_a_remote_write() {
    let mut fixture = fixture(false);
    *fixture.files.policy.lock().unwrap() = String::new();
    fixture.service = wire(
        fixture.service,
        "[[classify.rule]]\nclass = \"high\"\nlabels_any = [\"sensitive\"]\n",
    )
    .0;
    for (body, labels, readiness, risk) in [
        (
            "## 완료 기준\n- 확인한다.",
            vec![],
            IssueTriageReadiness::Ready,
            IssueAutomationRiskClass::Normal,
        ),
        (
            "## 범위\n문서",
            vec![],
            IssueTriageReadiness::NeedsInfo,
            IssueAutomationRiskClass::Normal,
        ),
        (
            "## 완료 기준\n- 확인한다.",
            vec!["sensitive".into()],
            IssueTriageReadiness::Ready,
            IssueAutomationRiskClass::High,
        ),
    ] {
        let preview = fixture
            .service
            .preview(IssueDraftPreviewRequest {
                target: target(),
                text: current_text(body),
                expected_labels: labels,
            })
            .unwrap();
        assert_eq!(assessed(&preview).readiness, readiness);
        assert_eq!(assessed(&preview).risk_class, risk);
    }
    assert_eq!(*fixture.remote.writes.lock().unwrap(), 0);
}

#[test]
fn an_unreadable_requirement_is_not_replaced_with_empty_requirements() {
    let mut fixture = fixture(false);
    *fixture.files.policy.lock().unwrap() = "[convention]\ntemplate = \"broken.yml\"".into();
    *fixture.files.template.lock().unwrap() = "[".into();
    fixture.service = wire(fixture.service, "").0;
    let preview = fixture
        .service
        .preview(IssueDraftPreviewRequest {
            target: target(),
            text: current_text("## 완료 기준\n- 확인한다."),
            expected_labels: Vec::new(),
        })
        .unwrap();
    assert!(matches!(
        value(&preview),
        IssueDraftReadiness::CannotAssess {
            reason: IssueDraftReadinessBlocker::RequirementsUnavailable,
            ..
        }
    ));
}

#[test]
fn invalid_spans_and_invalid_expected_labels_never_become_ready() {
    let mut fixture = fixture(false);
    *fixture.files.policy.lock().unwrap() = String::new();
    fixture.service = wire(fixture.service, "").0;
    let mut text = current_text("## 완료 기준\n- 확인한다.");
    text.spans.push(IssueDraftSpan {
        field: IssueDraftField::Body,
        start: usize::MAX,
        end: usize::MAX,
        class: None,
        decision: None,
    });
    let preview = fixture
        .service
        .preview(IssueDraftPreviewRequest {
            target: target(),
            text,
            expected_labels: Vec::new(),
        })
        .unwrap();
    assert!(matches!(
        value(&preview),
        IssueDraftReadiness::CannotAssess {
            reason: IssueDraftReadinessBlocker::InvalidSpan,
            ..
        }
    ));
    let preview = fixture
        .service
        .preview(IssueDraftPreviewRequest {
            target: target(),
            text: current_text("## 완료 기준\n- 확인한다."),
            expected_labels: vec!["private-label\ncanary".into()],
        })
        .unwrap();
    assert!(matches!(
        value(&preview),
        IssueDraftReadiness::CannotAssess {
            reason: IssueDraftReadinessBlocker::InvalidLabels,
            ..
        }
    ));
    assert!(
        !serde_json::to_string(value(&preview))
            .unwrap()
            .contains("private-label")
    );
}

#[test]
fn older_preview_requests_default_to_no_assumed_labels() {
    let request: IssueDraftPreviewRequest = serde_json::from_value(serde_json::json!({
        "target": {"cwd": CWD, "owner": "sample", "repository": "widgets", "number": 7},
        "text": {"title": "title", "body": "body"}
    }))
    .unwrap();
    assert!(request.expected_labels.is_empty());
    let creation: IssueCreationPreviewRequest = serde_json::from_value(serde_json::json!({
        "target": {"cwd": CWD, "owner": "sample", "repository": "widgets"},
        "intentId": "intent", "text": {"title": "title", "body": "body"}
    }))
    .unwrap();
    assert!(creation.expected_labels.is_empty());
}
