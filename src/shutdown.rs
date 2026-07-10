use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(unix)]
use anyhow::Context;
use anyhow::Result;

pub(crate) struct GracefulShutdown {
    requested: Arc<AtomicBool>,
}

impl GracefulShutdown {
    pub(crate) fn install() -> Result<Self> {
        let requested = Arc::new(AtomicBool::new(false));
        #[cfg(unix)]
        for signal in [
            signal_hook::consts::signal::SIGINT,
            signal_hook::consts::signal::SIGTERM,
            signal_hook::consts::signal::SIGHUP,
        ] {
            let _registration = signal_hook::flag::register(signal, requested.clone())
                .with_context(|| {
                    format!("failed to install graceful handler for signal {signal}")
                })?;
            // signal-hook does not restore the prior/default action after unregistering the final
            // handler. Akra installs these handlers only for process-lifetime service entrypoints,
            // so deliberately leave each registration active until process exit.
        }

        Ok(Self { requested })
    }

    pub(crate) fn is_requested(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::process::Command;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use super::GracefulShutdown;

    const CHILD_ENV: &str = "AKRA_SHUTDOWN_SIGNAL_TEST_CHILD";
    const READY_ENV: &str = "AKRA_SHUTDOWN_SIGNAL_TEST_READY";
    const TEST_NAME: &str =
        "shutdown::tests::process_signal_requests_shutdown_in_isolated_test_process";

    #[test]
    fn process_signal_requests_shutdown_in_isolated_test_process() {
        if std::env::var_os(CHILD_ENV).is_some() {
            let shutdown = GracefulShutdown::install().expect("shutdown handlers should install");
            let ready = std::env::var_os(READY_ENV).expect("child ready path should exist");
            std::fs::write(ready, b"ready").expect("child should publish readiness");
            let deadline = Instant::now() + Duration::from_secs(5);
            while !shutdown.is_requested() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(shutdown.is_requested());
            return;
        }

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after the epoch")
            .as_nanos();
        let ready = std::env::temp_dir().join(format!(
            "akra-shutdown-signal-test-{}-{nonce}",
            std::process::id()
        ));
        let mut child =
            Command::new(std::env::current_exe().expect("test executable should exist"))
                .args(["--exact", TEST_NAME, "--nocapture"])
                .env(CHILD_ENV, "1")
                .env(READY_ENV, &ready)
                .spawn()
                .expect("isolated signal test should spawn");
        let ready_deadline = Instant::now() + Duration::from_secs(5);
        while !ready.exists() && Instant::now() < ready_deadline {
            assert!(
                child
                    .try_wait()
                    .expect("child state should remain readable")
                    .is_none(),
                "isolated signal test exited before publishing readiness"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        if !ready.exists() {
            let _ = child.kill();
            let _ = child.wait();
            panic!("isolated signal test did not become ready");
        }
        let child_pid = libc::pid_t::try_from(child.id()).expect("child PID should fit pid_t");
        // SAFETY: the child PID is live and owns the handlers installed after publishing readiness.
        assert_eq!(unsafe { libc::kill(child_pid, libc::SIGTERM) }, 0);

        let exit_deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            if let Some(status) = child
                .try_wait()
                .expect("child state should remain readable")
            {
                break status;
            }
            if Instant::now() >= exit_deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("isolated signal test did not exit after SIGTERM");
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let _ = std::fs::remove_file(ready);
        assert!(status.success(), "isolated signal child failed: {status}");
    }
}
