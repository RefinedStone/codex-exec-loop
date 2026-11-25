//! Actual Windows sharing violations at the production journal's write-open boundary.
//! The fixture owns every locked file; no disk exhaustion or synthetic error is used.
use std::fs::OpenOptions;
use std::os::windows::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION;
use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_DELETE, FILE_SHARE_READ};

use super::{ApiCalls, World, public_text, publish_request, run_isolated};
use crate::application::port::outbound::issue_publish_intent_journal_port::{
    IssuePublicationLease, IssuePublishIntentJournalPort,
};
use crate::composition::production::build_issue_publish_journal;
use crate::domain::issue_publish_intent::{IssuePublishIntentRecord, IssuePublishIntentState};

struct SharingFailureJournal {
    inner: Arc<dyn IssuePublishIntentJournalPort>,
    runtime: PathBuf,
    fail_state: IssuePublishIntentState,
    failed: AtomicBool,
}

impl IssuePublishIntentJournalPort for SharingFailureJournal {
    fn acquire_publication_lease(
        &self,
        workspace: &str,
    ) -> Result<Box<dyn IssuePublicationLease + '_>> {
        self.inner.acquire_publication_lease(workspace)
    }

    fn load(&self, workspace: &str) -> Result<Vec<IssuePublishIntentRecord>> {
        self.inner.load(workspace)
    }

    fn save(&self, workspace: &str, record: &IssuePublishIntentRecord) -> Result<()> {
        if record.state != self.fail_state || self.failed.swap(true, Ordering::SeqCst) {
            return self.inner.save(workspace, record);
        }
        let path = self.runtime.join("issue-publish-intents.db");
        // Deny writers to this already committed, private fixture database only.
        // The separate publication reservation remains owned by the real service.
        let blocker = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
            .open(&path)
            .expect("the committed fixture database must open before sharing-failure injection");
        let denied = OpenOptions::new().write(true).open(&path).unwrap_err();
        assert_eq!(denied.raw_os_error(), Some(ERROR_SHARING_VIOLATION as i32));
        let result = self.inner.save(workspace, record);
        let error = result
            .as_ref()
            .expect_err("the real journal write-open must fail");
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .and_then(std::io::Error::raw_os_error),
            Some(ERROR_SHARING_VIOLATION as i32),
            "the production private SQLite boundary must return the actual Windows error: {error:#}"
        );
        drop(blocker);
        result
    }
}

async fn exercise(fail_state: IssuePublishIntentState, expected_writes: usize) {
    let world = World::new();
    let journal = Arc::new(SharingFailureJournal {
        inner: build_issue_publish_journal(),
        runtime: world.runtime(),
        fail_state,
        failed: AtomicBool::new(false),
    });
    let mut session = world.start_with_journal(journal.clone()).await;
    let request = publish_request(&world, &mut session, public_text(), false).await;
    let failed = session.call("akra/issue/publish", request.clone()).await;
    assert!(failed["error"].is_object(), "{failed}");
    assert!(journal.failed.load(Ordering::SeqCst));
    assert_eq!(world.writes(), expected_writes);
    assert_eq!(world.counts(), ApiCalls::default());
    session.close().await;

    let retained = build_issue_publish_journal()
        .load(world.workspace.to_str().unwrap())
        .unwrap();
    assert_eq!(retained.len(), 1);
    assert_eq!(
        retained[0].state,
        if expected_writes == 0 {
            IssuePublishIntentState::Recorded
        } else {
            IssuePublishIntentState::Sent
        }
    );

    let mut restarted = world.start().await;
    let opened = restarted.call("akra/issue/open", world.target()).await;
    assert!(opened["result"].is_object(), "{opened}");
    let rechecked = restarted.call("akra/issue/recheck", world.target()).await;
    assert!(rechecked["result"].is_object(), "{rechecked}");
    assert_eq!(
        world.writes(),
        expected_writes,
        "reopening and rechecking a failed save must not send a remote write"
    );
    if expected_writes == 1 {
        assert_eq!(opened["result"]["intent"]["state"], "confirmed", "{opened}");
        assert_eq!(
            rechecked["result"]["intent"]["state"], "confirmed",
            "{rechecked}"
        );
        assert_eq!(
            opened["result"]["intent"]["intentId"],
            retained[0].intent_id
        );
        let repeated = restarted.call("akra/issue/publish", request).await;
        assert_eq!(repeated["result"]["status"], "conflict", "{repeated}");
        assert_eq!(world.writes(), 1, "a stale approved edit must not resend");
    }
    assert_eq!(world.counts(), ApiCalls::default());
    restarted.close().await;
}

#[test]
fn windows_write_open_failure_before_sent_keeps_recorded_and_sends_nothing() {
    run_isolated(
        "storage_failures::windows_write_open_failure_before_sent_keeps_recorded_and_sends_nothing",
        exercise(IssuePublishIntentState::Sent, 0),
    );
}

#[test]
fn windows_write_open_failure_after_remote_apply_recovers_without_resending() {
    run_isolated(
        "storage_failures::windows_write_open_failure_after_remote_apply_recovers_without_resending",
        exercise(IssuePublishIntentState::Confirmed, 1),
    );
}
