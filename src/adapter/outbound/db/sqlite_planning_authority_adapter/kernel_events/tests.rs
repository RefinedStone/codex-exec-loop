use super::*;
use crate::domain::kernel_event::sample_task_evidence_events;

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
    let events = sample_task_evidence_events().map(|event| KernelEvent::task_evidence(&event));
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
        .append_kernel_events(&workspace, &[record(3, events[0].clone())])
        .unwrap();
    assert_eq!(
        adapter
            .load_kernel_events(
                &workspace,
                &["criterion_evidence_recorded", "criterion_waived"]
            )
            .unwrap()
            .len(),
        2
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
            .load_kernel_events(
                &workspace,
                &["criterion_evidence_recorded", "criterion_waived"]
            )
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
            .load_kernel_events(
                &workspace,
                &["criterion_evidence_recorded", "criterion_waived"]
            )
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
