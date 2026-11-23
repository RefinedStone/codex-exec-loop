use super::*;
use crate::domain::kernel_event::{sample_task_evidence_events, sample_task_verification_event};

struct Workspace(std::path::PathBuf);
impl Workspace {
    fn new(label: &str) -> Self {
        let path = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-evidence-kernel-events-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn name(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn record(sequence: i64, event: KernelEvent) -> RecordedKernelEvent {
    RecordedKernelEvent {
        sequence,
        recorded_at: "2026-11-23T00:00:00Z".into(),
        event,
    }
}

#[test]
fn task_evidence_receipts_survive_ack_loss_retention_and_reject_changed_payload() {
    let directory = Workspace::new("retention");
    let workspace = directory.name();
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let events: Vec<_> = sample_task_evidence_events()
        .into_iter()
        .chain(std::iter::once(sample_task_verification_event()))
        .map(|event| KernelEvent::task_evidence(&event))
        .collect();
    let kinds = [
        "criterion_evidence_recorded",
        "criterion_waived",
        "task_verification_finished",
    ];
    for (index, event) in events.iter().enumerate() {
        adapter
            .append_kernel_events(&workspace, &[record(index as i64 + 1, event.clone())])
            .unwrap();
        assert!(adapter.has_task_evidence_event(&workspace, event).unwrap());
        assert!(
            !adapter
                .has_issue_work_event(&workspace, event.task_evidence_event_id().unwrap())
                .unwrap()
        );
    }
    adapter
        .append_kernel_events(&workspace, &[record(4, events[0].clone())])
        .unwrap();
    assert_eq!(
        adapter
            .load_kernel_events(&workspace, &kinds)
            .unwrap()
            .len(),
        3
    );
    adapter
        .append_kernel_events(
            &workspace,
            &[record(
                RETAINED_KERNEL_EVENTS + 10,
                KernelEvent::TurnStarted {
                    thread_id: "thread-fixture".into(),
                    turn_id: "turn-fixture".into(),
                    op_id: None,
                },
            )],
        )
        .unwrap();
    assert!(
        adapter
            .load_kernel_events(&workspace, &kinds)
            .unwrap()
            .is_empty()
    );
    for (index, event) in events.iter().enumerate() {
        assert!(adapter.has_task_evidence_event(&workspace, event).unwrap());
        adapter
            .append_kernel_events(
                &workspace,
                &[record(
                    RETAINED_KERNEL_EVENTS + 11 + index as i64,
                    event.clone(),
                )],
            )
            .unwrap();
    }
    assert!(
        adapter
            .load_kernel_events(&workspace, &kinds)
            .unwrap()
            .is_empty(),
        "replayed ACK recovery must not resurrect trimmed history"
    );
    let mut changed = events[0].clone();
    if let KernelEvent::CriterionEvidenceRecorded { evidence, .. } = &mut changed {
        evidence.artifact.sha256 = "9".repeat(64);
    }
    assert!(
        adapter
            .has_task_evidence_event(&workspace, &changed)
            .is_err()
    );
    assert!(
        adapter
            .append_kernel_events(&workspace, &[record(RETAINED_KERNEL_EVENTS + 20, changed)])
            .is_err()
    );
    let mut changed_host = events[2].clone();
    let KernelEvent::TaskVerificationFinished { verification, .. } = &mut changed_host else {
        panic!("expected a host execution receipt");
    };
    verification.artifact.as_mut().unwrap().sha256 = "a".repeat(64);
    assert_ne!(changed_host, events[2]);
    assert!(
        adapter
            .has_task_evidence_event(&workspace, &changed_host)
            .is_err()
    );
    assert!(
        adapter
            .append_kernel_events(
                &workspace,
                &[record(RETAINED_KERNEL_EVENTS + 21, changed_host)]
            )
            .is_err()
    );
}

#[test]
fn a_conflicting_evidence_payload_rolls_back_other_receipts_in_the_same_append() {
    let directory = Workspace::new("rollback");
    let workspace = directory.name();
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let old = KernelEvent::task_evidence(&sample_task_evidence_events()[0]);
    adapter
        .append_kernel_events(&workspace, &[record(1, old.clone())])
        .unwrap();
    let mut fresh = KernelEvent::task_evidence(&sample_task_evidence_events()[1]);
    if let KernelEvent::CriterionWaived { event_id, .. } = &mut fresh {
        *event_id = "fresh-waiver".into();
    }
    let mut changed = old;
    if let KernelEvent::CriterionEvidenceRecorded {
        evidence_revision, ..
    } = &mut changed
    {
        *evidence_revision += 1;
    }
    assert!(
        adapter
            .append_kernel_events(&workspace, &[record(2, fresh.clone()), record(3, changed)])
            .is_err()
    );
    assert!(!adapter.has_task_evidence_event(&workspace, &fresh).unwrap());
    assert_eq!(adapter.last_kernel_event_sequence(&workspace).unwrap(), 1);
    assert_eq!(
        adapter
            .load_kernel_events(&workspace, &["criterion_waived"])
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn binding_pause_receipt_deduplicates_after_retention_and_rejects_changed_payload() {
    let directory = Workspace::new(&format!("binding-pause-{}", rand::random::<u64>()));
    let workspace = directory.name();
    let adapter = SqlitePlanningAuthorityAdapter::new();
    let event = crate::domain::kernel_event::sample_issue_binding_pause_event();
    adapter
        .append_kernel_events(&workspace, &[record(1, event.clone())])
        .unwrap();
    assert!(
        adapter
            .has_issue_binding_pause_event(&workspace, &event)
            .unwrap()
    );
    assert!(
        !adapter
            .has_issue_work_event(&workspace, event.issue_binding_pause_event_id().unwrap())
            .unwrap()
    );
    adapter
        .append_kernel_events(
            &workspace,
            &[record(
                RETAINED_KERNEL_EVENTS + 2,
                KernelEvent::TurnStarted {
                    thread_id: "trim".into(),
                    turn_id: "trim".into(),
                    op_id: None,
                },
            )],
        )
        .unwrap();
    assert!(
        adapter
            .load_kernel_events(&workspace, &["issue_binding_pause_completed"])
            .unwrap()
            .is_empty()
    );
    adapter
        .append_kernel_events(
            &workspace,
            &[record(RETAINED_KERNEL_EVENTS + 3, event.clone())],
        )
        .unwrap();
    assert!(
        adapter
            .load_kernel_events(&workspace, &["issue_binding_pause_completed"])
            .unwrap()
            .is_empty()
    );
    let mut changed = event;
    let KernelEvent::IssueBindingPauseCompleted { receipt, .. } = &mut changed else {
        panic!("pause fixture")
    };
    receipt.revision += 1;
    assert!(
        adapter
            .has_issue_binding_pause_event(&workspace, &changed)
            .is_err()
    );
    assert!(
        adapter
            .append_kernel_events(&workspace, &[record(RETAINED_KERNEL_EVENTS + 4, changed)])
            .is_err()
    );
}
