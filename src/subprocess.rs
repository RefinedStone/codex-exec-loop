//! Product configuration facade over explicit-timeout native process containment.
#[cfg(windows)]
pub(crate) use akra_native::subprocess::spawn_without_console;
pub(crate) use akra_native::subprocess::{
    ManagedChild, command_output_with_input_and_timeout,
    command_output_with_input_timeout_and_limits, command_output_with_timeout, spawn,
    wait_with_output_timeout, wait_with_output_timeout_and_limits,
};
use std::io;
use std::process::{Command, Output};
use std::time::Duration;

pub(crate) const SUBPROCESS_TIMEOUT_ENV: &str = "AKRA_SUBPROCESS_TIMEOUT_SECS";
const DEFAULT_SUBPROCESS_TIMEOUT_SECS: u64 = 30;

#[cfg(test)]
thread_local! {
    // 테스트 전용 제한 시간이다. 프로세스 환경 변수로 바꾸면 동시에 도는 다른 테스트의 하위 프로세스에도
    // 짧은 제한이 걸린다 (plan 18 K-D7). 그래서 그 테스트의 스레드에만 둔다.
    static TEST_SUBPROCESS_TIMEOUT: std::cell::Cell<Option<Duration>> =
        const { std::cell::Cell::new(None) };
}

/// 이 스레드의 하위 프로세스 제한 시간을 guard 가 사는 동안만 바꾼다. 테스트 전용이다.
#[cfg(test)]
pub(crate) struct TestSubprocessTimeoutGuard {
    previous: Option<Duration>,
}

#[cfg(test)]
impl TestSubprocessTimeoutGuard {
    pub(crate) fn set(timeout: Duration) -> Self {
        Self {
            previous: TEST_SUBPROCESS_TIMEOUT.replace(Some(timeout)),
        }
    }
}

#[cfg(test)]
impl Drop for TestSubprocessTimeoutGuard {
    fn drop(&mut self) {
        TEST_SUBPROCESS_TIMEOUT.set(self.previous);
    }
}

pub(crate) fn configured_subprocess_timeout() -> Duration {
    #[cfg(test)]
    if let Some(timeout) = TEST_SUBPROCESS_TIMEOUT.get() {
        return timeout;
    }
    if let Some(config) = crate::configuration::current_process_config() {
        return Duration::from_secs(config.config.subprocess.timeout_secs);
    }
    Duration::from_secs(parse_subprocess_timeout_secs(
        std::env::var(SUBPROCESS_TIMEOUT_ENV).ok().as_deref(),
    ))
}

pub(crate) fn parse_subprocess_timeout_secs(value: Option<&str>) -> u64 {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return DEFAULT_SUBPROCESS_TIMEOUT_SECS;
    };
    match value.parse::<u64>() {
        Ok(seconds) if seconds > 0 => seconds,
        _ => DEFAULT_SUBPROCESS_TIMEOUT_SECS,
    }
}

pub(crate) fn command_output(command: &mut Command, command_label: &str) -> io::Result<Output> {
    #[cfg(test)]
    let _timer = crate::test_utils::NativeTestTimer::new("command_output");
    command_output_with_timeout(command, command_label, configured_subprocess_timeout())
}

pub(crate) fn command_output_with_input(
    command: &mut Command,
    command_label: &str,
    input: &[u8],
) -> io::Result<Output> {
    command_output_with_input_and_timeout(
        command,
        command_label,
        input,
        configured_subprocess_timeout(),
    )
}

#[cfg(test)]
pub(crate) fn wait_with_output(child: ManagedChild, command_label: &str) -> io::Result<Output> {
    wait_with_output_timeout(child, command_label, configured_subprocess_timeout())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;
    #[test]
    fn timeout_env_parser_falls_back_for_missing_empty_zero_and_invalid_values() {
        assert_eq!(
            parse_subprocess_timeout_secs(None),
            DEFAULT_SUBPROCESS_TIMEOUT_SECS
        );
        assert_eq!(
            parse_subprocess_timeout_secs(Some("")),
            DEFAULT_SUBPROCESS_TIMEOUT_SECS
        );
        assert_eq!(
            parse_subprocess_timeout_secs(Some("0")),
            DEFAULT_SUBPROCESS_TIMEOUT_SECS
        );
        assert_eq!(
            parse_subprocess_timeout_secs(Some("not-a-number")),
            DEFAULT_SUBPROCESS_TIMEOUT_SECS
        );
        assert_eq!(parse_subprocess_timeout_secs(Some("7")), 7);
    }

    #[test]
    fn test_timeout_override_stays_on_its_own_thread() {
        let short = Duration::from_millis(1234);
        {
            let _guard = super::TestSubprocessTimeoutGuard::set(short);
            assert_eq!(super::configured_subprocess_timeout(), short);
            let other_thread = std::thread::spawn(super::configured_subprocess_timeout)
                .join()
                .expect("other thread should read the timeout");
            assert_ne!(other_thread, short);
        }
        assert_ne!(super::configured_subprocess_timeout(), short);
    }

    #[test]
    fn command_output_uses_configured_timeout_and_captures_successful_output() {
        let output = command_output(
            Command::new("sh").args(["-c", "printf stdout; printf stderr >&2"]),
            "sh -c printf",
        )
        .expect("command should complete before the default timeout");

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), "stdout");
        assert_eq!(String::from_utf8_lossy(&output.stderr), "stderr");
    }
    #[test]
    fn wait_with_output_uses_configured_timeout_for_existing_child() {
        let child = spawn(
            Command::new("sh")
                .args(["-c", "printf child-output"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )
        .expect("child should start");

        let output = wait_with_output(child, "sh -c printf child-output")
            .expect("child should complete before the default timeout");

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), "child-output");
        assert!(output.stderr.is_empty());
    }
}
