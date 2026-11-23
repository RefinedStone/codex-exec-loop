use super::*;
use crate::domain::issue_automation::IssueAutomationAutonomy;
use crate::domain::issue_automation::binding_pause::{
    IssueBindingPauseIntent, IssueBindingResumeBasis,
};

fn state(id: &str, revision: u64, paused: bool) -> IssueBindingPauseState {
    IssueBindingPauseState {
        binding_id: IssueAutomationBindingId::new(id).unwrap(),
        revision,
        paused,
        pending: None,
    }
}

fn pending(previous: &IssueBindingPauseState, paused: bool) -> IssueBindingPauseState {
    let mut next = previous.clone();
    next.revision += 1;
    next.pending = Some(IssueBindingPauseIntent {
        command_id: format!("change-{}-{}", previous.binding_id, next.revision),
        binding_id: previous.binding_id.clone(),
        paused,
        expected_revision: previous.revision,
        resume_basis: (!paused).then(|| IssueBindingResumeBasis {
            binding_revision: 0,
            autonomy_revision: 0,
            policy_version: "a".repeat(64),
            effective_level: IssueAutomationAutonomy::Gated,
        }),
    });
    next
}

#[test]
fn binding_pause_waits_for_admitted_effects_and_resume_rejects_old_generations() {
    let admission = Arc::new(IssueBindingAdmission::new());
    let original = state("binding-a", 0, false);
    let other = state("binding-b", 0, false);
    admission.register_if_absent(&original).unwrap();
    admission.register_if_absent(&other).unwrap();
    let in_flight = admission.acquire(&original.binding_id, 0).unwrap();
    let pausing = pending(&original, true);
    admission
        .begin_transition(&original.binding_id, || Ok(((), pausing.clone())))
        .unwrap();
    assert!(in_flight.validate().is_err());
    assert!(admission.acquire(&original.binding_id, 0).is_err());
    let other_effect = admission.acquire(&other.binding_id, 0).unwrap();
    let paused = state("binding-a", 1, true);
    let mut completion_calls = 0;
    assert!(
        admission
            .finish_if_drained(&original.binding_id, 1, || {
                completion_calls += 1;
                Ok(((), paused.clone()))
            })
            .unwrap()
            .is_none()
    );
    assert_eq!(completion_calls, 0);
    drop(in_flight);
    assert!(
        admission
            .finish_if_drained(&original.binding_id, 1, || Ok(((), paused.clone())))
            .unwrap()
            .is_some()
    );
    other_effect.validate().unwrap();
    let resuming = pending(&paused, false);
    admission
        .begin_transition(&original.binding_id, || Ok(((), resuming.clone())))
        .unwrap();
    assert!(admission.acquire(&original.binding_id, 2).is_err());
    let resumed = state("binding-a", 2, false);
    admission
        .finish_if_drained(&original.binding_id, 2, || Ok(((), resumed)))
        .unwrap()
        .unwrap();
    assert!(admission.acquire(&original.binding_id, 0).is_err());
    admission
        .acquire(&original.binding_id, 2)
        .unwrap()
        .validate()
        .unwrap();
    // A delayed read cannot reset the generation or undo closure.
    admission.register_if_absent(&original).unwrap();
    assert_eq!(admission.capture(&original.binding_id).unwrap(), 2);
}

#[test]
fn uncertain_pause_commit_and_unchanged_resume_cannot_reopen_admission() {
    let admission = Arc::new(IssueBindingAdmission::new());
    let original = state("binding-a", 0, false);
    admission.register_if_absent(&original).unwrap();
    assert!(
        admission
            .begin_transition::<()>(&original.binding_id, || {
                anyhow::bail!("commit response lost")
            })
            .is_err()
    );
    admission
        .begin_transition(&original.binding_id, || Ok(((), original.clone())))
        .unwrap();
    admission.register_if_absent(&original).unwrap();
    assert!(admission.capture(&original.binding_id).is_err());
}

#[test]
fn a_new_failed_pause_disarms_an_older_resume_completion_that_lost_its_response() {
    let admission = Arc::new(IssueBindingAdmission::new());
    let paused = state("binding-a", 1, true);
    let resuming = pending(&paused, false);
    admission.register_if_absent(&resuming).unwrap();
    let resumed = state("binding-a", 2, false);
    let old_event = IssueBindingPauseEvent {
        event_id: "old-resume".into(),
        receipt: crate::domain::issue_automation::binding_pause::IssueBindingPauseReceipt {
            command_id: resuming.pending.as_ref().unwrap().command_id.clone(),
            binding_id: resumed.binding_id.clone(),
            revision: resumed.revision,
            paused: false,
            outcome: IssueBindingPauseOutcome::Resumed,
            changed: true,
        },
    };
    // The old Resume is durable, but its acknowledgement has not repaired memory yet.
    admission.register_if_absent(&resumed).unwrap();
    assert!(
        admission
            .begin_transition::<()>(&resumed.binding_id, || {
                anyhow::bail!("new Pause commit response lost")
            })
            .is_err()
    );
    admission
        .reconcile_completed_event(&resumed, &old_event)
        .unwrap();
    assert!(admission.capture(&resumed.binding_id).is_err());
}

#[test]
fn owner_shutdown_closes_existing_guards_and_never_trusts_an_unregistered_binding() {
    let admission = Arc::new(IssueBindingAdmission::new());
    let original = state("binding-a", 0, false);
    assert!(admission.acquire(&original.binding_id, 0).is_err());
    admission.register_if_absent(&original).unwrap();
    let guard = admission.acquire(&original.binding_id, 0).unwrap();
    assert!(admission.has_inflight());
    admission.close_all();
    assert!(guard.validate().is_err());
    assert!(admission.register_if_absent(&original).is_err());
    assert!(admission.acquire(&original.binding_id, 0).is_err());
    drop(guard);
    assert!(!admission.has_inflight());
}
