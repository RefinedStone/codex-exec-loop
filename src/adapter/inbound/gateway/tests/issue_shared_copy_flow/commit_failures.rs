//! Exercise actual SQLite commit failure through the Gateway, not an in-memory error stub.
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use rusqlite::Connection;

use super::{ApiCalls, World, public_text, publish_request, run_isolated};
use crate::application::port::outbound::issue_publish_intent_journal_port::{
    IssuePublicationLease, IssuePublishIntentJournalPort,
};
use crate::composition::production::build_issue_publish_journal;
use crate::domain::issue_publish_intent::{IssuePublishIntentRecord, IssuePublishIntentState};

struct CommitFailureJournal {
    inner: Arc<dyn IssuePublishIntentJournalPort>,
    runtime: PathBuf,
    fail_state: IssuePublishIntentState,
    failed: AtomicBool,
}

impl IssuePublishIntentJournalPort for CommitFailureJournal {
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
        if record.state == self.fail_state && !self.failed.swap(true, Ordering::SeqCst) {
            // A real concurrent SQLite reader lets BEGIN IMMEDIATE and the write succeed,
            // but prevents the rollback-journal transaction from upgrading at COMMIT.
            let reader = Connection::open(self.runtime.join("issue-publish-intents.db"))?;
            reader.execute_batch("BEGIN; SELECT * FROM issue_publish_intents;")?;
            let result = self.inner.save(workspace, record);
            reader.execute_batch("ROLLBACK")?;
            assert!(
                result.is_err(),
                "the SQLite reader must force an actual commit failure"
            );
            assert!(format!("{:#}", result.as_ref().unwrap_err()).contains("locked"));
            result
        } else {
            self.inner.save(workspace, record)
        }
    }
}

async fn exercise(fail_state: IssuePublishIntentState, expected_writes: usize) {
    let world = World::new();
    let journal = Arc::new(CommitFailureJournal {
        inner: build_issue_publish_journal(),
        runtime: world.runtime(),
        fail_state,
        failed: AtomicBool::new(false),
    });
    let mut session = world.start_with_journal(journal.clone()).await;
    let request = publish_request(&world, &mut session, public_text(), false).await;
    let result = session.call("akra/issue/publish", request).await;
    assert!(result["error"].is_object(), "{result}");
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
    if expected_writes == 1 {
        assert_eq!(opened["result"]["intent"]["state"], "confirmed", "{opened}");
        let rechecked = restarted.call("akra/issue/recheck", world.target()).await;
        assert_eq!(
            rechecked["result"]["intent"]["state"], "confirmed",
            "{rechecked}"
        );
    }
    assert_eq!(
        world.writes(),
        expected_writes,
        "recovery only reads remote state"
    );
    assert_eq!(world.counts(), ApiCalls::default());
    restarted.close().await;
}

#[test]
fn failed_sent_commit_prevents_any_remote_write() {
    run_isolated(
        "commit_failures::failed_sent_commit_prevents_any_remote_write",
        exercise(IssuePublishIntentState::Sent, 0),
    );
}

#[test]
fn failed_completion_commit_recovers_from_durable_sent_by_reading_only() {
    run_isolated(
        "commit_failures::failed_completion_commit_recovers_from_durable_sent_by_reading_only",
        exercise(IssuePublishIntentState::Confirmed, 1),
    );
}
