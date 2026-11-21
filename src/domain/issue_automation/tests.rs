use crate::domain::issue_work::{
    IssueComparisonBasis, IssueFieldHash, IssueProjectScope, IssueProvider, IssueReference,
    IssueSharedCopy,
};

use super::*;

fn binding() -> IssueAutomationBinding {
    IssueAutomationBinding::new(
        IssueAutomationBindingId::new("binding-1").unwrap(),
        IssueProvider::GitHub,
        "github.com",
        "R_stable",
        IssueProjectScope {
            workspace_dir: "/work/project".to_owned(),
        },
    )
    .unwrap()
}

fn reference() -> IssueReference {
    IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".to_owned(),
        repository_id: "R_stable".to_owned(),
        issue_id: "I_stable".to_owned(),
        project_scope: IssueProjectScope {
            workspace_dir: "/work/project".to_owned(),
        },
    }
}

fn snapshot() -> IssueAutomationSnapshot {
    IssueAutomationSnapshot {
        basis: IssueComparisonBasis {
            field_hash: IssueFieldHash::of_fields("원격 제목", "고객 식별값: raw-only-sentinel"),
            provider_updated_at: "2026-11-20T12:00:00Z".to_owned(),
        },
        shared_copy: IssueSharedCopy {
            title: "공유 제목".to_owned(),
            body: "고객 식별값: [비공개]".to_owned(),
        },
        policy_version: "policy-v1".to_owned(),
    }
}

fn discovered() -> TrackedIssue {
    TrackedIssue::discovered(reference(), &binding(), snapshot()).unwrap()
}

fn persisted(
    state: IssueAutomationState,
    resume: Option<IssueDecisionResumePhase>,
) -> TrackedIssue {
    let mut record = discovered().to_record();
    record.state = state;
    record.decision_resume = resume;
    record.revision = 7;
    TrackedIssue::hydrate(record).unwrap()
}

#[test]
fn new_binding_observes_and_autonomy_never_changes_its_identity() {
    let mut binding = binding();
    assert_eq!(binding.autonomy(), IssueAutomationAutonomy::Observe);
    assert_eq!(binding.id().as_str(), "binding-1");
    assert_eq!(binding.provider(), IssueProvider::GitHub);
    assert_eq!(binding.server_id(), "github.com");
    assert_eq!(binding.repository_id(), "R_stable");
    assert_eq!(binding.project_scope().workspace_dir, "/work/project");
    let original = binding.clone();

    assert!(!binding.set_autonomy(IssueAutomationAutonomy::Observe));
    assert!(binding.set_autonomy(IssueAutomationAutonomy::Auto));
    original.validate_replacement(&binding).unwrap();
    assert_eq!(binding.id(), original.id());
    assert_eq!(
        IssueAutomationBinding::hydrate(binding.to_record()).unwrap(),
        binding
    );

    let levels = [
        (IssueAutomationAutonomy::Off, "off"),
        (IssueAutomationAutonomy::Observe, "observe"),
        (IssueAutomationAutonomy::Propose, "propose"),
        (IssueAutomationAutonomy::Gated, "gated"),
        (IssueAutomationAutonomy::Auto, "auto"),
    ];
    for (level, wire) in levels {
        assert_eq!(serde_json::to_value(level).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<IssueAutomationAutonomy>(wire.into()).unwrap(),
            level
        );
    }
    assert!(levels.windows(2).all(|pair| pair[0].0 < pair[1].0));
    assert!(serde_json::from_str::<IssueAutomationAutonomy>("\"fully_automatic\"").is_err());
}

#[test]
fn binding_replacement_rejects_each_stable_coordinate_change() {
    let original = binding();
    let changes: &[fn(&mut PersistedIssueAutomationBinding)] = &[
        |record| record.id = IssueAutomationBindingId::new("binding-2").unwrap(),
        |record| record.provider = IssueProvider::Jira,
        |record| record.server_id = "another-server".to_owned(),
        |record| record.repository_id = "R_another".to_owned(),
        |record| record.project_scope.workspace_dir = "/work/other".to_owned(),
    ];
    for change in changes {
        let mut record = original.to_record();
        change(&mut record);
        let candidate = IssueAutomationBinding::hydrate(record).unwrap();
        assert_eq!(
            original.validate_replacement(&candidate),
            Err(IssueAutomationError::BindingChanged)
        );
    }
}

#[test]
fn opaque_binding_ids_are_preserved_and_empty_or_nul_ids_are_refused() {
    let id = IssueAutomationBindingId::new("연결/ R_stable ").unwrap();
    let wire = serde_json::to_string(&id).unwrap();
    assert_eq!(
        serde_json::from_str::<IssueAutomationBindingId>(&wire).unwrap(),
        id
    );
    assert_eq!(id.as_str(), "연결/ R_stable ");
    for invalid in ["", " \t\n", "binding\0suffix"] {
        assert!(IssueAutomationBindingId::new(invalid).is_err());
        assert!(serde_json::from_value::<IssueAutomationBindingId>(invalid.into()).is_err());
    }
}

#[test]
fn discovery_requires_exact_source_and_project_binding_but_not_an_issue_number() {
    let binding = binding();
    let changes: &[fn(&mut IssueReference)] = &[
        |reference| reference.provider = IssueProvider::Jira,
        |reference| reference.server_id = "elsewhere".to_owned(),
        |reference| reference.repository_id = "R_elsewhere".to_owned(),
        |reference| reference.project_scope.workspace_dir = "/work/elsewhere".to_owned(),
    ];
    for change in changes {
        let mut reference = reference();
        change(&mut reference);
        assert!(!binding.matches_reference(&reference));
        assert_eq!(
            TrackedIssue::discovered(reference, &binding, snapshot()),
            Err(IssueAutomationError::BindingChanged)
        );
    }
    let mut another = reference();
    another.issue_id = "I_another_stable_id".to_owned();
    let issue = TrackedIssue::discovered(another.clone(), &binding, snapshot()).unwrap();
    assert_eq!(issue.reference(), &another);
    assert_eq!(issue.binding_id(), binding.id());
    assert_eq!(issue.state(), IssueAutomationState::Discovered);
    assert_eq!(issue.flags(), IssueAutomationFlags::default());
    assert_eq!(issue.revision(), 0);
}

#[test]
fn state_wire_names_are_closed_and_flags_are_not_lifecycle_states() {
    use IssueAutomationState as State;
    for (state, wire) in [
        (State::Discovered, "discovered"),
        (State::Ineligible, "ineligible"),
        (State::NeedsInfo, "needs_info"),
        (State::AwaitingDecision, "awaiting_decision"),
        (State::Ready, "ready"),
        (State::Queued, "queued"),
        (State::Running, "running"),
        (State::InReview, "in_review"),
        (State::Done, "done"),
        (State::Failed, "failed"),
        (State::Cancelled, "cancelled"),
    ] {
        assert_eq!(serde_json::to_value(state).unwrap(), wire);
        assert_eq!(serde_json::from_value::<State>(wire.into()).unwrap(), state);
    }
    for invalid in ["held", "stale", "unknown", "closed", "approved"] {
        assert!(serde_json::from_value::<State>(invalid.into()).is_err());
    }
}

#[test]
fn persisted_transition_table_rejects_every_unlisted_edge() {
    use IssueAutomationState as State;
    use IssueDecisionResumePhase as Resume;

    // 상태와 결정 전의 단계를 함께 검사한다. 이 표는 저장 경계에서 허용하는 한 간선이다.
    let phases: &[(&str, State, Option<Resume>, &[&str])] = &[
        (
            "discovered",
            State::Discovered,
            None,
            &[
                "discovered",
                "ineligible",
                "needs_info",
                "ready",
                "await_discovered",
                "cancelled",
            ],
        ),
        (
            "ineligible",
            State::Ineligible,
            None,
            &["ineligible", "discovered", "cancelled"],
        ),
        (
            "needs_info",
            State::NeedsInfo,
            None,
            &["needs_info", "discovered", "cancelled"],
        ),
        (
            "ready",
            State::Ready,
            None,
            &["ready", "discovered", "await_ready", "queued", "cancelled"],
        ),
        (
            "queued",
            State::Queued,
            None,
            &["queued", "running", "cancelled"],
        ),
        (
            "running",
            State::Running,
            None,
            &[
                "running",
                "await_running",
                "in_review",
                "failed",
                "cancelled",
            ],
        ),
        (
            "in_review",
            State::InReview,
            None,
            &[
                "in_review",
                "await_review",
                "running",
                "done",
                "failed",
                "cancelled",
            ],
        ),
        ("done", State::Done, None, &["done"]),
        (
            "failed",
            State::Failed,
            None,
            &["failed", "await_failed", "discovered", "done", "cancelled"],
        ),
        ("cancelled", State::Cancelled, None, &["cancelled"]),
        (
            "await_discovered",
            State::AwaitingDecision,
            Some(Resume::Discovered),
            &["await_discovered", "discovered", "cancelled"],
        ),
        (
            "await_ready",
            State::AwaitingDecision,
            Some(Resume::Ready),
            &["await_ready", "ready", "discovered", "cancelled"],
        ),
        (
            "await_running",
            State::AwaitingDecision,
            Some(Resume::Running),
            &["await_running", "running", "cancelled"],
        ),
        (
            "await_review",
            State::AwaitingDecision,
            Some(Resume::InReview),
            &["await_review", "in_review", "cancelled"],
        ),
        (
            "await_failed",
            State::AwaitingDecision,
            Some(Resume::Failed),
            &["await_failed", "failed", "discovered", "cancelled"],
        ),
    ];
    for &(name, state, resume, allowed) in phases {
        let current = persisted(state, resume);
        for &(next_name, next_state, next_resume, _) in phases {
            let next = persisted(next_state, next_resume);
            assert_eq!(
                current.validate_replacement(&next).is_ok(),
                allowed.contains(&next_name),
                "{name} -> {next_name}"
            );
        }
    }
}

#[test]
fn each_triage_result_requires_discovery_and_can_be_reassessed() {
    for (outcome, expected) in [
        (
            IssueTriageResult::Ineligible,
            IssueAutomationState::Ineligible,
        ),
        (
            IssueTriageResult::NeedsInfo,
            IssueAutomationState::NeedsInfo,
        ),
        (IssueTriageResult::Ready, IssueAutomationState::Ready),
    ] {
        let mut issue = discovered();
        assert!(issue.transition(IssueTransition::Assess(outcome)).unwrap());
        assert_eq!(issue.state(), expected);
        assert!(!issue.transition(IssueTransition::Assess(outcome)).unwrap());
        assert!(issue.transition(IssueTransition::Reassess).unwrap());
        assert_eq!(issue.state(), IssueAutomationState::Discovered);
    }
}

#[test]
fn decision_resume_round_trip_keeps_exact_prior_phase() {
    use IssueDecisionResumePhase as Resume;
    for phase in [
        Resume::Discovered,
        Resume::Ready,
        Resume::Running,
        Resume::InReview,
        Resume::Failed,
    ] {
        let mut issue = persisted(phase.state(), None);
        issue.set_flag(IssueAutomationFlag::Held, true);
        let original = issue.clone();
        assert!(issue.transition(IssueTransition::AwaitDecision).unwrap());
        assert_eq!(issue.state(), IssueAutomationState::AwaitingDecision);
        assert_eq!(issue.decision_resume(), Some(phase));
        assert!(!issue.transition(IssueTransition::AwaitDecision).unwrap());
        let wire = serde_json::to_string(&issue.to_record()).unwrap();
        issue = TrackedIssue::hydrate(serde_json::from_str(&wire).unwrap()).unwrap();
        assert!(issue.transition(IssueTransition::ResumeDecision).unwrap());
        assert_eq!(issue, original);
    }
}

#[test]
fn decision_record_cannot_swap_its_saved_resume_phase() {
    let current = persisted(
        IssueAutomationState::AwaitingDecision,
        Some(IssueDecisionResumePhase::Running),
    );
    let candidate = persisted(
        IssueAutomationState::AwaitingDecision,
        Some(IssueDecisionResumePhase::Ready),
    );
    assert!(current.validate_replacement(&candidate).is_err());
}

#[test]
fn active_and_queued_work_cannot_be_reset_by_reassessment() {
    for (state, resume) in [
        (IssueAutomationState::Queued, None),
        (IssueAutomationState::Running, None),
        (IssueAutomationState::InReview, None),
        (IssueAutomationState::Failed, None),
        (
            IssueAutomationState::AwaitingDecision,
            Some(IssueDecisionResumePhase::Running),
        ),
        (
            IssueAutomationState::AwaitingDecision,
            Some(IssueDecisionResumePhase::InReview),
        ),
        (
            IssueAutomationState::AwaitingDecision,
            Some(IssueDecisionResumePhase::Failed),
        ),
    ] {
        let mut issue = persisted(state, resume);
        let original = issue.clone();
        assert!(issue.transition(IssueTransition::Reassess).is_err());
        assert_eq!(issue, original);
    }
}

#[test]
fn retrying_failure_requires_the_explicit_failure_command() {
    for (state, resume) in [
        (IssueAutomationState::Failed, None),
        (
            IssueAutomationState::AwaitingDecision,
            Some(IssueDecisionResumePhase::Failed),
        ),
    ] {
        let mut issue = persisted(state, resume);
        issue.set_flag(IssueAutomationFlag::Stale, true);
        assert!(issue.transition(IssueTransition::RetryFailed).unwrap());
        assert_eq!(issue.state(), IssueAutomationState::Discovered);
        assert_eq!(issue.decision_resume(), None);
        assert!(issue.flags().stale);
        assert_eq!(issue.revision(), 7);
    }
    let mut running = persisted(IssueAutomationState::Running, None);
    assert!(running.transition(IssueTransition::RetryFailed).is_err());
}

#[test]
fn execution_and_review_report_facts_without_clearing_flags_or_issuing_revisions() {
    let mut issue = persisted(IssueAutomationState::Ready, None);
    issue.set_flag(IssueAutomationFlag::Held, true);
    issue.set_flag(IssueAutomationFlag::Unknown, true);
    let flags = issue.flags();
    for (transition, state) in [
        (IssueTransition::Queue, IssueAutomationState::Queued),
        (IssueTransition::Start, IssueAutomationState::Running),
        (IssueTransition::OpenReview, IssueAutomationState::InReview),
        (IssueTransition::Rework, IssueAutomationState::Running),
        (IssueTransition::OpenReview, IssueAutomationState::InReview),
        (IssueTransition::Complete, IssueAutomationState::Done),
    ] {
        assert!(issue.transition(transition).unwrap());
        assert_eq!(issue.state(), state);
        assert_eq!(issue.flags(), flags);
        assert_eq!(issue.revision(), 7);
    }
    assert!(!issue.transition(IssueTransition::Complete).unwrap());
    assert!(issue.transition(IssueTransition::Cancel).is_err());
    assert!(issue.transition(IssueTransition::Reassess).is_err());
}

#[test]
fn trusted_completion_after_exhaustion_is_a_single_fact_transition_and_preserves_flags() {
    let mut failed = persisted(IssueAutomationState::Failed, None);
    failed.set_flag(IssueAutomationFlag::Held, true);
    failed.set_flag(IssueAutomationFlag::Stale, true);
    failed.set_flag(IssueAutomationFlag::Unknown, true);
    let current = failed.clone();
    assert!(failed.transition(IssueTransition::Complete).unwrap());
    assert_eq!(failed.state(), IssueAutomationState::Done);
    assert_eq!(failed.flags(), current.flags());
    assert_eq!(failed.revision(), current.revision());
    current.validate_replacement(&failed).unwrap();
    assert!(!failed.transition(IssueTransition::Complete).unwrap());

    let mut cancelled = persisted(IssueAutomationState::Cancelled, None);
    let preserved = cancelled.clone();
    assert!(cancelled.transition(IssueTransition::Complete).is_err());
    assert_eq!(cancelled, preserved);
}

#[test]
fn exhausted_attempts_and_cancelled_work_do_not_restart_automatically() {
    for state in [
        IssueAutomationState::Running,
        IssueAutomationState::InReview,
    ] {
        let mut issue = persisted(state, None);
        assert!(issue.transition(IssueTransition::ExhaustAttempts).unwrap());
        assert_eq!(issue.state(), IssueAutomationState::Failed);
        assert!(!issue.transition(IssueTransition::ExhaustAttempts).unwrap());
        assert!(issue.transition(IssueTransition::Start).is_err());
    }

    let mut issue = persisted(IssueAutomationState::Queued, None);
    assert!(issue.transition(IssueTransition::AwaitDecision).is_err());
    assert!(issue.transition(IssueTransition::Cancel).unwrap());
    assert!(!issue.transition(IssueTransition::Cancel).unwrap());
    let cancelled = issue.clone();
    for transition in [
        IssueTransition::Reassess,
        IssueTransition::RetryFailed,
        IssueTransition::Queue,
        IssueTransition::Start,
        IssueTransition::OpenReview,
    ] {
        assert!(issue.transition(transition).is_err());
        assert_eq!(issue, cancelled);
    }
}

#[test]
fn multiple_in_memory_edges_cannot_skip_a_persisted_phase() {
    let current = persisted(IssueAutomationState::Discovered, None);
    let mut candidate = current.clone();
    candidate
        .transition(IssueTransition::Assess(IssueTriageResult::Ready))
        .unwrap();
    current.validate_replacement(&candidate).unwrap();
    candidate.transition(IssueTransition::Queue).unwrap();
    assert_eq!(
        current.validate_replacement(&candidate),
        Err(IssueAutomationError::InvalidStateReplacement {
            from: IssueAutomationState::Discovered,
            to: IssueAutomationState::Queued,
        })
    );
    assert_eq!(candidate.revision(), current.revision());
}

#[test]
fn all_flag_combinations_survive_storage_and_toggling_preserves_other_flags() {
    for bits in 0_u8..8 {
        let expected = IssueAutomationFlags {
            held: bits & 1 != 0,
            stale: bits & 2 != 0,
            unknown: bits & 4 != 0,
        };
        let mut record = discovered().to_record();
        record.revision = 1;
        record.flags = expected;
        let wire = serde_json::to_string(&record).unwrap();
        let mut issue = TrackedIssue::hydrate(serde_json::from_str(&wire).unwrap()).unwrap();
        assert_eq!(issue.flags(), expected);
        assert!(!issue.set_flag(IssueAutomationFlag::Held, expected.held));
        assert!(issue.set_flag(IssueAutomationFlag::Held, !expected.held));
        assert_eq!(
            issue.flags(),
            IssueAutomationFlags {
                held: !expected.held,
                ..expected
            }
        );
        issue.set_flag(IssueAutomationFlag::Held, expected.held);
        assert!(issue.set_flag(IssueAutomationFlag::Stale, !expected.stale));
        assert_eq!(
            issue.flags(),
            IssueAutomationFlags {
                stale: !expected.stale,
                ..expected
            }
        );
        issue.set_flag(IssueAutomationFlag::Stale, expected.stale);
        assert!(issue.set_flag(IssueAutomationFlag::Unknown, !expected.unknown));
        assert_eq!(
            issue.flags(),
            IssueAutomationFlags {
                unknown: !expected.unknown,
                ..expected
            }
        );
        assert_eq!(issue.state(), IssueAutomationState::Discovered);
        assert_eq!(issue.revision(), 1);
    }
}

#[test]
fn source_hash_is_independent_of_masked_copy_and_no_original_fields_are_serialized() {
    let issue = persisted(IssueAutomationState::Discovered, None);
    let snapshot = issue.snapshot();
    assert_ne!(
        snapshot.basis.field_hash,
        IssueFieldHash::of_fields(&snapshot.shared_copy.title, &snapshot.shared_copy.body)
    );
    let wire = serde_json::to_string(&issue.to_record()).unwrap();
    assert!(!wire.contains("raw-only-sentinel"));
    assert!(!wire.contains("원격 제목"));
    assert!(wire.contains("고객 식별값: [비공개]"));

    let mut next_snapshot = snapshot.clone();
    next_snapshot.basis.field_hash =
        IssueFieldHash::of_fields("원격 제목", "고객 식별값: another-raw-value");
    let mut changed = issue.clone();
    assert!(changed.replace_snapshot(next_snapshot).unwrap());
    assert_eq!(issue.snapshot().shared_copy, changed.snapshot().shared_copy);
    assert!(!issue.same_content(&changed));
    issue.validate_replacement(&changed).unwrap();
}

#[test]
fn snapshot_updates_keep_decision_phase_flags_identity_and_base_revision() {
    let mut issue = persisted(
        IssueAutomationState::AwaitingDecision,
        Some(IssueDecisionResumePhase::InReview),
    );
    issue.set_flag(IssueAutomationFlag::Stale, true);
    issue.set_flag(IssueAutomationFlag::Unknown, true);
    let original = issue.clone();
    let mut next = issue.snapshot().clone();
    next.policy_version = "policy-v2".to_owned();
    next.basis.provider_updated_at = "2026-11-20T13:00:00Z".to_owned();
    assert!(issue.replace_snapshot(next.clone()).unwrap());
    assert!(!issue.replace_snapshot(next).unwrap());
    assert_eq!(issue.state(), original.state());
    assert_eq!(issue.decision_resume(), original.decision_resume());
    assert_eq!(issue.flags(), original.flags());
    assert_eq!(issue.reference(), original.reference());
    assert_eq!(issue.binding_id(), original.binding_id());
    assert_eq!(issue.revision(), original.revision());
}

#[test]
fn invalid_snapshots_fail_atomically_and_cannot_hydrate() {
    let invalid_changes: &[fn(&mut IssueAutomationSnapshot)] = &[
        |snapshot| snapshot.basis.field_hash.version = 0,
        |snapshot| snapshot.basis.field_hash.sha256 = "0".repeat(63),
        |snapshot| snapshot.basis.field_hash.sha256 = "A".repeat(64),
        |snapshot| snapshot.basis.field_hash.sha256 = "g".repeat(64),
        |snapshot| snapshot.basis.provider_updated_at = " ".to_owned(),
        |snapshot| snapshot.policy_version = "policy\0invalid".to_owned(),
    ];
    for change in invalid_changes {
        let mut issue = persisted(IssueAutomationState::Discovered, None);
        let original = issue.clone();
        let mut invalid = issue.snapshot().clone();
        change(&mut invalid);
        assert!(issue.replace_snapshot(invalid.clone()).is_err());
        assert_eq!(issue, original);
        let mut record = original.to_record();
        record.snapshot = invalid;
        assert!(TrackedIssue::hydrate(record).is_err());
    }
}

#[test]
fn hydration_requires_a_saved_revision_and_consistent_resume_phase() {
    let new = discovered();
    assert_eq!(
        TrackedIssue::hydrate(new.to_record()),
        Err(IssueAutomationError::InvalidRevision)
    );
    let mut record = new.to_record();
    record.revision = MAX_ISSUE_AUTOMATION_REVISION;
    assert!(TrackedIssue::hydrate(record.clone()).is_ok());
    record.revision += 1;
    assert_eq!(
        TrackedIssue::hydrate(record.clone()),
        Err(IssueAutomationError::InvalidRevision)
    );
    record.revision = 1;
    record.state = IssueAutomationState::AwaitingDecision;
    assert_eq!(
        TrackedIssue::hydrate(record.clone()),
        Err(IssueAutomationError::InvalidDecisionResume)
    );
    record.decision_resume = Some(IssueDecisionResumePhase::Running);
    assert!(TrackedIssue::hydrate(record.clone()).is_ok());
    record.state = IssueAutomationState::Running;
    assert_eq!(
        TrackedIssue::hydrate(record),
        Err(IssueAutomationError::InvalidDecisionResume)
    );
}

#[test]
fn hydration_refuses_empty_identity_and_unknown_record_fields() {
    let invalid_references: &[fn(&mut IssueReference)] = &[
        |reference| reference.server_id = " ".to_owned(),
        |reference| reference.repository_id = " ".to_owned(),
        |reference| reference.issue_id = " ".to_owned(),
        |reference| reference.project_scope.workspace_dir = " ".to_owned(),
    ];
    for change in invalid_references {
        let mut record = persisted(IssueAutomationState::Discovered, None).to_record();
        change(&mut record.reference);
        assert!(TrackedIssue::hydrate(record).is_err());
    }
    let invalid_bindings: &[fn(&mut PersistedIssueAutomationBinding)] = &[
        |record| record.server_id = " ".to_owned(),
        |record| record.repository_id = " ".to_owned(),
        |record| record.project_scope.workspace_dir = " ".to_owned(),
    ];
    for change in invalid_bindings {
        let mut record = binding().to_record();
        change(&mut record);
        assert!(IssueAutomationBinding::hydrate(record).is_err());
    }

    let record = persisted(IssueAutomationState::Discovered, None).to_record();
    let mut value = serde_json::to_value(&record).unwrap();
    value["rawFields"] = serde_json::json!({"body": "do-not-store"});
    assert!(serde_json::from_value::<PersistedTrackedIssue>(value).is_err());
    let mut value = serde_json::to_value(&record).unwrap();
    value["flags"]["unexpected"] = true.into();
    assert!(serde_json::from_value::<PersistedTrackedIssue>(value).is_err());
    let mut value = serde_json::to_value(&record).unwrap();
    value["snapshot"]["rawBody"] = "do-not-store".into();
    assert!(serde_json::from_value::<PersistedTrackedIssue>(value).is_err());
}

#[test]
fn tracked_reference_and_binding_remain_immutable_after_hydration() {
    let original = persisted(IssueAutomationState::Discovered, None);
    let changes: &[fn(&mut IssueReference)] = &[
        |reference| reference.provider = IssueProvider::Jira,
        |reference| reference.server_id = "new-server".to_owned(),
        |reference| reference.repository_id = "R_new".to_owned(),
        |reference| reference.issue_id = "I_new".to_owned(),
        |reference| reference.project_scope.workspace_dir = "/work/new".to_owned(),
    ];
    for change in changes {
        let mut record = original.to_record();
        change(&mut record.reference);
        let candidate = TrackedIssue::hydrate(record).unwrap();
        assert_eq!(
            original.validate_replacement(&candidate),
            Err(IssueAutomationError::IdentityChanged)
        );
        assert!(!original.same_content(&candidate));
    }
    let mut record = original.to_record();
    record.binding_id = IssueAutomationBindingId::new("binding-other").unwrap();
    let candidate = TrackedIssue::hydrate(record).unwrap();
    assert_eq!(
        original.validate_replacement(&candidate),
        Err(IssueAutomationError::BindingChanged)
    );
    assert!(!original.same_content(&candidate));
}

#[test]
fn content_equality_excludes_only_store_revision() {
    let original = persisted(
        IssueAutomationState::AwaitingDecision,
        Some(IssueDecisionResumePhase::Running),
    );
    let mut next_record = original.to_record();
    next_record.revision += 1;
    let next_revision = TrackedIssue::hydrate(next_record).unwrap();
    assert!(original.same_content(&next_revision));
    assert_ne!(original, next_revision);
    assert_eq!(
        original.validate_replacement(&next_revision),
        Err(IssueAutomationError::RevisionChanged)
    );

    let changes: &[fn(&mut PersistedTrackedIssue)] = &[
        |record| record.snapshot.basis.provider_updated_at = "later".to_owned(),
        |record| record.snapshot.policy_version = "policy-v2".to_owned(),
        |record| record.snapshot.shared_copy.title.push('!'),
        |record| record.snapshot.shared_copy.body.push('!'),
        |record| record.snapshot.basis.field_hash = IssueFieldHash::of_fields("changed", "source"),
        |record| record.flags.held = true,
        |record| record.flags.stale = true,
        |record| record.flags.unknown = true,
        |record| record.decision_resume = Some(IssueDecisionResumePhase::Ready),
        |record| {
            record.state = IssueAutomationState::Cancelled;
            record.decision_resume = None;
        },
    ];
    for change in changes {
        let mut record = original.to_record();
        change(&mut record);
        let candidate = TrackedIssue::hydrate(record).unwrap();
        assert!(!original.same_content(&candidate));
    }
}
