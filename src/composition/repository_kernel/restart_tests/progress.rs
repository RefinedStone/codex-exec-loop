//! Test-owned phase signals and one-shot delays; no source content enters diagnostics.
use super::*;
use std::collections::BTreeMap;
use std::sync::{Condvar, Mutex, mpsc};
use std::thread::ThreadId;

pub(super) const RESTART_WAIT: Duration = Duration::from_secs(10);
pub(super) const HOST_WAIT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Phase {
    StoreOpen,
    RuntimeStarting,
    RuntimeReady,
    DiscoveryStarted,
    DiscoveryReturned,
    ResyncStarted,
    WorkSubmitted,
    PermitGranted,
    HostMailbox,
}

#[derive(Default)]
struct State {
    sequence: u64,
    counts: BTreeMap<Phase, usize>,
    last: Option<Phase>,
    paused: Option<Phase>,
}

struct Delay {
    phase: Phase,
    after: Option<Phase>,
    owner: Option<ThreadId>,
    entered: mpsc::Sender<()>,
    released: mpsc::Receiver<()>,
}

#[derive(Default)]
pub(super) struct Progress {
    state: Mutex<State>,
    changed: Condvar,
    delays: Mutex<Vec<Delay>>,
}

/// Dropping a test's release handle also unblocks the worker during assertion unwinding.
pub(super) struct Pause {
    pub(super) phase: Phase,
    pub(super) entered: mpsc::Receiver<()>,
    release: Option<mpsc::Sender<()>>,
}

impl Pause {
    pub(super) fn channels(phase: Phase) -> (Self, mpsc::Sender<()>, mpsc::Receiver<()>) {
        let (entered, entry) = mpsc::channel();
        let (release, released) = mpsc::channel();
        (
            Self {
                phase,
                entered: entry,
                release: Some(release),
            },
            entered,
            released,
        )
    }

    pub(super) fn release(mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }
}
impl Drop for Pause {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }
}

impl Progress {
    pub(super) fn pause(&self, phase: Phase) -> Pause {
        self.pause_after(phase, None)
    }

    /// The next store opening on the discovery worker is delayed, not a parent's diagnostic read.
    pub(super) fn pause_store_after_discovery(&self) -> Pause {
        self.pause_after(Phase::StoreOpen, Some(Phase::DiscoveryReturned))
    }

    fn pause_after(&self, phase: Phase, after: Option<Phase>) -> Pause {
        let (pause, entered, released) = Pause::channels(phase);
        self.delays.lock().unwrap().push(Delay {
            phase,
            after,
            owner: None,
            entered,
            released,
        });
        pause
    }

    pub(super) fn hit(&self, phase: Phase) {
        let current = std::thread::current().id();
        let delay = {
            let mut delays = self.delays.lock().unwrap();
            for delay in &mut *delays {
                if delay.after == Some(phase) && delay.owner.is_none() {
                    delay.owner = Some(current);
                }
            }
            delays
                .iter()
                .position(|delay| {
                    delay.phase == phase && (delay.after.is_none() || delay.owner == Some(current))
                })
                .map(|index| delays.remove(index))
        };
        {
            let mut state = self.state.lock().unwrap();
            state.sequence += 1;
            *state.counts.entry(phase).or_default() += 1;
            state.last = Some(phase);
            if delay.is_some() {
                state.paused = Some(phase);
            }
            self.changed.notify_all();
        }
        if let Some(delay) = delay {
            let _ = delay.entered.send(());
            let result = delay.released.recv_timeout(RESTART_WAIT);
            {
                let mut state = self.state.lock().unwrap();
                state.paused = None;
                state.sequence += 1;
                self.changed.notify_all();
            }
            assert!(
                result.is_ok(),
                "restart fixture release missing: phase={phase:?}"
            );
        }
    }

    pub(super) fn wait_phase(&self, phase: Phase, deadline: Instant) -> bool {
        let mut state = self.state.lock().unwrap();
        while state.counts.get(&phase).copied().unwrap_or_default() == 0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            state = self.changed.wait_timeout(state, remaining).unwrap().0;
        }
        true
    }

    pub(super) fn wait_change(&self, sequence: u64, timeout: Duration) {
        let state = self.state.lock().unwrap();
        if state.sequence == sequence {
            drop(self.changed.wait_timeout(state, timeout).unwrap());
        }
    }

    pub(super) fn sequence(&self) -> u64 {
        self.state.lock().unwrap().sequence
    }

    pub(super) fn summary(&self) -> String {
        let state = self.state.lock().unwrap();
        format!(
            "sequence={} last={:?} paused={:?} phases={:?}",
            state.sequence, state.last, state.paused, state.counts
        )
    }
}
