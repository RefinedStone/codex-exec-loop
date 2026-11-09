//! akra-gateway 의 upstream: Gateway 전용 `codex app-server --listen unix://…` 를 실행하고 감시한다 (P-02).
//! 클라이언트 세션마다 `codex app-server proxy --sock …` 자식 프로세스를 열고, 그 stdio 를 바이트
//! 스트림으로 돌려준다 (P-03, P-09). tokio 는 Windows 에서 unix 소켓 클라이언트를 제공하지 않으므로
//! macOS 와 Windows 가 같은 proxy 경로를 쓴다.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context as TaskContext, Poll};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tokio::io::{AsyncRead, AsyncWrite, Join, ReadBuf};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::watch;

use super::connection::{AppServerConnectionConfig, app_server_command, codex_command};
use crate::application::port::outbound::gateway_upstream_port::{
    GatewayUpstreamPort, GatewayUpstreamSnapshot, GatewayUpstreamStream,
};
use crate::subprocess::{self, ManagedChild};

const UPSTREAM_SOCKET_FILE_NAME: &str = "codex-app-server.sock";
/// 지금 upstream 의 pid 를 적는 파일이다. Gateway 가 비정상 종료되면 다음 Gateway 가 남은 upstream 을 정리한다.
const UPSTREAM_PID_FILE_NAME: &str = "gateway-upstream.pid";
#[cfg(unix)]
const STALE_UPSTREAM_STOP_TIMEOUT: Duration = Duration::from_secs(5);
const UPSTREAM_READY_TIMEOUT: Duration = Duration::from_secs(10);
const UPSTREAM_POLL_INTERVAL: Duration = Duration::from_millis(250);
const CODEX_VERSION_TIMEOUT: Duration = Duration::from_secs(10);
const RESTART_BACKOFF_INITIAL: Duration = Duration::from_secs(1);
const RESTART_BACKOFF_MAX: Duration = Duration::from_secs(30);
const RESTART_BACKOFF_RESET_AFTER: Duration = Duration::from_secs(60);
/// macOS 의 `sun_path` 는 NUL 을 포함해 104 바이트이다. 가장 짧은 한도에 맞춘다.
const MAX_UNIX_SOCKET_PATH_BYTES: usize = 103;

pub(crate) struct AppServerGatewayUpstream {
    shared: Arc<UpstreamShared>,
}

struct UpstreamShared {
    config: AppServerConnectionConfig,
    socket_path: PathBuf,
    codex_version: Option<String>,
    started_at: String,
    restarts: AtomicU64,
    generation: watch::Sender<u64>,
    ready: AtomicBool,
    stopping: AtomicBool,
    child: Mutex<Option<ManagedChild>>,
}

impl AppServerGatewayUpstream {
    /// upstream app-server 를 시작하고 준비될 때까지 기다린 뒤 감시 스레드를 띄운다.
    /// `run_dir` 는 호출자가 현재 사용자 전용으로 만든 디렉터리여야 한다.
    pub(crate) fn start(run_dir: &Path) -> Result<Self> {
        let config = AppServerConnectionConfig::from_environment();
        config.ensure_executable_is_pinned()?;
        let socket_path = run_dir.join(UPSTREAM_SOCKET_FILE_NAME);
        ensure_socket_path_fits(&socket_path)?;
        let codex_version = read_codex_version(&config);
        stop_stale_upstream(&socket_path);
        let child = spawn_upstream(&config, &socket_path)?;
        let (generation, _) = watch::channel(0);
        let shared = Arc::new(UpstreamShared {
            config,
            socket_path,
            codex_version,
            started_at: chrono::Utc::now().to_rfc3339(),
            restarts: AtomicU64::new(0),
            generation,
            ready: AtomicBool::new(true),
            stopping: AtomicBool::new(false),
            child: Mutex::new(Some(child)),
        });
        let supervised = shared.clone();
        thread::Builder::new()
            .name("akra-gateway-upstream".to_string())
            .spawn(move || supervise_upstream(&supervised))
            .context("failed to start gateway upstream supervisor")?;
        Ok(Self { shared })
    }
}

impl GatewayUpstreamPort for AppServerGatewayUpstream {
    fn open_session(&self) -> Result<Box<dyn GatewayUpstreamStream>> {
        if !self.shared.ready.load(Ordering::SeqCst) {
            bail!("codex app-server upstream is not ready")
        }
        let mut command = codex_command(&self.shared.config);
        command
            .arg("app-server")
            .arg("proxy")
            .arg("--sock")
            .arg(codex_socket_argument(&self.shared.socket_path))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = subprocess::spawn(&mut command)
            .context("failed to spawn `codex app-server proxy` for a gateway session")?;
        let stdin = child.take_stdin().context("proxy stdin was not piped")?;
        let stdout = child.take_stdout().context("proxy stdout was not piped")?;
        if let Some(stderr) = child.take_stderr() {
            log_child_stderr("codex app-server proxy", stderr);
        }
        let io = tokio::io::join(ChildStdout::from_std(stdout)?, ChildStdin::from_std(stdin)?);
        Ok(Box::new(ProxySessionStream { io, _child: child }))
    }

    fn snapshot(&self) -> GatewayUpstreamSnapshot {
        GatewayUpstreamSnapshot {
            codex_version: self.shared.codex_version.clone(),
            started_at: self.shared.started_at.clone(),
            restarts: self.shared.restarts.load(Ordering::SeqCst),
        }
    }

    fn generation(&self) -> watch::Receiver<u64> {
        self.shared.generation.subscribe()
    }

    fn shutdown(&self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        self.shared.ready.store(false, Ordering::SeqCst);
        stop_child(&self.shared);
        let _ = std::fs::remove_file(&self.shared.socket_path);
        let _ = std::fs::remove_file(upstream_pid_path(&self.shared.socket_path));
    }
}

/// proxy 자식 프로세스의 stdio 이다. 필드 순서대로 drop 되므로 파이프를 먼저 닫고 프로세스를 정리한다.
struct ProxySessionStream {
    io: Join<ChildStdout, ChildStdin>,
    _child: ManagedChild,
}

impl AsyncRead for ProxySessionStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_read(context, buffer)
    }
}

impl AsyncWrite for ProxySessionStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.io).poll_write(context, data)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(context)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        context: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(context)
    }
}

fn supervise_upstream(shared: &UpstreamShared) {
    let mut backoff = RESTART_BACKOFF_INITIAL;
    let mut running_since = Instant::now();
    loop {
        wait_for_child_exit(shared);
        if shared.stopping.load(Ordering::SeqCst) {
            return;
        }
        // 기존 세션은 죽은 upstream 에 묶여 있으므로 세대를 올려 닫게 한다.
        shared.ready.store(false, Ordering::SeqCst);
        shared.generation.send_modify(|generation| *generation += 1);
        backoff = next_restart_backoff(backoff, running_since.elapsed());
        tracing::warn!(
            backoff_ms = backoff.as_millis() as u64,
            "gateway upstream app-server exited; restarting"
        );
        loop {
            if sleep_unless_stopping(shared, backoff) {
                return;
            }
            match spawn_upstream(&shared.config, &shared.socket_path) {
                Ok(child) => {
                    *lock_child(shared) = Some(child);
                    shared.restarts.fetch_add(1, Ordering::SeqCst);
                    shared.ready.store(true, Ordering::SeqCst);
                    running_since = Instant::now();
                    break;
                }
                Err(error) => {
                    tracing::warn!(error = %format!("{error:#}"), "gateway upstream restart failed");
                    backoff = next_restart_backoff(backoff, Duration::ZERO);
                }
            }
        }
    }
}

fn wait_for_child_exit(shared: &UpstreamShared) {
    loop {
        if shared.stopping.load(Ordering::SeqCst) {
            return;
        }
        let exited = match lock_child(shared).as_mut() {
            Some(child) => !matches!(child.try_wait(), Ok(None)),
            None => true,
        };
        if exited {
            return;
        }
        thread::sleep(UPSTREAM_POLL_INTERVAL);
    }
}

/// `true` 를 돌려주면 정지 요청이 들어온 것이다.
fn sleep_unless_stopping(shared: &UpstreamShared, duration: Duration) -> bool {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if shared.stopping.load(Ordering::SeqCst) {
            return true;
        }
        thread::sleep(UPSTREAM_POLL_INTERVAL.min(deadline - Instant::now()));
    }
    shared.stopping.load(Ordering::SeqCst)
}

fn stop_child(shared: &UpstreamShared) {
    if let Some(mut child) = lock_child(shared).take()
        && let Err(error) = child.terminate_and_wait()
    {
        tracing::warn!(error = %error, "failed to stop gateway upstream app-server");
    }
}

fn lock_child(shared: &UpstreamShared) -> std::sync::MutexGuard<'_, Option<ManagedChild>> {
    shared
        .child
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// 오래 정상으로 돌았으면 처음 간격으로 돌아가고, 아니면 두 배씩 늘려 최대값에서 멈춘다.
fn next_restart_backoff(current: Duration, ran_for: Duration) -> Duration {
    if ran_for >= RESTART_BACKOFF_RESET_AFTER {
        return RESTART_BACKOFF_INITIAL;
    }
    (current * 2).min(RESTART_BACKOFF_MAX)
}

fn spawn_upstream(config: &AppServerConnectionConfig, socket_path: &Path) -> Result<ManagedChild> {
    remove_stale_socket(socket_path)?;
    let mut command = app_server_command(config);
    command
        .arg("--listen")
        .arg(format!("unix://{}", codex_socket_argument(socket_path)))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = subprocess::spawn(&mut command)
        .context("failed to spawn gateway upstream `codex app-server --listen`")?;
    if let Some(stderr) = child.take_stderr() {
        log_child_stderr("codex app-server upstream", stderr);
    }
    if let Err(error) = std::fs::write(upstream_pid_path(socket_path), child.id().to_string()) {
        tracing::warn!(error = %error, "failed to record the gateway upstream pid");
    }
    wait_until_socket_ready(&mut child, socket_path)?;
    Ok(child)
}

fn upstream_pid_path(socket_path: &Path) -> PathBuf {
    socket_path.with_file_name(UPSTREAM_PID_FILE_NAME)
}

/*
 * 앞의 Gateway 가 비정상 종료되어 남은 upstream 을 끝낸다. Windows 는 job object 가 자식을 함께 끝내지만
 * macOS 는 upstream 이 고아로 남아 codex 소켓 잠금을 쥔다. 그러면 새 upstream 이 시작하지 못하고 Gateway 가
 * 바로 끝난다. 호출자는 단일 인스턴스 잠금을 이미 잡았으므로 기록된 upstream 은 살아 있는 Gateway 의 것이
 * 아니다. pid 재사용에 대비해 명령줄이 이 소켓의 app-server 일 때만 끝낸다.
 */
#[cfg(unix)]
fn stop_stale_upstream(socket_path: &Path) {
    let pid_path = upstream_pid_path(socket_path);
    let Ok(recorded) = std::fs::read_to_string(&pid_path) else {
        return;
    };
    let _ = std::fs::remove_file(&pid_path);
    let Ok(pid) = recorded.trim().parse::<libc::pid_t>() else {
        return;
    };
    if pid <= 1 || !process_serves_socket(pid, &codex_socket_argument(socket_path)) {
        return;
    }
    tracing::warn!(
        pid,
        "stopping a gateway upstream app-server left by a previous gateway"
    );
    // upstream 은 자기 프로세스 그룹의 리더이다 (`subprocess::spawn`). 그룹째 끝내서 손자도 정리한다.
    let target = if unsafe { libc::getpgid(pid) } == pid {
        -pid
    } else {
        pid
    };
    unsafe { libc::kill(target, libc::SIGTERM) };
    let deadline = Instant::now() + STALE_UPSTREAM_STOP_TIMEOUT;
    while Instant::now() < deadline {
        if unsafe { libc::kill(pid, 0) } != 0 {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    unsafe { libc::kill(target, libc::SIGKILL) };
}

#[cfg(not(unix))]
fn stop_stale_upstream(socket_path: &Path) {
    // Windows 의 job object 는 Gateway 가 끝날 때 upstream 도 끝낸다. 남은 기록만 지운다.
    let _ = std::fs::remove_file(upstream_pid_path(socket_path));
}

#[cfg(unix)]
fn process_serves_socket(pid: libc::pid_t, socket_argument: &str) -> bool {
    let Ok(output) = std::process::Command::new("ps")
        .args(["-ww", "-o", "command=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return false;
    };
    let command_line = String::from_utf8_lossy(&output.stdout);
    command_line.contains("app-server") && command_line.contains(socket_argument)
}

fn wait_until_socket_ready(child: &mut ManagedChild, socket_path: &Path) -> Result<()> {
    let deadline = Instant::now() + UPSTREAM_READY_TIMEOUT;
    loop {
        if socket_path.exists() {
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            bail!("gateway upstream app-server exited before listening ({status})")
        }
        if Instant::now() >= deadline {
            bail!(
                "gateway upstream app-server did not create `{}` within {}s",
                socket_path.display(),
                UPSTREAM_READY_TIMEOUT.as_secs()
            )
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn remove_stale_socket(socket_path: &Path) -> Result<()> {
    match std::fs::remove_file(socket_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to remove stale upstream socket `{}`",
                socket_path.display()
            )
        }),
    }
}

fn ensure_socket_path_fits(socket_path: &Path) -> Result<()> {
    let length = codex_socket_argument(socket_path).len();
    if length > MAX_UNIX_SOCKET_PATH_BYTES {
        bail!(
            "socket path `{}` is {length} bytes; the limit is {MAX_UNIX_SOCKET_PATH_BYTES}. Set AKRA_HOME to a shorter path",
            socket_path.display()
        )
    }
    Ok(())
}

/// codex 에 넘길 소켓 경로이다. Windows 에서는 `unix://` URL 과 같은 형식이 되도록 `/` 로 바꾼다.
fn codex_socket_argument(socket_path: &Path) -> String {
    let text = socket_path.to_string_lossy();
    if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text.into_owned()
    }
}

fn read_codex_version(config: &AppServerConnectionConfig) -> Option<String> {
    let mut command = codex_command(config);
    command.arg("--version");
    let output = subprocess::command_output_with_timeout(
        &mut command,
        "codex --version",
        CODEX_VERSION_TIMEOUT,
    )
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let version = text.trim();
    (!version.is_empty()).then(|| version.to_string())
}

fn log_child_stderr(label: &'static str, stderr: impl Read + Send + 'static) {
    let _ = thread::Builder::new()
        .name("akra-gateway-child-stderr".to_string())
        .spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                tracing::debug!(source = label, line = %line, "gateway child stderr");
            }
        });
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::{
        MAX_UNIX_SOCKET_PATH_BYTES, RESTART_BACKOFF_INITIAL, RESTART_BACKOFF_MAX,
        ensure_socket_path_fits, next_restart_backoff,
    };
    #[cfg(unix)]
    use super::{codex_socket_argument, stop_stale_upstream, upstream_pid_path};

    #[test]
    fn restart_backoff_doubles_until_the_cap_and_resets_after_a_healthy_run() {
        let mut backoff = RESTART_BACKOFF_INITIAL;
        let mut observed = Vec::new();
        for _ in 0..7 {
            backoff = next_restart_backoff(backoff, Duration::from_secs(1));
            observed.push(backoff.as_secs());
        }
        assert_eq!(observed, vec![2, 4, 8, 16, 30, 30, 30]);
        assert_eq!(
            next_restart_backoff(RESTART_BACKOFF_MAX, Duration::from_secs(61)),
            RESTART_BACKOFF_INITIAL
        );
    }

    #[test]
    fn socket_path_longer_than_the_portable_limit_is_rejected() {
        let fits = PathBuf::from("/").join("a".repeat(MAX_UNIX_SOCKET_PATH_BYTES - 1));
        assert!(ensure_socket_path_fits(&fits).is_ok());

        let too_long = PathBuf::from("/").join("a".repeat(MAX_UNIX_SOCKET_PATH_BYTES));
        let error = ensure_socket_path_fits(&too_long).unwrap_err().to_string();
        assert!(error.contains("Set AKRA_HOME to a shorter path"), "{error}");
    }

    /// 남은 upstream 대신 같은 명령줄 모양의 셸을 띄운다 (`exec` 하지 않아 명령줄이 남는다). 그룹 리더로 띄워 Gateway 와 같은 조건을 만든다.
    #[cfg(unix)]
    fn spawn_fake_upstream(command_line_tail: &str) -> std::process::Child {
        use std::os::unix::process::CommandExt;
        std::process::Command::new("/bin/sh")
            .args([
                "-c",
                "sleep 30; :",
                "sh",
                "app-server",
                "--listen",
                command_line_tail,
            ])
            .process_group(0)
            .spawn()
            .expect("fake upstream should start")
    }

    #[cfg(unix)]
    fn unique_run_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "akra-gateway-upstream-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).expect("run dir");
        path
    }

    #[cfg(unix)]
    fn wait_for_exit(child: &mut std::process::Child) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if child.try_wait().expect("try_wait should work").is_some() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[cfg(unix)]
    #[test]
    fn a_recorded_upstream_for_this_socket_is_stopped_and_forgotten() {
        let run_dir = unique_run_dir();
        let socket_path = run_dir.join("codex-app-server.sock");
        let mut stale =
            spawn_fake_upstream(&format!("unix://{}", codex_socket_argument(&socket_path)));
        std::fs::write(upstream_pid_path(&socket_path), stale.id().to_string()).expect("pid file");
        // 셸이 시작해 명령줄이 보일 때까지 기다린다.
        std::thread::sleep(Duration::from_millis(200));

        stop_stale_upstream(&socket_path);

        assert!(wait_for_exit(&mut stale), "the stale upstream must stop");
        assert!(!upstream_pid_path(&socket_path).exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_recorded_pid_of_another_program_is_left_alone() {
        let run_dir = unique_run_dir();
        let socket_path = run_dir.join("codex-app-server.sock");
        let mut other = spawn_fake_upstream("unix:///somewhere/else.sock");
        std::fs::write(upstream_pid_path(&socket_path), other.id().to_string()).expect("pid file");
        std::thread::sleep(Duration::from_millis(200));

        stop_stale_upstream(&socket_path);

        assert!(
            other.try_wait().expect("try_wait should work").is_none(),
            "a reused pid must not be killed"
        );
        let _ = other.kill();
        let _ = other.wait();
        assert!(!upstream_pid_path(&socket_path).exists());
    }
}
