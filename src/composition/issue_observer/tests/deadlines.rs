use super::*;
use std::sync::atomic::AtomicU64;

fn open_decision() -> IssueObserverCommittedFact {
    use crate::application::service::issue_coordinator::IssueCoordinatorFact;
    use crate::domain::issue_automation::decision::{
        IssueDecisionBasis, IssueDecisionFact, IssueDecisionId, IssueDecisionKind,
        IssueDecisionState, IssueDecisionSubject,
    };
    use crate::domain::issue_automation::policy::IssueAutomationGate;
    use crate::domain::issue_work::{IssueProjectScope, IssueProvider, IssueReference};

    IssueObserverCommittedFact::Coordinated(IssueCoordinatorFact::Decision {
        fact: IssueDecisionFact {
            decision_id: IssueDecisionId::new("new-short-deadline").unwrap(),
            subject: IssueDecisionSubject::Issue(IssueReference {
                provider: IssueProvider::GitHub,
                server_id: "github.com".into(),
                repository_id: "R_deadline".into(),
                issue_id: "I_deadline".into(),
                project_scope: IssueProjectScope {
                    workspace_dir: "/workspace/deadline".into(),
                },
            }),
            gate: IssueAutomationGate::Start,
            kind: IssueDecisionKind::StartApproval,
            basis: IssueDecisionBasis {
                authorization_epoch: 0,
                policy_version: "a".repeat(64),
                field_hash: None,
                work_order_id: None,
                gate_revision: 1,
            },
            state: IssueDecisionState::Open,
            answer_by: None,
            option: None,
        },
        revision: 1,
    })
}

#[test]
fn cached_two_and_four_second_deadlines_precede_five_second_tracking_without_host_reads() {
    let mut h = Harness::new(IssueCollectionMode::Off);
    let track = h.track();
    let reads = Arc::new(AtomicUsize::new(0));
    let worker_reads = reads.clone();
    let host_thread = std::thread::current().id();
    h.observer.decision_deadline_reader = Some(Arc::new(move |at| {
        assert_ne!(std::thread::current().id(), host_thread);
        worker_reads.fetch_add(1, Ordering::SeqCst);
        Ok(if at < 3_000 {
            Some(3_000)
        } else if at < 5_000 {
            Some(5_000)
        } else {
            None
        })
    }));
    let now = Instant::now();
    h.observer.poll(now, 1_000);
    h.finish_tracking(track.read(), Ok(()), now);
    for _ in 0..10 {
        assert_eq!(
            h.observer.next_deadline(now),
            Some(now + Duration::from_secs(2))
        );
    }
    assert_eq!(
        reads.load(Ordering::SeqCst),
        1,
        "next_deadline only reads the cache"
    );
    h.observer.poll(now + Duration::from_millis(1_999), 2_999);
    track.assert_calls(1);
    h.observer.poll(now + Duration::from_secs(2), 3_000);
    let reminder = track.read();
    assert_eq!(reminder.at_unix_ms, 3_000);
    h.finish_tracking(reminder, Ok(()), now + Duration::from_secs(2));
    assert_eq!(
        h.observer.next_deadline(now),
        Some(now + Duration::from_secs(4))
    );
    h.observer.poll(now + Duration::from_millis(3_999), 4_999);
    track.assert_calls(2);
    h.observer.poll(now + Duration::from_secs(4), 5_000);
    let due = track.read();
    assert_eq!(due.at_unix_ms, 5_000);
    h.finish_tracking(due, Ok(()), now + Duration::from_secs(4));
    assert_eq!(
        h.observer.next_deadline(now),
        Some(now + Duration::from_secs(9))
    );
    h.observer.poll(now + Duration::from_secs(4), 5_000);
    track.assert_calls(3);
    assert_eq!(reads.load(Ordering::SeqCst), 3);
}

#[test]
fn deadline_mapping_keeps_the_worker_start_clock_when_completion_is_delayed() {
    let mut h = Harness::new(IssueCollectionMode::Off);
    let track = h.track();
    h.observer.decision_deadline_reader = Some(Arc::new(|at| {
        assert_eq!(at, 1_000);
        Ok(Some(3_000))
    }));
    let now = Instant::now();
    h.observer.poll(now, 1_000);
    let first = track.read();
    h.finish_tracking(first, Ok(()), now + Duration::from_secs(1));
    assert_eq!(
        h.observer.next_deadline(now),
        Some(now + Duration::from_secs(2))
    );
    assert_eq!(
        h.observer.next_tracking_run,
        Some(now + Duration::from_secs(6))
    );
}

#[test]
fn past_empty_and_failed_deadline_reads_keep_the_fallback_without_a_busy_loop() {
    for scenario in 0..4 {
        let mut h = Harness::new(IssueCollectionMode::Off);
        let track = h.track();
        h.observer.decision_deadline_reader = Some(Arc::new(move |_| match scenario {
            0 => Ok(None),
            1 => Ok(Some(1_000)),
            2 => Ok(Some(999)),
            _ => Err(anyhow::anyhow!("deadline read is temporarily unavailable")),
        }));
        let now = Instant::now();
        h.observer.poll(now, 1_000);
        h.finish_tracking(track.read(), Ok(()), now);
        assert_eq!(h.observer.next_deadline(now), Some(now + TRACKING_INTERVAL));
        for _ in 0..3 {
            h.observer.poll(now, 1_000);
        }
        track.assert_calls(1);
        assert_eq!(h.observer.consecutive_failures, 0);
    }
}

#[test]
fn a_new_open_commit_refreshes_deadlines_before_the_old_tracking_interval() {
    let mut h = Harness::new(IssueCollectionMode::Observe);
    let track = h.track();
    let next = Arc::new(AtomicU64::new(0));
    let next_from_store = next.clone();
    h.observer.decision_deadline_reader = Some(Arc::new(move |_| {
        let next = next_from_store.load(Ordering::SeqCst);
        Ok((next > 0).then_some(next))
    }));
    let now = Instant::now();
    h.observer.poll(now, 1_000);
    let collection = h.read();
    h.finish_tracking(track.read(), Ok(()), now);
    assert_eq!(h.observer.next_deadline(now), Some(now + TRACKING_INTERVAL));

    // The collector's real committed channel publishes a newly persisted short request.
    next.store(3_000, Ordering::SeqCst);
    h.observer
        .committed_sender
        .send(vec![open_decision()])
        .unwrap();
    let published = h.observer.poll(now + Duration::from_millis(1), 1_001);
    assert_eq!(published.committed.len(), 1);
    let refreshed = track.read();
    assert_eq!(refreshed.at_unix_ms, 1_001);
    h.finish_tracking(refreshed, Ok(()), now + Duration::from_millis(1));
    assert_eq!(
        h.observer.next_deadline(now),
        Some(now + Duration::from_secs(2))
    );
    track.assert_calls(2);
    h.finish(
        collection,
        Ok(success(projection(IssueCollectionMode::Observe, 1))),
        now + Duration::from_millis(2),
    );
    assert_eq!(
        h.observer.next_deadline(now),
        Some(now + Duration::from_secs(2))
    );
}
