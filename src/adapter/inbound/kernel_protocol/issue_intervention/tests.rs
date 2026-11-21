use super::*;
use crate::adapter::inbound::kernel_protocol::{
    CommandRequest, CommandResult, KernelCommandInput, describe,
};
use crate::application::port::inbound::stored_command_port::StoredCommandResult;
use crate::domain::issue_work::{
    IssueProjectScope, IssueProvider, IssueReference, IssueSharedCopy,
};
use crate::domain::kernel_event::KernelEvent as Record;
use crate::domain::kernel_spec::{
    CommandEffect, CommandIdempotency, CommandState, KernelCommandKind,
};
use serde_json::{Value, json};

fn reference() -> IssueReference {
    IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".to_owned(),
        repository_id: "R_intervention".to_owned(),
        issue_id: "I_intervention".to_owned(),
        project_scope: IssueProjectScope {
            workspace_dir: "/workspace".to_owned(),
        },
    }
}

fn request(action: IssueInterventionAction) -> IssueInterventionRequest {
    IssueInterventionRequest {
        command_id: "intervention-1".to_owned(),
        issue: reference(),
        action,
    }
}

fn receipt(action: &str) -> IssueInterventionReceipt {
    IssueInterventionReceipt {
        issue: reference(),
        command_id: "intervention-1".to_owned(),
        action: action.to_owned(),
        work_order_id: Some("work-1".to_owned()),
        authorization_epoch: 9,
        changed: true,
        delivery: None,
        handoff: None,
    }
}

fn assert_schema(section: &str, name: &str, property: &str, value: &Value) {
    let spec = serde_json::to_value(describe()).unwrap();
    let mut schema = spec[section]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == name)
        .unwrap()[property]
        .clone();
    schema
        .as_object_mut()
        .unwrap()
        .insert("$defs".to_owned(), spec["$defs"].clone());
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
fn intervention_commands_route_by_stable_issue_and_keep_their_idempotent_request() {
    let cases = [
        (IssueInterventionAction::Hold, KernelCommandKind::HoldIssue),
        (
            IssueInterventionAction::Release,
            KernelCommandKind::ReleaseIssue,
        ),
        (
            IssueInterventionAction::Steer {
                message: "지시 본문\n둘째 줄".to_owned(),
            },
            KernelCommandKind::SteerIssue,
        ),
        (
            IssueInterventionAction::TakeOver,
            KernelCommandKind::TakeOverIssue,
        ),
        (
            IssueInterventionAction::Retriage,
            KernelCommandKind::RetriageIssue,
        ),
        (
            IssueInterventionAction::Cancel { confirm: true },
            KernelCommandKind::CancelIssueWork,
        ),
        (
            IssueInterventionAction::Track,
            KernelCommandKind::TrackIssue,
        ),
        (
            IssueInterventionAction::Untrack,
            KernelCommandKind::UntrackIssue,
        ),
    ];
    for (action, kind) in cases {
        let request = request(action);
        let stored = StoredCommand::InterveneIssue {
            request: request.clone(),
        };
        assert_eq!(stored.kind(), kind);
        let command = KernelCommand::from_stored(stored.clone());
        assert_eq!(command.kind(), kind);
        let wire = serde_json::to_value(&command).unwrap();
        assert_eq!(wire["type"], request.action.label());
        assert_eq!(wire["command_id"], request.command_id);
        assert_schema("commands", kind.name(), "input", &wire);
        let decoded: KernelCommand = serde_json::from_value(wire).unwrap();
        assert_eq!(
            decoded.into_input("/workspace").unwrap(),
            KernelCommandInput::Stored(stored)
        );
        assert_eq!(kind.spec().state, CommandState::Store);
        assert_eq!(kind.spec().idempotency, CommandIdempotency::Keyed);
        assert!(kind.is_recorded());
        assert_eq!(
            kind.spec().effect,
            if kind == KernelCommandKind::CancelIssueWork {
                CommandEffect::Destructive
            } else {
                CommandEffect::Write
            }
        );
    }
}

#[test]
fn intervention_rejects_foreign_or_malformed_identity_before_storage() {
    let mut invalid = Vec::new();
    let mut foreign = reference();
    foreign.project_scope.workspace_dir = "/other".to_owned();
    invalid.push(foreign);
    let mut blank = reference();
    blank.issue_id.clear();
    invalid.push(blank);
    let mut controls = reference();
    controls.server_id = "server\nraw-canary".to_owned();
    invalid.push(controls);
    for issue in invalid {
        let command = KernelCommand::HoldIssue {
            issue: (&issue).into(),
            command_id: "valid-command".to_owned(),
        };
        let error = command.into_input("/workspace").unwrap_err();
        assert_eq!(error.message, "invalid issue reference");
        assert!(!error.message.contains("raw-canary"));
    }
}

#[test]
fn intervention_rejects_unbounded_identifiers_and_blank_guidance_without_echoing_text() {
    for command_id in [
        "".to_owned(),
        " \t".to_owned(),
        "x".repeat(257),
        "raw-canary\n".to_owned(),
    ] {
        let command = KernelCommand::HoldIssue {
            issue: (&reference()).into(),
            command_id,
        };
        let error = command.into_input("/workspace").unwrap_err();
        assert!(!error.message.contains("raw-canary"));
    }
    for message in [
        " \n\t".to_owned(),
        format!("raw-guidance-canary{}", "x".repeat(65536)),
    ] {
        let command = command_from_request(request(IssueInterventionAction::Steer { message }));
        let error = command.into_input("/workspace").unwrap_err();
        assert!(!error.message.contains("raw-guidance-canary"));
    }
    let command = command_from_request(request(IssueInterventionAction::Steer {
        message: "한".repeat(22000),
    }));
    assert!(
        command.into_input("/workspace").is_err(),
        "the limit counts UTF-8 bytes"
    );
}

#[test]
fn cancel_requires_explicit_confirmation_and_preserves_false_through_every_conversion() {
    let mut wire = serde_json::to_value(command_from_request(request(
        IssueInterventionAction::Cancel { confirm: false },
    )))
    .unwrap();
    wire.as_object_mut().unwrap().remove("confirm");
    let command: KernelCommand = serde_json::from_value(wire).unwrap();
    assert!(!command.confirmed());
    let descriptor = CommandRequest {
        command: command.clone(),
        idempotency_key: None,
    }
    .descriptor()
    .unwrap()
    .unwrap();
    assert!(!descriptor.confirmed);
    let KernelCommandInput::Stored(stored) = command.into_input("/workspace").unwrap() else {
        panic!("cancel must route to the intervention service");
    };
    assert!(matches!(&stored, StoredCommand::InterveneIssue { request }
        if request.action == IssueInterventionAction::Cancel { confirm: false }));
    assert!(!KernelCommand::from_stored(stored).confirmed());
    let confirmed =
        command_from_request(request(IssueInterventionAction::Cancel { confirm: true }));
    assert!(confirmed.confirmed());
}

#[test]
fn intervention_wire_cannot_supply_permissions_or_thread_destinations() {
    for (name, value) in [
        ("requires_human_review", json!(false)),
        ("authorization_epoch", json!(42)),
        ("permit", json!(true)),
        ("thread_id", json!("unrelated-thread")),
        ("human_confirmation", json!(true)),
    ] {
        let mut wire = serde_json::to_value(command_from_request(request(
            IssueInterventionAction::Steer {
                message: "guidance".to_owned(),
            },
        )))
        .unwrap();
        wire[name] = value;
        assert!(
            serde_json::from_value::<KernelCommand>(wire).is_err(),
            "{name}"
        );
    }
}

#[test]
fn intervention_descriptors_fingerprint_guidance_and_command_id_without_copying_them() {
    let make = |message: &str, command_id: &str| {
        CommandRequest {
            command: KernelCommand::SteerIssue {
                issue: (&reference()).into(),
                command_id: command_id.to_owned(),
                message: message.to_owned(),
            },
            idempotency_key: Some("outer-key".to_owned()),
        }
        .descriptor()
        .unwrap()
        .unwrap()
    };
    let first = make("raw-guidance-canary", "command-id-canary");
    assert!(first.thread_id.is_empty());
    assert!(!format!("{first:?}").contains("raw-guidance-canary"));
    assert!(!format!("{first:?}").contains("command-id-canary"));
    assert_eq!(
        first.fingerprint,
        make("raw-guidance-canary", "command-id-canary").fingerprint
    );
    assert_ne!(
        first.fingerprint,
        make("changed guidance", "command-id-canary").fingerprint
    );
    assert_ne!(
        first.fingerprint,
        make("raw-guidance-canary", "another-command").fingerprint
    );
}

#[test]
fn intervention_receipt_keeps_unknown_delivery_and_shared_handoff_in_the_result_schema() {
    let mut receipt = receipt("steer_issue");
    receipt.delivery = Some(IssueSteerDelivery::Unknown);
    receipt.handoff = Some(IssueSharedCopy {
        title: "Shared handoff".to_owned(),
        body: "Shared copy only".to_owned(),
    });
    let wire = serde_json::to_value(CommandResult::from_stored(
        StoredCommandResult::IssueIntervened {
            receipt: Box::new(receipt),
        },
    ))
    .unwrap();
    assert_eq!(wire["type"], "issue_intervened");
    assert_eq!(wire["receipt"]["delivery"], "unknown");
    assert_eq!(wire["receipt"]["handoff"]["body"], "Shared copy only");
    assert_eq!(wire["receipt"]["authorization_epoch"], 9);
    for name in [
        "hold_issue",
        "release_issue",
        "steer_issue",
        "take_over_issue",
        "retriage_issue",
        "cancel_issue_work",
        "track_issue",
        "untrack_issue",
    ] {
        assert_schema("commands", name, "result", &wire);
    }
}

#[test]
fn intervention_records_exclude_content_and_replayed_receipts_do_not_repeat_events() {
    for (action, first_kind) in [
        ("hold_issue", "issue_held"),
        ("release_issue", "issue_released"),
    ] {
        let mut receipt = receipt(action);
        receipt.handoff = Some(IssueSharedCopy {
            title: "handoff-title-canary".to_owned(),
            body: "handoff-body-canary".to_owned(),
        });
        let records = Record::issue_intervened(&receipt);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].kind(), first_kind);
        assert_eq!(records[1].kind(), "issue_work_permit_revoked");
        for record in records {
            let wire = serde_json::to_value(&record).unwrap();
            assert!(!wire.to_string().contains("canary"));
            assert_eq!(record.thread_id(), "");
            assert_eq!(record.op_id(), None);
            assert_schema("records", record.kind(), "payload", &wire);
        }
        receipt.changed = false;
        assert!(Record::issue_intervened(&receipt).is_empty());
    }
    assert!(Record::issue_intervened(&receipt("steer_issue")).is_empty());
}

mod runtime;
