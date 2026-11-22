use super::*;
use crate::adapter::inbound::kernel_protocol::{
    CommandRequest, CommandResult, KernelCommand, KernelCommandInput, describe,
};
use crate::application::port::inbound::stored_command_port::{StoredCommand, StoredCommandResult};
use crate::domain::issue_automation::autonomy::IssueAutonomyReady;
use crate::domain::kernel_spec::{
    CommandCompletion, CommandEffect, CommandIdempotency, CommandScope, CommandState,
    KernelCommandKind, ReasonCode,
};
use serde_json::{Value, json};

fn command(level: IssueAutonomyLevel) -> KernelCommand {
    KernelCommand::SetIssueAutonomy {
        command_id: "autonomy-1".to_owned(),
        level,
    }
}

fn ready() -> IssueAutonomyProjection {
    IssueAutonomyProjection::Ready(IssueAutonomyReady {
        requested_level: None,
        effective_level: IssueAutomationAutonomy::Observe,
        cap: IssueAutomationAutonomy::Observe,
        revision: 0,
        policy_version: "a".repeat(64),
        pending: false,
        choices: vec![
            IssueAutonomyChoice {
                level: IssueAutonomyLevel::Off,
                enabled: true,
                requires_confirmation: false,
                reason: None,
            },
            IssueAutonomyChoice {
                level: IssueAutonomyLevel::Observe,
                enabled: true,
                requires_confirmation: false,
                reason: None,
            },
            IssueAutonomyChoice {
                level: IssueAutonomyLevel::Propose,
                enabled: false,
                requires_confirmation: true,
                reason: Some("파일 정책 상한을 넘는 수준은 선택할 수 없습니다.".into()),
            },
        ],
    })
}

fn assert_command_schema(property: &str, value: &Value) {
    let spec = serde_json::to_value(describe()).unwrap();
    let mut schema = spec["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == "set_issue_autonomy")
        .unwrap()[property]
        .clone();
    schema
        .as_object_mut()
        .unwrap()
        .insert("$defs".into(), spec["$defs"].clone());
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
fn autonomy_commands_round_trip_all_selectable_levels_and_describe_the_storage_boundary() {
    for level in IssueAutonomyLevel::ALL {
        let wire = command(level);
        assert!(!wire.confirmed());
        assert_eq!(wire.kind(), KernelCommandKind::SetIssueAutonomy);
        let json = serde_json::to_value(&wire).unwrap();
        assert_command_schema("input", &json);
        let decoded: KernelCommand = serde_json::from_value(json).unwrap();
        assert!(matches!(
            decoded.into_input("/workspace").unwrap(),
            KernelCommandInput::Stored(StoredCommand::SetIssueAutonomy { command_id, level: actual })
                if command_id == "autonomy-1" && actual == level
        ));
        assert_eq!(
            KernelCommand::from_stored(StoredCommand::SetIssueAutonomy {
                command_id: "autonomy-1".into(),
                level,
            }),
            wire
        );
    }
    let spec = KernelCommandKind::SetIssueAutonomy.spec();
    assert_eq!(spec.scope, CommandScope::Issue);
    assert_eq!(spec.state, CommandState::Store);
    assert_eq!(spec.effect, CommandEffect::Write);
    assert_eq!(spec.idempotency, CommandIdempotency::Keyed);
    assert_eq!(spec.completion, CommandCompletion::Records);
    assert_eq!(spec.rejections, &[ReasonCode::HumanConfirmationRequired]);
}

#[test]
fn autonomy_commands_reject_forged_confirmation_unsupported_levels_and_invalid_ids() {
    let wire = serde_json::to_value(command(IssueAutonomyLevel::Propose)).unwrap();
    for field in [
        "confirm",
        "confirmation",
        "caller",
        "from",
        "revision",
        "policy_version",
        "cap",
    ] {
        let mut forged = wire.clone();
        forged[field] = json!("human");
        assert!(
            serde_json::from_value::<KernelCommand>(forged).is_err(),
            "{field}"
        );
    }
    for level in ["gated", "auto"] {
        let mut invalid = wire.clone();
        invalid["level"] = json!(level);
        assert!(serde_json::from_value::<KernelCommand>(invalid).is_err());
    }
    for command_id in [
        "".to_owned(),
        " ".into(),
        " padded".into(),
        "trailing ".into(),
        "line\nbreak".into(),
        "x".repeat(257),
        "한".repeat(128),
    ] {
        let error = KernelCommand::SetIssueAutonomy {
            command_id,
            level: IssueAutonomyLevel::Propose,
        }
        .into_input("/workspace")
        .unwrap_err();
        assert_eq!(error.message, "invalid issue autonomy command");
    }
    assert!(
        KernelCommand::SetIssueAutonomy {
            command_id: "x".repeat(256),
            level: IssueAutonomyLevel::Off,
        }
        .into_input("/workspace")
        .is_ok()
    );
}

#[test]
fn autonomy_receipts_bind_the_command_id_and_level_without_claiming_human_authority() {
    let descriptor = |command| {
        CommandRequest {
            command,
            idempotency_key: Some("autonomy-key".into()),
        }
        .descriptor()
        .unwrap()
        .unwrap()
    };
    let first = descriptor(command(IssueAutonomyLevel::Propose));
    assert!(!first.confirmed);
    assert!(first.thread_id.is_empty());
    assert_eq!(first, descriptor(command(IssueAutonomyLevel::Propose)));
    assert_ne!(
        first.fingerprint,
        descriptor(command(IssueAutonomyLevel::Observe)).fingerprint
    );
    assert_ne!(
        first.fingerprint,
        descriptor(KernelCommand::SetIssueAutonomy {
            command_id: "autonomy-2".into(),
            level: IssueAutonomyLevel::Propose,
        })
        .fingerprint
    );
}

#[test]
fn autonomy_result_preserves_server_choice_reasons_and_unavailable_states() {
    let result = CommandResult::from_stored(StoredCommandResult::IssueAutonomyChanged {
        command_id: "autonomy-1".into(),
        changed: false,
        autonomy: Box::new(ready()),
    });
    let wire = serde_json::to_value(result).unwrap();
    assert_eq!(wire["type"], "issue_autonomy_changed");
    assert_eq!(wire["command_id"], "autonomy-1");
    assert_eq!(wire["changed"], false);
    let state = &wire["autonomy"];
    assert_eq!(state["status"], "ready");
    assert!(state["requested_level"].is_null());
    assert_eq!(state["effective_level"], "observe");
    assert_eq!(state["cap"], "observe");
    assert_eq!(state["revision"], 0);
    assert_eq!(state["pending"], false);
    assert_eq!(state["choices"].as_array().unwrap().len(), 3);
    assert_eq!(state["choices"][2]["enabled"], false);
    assert_eq!(state["choices"][2]["requires_confirmation"], true);
    assert!(state["choices"][2]["reason"].is_string());
    assert_command_schema("result", &wire);
    for (projection, status) in [
        (IssueAutonomyProjection::Unavailable, "unavailable"),
        (IssueAutonomyProjection::PolicyBlocked, "policy_blocked"),
    ] {
        let wire = serde_json::to_value(IssueAutonomyState::from_domain(&projection)).unwrap();
        assert_eq!(wire, json!({"status":status}));
        assert!(serde_json::from_value::<IssueAutonomyState>(wire).is_ok());
    }
}
