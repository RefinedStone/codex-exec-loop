use std::io;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

pub(crate) const SUBPROCESS_TIMEOUT_ENV: &str = "CODEX_EXEC_LOOP_SUBPROCESS_TIMEOUT_SECS";
const DEFAULT_SUBPROCESS_TIMEOUT_SECS: u64 = 30;
const POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(crate) fn configured_subprocess_timeout() -> Duration {
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
    command_output_with_timeout(command, command_label, configured_subprocess_timeout())
}

pub(crate) fn command_output_with_timeout(
    command: &mut Command,
    command_label: &str,
    timeout: Duration,
) -> io::Result<Output> {
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    let child = command.spawn()?;
    wait_with_output_timeout(child, command_label, timeout)
}

pub(crate) fn wait_with_output(child: Child, command_label: &str) -> io::Result<Output> {
    wait_with_output_timeout(child, command_label, configured_subprocess_timeout())
}

pub(crate) fn wait_with_output_timeout(
    mut child: Child,
    command_label: &str,
    timeout: Duration,
) -> io::Result<Output> {
    let started_at = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output();
        }
        if started_at.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(timeout_error(command_label, timeout));
        }
        std::thread::sleep(POLL_INTERVAL.min(timeout.saturating_sub(started_at.elapsed())));
    }
}

fn timeout_error(command_label: &str, timeout: Duration) -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "command `{command_label}` timed out after {}",
            format_duration(timeout)
        ),
    )
}

fn format_duration(duration: Duration) -> String {
    if duration.as_secs() > 0 && duration.subsec_millis() == 0 {
        return format!("{}s", duration.as_secs());
    }
    format!("{}ms", duration.as_millis())
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_SUBPROCESS_TIMEOUT_SECS, command_output, command_output_with_timeout,
        format_duration, parse_subprocess_timeout_secs, wait_with_output,
    };
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

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
    fn command_output_kills_slow_process_after_timeout() {
        let started_at = Instant::now();
        let error = command_output_with_timeout(
            Command::new("sh").args(["-c", "sleep 2"]),
            "sh -c sleep 2",
            Duration::from_millis(50),
        )
        .expect_err("slow command should time out");

        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(
            error.to_string().contains("timed out after"),
            "timeout error should include diagnostic text: {error}"
        );
        assert!(
            started_at.elapsed() < Duration::from_secs(2),
            "timeout should return before the child process naturally exits"
        );
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
        let child = Command::new("sh")
            .args(["-c", "printf child-output"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("child should start");

        let output = wait_with_output(child, "sh -c printf child-output")
            .expect("child should complete before the default timeout");

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), "child-output");
        assert!(output.stderr.is_empty());
    }

    #[test]
    fn timeout_duration_format_uses_seconds_only_for_whole_seconds() {
        assert_eq!(format_duration(Duration::from_secs(2)), "2s");
        assert_eq!(format_duration(Duration::from_millis(250)), "250ms");
        assert_eq!(format_duration(Duration::from_millis(1_250)), "1250ms");
    }
}
