use super::*;
use crate::domain::issue_automation::IssueAutomationBindingId;
use crate::domain::issue_automation::binding_pause::{
    IssueBindingPauseIntent, IssueBindingPauseOutcome, IssueBindingPauseReceipt,
    IssueBindingPauseState as DomainState, StoredIssueBindingPauseCommand,
};

fn pending() -> StoredIssueBindingPauseCommand {
    StoredIssueBindingPauseCommand {
        intent: IssueBindingPauseIntent {
            command_id: "pause-1".into(),
            binding_id: IssueAutomationBindingId::new("binding-1").unwrap(),
            paused: true,
            expected_revision: 3,
            resume_basis: None,
        },
        accepted_revision: 4,
        receipt: None,
    }
}

#[test]
fn acceptance_is_not_a_completed_pause_and_receipt_keeps_the_original_fact() {
    let mut command = pending();
    let accepted = serde_json::to_value(IssueBindingPauseRequestedState::from(&command)).unwrap();
    assert!(accepted["receipt"].is_null());
    assert_eq!(accepted["accepted_revision"], 4);
    command.receipt = Some(IssueBindingPauseReceipt {
        command_id: command.intent.command_id.clone(),
        binding_id: command.intent.binding_id.clone(),
        revision: 4,
        paused: true,
        outcome: IssueBindingPauseOutcome::Paused,
        changed: true,
    });
    let completed = serde_json::to_value(IssueBindingPauseRequestedState::from(&command)).unwrap();
    assert_eq!(completed["receipt"]["outcome"], "paused");
    assert_eq!(completed["receipt"]["revision"], 4);
}

#[test]
fn current_binding_query_cannot_borrow_another_bindings_run_state() {
    let command = pending();
    let state = DomainState {
        binding_id: command.intent.binding_id.clone(),
        revision: 4,
        paused: false,
        pending: Some(command.intent),
    };
    let current = serde_json::to_value(IssueBindingPauseState::from_current(
        "binding-1",
        Some(&state),
    ))
    .unwrap();
    assert_eq!(current["status"], "ready");
    assert_eq!(current["pending"]["paused"], true);
    assert!(current["pending"].get("resume_basis").is_none());
    assert_eq!(
        serde_json::to_value(IssueBindingPauseState::from_current(
            "binding-2",
            Some(&state)
        ))
        .unwrap()["status"],
        "unavailable"
    );
    assert_eq!(
        serde_json::to_value(IssueBindingPauseState::from_current("binding-1", None)).unwrap()["status"],
        "unavailable"
    );
}

#[test]
fn binding_pause_wire_rejects_unknown_fields_and_revision_overflow() {
    let wire = serde_json::json!({"type":"set_issue_binding_paused","command_id":"pause-1","binding_id":"binding-1","paused":true,"expected_revision":0});
    let command: super::super::KernelCommand = serde_json::from_value(wire.clone()).unwrap();
    assert!(command.into_input("/repo").is_ok());
    let mut spoofed = wire.clone();
    spoofed["confirmed"] = true.into();
    assert!(serde_json::from_value::<super::super::KernelCommand>(spoofed).is_err());
    assert!(validate_input("pause-1", " binding-1", 0).is_err());
    assert!(
        validate_input(
            "pause-1",
            "binding-1",
            crate::domain::issue_automation::MAX_ISSUE_AUTOMATION_REVISION
        )
        .is_err()
    );
}
