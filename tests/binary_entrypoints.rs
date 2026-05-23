use std::io::{self, BufRead, BufReader, Read};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_BIN: &str = env!("CARGO_BIN_EXE_codex-exec-loop-native");
const AKRA_BIN: &str = env!("CARGO_BIN_EXE_akra");
const ADMIN_BIN: &str = env!("CARGO_BIN_EXE_akra-admin");
const TELEGRAM_BIN: &str = env!("CARGO_BIN_EXE_akra-telegram");

#[test]
fn default_binary_and_akra_wrapper_share_help_and_error_contracts() {
    let default_help = run_command(DEFAULT_BIN, &["--help"]);
    assert_success(&default_help);
    assert_contains(&default_help.stdout, "akra admin [--port <port>]");

    let akra_help = run_command(AKRA_BIN, &["--help"]);
    assert_success(&akra_help);
    assert_contains(&akra_help.stdout, "Usage: akra telegram");

    let unsupported = run_command(AKRA_BIN, &["not-a-command"]);
    assert_failure(&unsupported);
    assert_contains(&unsupported.stderr, "unsupported command: not-a-command");
}

#[test]
fn admin_binary_reports_actual_ephemeral_port_and_exits_on_interrupt() {
    let mut child = Command::new(ADMIN_BIN)
        .args(["--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("admin binary should spawn");

    let stdout = child.stdout.take().expect("admin stdout should be piped");
    let stderr = child.stderr.take().expect("admin stderr should be piped");
    let first_line = read_first_line_with_timeout(stdout, Duration::from_secs(5));
    let stderr_reader = thread::spawn(move || read_to_string(stderr));

    assert_contains(
        first_line.as_bytes(),
        "local planning admin server listening on http://127.0.0.1:",
    );
    assert!(
        !first_line.trim_end().ends_with(":0"),
        "admin server should report the actual ephemeral port, not the requested port 0: {first_line:?}"
    );

    interrupt_child(&mut child).expect("admin process should receive interrupt");
    let status = wait_for_child(&mut child, Duration::from_secs(5))
        .expect("admin process wait should succeed")
        .unwrap_or_else(|| {
            child
                .kill()
                .expect("timed out admin process should be killed");
            child.wait().expect("killed admin process should be waited")
        });
    let stderr = stderr_reader.join().expect("stderr reader should finish");

    assert!(
        status.success(),
        "admin process should shut down cleanly after interrupt; status={status:?}, stderr={stderr}"
    );
}

#[test]
fn admin_and_telegram_binaries_report_bootstrap_argument_errors() {
    let admin_error = run_command(ADMIN_BIN, &["--unknown"]);
    assert_failure(&admin_error);
    assert_contains(&admin_error.stderr, "unsupported argument: --unknown");

    let telegram_help = run_command(TELEGRAM_BIN, &["--help"]);
    assert_success(&telegram_help);
    assert_contains(&telegram_help.stdout, "Usage: akra telegram");

    let telegram_error = run_command(
        TELEGRAM_BIN,
        &["--token", "test", "--poll-timeout-seconds", "0"],
    );
    assert_failure(&telegram_error);
    assert_contains(
        &telegram_error.stderr,
        "--poll-timeout-seconds must be greater than zero",
    );
}

fn run_command(binary: &str, args: &[&str]) -> Output {
    Command::new(binary)
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("failed to run {binary}: {error}"))
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "expected success, status={:?}, stdout={}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_failure(output: &Output) {
    assert!(
        !output.status.success(),
        "expected failure, status={:?}, stdout={}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_contains(haystack: &[u8], needle: &str) {
    let haystack = String::from_utf8_lossy(haystack);
    assert!(
        haystack.contains(needle),
        "expected output to contain {needle:?}, got {haystack:?}"
    );
}

fn read_first_line_with_timeout(stdout: impl Read + Send + 'static, timeout: Duration) -> String {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
        let _ = sender.send(result);
    });
    receiver
        .recv_timeout(timeout)
        .expect("timed out waiting for admin server startup line")
        .expect("admin server startup line should be readable")
}

fn read_to_string(stderr: impl Read) -> String {
    let mut body = String::new();
    let mut reader = BufReader::new(stderr);
    reader
        .read_to_string(&mut body)
        .expect("stderr should be readable");
    body
}

fn interrupt_child(child: &mut Child) -> io::Result<()> {
    #[cfg(unix)]
    {
        let status = Command::new("kill")
            .args(["-INT", &child.id().to_string()])
            .status()?;
        if !status.success() {
            return Err(io::Error::other(format!(
                "kill -INT failed with status {status:?}"
            )));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        child.kill()
    }
}

fn wait_for_child(child: &mut Child, timeout: Duration) -> io::Result<Option<ExitStatus>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        thread::sleep(Duration::from_millis(20));
    }
}
