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
    use std::time::Instant;

    use super::*;

    /// 읽을 때마다 다음 커밋을 답한다. 두 번째 읽기는 실패한다.
    #[derive(Default)]
    struct CountingPort(AtomicUsize);

    impl RepositoryStatePort for CountingPort {
        fn repository_state(
            &self,
            _workspace_dir: &str,
        ) -> anyhow::Result<Option<RepositoryState>> {
            let read = self.0.fetch_add(1, Ordering::SeqCst);
            if read == 1 {
                anyhow::bail!("git is busy")
            }
            Ok(Some(RepositoryState {
                head: format!("commit-{read}"),
                branch: None,
            }))
        }
    }

    fn wait_for(observer: &RepositoryObserver, head: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while observer.latest().map(|state| state.head).as_deref() != Some(head) {
            assert!(Instant::now() < deadline, "the observer never read {head}");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn the_observer_keeps_reading_after_a_failed_read_and_stops_when_dropped() {
        let port = Arc::new(CountingPort::default());
        let observer = RepositoryObserver::start_with_interval(
            port.clone(),
            "/workspace".to_string(),
            Duration::from_millis(20),
        );
        wait_for(&observer, "commit-0");
        // 실패한 두 번째 읽기는 상태를 지우지 않는다. 세 번째 읽기가 새 상태를 준다.
        wait_for(&observer, "commit-2");

        drop(observer);
        std::thread::sleep(Duration::from_millis(100));
        let reads = port.0.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(port.0.load(Ordering::SeqCst), reads);
    }
}
