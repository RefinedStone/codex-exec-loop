use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub(crate) struct ParallelModeAutomationGuard {
    active_epochs: Arc<Mutex<BTreeMap<String, u64>>>,
}

#[derive(Clone, Debug)]
pub(crate) struct ParallelModeAutomationPermit {
    guard: ParallelModeAutomationGuard,
    workspace_directory: String,
    epoch_id: u64,
    continuation_permit: Option<crate::domain::planning::PostTurnContinuationPermit>,
}

impl ParallelModeAutomationPermit {
    pub(crate) fn is_active(&self) -> bool {
        self.guard
            .is_active(&self.workspace_directory, self.epoch_id)
            && self
                .continuation_permit
                .as_ref()
                .is_none_or(|permit| permit.is_current())
    }

    pub(crate) fn with_continuation_permit(
        mut self,
        permit: crate::domain::planning::PostTurnContinuationPermit,
    ) -> Self {
        self.continuation_permit = Some(permit);
        self
    }

    pub(crate) fn with_active_commit<T>(&self, operation: impl FnOnce() -> T) -> Option<T> {
        match self.continuation_permit.as_ref() {
            Some(continuation) => continuation
                .with_current(|| {
                    self.guard.with_active_epoch(
                        &self.workspace_directory,
                        self.epoch_id,
                        operation,
                    )
                })
                .flatten(),
            None => {
                self.guard
                    .with_active_epoch(&self.workspace_directory, self.epoch_id, operation)
            }
        }
    }
}

impl std::fmt::Debug for ParallelModeAutomationGuard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ParallelModeAutomationGuard")
            .finish_non_exhaustive()
    }
}

impl ParallelModeAutomationGuard {
    pub(crate) fn activate(&self, workspace_directory: impl Into<String>, epoch_id: u64) {
        self.active_epochs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(workspace_directory.into(), epoch_id);
    }

    pub(crate) fn cancel(&self, workspace_directory: &str) {
        self.active_epochs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(workspace_directory);
    }

    pub(crate) fn is_active(&self, workspace_directory: &str, epoch_id: u64) -> bool {
        self.active_epochs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(workspace_directory)
            .is_some_and(|active_epoch_id| *active_epoch_id == epoch_id)
    }

    fn with_active_epoch<T>(
        &self,
        workspace_directory: &str,
        epoch_id: u64,
        operation: impl FnOnce() -> T,
    ) -> Option<T> {
        /*
         * Keep the epoch mutex through the bounded authority commit. Epoch cancellation or
         * replacement either linearizes before this check and rejects the commit, or waits
         * until the accepted commit completes. Callers that also carry a post-turn permit
         * acquire its continuation mutex first, so the global lock order is continuation ->
         * epoch and cancellation never needs the continuation mutex.
         */
        let active_epochs = self
            .active_epochs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        active_epochs
            .get(workspace_directory)
            .is_some_and(|active_epoch_id| *active_epoch_id == epoch_id)
            .then(operation)
    }

    pub(crate) fn permit(
        &self,
        workspace_directory: impl Into<String>,
        epoch_id: u64,
    ) -> ParallelModeAutomationPermit {
        ParallelModeAutomationPermit {
            guard: self.clone(),
            workspace_directory: workspace_directory.into(),
            epoch_id,
            continuation_permit: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::ParallelModeAutomationGuard;

    #[test]
    fn guard_rejects_closed_and_superseded_epochs() {
        let guard = ParallelModeAutomationGuard::default();

        assert!(!guard.is_active("/workspace", 1));
        guard.activate("/workspace", 1);
        assert!(guard.is_active("/workspace", 1));

        guard.activate("/workspace", 2);
        assert!(!guard.is_active("/workspace", 1));
        assert!(guard.is_active("/workspace", 2));

        guard.cancel("/workspace");
        assert!(!guard.is_active("/workspace", 2));
    }

    #[test]
    fn automation_permit_composes_post_turn_generation_with_epoch_identity() {
        let guard = ParallelModeAutomationGuard::default();
        guard.activate("/workspace", 7);
        let continuation_gate = crate::domain::planning::PostTurnContinuationGate::default();
        let permit = guard
            .permit("/workspace", 7)
            .with_continuation_permit(continuation_gate.capture());

        assert!(permit.is_active());
        assert_eq!(permit.with_active_commit(|| "committed"), Some("committed"));
        continuation_gate.advance();
        assert!(!permit.is_active());
        assert_eq!(permit.with_active_commit(|| "must not run"), None);
    }

    #[test]
    fn active_commit_linearizes_epoch_replacement_after_validation() {
        let guard = ParallelModeAutomationGuard::default();
        guard.activate("/workspace", 7);
        let permit = guard.permit("/workspace", 7);
        let (commit_entered_sender, commit_entered_receiver) = mpsc::channel();
        let (release_commit_sender, release_commit_receiver) = mpsc::channel();

        let commit_thread = thread::spawn(move || {
            permit.with_active_commit(|| {
                commit_entered_sender
                    .send(())
                    .expect("commit entry should be observed");
                release_commit_receiver
                    .recv()
                    .expect("commit should be released");
                "committed"
            })
        });
        commit_entered_receiver
            .recv()
            .expect("active epoch commit should enter");

        let replacement_guard = guard.clone();
        let (replacement_started_sender, replacement_started_receiver) = mpsc::channel();
        let (replacement_finished_sender, replacement_finished_receiver) = mpsc::channel();
        let replacement_thread = thread::spawn(move || {
            replacement_started_sender
                .send(())
                .expect("replacement start should be observed");
            replacement_guard.activate("/workspace", 8);
            replacement_finished_sender
                .send(())
                .expect("replacement completion should be observed");
        });
        replacement_started_receiver
            .recv()
            .expect("replacement should start");
        assert!(
            replacement_finished_receiver
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "epoch replacement must wait for an already accepted bounded commit"
        );

        release_commit_sender
            .send(())
            .expect("bounded commit should be released");
        assert_eq!(
            commit_thread.join().expect("commit thread should join"),
            Some("committed")
        );
        replacement_finished_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("replacement should complete after the commit");
        replacement_thread
            .join()
            .expect("replacement thread should join");
        assert!(!guard.is_active("/workspace", 7));
        assert!(guard.is_active("/workspace", 8));
    }

    #[test]
    fn active_commit_linearizes_epoch_cancellation_after_validation() {
        let guard = ParallelModeAutomationGuard::default();
        guard.activate("/workspace", 7);
        let permit = guard.permit("/workspace", 7);
        let (commit_entered_sender, commit_entered_receiver) = mpsc::channel();
        let (release_commit_sender, release_commit_receiver) = mpsc::channel();

        let commit_thread = thread::spawn(move || {
            permit.with_active_commit(|| {
                commit_entered_sender
                    .send(())
                    .expect("commit entry should be observed");
                release_commit_receiver
                    .recv()
                    .expect("commit should be released");
                "committed"
            })
        });
        commit_entered_receiver
            .recv()
            .expect("active epoch commit should enter");

        let cancellation_guard = guard.clone();
        let (cancellation_started_sender, cancellation_started_receiver) = mpsc::channel();
        let (cancellation_finished_sender, cancellation_finished_receiver) = mpsc::channel();
        let cancellation_thread = thread::spawn(move || {
            cancellation_started_sender
                .send(())
                .expect("cancellation start should be observed");
            cancellation_guard.cancel("/workspace");
            cancellation_finished_sender
                .send(())
                .expect("cancellation completion should be observed");
        });
        cancellation_started_receiver
            .recv()
            .expect("cancellation should start");
        assert!(
            cancellation_finished_receiver
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "epoch cancellation must wait for an already accepted bounded commit"
        );

        release_commit_sender
            .send(())
            .expect("bounded commit should be released");
        assert_eq!(
            commit_thread.join().expect("commit thread should join"),
            Some("committed")
        );
        cancellation_finished_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("cancellation should complete after the commit");
        cancellation_thread
            .join()
            .expect("cancellation thread should join");
        assert!(!guard.is_active("/workspace", 7));
    }
}
