#![cfg(unix)]

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(3);

#[test]
fn native_tui_and_app_server_exit_after_their_controlling_pty_disconnects() {
    let fixture = IsolatedTuiFixture::new();
    let (mut master, slave) = open_pty(100, 30).expect("PTY fixture should open");
    set_nonblocking(&master).expect("PTY master should become nonblocking");

    let mut child = spawn_native_tui(&fixture, &slave);
    drop(slave);

    let app_server_pids = wait_for_terminal_startup(&fixture, &mut child, &mut master);
    drop(master);

    let (status, stderr) = child
        .wait_for_exit(DISCONNECT_TIMEOUT)
        .expect("native TUI should exit after its controlling PTY disconnects");
    assert!(
        status.success(),
        "native TUI should shut down cleanly after PTY disconnect: {status}; stderr: {stderr}"
    );
    fixture.wait_for_app_servers_to_exit(&app_server_pids, DISCONNECT_TIMEOUT);
}

fn open_pty(columns: u16, rows: u16) -> io::Result<(File, File)> {
    let mut master_fd: RawFd = -1;
    let mut slave_fd: RawFd = -1;
    let mut window_size = libc::winsize {
        ws_row: rows,
        ws_col: columns,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let window_size_ptr = std::ptr::addr_of_mut!(window_size);
    // SAFETY: openpty initializes both descriptors on success. Null termios requests defaults,
    // and window_size_ptr remains valid for the duration of the call.
    let result = unsafe {
        libc::openpty(
            &mut master_fd,
            &mut slave_fd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            window_size_ptr,
        )
    };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: openpty returned two newly owned descriptors.
    let master = unsafe { File::from_raw_fd(master_fd) };
    // SAFETY: openpty returned two newly owned descriptors.
    let slave = unsafe { File::from_raw_fd(slave_fd) };
    set_close_on_exec(&master)?;
    set_close_on_exec(&slave)?;
    Ok((master, slave))
}

fn set_close_on_exec(file: &File) -> io::Result<()> {
    let fd = std::os::fd::AsRawFd::as_raw_fd(file);
    // SAFETY: fd belongs to a live File and F_GETFD does not mutate memory.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd remains live and F_SETFD receives the existing flags plus FD_CLOEXEC.
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn set_nonblocking(file: &File) -> io::Result<()> {
    let fd = std::os::fd::AsRawFd::as_raw_fd(file);
    // SAFETY: fd belongs to a live File and F_GETFL does not mutate memory.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd remains live and F_SETFL receives the existing flags plus O_NONBLOCK.
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn spawn_native_tui(fixture: &IsolatedTuiFixture, slave: &File) -> ChildGuard {
    let stdin = slave.try_clone().expect("PTY stdin should clone");
    let stdout = slave.try_clone().expect("PTY stdout should clone");
    let mut command = Command::new(env!("CARGO_BIN_EXE_codex-exec-loop-native"));
    command
        .current_dir(&fixture.workspace)
        .env_clear()
        .env("HOME", &fixture.home)
        .env("USERPROFILE", &fixture.home)
        .env("USER", "akra-pty-test")
        .env("LOGNAME", "akra-pty-test")
        .env("SHELL", "/bin/sh")
        .env("PATH", fixture.process_path())
        .env("LANG", "C.UTF-8")
        .env("LC_ALL", "C.UTF-8")
        .env("TERM", "xterm-256color")
        .env("AKRA_HOME", &fixture.akra_home)
        .env("CODEX_HOME", &fixture.codex_home)
        .env("AKRA_APP_SERVER_PROMPT_LOG", "0")
        .env("CODEX_EXEC_LOOP_SHOW_STARTUP_ASCII_ART", "0")
        .stdin(Stdio::from(stdin))
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::piped());

    // SAFETY: pre_exec uses only async-signal-safe libc calls. The child becomes a session leader
    // and claims its PTY slave as the controlling terminal before exec.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            if libc::ioctl(libc::STDIN_FILENO, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }

    ChildGuard::new(command.spawn().expect("native TUI fixture should spawn"))
}

fn wait_for_terminal_startup(
    fixture: &IsolatedTuiFixture,
    child: &mut ChildGuard,
    master: &mut File,
) -> Vec<libc::pid_t> {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let mut output = Vec::new();
    let mut cursor_queries_answered = 0;
    let mut buffer = [0_u8; 4096];
    loop {
        match master.read(&mut buffer) {
            Ok(0) => {}
            Ok(read) => {
                output.extend_from_slice(&buffer[..read]);
                let cursor_queries_seen = output
                    .windows(b"\x1b[6n".len())
                    .filter(|window| *window == b"\x1b[6n")
                    .count();
                while cursor_queries_answered < cursor_queries_seen {
                    master
                        .write_all(b"\x1b[1;1R")
                        .expect("PTY should accept a cursor-position response");
                    cursor_queries_answered += 1;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => panic!("PTY startup output should remain readable: {error}"),
        }

        assert!(
            child.try_wait().is_none(),
            "native TUI exited before terminal startup; output: {}",
            String::from_utf8_lossy(&output)
        );
        let terminal_started = output
            .windows(b"Akra".len())
            .any(|window| window == b"Akra");
        let app_server_pids = fixture.live_initialized_app_server_pids();
        if terminal_started && !app_server_pids.is_empty() {
            // Visible Akra copy proves that the first terminal transaction completed, while the
            // live initialized child proves startup reached the production app-server boundary.
            // Give the frontend enough time to enter its blocking event poll before disconnecting.
            thread::sleep(Duration::from_millis(500));
            assert!(
                child.try_wait().is_none(),
                "native TUI exited before PTY disconnect; output: {}",
                String::from_utf8_lossy(&output)
            );
            let app_server_pids = fixture.live_initialized_app_server_pids();
            assert!(
                !app_server_pids.is_empty(),
                "initialized app-server exited before PTY disconnect"
            );
            return app_server_pids;
        }
        assert!(
            Instant::now() < deadline,
            "native TUI and app-server did not finish startup before timeout; methods: {:?}; output: {}",
            fixture.logged_app_server_methods(),
            String::from_utf8_lossy(&output)
        );
        thread::sleep(Duration::from_millis(10));
    }
}

struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn try_wait(&mut self) -> Option<ExitStatus> {
        self.child
            .as_mut()
            .expect("child should remain owned")
            .try_wait()
            .expect("child state should remain readable")
    }

    fn wait_for_exit(&mut self, timeout: Duration) -> Option<(ExitStatus, String)> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.try_wait().is_some() {
                let mut child = self.child.take().expect("exited child should remain owned");
                let status = child
                    .wait()
                    .expect("exited child status should remain readable");
                let mut stderr = String::new();
                if let Some(mut stream) = child.stderr.take() {
                    let _ = stream.read_to_string(&mut stderr);
                }
                return Some((status, stderr));
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        if child.try_wait().ok().flatten().is_none() {
            let process_group =
                libc::pid_t::try_from(child.id()).expect("child PID should fit pid_t");
            // SAFETY: pre_exec made the child its own session and process-group leader. A negative
            // PID therefore targets only the isolated test process group.
            let _ = unsafe { libc::kill(-process_group, libc::SIGKILL) };
        }
        let _ = child.wait();
    }
}

struct IsolatedTuiFixture {
    root: PathBuf,
    trusted_launcher_root: PathBuf,
    fake_bin: PathBuf,
    workspace: PathBuf,
    home: PathBuf,
    akra_home: PathBuf,
    codex_home: PathBuf,
    app_server_pid_log: PathBuf,
    app_server_request_log: PathBuf,
}

impl IsolatedTuiFixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after the epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "akra-tui-terminal-disconnect-{}-{nonce}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        let home = root.join("home");
        let akra_home = root.join("akra-home");
        let codex_home = root.join("codex-home");
        let app_server_pid_log = root.join("app-server-pids");
        let app_server_request_log = root.join("app-server-requests.jsonl");
        for directory in [&root, &workspace, &home, &akra_home, &codex_home] {
            create_private_directory(directory);
        }

        // Production rejects repository- and system-temp-controlled launchers. Install the fake
        // under the current user's private home so this fixture traverses the same trusted
        // executable pinning path as a real Codex installation.
        let trusted_home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .expect("test host HOME should be available");
        let trusted_launcher_root = trusted_home.join(format!(
            ".akra-tui-terminal-disconnect-{}-{nonce}",
            std::process::id()
        ));
        let fake_bin = trusted_launcher_root.join("bin");
        create_private_directory(&trusted_launcher_root);
        create_private_directory(&fake_bin);
        install_fake_codex(&fake_bin, &app_server_pid_log, &app_server_request_log);

        Self {
            root,
            trusted_launcher_root,
            fake_bin,
            workspace,
            home,
            akra_home,
            codex_home,
            app_server_pid_log,
            app_server_request_log,
        }
    }

    fn process_path(&self) -> std::ffi::OsString {
        std::env::join_paths([
            self.fake_bin.as_path(),
            Path::new("/usr/bin"),
            Path::new("/bin"),
        ])
        .expect("fake app-server PATH should join")
    }

    fn logged_app_server_methods(&self) -> Vec<String> {
        fs::read_to_string(&self.app_server_request_log)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter_map(|request| request["method"].as_str().map(str::to_string))
            .collect()
    }

    fn app_server_pids(&self) -> Vec<libc::pid_t> {
        fs::read_to_string(&self.app_server_pid_log)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.parse().ok())
            .collect()
    }

    fn live_initialized_app_server_pids(&self) -> Vec<libc::pid_t> {
        if !self
            .logged_app_server_methods()
            .iter()
            .any(|method| method == "account/read")
        {
            return Vec::new();
        }
        self.app_server_pids()
            .into_iter()
            .filter(|pid| process_is_alive(*pid))
            .collect()
    }

    fn wait_for_app_servers_to_exit(&self, expected: &[libc::pid_t], timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            let observed = self.app_server_pids();
            let live = observed
                .iter()
                .copied()
                .filter(|pid| process_is_alive(*pid))
                .collect::<Vec<_>>();
            if live.is_empty() {
                assert!(
                    expected.iter().all(|pid| observed.contains(pid)),
                    "app-server PID log lost a child observed before disconnect"
                );
                return;
            }
            assert!(
                Instant::now() < deadline,
                "app-server children remained alive after native TUI exit: {live:?}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for IsolatedTuiFixture {
    fn drop(&mut self) {
        // ChildGuard force-kills the native process on panic, but app-server intentionally owns a
        // separate process group. Contain that failure path explicitly before deleting its PID log.
        for pid in self
            .app_server_pids()
            .into_iter()
            .filter(|pid| process_is_alive(*pid))
        {
            // SAFETY: production subprocess containment makes the app-server PID its process-group
            // leader. The direct-PID fallback also covers an exec failure before group setup.
            if unsafe { libc::kill(-pid, libc::SIGKILL) } == -1 {
                let _ = unsafe { libc::kill(pid, libc::SIGKILL) };
            }
        }
        let _ = fs::remove_dir_all(&self.root);
        let _ = fs::remove_dir_all(&self.trusted_launcher_root);
    }
}

fn install_fake_codex(fake_bin: &Path, pid_log: &Path, request_log: &Path) {
    let node = find_host_node();
    let installed_node = fake_bin.join("node");
    // A hosted tool-cache ancestor may be world-writable even when Node itself is safe. Copy the
    // executable into the private fixture instead of symlinking across that untrusted boundary.
    fs::copy(&node, &installed_node).expect("trusted Node fixture should copy");

    let launcher = fake_bin.join("codex");
    fs::write(&launcher, fake_codex_script(pid_log, request_log))
        .expect("fake Codex launcher should write");
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&installed_node, fs::Permissions::from_mode(0o700))
        .expect("trusted Node fixture should become private and executable");
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o700))
        .expect("fake Codex launcher should become executable");
}

fn find_host_node() -> PathBuf {
    let path = std::env::var_os("PATH").expect("test host PATH should be available");
    for directory in std::env::split_paths(&path).filter(|directory| directory.is_absolute()) {
        for name in ["node", "nodejs"] {
            let candidate = directory.join(name);
            if !candidate.is_file() {
                continue;
            }
            if Command::new(&candidate)
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
            {
                return fs::canonicalize(&candidate).unwrap_or(candidate);
            }
        }
    }
    panic!("terminal lifecycle test requires a host Node executable");
}

fn fake_codex_script(pid_log: &Path, request_log: &Path) -> String {
    let pid_log = serde_json::to_string(
        pid_log
            .to_str()
            .expect("fake app-server PID log path should be UTF-8"),
    )
    .expect("fake app-server PID log path should serialize");
    let request_log = serde_json::to_string(
        request_log
            .to_str()
            .expect("fake app-server request log path should be UTF-8"),
    )
    .expect("fake app-server request log path should serialize");
    r#"#!/usr/bin/env node
const fs = require("node:fs");
const readline = require("node:readline");

const pidLog = __PID_LOG__;
const requestLog = __REQUEST_LOG__;
fs.appendFileSync(pidLog, `${process.pid}\n`, { encoding: "utf8", mode: 0o600 });

function send(value) {
  process.stdout.write(`${JSON.stringify(value)}\n`);
}

const input = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
input.on("line", (line) => {
  const request = JSON.parse(line);
  fs.appendFileSync(requestLog, `${JSON.stringify(request)}\n`, {
    encoding: "utf8",
    mode: 0o600,
  });
  if (!Object.prototype.hasOwnProperty.call(request, "id")) {
    return;
  }

  if (request.method === "initialize") {
    send({
      id: request.id,
      result: {
        userAgent: "codex-app-server/pty-lifecycle-fixture",
        platformFamily: "unix",
        platformOs: process.platform,
      },
    });
  } else if (request.method === "account/read") {
    send({
      id: request.id,
      result: {
        account: {
          type: "chatgpt",
          email: "pty-fixture@example.com",
          planType: "test",
        },
        requiresOpenAIAuth: false,
      },
    });
  } else if (request.method === "thread/list") {
    send({ id: request.id, result: { data: [], nextCursor: null } });
  } else {
    send({
      id: request.id,
      error: { message: `unexpected fixture method ${request.method}` },
    });
  }
});
"#
    .replace("__PID_LOG__", &pid_log)
    .replace("__REQUEST_LOG__", &request_log)
}

fn process_is_alive(pid: libc::pid_t) -> bool {
    // SAFETY: signal zero performs a liveness/permission check without delivering a signal.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

fn create_private_directory(path: &Path) {
    fs::create_dir(path).unwrap_or_else(|error| {
        panic!(
            "private fixture directory should create at {}: {error}",
            path.display()
        )
    });
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap_or_else(|error| {
        panic!(
            "private fixture directory permissions should set at {}: {error}",
            path.display()
        )
    });
}
