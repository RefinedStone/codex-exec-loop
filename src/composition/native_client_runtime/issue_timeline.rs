use super::NativeClientRuntime;
use crate::domain::issue_automation::timeline::{
    ISSUE_TIMELINE_READ_LIMIT, IssueTimelineProjection, project_issue_timeline,
};
use crate::domain::issue_work::IssueReference;

impl NativeClientRuntime {
    pub(super) fn issue_timeline(&self, issue: &IssueReference) -> IssueTimelineProjection {
        let Some((log, workspace_dir)) = &self.kernel_event_log else {
            return IssueTimelineProjection::Unavailable;
        };
        let Ok(Some(records)) =
            log.load_recent_issue_kernel_events(workspace_dir, issue, ISSUE_TIMELINE_READ_LIMIT)
        else {
            return IssueTimelineProjection::Unavailable;
        };
        project_issue_timeline(issue, &records, &self.recent_kernel_records)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::application::port::outbound::kernel_event_log_port::KernelEventLogPort;
    use crate::composition::native_client_runtime::tests::recording_runtime_for_test;
    use crate::domain::issue_work::{IssueProjectScope, IssueProvider};
    use crate::domain::kernel_event::{KernelEvent, RecordedKernelEvent};

    struct TimelineLog {
        records: Option<Vec<RecordedKernelEvent>>,
        fail: bool,
        reads: AtomicUsize,
    }

    impl KernelEventLogPort for TimelineLog {
        fn append_kernel_events(&self, _: &str, _: &[RecordedKernelEvent]) -> anyhow::Result<()> {
            panic!("timeline must never append events");
        }

        fn load_kernel_events(&self, _: &str, _: &[&str]) -> anyhow::Result<Vec<KernelEvent>> {
            panic!("timeline must use the bounded issue query");
        }

        fn load_recent_issue_kernel_events(
            &self,
            workspace_dir: &str,
            issue: &IssueReference,
            limit: usize,
        ) -> anyhow::Result<Option<Vec<RecordedKernelEvent>>> {
            assert_eq!(workspace_dir, "/workspace");
            assert_eq!(issue, &reference());
            assert_eq!(limit, 21);
            self.reads.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                anyhow::bail!("fixture read failure");
            }
            Ok(self.records.clone())
        }
    }

    fn reference() -> IssueReference {
        IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".into(),
            repository_id: "R_project".into(),
            issue_id: "I_42".into(),
            project_scope: IssueProjectScope {
                workspace_dir: "/workspace".into(),
            },
        }
    }

    fn record(sequence: i64) -> RecordedKernelEvent {
        RecordedKernelEvent {
            sequence,
            recorded_at: "2026-11-22T00:00:00Z".into(),
            event: KernelEvent::issue_tracked(&reference(), "binding-1", sequence as u64),
        }
    }

    #[test]
    fn issue_timeline_runtime_includes_not_yet_persisted_records_without_commands_or_writes() {
        let (mut runtime, turns) = recording_runtime_for_test();
        let log = Arc::new(TimelineLog {
            records: Some(vec![record(1)]),
            fail: false,
            reads: AtomicUsize::new(0),
        });
        runtime.kernel_event_log = Some((log.clone(), "/workspace".into()));
        runtime.recent_kernel_records.extend([record(1), record(2)]);
        let IssueTimelineProjection::Ready { entries, has_older } =
            runtime.issue_timeline(&reference())
        else {
            panic!("stored and runtime records should be available");
        };
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.sequence)
                .collect::<Vec<_>>(),
            [1, 2]
        );
        assert!(!has_older);
        assert_eq!(log.reads.load(Ordering::SeqCst), 1);
        assert!(turns.lock().unwrap().is_empty());
        assert!(runtime.published_kernel_records.is_empty());
        assert!(runtime.issue_observer.is_none());
    }

    #[test]
    fn issue_timeline_runtime_does_not_present_partial_history_as_ready_after_read_failure() {
        let (mut runtime, _) = recording_runtime_for_test();
        runtime.recent_kernel_records.push_back(record(1));
        assert_eq!(
            runtime.issue_timeline(&reference()),
            IssueTimelineProjection::Unavailable
        );
        for (records, fail) in [(None, false), (Some(Vec::new()), true)] {
            runtime.kernel_event_log = Some((
                Arc::new(TimelineLog {
                    records,
                    fail,
                    reads: AtomicUsize::new(0),
                }),
                "/workspace".into(),
            ));
            assert_eq!(
                runtime.issue_timeline(&reference()),
                IssueTimelineProjection::Unavailable
            );
        }
    }
}
