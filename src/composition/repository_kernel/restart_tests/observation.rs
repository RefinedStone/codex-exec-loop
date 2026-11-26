//! Bounded waits identify readiness, durable storage and host reply failures separately.
use super::progress::Pause;
use super::*;
use crate::composition::kernel_host::KernelHostView;

impl Fixture {
    pub(super) fn diagnostic(&self, stage: &str, started: Instant, detail: &str) -> String {
        let selection = self
            .store
            .issue_autonomy_selection(&self.workspace)
            .map(|state| {
                format!(
                    "revision={} pending={} requested={:?}",
                    state.revision,
                    state.pending.is_some(),
                    state.requested_level
                )
            })
            .unwrap_or_else(|_| "unavailable".into());
        let collection = self
            .store
            .get_collection(&self.workspace)
            .map(|state| {
                state
                    .map(|state| {
                        format!(
                            "epoch={} revision={} mode={:?} failures={} last_failure={}",
                            state.epoch,
                            state.revision,
                            state.config.mode,
                            state.status.consecutive_failures,
                            state.status.last_failure.is_some()
                        )
                    })
                    .unwrap_or_else(|| "absent".into())
            })
            .unwrap_or_else(|_| "unavailable".into());
        let work = self
            .store
            .get_issue_work(&self.workspace, &self.source.observation.issue)
            .map(|work| {
                work.map(|work| {
                    format!(
                        "revision={} tasks={} permit_revision={} permit_source={} blocked={:?}",
                        work.revision,
                        work.task_ids.len(),
                        work.permit_revision,
                        work.permit_source.is_some(),
                        work.blocked_reason
                    )
                })
                .unwrap_or_else(|| "absent".into())
            })
            .unwrap_or_else(|_| "unavailable".into());
        let execution = self
            .execution
            .0
            .try_lock()
            .map(|state| {
                format!(
                    "submissions={} grants={}",
                    state.submissions,
                    state.grants.len()
                )
            })
            .unwrap_or_else(|_| "busy".into());
        format!(
            "restart wait failed: stage={stage} elapsed_ms={} detail={detail}; {}; selection[{selection}]; collection[{collection}]; work[{work}]; execution[{execution}]",
            started.elapsed().as_millis(),
            self.progress.summary()
        )
    }

    pub(super) fn wait_phase(
        &self,
        phase: Phase,
        deadline: Instant,
        started: Instant,
    ) -> Result<(), String> {
        if self.progress.wait_phase(phase, deadline) {
            Ok(())
        } else {
            Err(self.diagnostic(&format!("{phase:?}"), started, "phase signal missing"))
        }
    }

    pub(super) fn wait_pause(&self, pause: &Pause) {
        let started = Instant::now();
        pause
            .entered
            .recv_timeout(RESTART_WAIT)
            .unwrap_or_else(|error| {
                panic!(
                    "{}",
                    self.diagnostic(
                        &format!("{:?}", pause.phase),
                        started,
                        &format!("pause not entered: {error:?}")
                    )
                )
            });
    }

    pub(super) fn wait_restart_work(
        &self,
        expected: &str,
        predicate: impl Fn(&crate::domain::issue_automation::coordinator::IssueWorkCheckpoint) -> bool,
    ) -> crate::domain::issue_automation::coordinator::IssueWorkCheckpoint {
        self.try_wait_restart_work(expected, RESTART_WAIT, predicate)
            .unwrap_or_else(|error| panic!("{error}"))
    }

    pub(super) fn try_wait_restart_work(
        &self,
        expected: &str,
        timeout: Duration,
        predicate: impl Fn(&crate::domain::issue_automation::coordinator::IssueWorkCheckpoint) -> bool,
    ) -> Result<crate::domain::issue_automation::coordinator::IssueWorkCheckpoint, String> {
        let started = Instant::now();
        let deadline = started + timeout;
        self.wait_phase(Phase::DiscoveryReturned, deadline, started)?;
        let initial = self
            .store
            .get_issue_work(&self.workspace, &self.source.observation.issue)
            .map_err(|_| self.diagnostic(expected, started, "work checkpoint read failed"))?;
        // A reopened ledger may already contain its work. New work waits on fixture-owned
        // execution signals before checking the persisted checkpoint, within the same watchdog.
        if initial.is_none() {
            self.wait_phase(Phase::WorkSubmitted, deadline, started)?;
            self.wait_phase(Phase::PermitGranted, deadline, started)?;
        }
        loop {
            let work = self
                .store
                .get_issue_work(&self.workspace, &self.source.observation.issue)
                .map_err(|_| self.diagnostic(expected, started, "work checkpoint read failed"))?;
            if let Some(work) = work
                && predicate(&work)
            {
                return Ok(work);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(self.diagnostic(expected, started, "durable checkpoint not committed"));
            }
            // There is no notification for the final SQLite commit. Phase signals wake this
            // wait immediately; a bounded fallback observes commits that follow the last signal.
            self.progress.wait_change(
                self.progress.sequence(),
                remaining.min(Duration::from_millis(10)),
            );
        }
    }

    pub(super) fn observe_host(&self, kernels: &RepositoryKernelRegistry) -> KernelHostView {
        self.try_observe_host(kernels, HOST_WAIT)
            .unwrap_or_else(|error| panic!("{error}"))
    }

    pub(super) fn try_observe_host(
        &self,
        kernels: &RepositoryKernelRegistry,
        timeout: Duration,
    ) -> Result<KernelHostView, String> {
        let started = Instant::now();
        let entry = kernels.entry(&self.candidate.repository_key).map_err(|_| {
            self.diagnostic("host_mailbox", started, "repository entry unavailable")
        })?;
        let slot = entry.lock();
        let kernel = slot
            .as_ref()
            .ok_or_else(|| self.diagnostic("host_mailbox", started, "host not installed"))?;
        kernel
            .host
            .observe(timeout)
            .map_err(|error| self.diagnostic("host_mailbox", started, &format!("reply={error:?}")))
    }

    pub(super) fn pause_host(&self, kernels: &RepositoryKernelRegistry) -> Pause {
        let (pause, entered, released) = Pause::channels(Phase::HostMailbox);
        kernels
            .entry(&self.candidate.repository_key)
            .unwrap()
            .lock()
            .as_ref()
            .unwrap()
            .host
            .pause_for_test(entered, released)
            .unwrap();
        pause
    }
}
