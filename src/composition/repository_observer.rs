//! 작업 공간의 git 상태를 전용 스레드에서 주기적으로 읽는다 (plan 20 §4.1). 커널 스레드는 git 을 기다리지 않는다.
//! 커널은 주기마다 마지막으로 읽은 상태만 본다. 읽기가 실패하면 지난 상태를 그대로 둔다.

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::application::port::outbound::repository_state_port::RepositoryStatePort;
use crate::composition::core_effect_worker::spawn_worker_with_panic_fallback;
use crate::domain::kernel_event::RepositoryState;

/// 커널 호스트가 한가할 때 깨는 간격과 같다. 더 자주 읽어도 커널이 보지 않는다.
const OBSERVATION_INTERVAL: Duration = Duration::from_secs(30);

pub(crate) struct RepositoryObserver {
    latest: Arc<Mutex<Option<RepositoryState>>>,
    // 버리면 관찰 스레드가 다음 대기에서 끝난다. 읽는 중인 git 을 기다리지 않는다.
    _stop: Sender<()>,
}

impl RepositoryObserver {
    pub(crate) fn start(port: Arc<dyn RepositoryStatePort>, workspace_dir: String) -> Self {
        Self::start_with_interval(port, workspace_dir, OBSERVATION_INTERVAL)
    }

    fn start_with_interval(
        port: Arc<dyn RepositoryStatePort>,
        workspace_dir: String,
        interval: Duration,
    ) -> Self {
        let latest = Arc::new(Mutex::new(None));
        let (stop, stopped) = mpsc::channel::<()>();
        let seen = latest.clone();
        // 스레드를 기다리지 않으므로 handle 을 버린다.
        let _ = spawn_worker_with_panic_fallback(
            move || {
                loop {
                    match port.repository_state(&workspace_dir) {
                        Ok(Some(state)) => {
                            *seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) =
                                Some(state);
                        }
                        // 상태가 없는 폴더는 사건을 내지 않는다.
                        Ok(None) => {}
                        Err(error) => tracing::warn!(
                            error_chars = error.to_string().chars().count(),
                            "repository state read failed"
                        ),
                    }
                    if stopped.recv_timeout(interval) != Err(RecvTimeoutError::Timeout) {
                        return;
                    }
                }
            },
            || tracing::warn!("repository observer stopped after a panic"),
        );
        Self {
            latest,
            _stop: stop,
        }
    }

    /// 마지막으로 읽은 상태이다. 아직 읽지 못했으면 `None` 이다.
    pub(crate) fn latest(&self) -> Option<RepositoryState> {
        self.latest
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// Each read is held at its entry until the test explicitly releases it.
    /// This prevents a fast observer from skipping the intermediate snapshot
    /// before the test thread has an opportunity to inspect it.
    struct ControlledPort {
        next_read: AtomicUsize,
        read_entered: Sender<usize>,
        allow_read: Mutex<mpsc::Receiver<()>>,
        exited: Sender<()>,
    }

    impl RepositoryStatePort for ControlledPort {
        fn repository_state(
            &self,
            _workspace_dir: &str,
        ) -> anyhow::Result<Option<RepositoryState>> {
            let read = self.next_read.fetch_add(1, Ordering::SeqCst);
            self.read_entered
                .send(read)
                .expect("test read entry receiver");
            self.allow_read
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .expect("test must release each observer read");
            if read == 1 {
                anyhow::bail!("git is busy")
            }
            Ok(Some(RepositoryState {
                head: format!("commit-{read}"),
                branch: None,
            }))
        }
    }

    impl Drop for ControlledPort {
        fn drop(&mut self) {
            let _ = self.exited.send(());
        }
    }

    #[test]
    fn the_observer_keeps_reading_after_a_failed_read_and_stops_when_dropped() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (allow_tx, allow_rx) = mpsc::channel();
        let (exited_tx, exited_rx) = mpsc::channel();
        let port = Arc::new(ControlledPort {
            next_read: AtomicUsize::new(0),
            read_entered: entered_tx,
            allow_read: Mutex::new(allow_rx),
            exited: exited_tx,
        });

        // Move our only Arc into the worker. ControlledPort::drop then becomes an
        // exact worker-exit acknowledgement, not a timed guess after two sleeps.
        let observer = RepositoryObserver::start_with_interval(
            port,
            "/workspace".to_owned(),
            Duration::from_millis(20),
        );
        let read_started = |expected: usize| {
            assert_eq!(
                entered_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
                expected,
                "observer read order is controlled by explicit channel releases",
            );
        };

        read_started(0);
        assert!(observer.latest().is_none(), "the first read is held");
        allow_tx.send(()).unwrap();
        read_started(1);
        assert_eq!(observer.latest().unwrap().head, "commit-0");

        // Fail the second read. The previous successful snapshot must survive.
        allow_tx.send(()).unwrap();
        read_started(2);
        assert_eq!(observer.latest().unwrap().head, "commit-0");

        allow_tx.send(()).unwrap();
        read_started(3);
        assert_eq!(observer.latest().unwrap().head, "commit-2");

        // Drop while the fourth read is held; then release it so the worker
        // can reach its stop-channel check and terminate deterministically.
        drop(observer);
        allow_tx.send(()).unwrap();
        exited_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("worker must drop its port after observer shutdown");
        assert!(
            matches!(entered_rx.try_recv(), Err(mpsc::TryRecvError::Disconnected)),
            "no additional read may be started after the observer stops",
        );
    }
}
