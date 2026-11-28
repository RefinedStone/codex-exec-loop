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
fn native_tui_fails_closed_if_gateway_cannot_start() {
    let fixture = IsolatedTuiFixture::new().with_short_akra_home();
    let (mut master, slave) = open_pty(100, 30).expect("PTY fixture should open");
    set_nonblocking(&master).expect("PTY master should become nonblocking");

    let mut child = spawn_native_tui(&fixture, &slave, &fixture.root.join("missing-gateway"));
    drop(slave);

    let (status, stderr) = child
        .wait_for_exit(DISCONNECT_TIMEOUT)
        .expect("native TUI should fail before entering fullscreen");
    assert!(
        !status.success() && stderr.contains("start the Akra Gateway"),
        "Gateway startup failure must not fall back to a local writer: {status}; stderr: {stderr}"
    );
    let mut output = Vec::new();
    read_terminal_output(&mut master, &mut output);
    assert!(!output.windows(8).any(|bytes| bytes == b"\x1b[?1049h"));
    assert!(fixture.app_server_pids().is_empty());
}

/// plan 18 K6-1: with a Gateway running, the TUI opens its conversation through the Gateway relay. It does
/// not start its own app-server, and the relay ends with the TUI.
#[test]
fn native_tui_opens_its_conversation_through_a_running_gateway() {
    let fixture = IsolatedTuiFixture::new().with_short_akra_home();
    let gateway = FakeGateway::start(&fixture.akra_home.join("run"), &fixture.workspace);
    let (mut master, slave) = open_pty(100, 30).expect("PTY fixture should open");
    set_nonblocking(&master).expect("PTY master should become nonblocking");

    let mut child = spawn_native_tui(&fixture, &slave, &fixture.root.join("missing-gateway"));
    drop(slave);

    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let mut output = Vec::new();
    let mut opened_sessions = false;
    let mut terminal = vt100::Parser::new(30, 100, 0);
    let mut opened_history = false;
    let mut started_thread = false;
    let mut requested_again = false;
    let mut paused = false;
    let mut attempted_offline = false;
    let mut resumed = false;
    loop {
        let before = output.len();
        read_terminal_output(&mut master, &mut output);
        terminal.process(&output[before..]);
        assert!(
            child.try_wait().is_none(),
            "native TUI exited before terminal startup; output: {}",
            String::from_utf8_lossy(&output)
        );
        let terminal_started = output
            .windows(b"Akra".len())
            .any(|window| window == b"Akra");
        let rendered = terminal.screen().contents();
        if terminal_started && !started_thread && rendered.contains("READY") {
            master.write_all(b"\x14").expect("Ctrl+T creates a session");
            started_thread = true;
            output.clear();
        } else if started_thread && !paused && rendered.contains("outcome unknown") {
            gateway.set_online(false);
            paused = true;
            output.clear();
        } else if paused && !attempted_offline && rendered.contains("BLOCKED") {
            master
                .write_all(b"\x14")
                .expect("try creation during outage");
            attempted_offline = true;
            output.clear();
        } else if attempted_offline && !resumed && rendered.contains("nothing was sent") {
            assert_eq!(
                gateway
                    .methods()
                    .iter()
                    .filter(|method| *method == "thread/start")
                    .count(),
                1
            );
            gateway.set_online(true);
            resumed = true;
            output.clear();
        } else if resumed && !requested_again && rendered.contains("READY") {
            master
                .write_all(b"\x14")
                .expect("try creation again after reconnect");
            requested_again = true;
            output.clear();
        } else if requested_again && !opened_sessions && rendered.contains("Creation still unknown")
        {
            master
                .write_all(b"\x0f")
                .expect("Ctrl+O opens session list");
            opened_sessions = true;
            output.clear();
        } else if opened_sessions && !opened_history && rendered.contains("Gateway PTY session") {
            assert!(!rendered.contains("Foreign recovery decoy"));
            assert!(
                !gateway
                    .methods()
                    .iter()
                    .any(|method| method == "thread/read")
            );
            master.write_all(b"\r").expect("open the selected session");
            opened_history = true;
        } else if rendered.contains("gateway-pty-history") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Gateway recovery stalled (started: {started_thread}, paused: {paused}, offline attempt: {attempted_offline}, resumed: {resumed}, repeated: {requested_again}, sessions: {opened_sessions}, selection: {opened_history}); Gateway methods: {:?}; own app-server methods: {:?}; screen: {}",
            gateway.methods(),
            fixture.logged_app_server_methods(),
            terminal.screen().contents()
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        fixture.app_server_pids().is_empty(),
        "the TUI started its own app-server although the Gateway runs"
    );
    assert!(
        gateway
            .methods()
            .iter()
            .any(|method| method == "thread/read")
    );
    assert_eq!(
        gateway
            .methods()
            .iter()
            .filter(|method| *method == "thread/start")
            .count(),
        1
    );
    assert!(
        gateway
            .methods()
            .iter()
            .any(|method| method == "thread/list")
    );

    output.clear();
    master
        .write_all(b"fixture-only prompt\r")
        .expect("submit one prompt through the Gateway");
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let mut repeated_prompt = false;
    loop {
        let before = output.len();
        read_terminal_output(&mut master, &mut output);
        terminal.process(&output[before..]);
        let rendered = terminal.screen().contents();
        if repeated_prompt && rendered.contains("Unknown outcome: command not sent") {
            break;
        }
        if !repeated_prompt && rendered.contains("outcome unknown") {
            assert_eq!(gateway.prompt_commands().len(), 1);
            master.write_all(b"\r").expect("try the same draft again");
            repeated_prompt = true;
            output.clear();
        }
        assert!(
            Instant::now() < deadline,
            "the dropped mutation reply must reach the operator: {:?}",
            gateway.methods()
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        gateway.prompt_commands(),
        vec![serde_json::json!({
            "type": "submit_prompt",
            "thread_id": "gateway-pty",
            "prompt": "fixture-only prompt"
        })],
        "one operator submission reaches the fake kernel exactly once"
    );
    assert!(fixture.app_server_pids().is_empty());

    drop(master);
    let (status, stderr) = child
        .wait_for_exit(DISCONNECT_TIMEOUT)
        .expect("native TUI should exit after its controlling PTY disconnects");
    assert!(
        status.success(),
        "native TUI should exit cleanly: {status}; stderr: {stderr}"
    );
    let deadline = Instant::now() + DISCONNECT_TIMEOUT;
    while gateway.open_sessions() > 0 {
        assert!(
            Instant::now() < deadline,
            "the relay session outlived the TUI"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn read_terminal_output(master: &mut File, output: &mut Vec<u8>) {
    let mut buffer = [0_u8; 4096];
    match master.read(&mut buffer) {
        Ok(read) => {
            let before = count_cursor_queries(output);
            output.extend_from_slice(&buffer[..read]);
            for _ in before..count_cursor_queries(output) {
                master
                    .write_all(b"\x1b[1;1R")
                    .expect("PTY should accept a cursor-position response");
            }
        }
        Err(error)
            if error.kind() == io::ErrorKind::WouldBlock
                || error.raw_os_error() == Some(libc::EIO) => {}
        Err(error) => panic!("PTY startup output should remain readable: {error}"),
    }
}

fn count_cursor_queries(output: &[u8]) -> usize {
    output
        .windows(b"\x1b[6n".len())
        .filter(|window| *window == b"\x1b[6n")
        .count()
}

/// A fake Gateway on the client endpoint of `AKRA_HOME`. It answers the app-server calls of TUI startup
/// itself, as the real Gateway relays them to its upstream.
struct FakeGateway {
    state: std::sync::Arc<std::sync::Mutex<FakeGatewayState>>,
    online: tokio::sync::watch::Sender<bool>,
}

#[derive(Default)]
struct FakeGatewayState {
    methods: Vec<String>,
    prompt_commands: Vec<serde_json::Value>,
    open_sessions: usize,
}

impl FakeGateway {
    fn start(run_dir: &Path, workspace: &Path) -> Self {
        create_private_directory(run_dir);
        let socket = run_dir.join("gateway.sock");
        let workspace = workspace.to_string_lossy().into_owned();
        let state = std::sync::Arc::new(std::sync::Mutex::new(FakeGatewayState::default()));
        let served = state.clone();
        let (online, mut online_changes) = tokio::sync::watch::channel(true);
        let (ready, bound) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("fake Gateway runtime");
            runtime.block_on(async move {
                let listener =
                    tokio::net::UnixListener::bind(&socket).expect("fake Gateway should bind");
                ready.send(()).expect("test waits for the fake Gateway");
                loop {
                    let Ok((stream, _)) = listener.accept().await else {
                        return;
                    };
                    if !*online_changes.borrow_and_update() {
                        continue;
                    }
                    tokio::spawn(serve_gateway_session(
                        stream,
                        served.clone(),
                        workspace.clone(),
                        online_changes.clone(),
                    ));
                }
            });
        });
        bound.recv().expect("fake Gateway should start");
        Self { state, online }
    }

    fn set_online(&self, online: bool) {
        self.online.send(online).expect("fake listener stays alive");
    }

    fn methods(&self) -> Vec<String> {
        self.state.lock().unwrap().methods.clone()
    }

    fn open_sessions(&self) -> usize {
        self.state.lock().unwrap().open_sessions
    }

    fn prompt_commands(&self) -> Vec<serde_json::Value> {
        self.state.lock().unwrap().prompt_commands.clone()
    }
}

async fn serve_gateway_session(
    stream: tokio::net::UnixStream,
    state: std::sync::Arc<std::sync::Mutex<FakeGatewayState>>,
    workspace: String,
    mut online: tokio::sync::watch::Receiver<bool>,
) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    // The TUI's startup probe connects and leaves without a handshake.
    let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else {
        return;
    };
    state.lock().unwrap().open_sessions += 1;
    while let Some(Ok(message)) = tokio::select! {
        _ = online.changed() => None,
        message = socket.next() => message,
    } {
        let Message::Text(text) = message else {
            continue;
        };
        let request: serde_json::Value = serde_json::from_str(&text).expect("JSON request");
        let method = request["method"].as_str().unwrap_or_default().to_string();
        state.lock().unwrap().methods.push(method.clone());
        if request.get("id").is_none() {
            continue;
        }
        let reply = match method.as_str() {
            "akra/kernel/attach" => {
                serde_json::json!({"id":request["id"],"result":{"attached":true}})
            }
            "akra/kernel/command" => {
                let command = &request["params"]["command"];
                if command["type"] == "submit_prompt" {
                    assert!(request["params"]["idempotency_key"].as_str().is_some());
                    state.lock().unwrap().prompt_commands.push(command.clone());
                    // Simulate the write reaching the kernel, then lose the reply.
                    break;
                }
                serde_json::json!({"id":request["id"],"result":{"processed":true}})
            }
            "akra/kernel/query" => {
                let result = match request["params"]["query"]["type"].as_str() {
                    Some("sessions") => {
                        serde_json::json!({"type":"sessions","status":"ready","sessions":[
                            {"thread_id":"gateway-pty","title":"Gateway PTY session","workspace":workspace,
                             "updated_at_epoch":1700000000}
                        ]})
                    }
                    Some("auto_follow") => {
                        serde_json::json!({"type":"auto_follow","max_turns":5,"paused":false})
                    }
                    Some("thread_activity") => {
                        serde_json::json!({"type":"thread_activity","threads":[]})
                    }
                    _ => panic!("unexpected PTY kernel query: {request}"),
                };
                serde_json::json!({"id":request["id"],"result":result})
            }
            "initialize" => serde_json::json!({"id": request["id"], "result": {
                "userAgent": "codex-app-server/gateway-relay-fixture",
                "platformFamily": "unix",
                "platformOs": std::env::consts::OS,
            }}),
            "account/read" => serde_json::json!({"id": request["id"], "result": {
                "account": {"type": "chatgpt", "email": "relay-fixture@example.com", "planType": "test"},
                "requiresOpenAIAuth": false,
            }}),
            "thread/list" => {
                assert_eq!(request["params"]["cwd"], workspace);
                serde_json::json!({"id": request["id"], "result": {"data": [{
                    "id":"gateway-pty","name":"Gateway PTY session","cwd":workspace,
                    "preview":"","source":"cli","modelProvider":"openai","updatedAt":1700000000,
                    "path":null,"status":{"type":"idle"},"gitInfo":null
                }, {
                    "id":"foreign-pty","name":"Foreign recovery decoy","cwd":"/other-workspace",
                    "preview":"","source":"cli","modelProvider":"openai","updatedAt":1700000001,
                    "path":null,"status":{"type":"idle"},"gitInfo":null
                }], "nextCursor": null}})
            }
            "thread/start" => break,
            "thread/read" => {
                assert_eq!(request["params"]["threadId"], "gateway-pty");
                serde_json::json!({"id":request["id"],"result":{"thread":{
                "id":"gateway-pty","name":"Gateway PTY session","cwd":workspace,
                "preview":"","source":"cli","modelProvider":"openai","updatedAt":1700000000,
                "path":null,"status":{"type":"idle"},"gitInfo":null,"turns":[{
                    "id":"saved-turn","status":"completed","items":[
                        {"type":"agentMessage","id":"saved-item","text":"gateway-pty-history"}
                    ]
                }]
                }}})
            }
            other => serde_json::json!({"id": request["id"], "error": {
                "message": format!("unexpected fixture method {other}"),
            }}),
        };
        if socket.send(Message::text(reply.to_string())).await.is_err() {
            break;
        }
    }
    state.lock().unwrap().open_sessions -= 1;
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

fn spawn_native_tui(fixture: &IsolatedTuiFixture, slave: &File, gateway: &Path) -> ChildGuard {
    let stdin = slave.try_clone().expect("PTY stdin should clone");
    let stdout = slave.try_clone().expect("PTY stdout should clone");
    let mut command = Command::new(env!("CARGO_BIN_EXE_akra-ade"));
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
        .env("AKRA_GATEWAY_BIN", gateway)
        .env("CODEX_HOME", &fixture.codex_home)
        .env("AKRA_APP_SERVER_PROMPT_LOG", "0")
        .env("AKRA_SHOW_STARTUP_VISUAL", "0")
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
    short_akra_home: Option<PathBuf>,
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
            short_akra_home: None,
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

    /// The Gateway socket lives under `AKRA_HOME`. A unix socket path has at most 103 bytes on macOS, and
    /// the system temp folder there is long, so this home sits under `/tmp`.
    fn with_short_akra_home(mut self) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after the epoch")
            .subsec_nanos();
        let short = PathBuf::from(format!("/tmp/akra-k61-{}-{nonce}", std::process::id()));
        create_private_directory(&short);
        self.akra_home = short.clone();
        self.short_akra_home = Some(short);
        self
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
        if let Some(short_akra_home) = &self.short_akra_home {
            let _ = fs::remove_dir_all(short_akra_home);
        }
        let _ = fs::remove_dir_all(&self.trusted_launcher_root);
    }
}

fn install_fake_codex(fake_bin: &Path, pid_log: &Path, request_log: &Path) {
    let node = find_host_node();
    let installed_node = fake_bin.join("node");
    // A hosted tool-cache ancestor may be world-writable even when Node itself is safe. Copy the
    // executable into the private fixture instead of symlinking across that untrusted boundary.
    fs::copy(&node, &installed_node).expect("trusted Node fixture should copy");
    copy_adjacent_node_runtime_libraries(&node, fake_bin);

    let launcher = fake_bin.join("codex");
    fs::write(&launcher, fake_codex_script(pid_log, request_log))
        .expect("fake Codex launcher should write");
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&installed_node, fs::Permissions::from_mode(0o700))
        .expect("trusted Node fixture should become private and executable");
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o700))
        .expect("fake Codex launcher should become executable");
    assert!(
        Command::new(&installed_node)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success()),
        "private Node fixture and its adjacent runtime libraries should be executable"
    );
}

fn copy_adjacent_node_runtime_libraries(node: &Path, fake_bin: &Path) {
    // Homebrew Node 26 links libnode through @rpath/../lib. Copying only the executable
    // therefore aborts in dyld after relocation. Standalone Node builds have no adjacent
    // libnode file and need no extra fixture setup.
    let Some(source_lib) = node
        .parent()
        .and_then(Path::parent)
        .map(|root| root.join("lib"))
        .filter(|path| path.is_dir())
    else {
        return;
    };
    let runtime_libraries = fs::read_dir(&source_lib)
        .expect("adjacent Node runtime library directory should be readable")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("libnode."))
        })
        .collect::<Vec<_>>();
    if runtime_libraries.is_empty() {
        return;
    }

    let private_lib = fake_bin
        .parent()
        .expect("fake bin should have a trusted launcher root")
        .join("lib");
    create_private_directory(&private_lib);
    for library in runtime_libraries {
        let destination = private_lib.join(
            library
                .file_name()
                .expect("Node runtime library should have a file name"),
        );
        fs::copy(&library, &destination).expect("Node runtime library fixture should copy");
    }
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
