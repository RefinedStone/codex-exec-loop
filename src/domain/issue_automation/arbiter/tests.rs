use super::*;
use crate::domain::issue_automation::decision::{IssueDecisionOption, IssueDecisionSubject};
use crate::domain::issue_work::{IssueFieldHash, IssueProjectScope, IssueProvider, IssueReference};

fn request() -> IssueDecisionRequest {
    let shared = |text: &str| IssueDecisionSharedText::new_shared(text).unwrap();
    IssueDecisionRequest::new(
        IssueDecisionId::new("arbiter-domain").unwrap(),
        IssueDecisionDefinition {
            subject: IssueDecisionSubject::Issue(IssueReference {
                provider: IssueProvider::GitHub,
                server_id: "github.com".into(),
                repository_id: "R_arbiter".into(),
                issue_id: "I_arbiter".into(),
                project_scope: IssueProjectScope {
                    workspace_dir: "/workspace/arbiter".into(),
                },
            }),
            gate: IssueAutomationGate::Start,
            kind: IssueDecisionKind::StartApproval,
            question: shared("작업을 시작할까요?"),
            options: vec![IssueDecisionOption {
                id: "start".into(),
                label: shared("시작"),
                effect: shared("작업을 시작한다."),
            }],
            recommended: None,
            basis: IssueDecisionBasis {
                authorization_epoch: 0,
                policy_version: "a".repeat(64),
                field_hash: Some(IssueFieldHash::of_fields("title", "body")),
                work_order_id: None,
                gate_revision: 1,
                delivery: None,
            },
            on_timeout: IssueDecisionTimeout::Arbiter,
            due_at: Some("1970-01-01T00:00:01Z".into()),
            asked_by: "triage".into(),
            evidence: vec![],
            delivery_review: None,
        },
    )
    .unwrap()
}

#[test]
fn arbiter_reply_is_closed_json_with_integer_confidence_and_an_existing_option() {
    let request = request();
    let accepted =
        r#"{"decision":"start","confidence":9000,"reasons":["공유된 근거"],"blockers":[]}"#;
    let reply = IssueArbiterModelReply::parse_json(accepted, &request).unwrap();
    assert_eq!(reply.confidence, 9000);
    assert_eq!(reply.decision, "start");
    for invalid in [
        r#"{"decision":"other","confidence":10000,"reasons":[],"blockers":[]}"#,
        r#"{"decision":"start","confidence":1.0,"reasons":[],"blockers":[]}"#,
        r#"{"decision":"start","confidence":10001,"reasons":[],"blockers":[]}"#,
        r#"{"decision":"start","confidence":9000,"reasons":[],"blockers":[],"by":"human"}"#,
        r#"{"decision":"start","decision":"start","confidence":9000,"reasons":[],"blockers":[]}"#,
        r#"{"decision":"start","confidence":9000,"reasons":[""],"blockers":[]}"#,
        r#"{"decision":"start","confidence":9000,"reasons":[]}"#,
        r#"```json
{"decision":"start","confidence":9000,"reasons":[],"blockers":[]}
```"#,
    ] {
        assert!(
            IssueArbiterModelReply::parse_json(invalid, &request).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn arbiter_reply_limits_are_checked_before_any_shared_note_is_created() {
    let request = request();
    for reasons in [vec!["x".repeat(1025)], vec!["x".into(); 9]] {
        let raw = serde_json::json!({"decision":"start","confidence":9000,
            "reasons":reasons,"blockers":[]})
        .to_string();
        assert!(IssueArbiterModelReply::parse_json(&raw, &request).is_err());
    }
    assert!(
        IssueArbiterModelReply::parse_shape(&" ".repeat(MAX_ISSUE_ARBITER_RESPONSE_BYTES + 1))
            .is_err()
    );
}

#[test]
fn arbiter_threshold_and_blockers_never_relax_the_safe_default() {
    let settings = IssueArbiterSettings::default();
    assert_eq!(
        (
            settings.min_confidence,
            settings.max_running,
            settings.max_started_per_day,
            settings.timeout_seconds
        ),
        (9000, 1, 5, 180)
    );
    for (confidence, has_blockers, expected) in [
        (8999, false, Some(IssueArbiterWaitReason::LowConfidence)),
        (9000, false, None),
        (10000, true, Some(IssueArbiterWaitReason::BlockersPresent)),
        (10001, false, Some(IssueArbiterWaitReason::InvalidResponse)),
    ] {
        let proposal = IssueArbiterProposal {
            decision: "start".into(),
            confidence,
            has_blockers,
            note: None,
        };
        assert_eq!(proposal.wait_reason(&settings), expected);
    }
    assert!(
        IssueArbiterSettings {
            max_running: 0,
            max_started_per_day: 0,
            ..settings
        }
        .validate()
        .is_ok()
    );
    assert!(
        IssueArbiterSettings {
            min_confidence: 10001,
            ..settings
        }
        .validate()
        .is_err()
    );
    assert!(
        IssueArbiterSettings {
            timeout_seconds: 0,
            ..settings
        }
        .validate()
        .is_err()
    );
}

#[test]
fn arbiter_input_fingerprints_reject_missing_or_unbounded_provenance() {
    let input = IssueArbiterInputBasis {
        binding_run_revision: 0,
        decision_basis: request().definition().basis.clone(),
        decision_revision: 1,
        repository_commit_sha: "b".repeat(40),
        evidence_sha256: "c".repeat(64),
        model_route_revision: "route:sha256".into(),
        input_sha256: "d".repeat(64),
    };
    input.validate().unwrap();
    for field in [
        "revision",
        "binding_revision",
        "commit",
        "evidence",
        "input",
        "route",
    ] {
        let mut invalid = input.clone();
        match field {
            "revision" => invalid.decision_revision = 0,
            "binding_revision" => invalid.binding_run_revision = MAX_ISSUE_AUTOMATION_REVISION + 1,
            "commit" => invalid.repository_commit_sha = "HEAD".into(),
            "evidence" => invalid.evidence_sha256 = "c".repeat(63),
            "input" => invalid.input_sha256 = "private source body".into(),
            "route" => invalid.model_route_revision = "route with whitespace".into(),
            _ => unreachable!(),
        }
        assert!(invalid.validate().is_err(), "{field}");
    }
}

#[test]
fn arbiter_legacy_input_json_keeps_its_bytes_until_a_binding_generation_exists() {
    let mut input = IssueArbiterInputBasis {
        decision_basis: request().definition().basis.clone(),
        decision_revision: 1,
        binding_run_revision: 0,
        repository_commit_sha: "b".repeat(40),
        evidence_sha256: "c".repeat(64),
        model_route_revision: "route:1".into(),
        input_sha256: "d".repeat(64),
    };
    let original = serde_json::to_string(&input).unwrap();
    assert!(!original.contains("bindingRunRevision"));
    assert_eq!(
        serde_json::from_str::<IssueArbiterInputBasis>(&original).unwrap(),
        input
    );
    input.binding_run_revision = 2;
    let newer = serde_json::to_string(&input).unwrap();
    assert!(newer.contains("\"bindingRunRevision\":2"));
    assert_ne!(newer, original);
    assert_eq!(
        serde_json::from_str::<IssueArbiterInputBasis>(&newer).unwrap(),
        input
    );
}

#[test]
fn arbiter_outcome_wire_is_tagged_and_cannot_include_model_text() {
    let outcome = IssueArbiterFinishedOutcome::Waiting {
        reason: IssueArbiterWaitReason::LowConfidence,
    };
    assert_eq!(
        serde_json::to_value(&outcome).unwrap(),
        serde_json::json!({"kind":"waiting","reason":"low_confidence"})
    );
    for (outcome, kind) in [
        (IssueArbiterFinishedOutcome::Applied {}, "applied"),
        (IssueArbiterFinishedOutcome::Superseded {}, "superseded"),
        (IssueArbiterFinishedOutcome::Discarded {}, "discarded"),
    ] {
        let wire = serde_json::json!({"kind":kind});
        assert_eq!(serde_json::to_value(&outcome).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<IssueArbiterFinishedOutcome>(wire).unwrap(),
            outcome
        );
        assert!(
            serde_json::from_value::<IssueArbiterFinishedOutcome>(
                serde_json::json!({"kind":kind,"by":"human"})
            )
            .is_err(),
            "{kind} must reject fields outside its closed outcome schema"
        );
    }
    for invalid in [
        serde_json::json!({"kind":"waiting","reason":"unrecognized"}),
        serde_json::json!({"kind":"waiting","reason":"low_confidence","reasons":["private model text"]}),
    ] {
        assert!(serde_json::from_value::<IssueArbiterFinishedOutcome>(invalid).is_err());
    }
}

#[test]
fn an_agent_cannot_answer_an_arbiter_template_even_through_the_auto_gate() {
    use crate::domain::issue_automation::decision::IssueDecisionAgentAnswer;
    let request = request();
    for gate in [
        IssueAutomationGateAction::Auto,
        IssueAutomationGateAction::TimeoutArbiter,
    ] {
        assert!(
            request
                .answer_for_gate(
                    &request.definition().basis,
                    gate,
                    IssueDecisionAgentAnswer {
                        option: "start".into(),
                        note: None,
                        caller: Some("Desktop".into()),
                        channel: "native".into(),
                        at: "1970-01-01T00:00:01Z".into()
                    },
                    None
                )
                .is_err()
        );
    }
    assert_eq!(request.state(), IssueDecisionState::Open);
}
