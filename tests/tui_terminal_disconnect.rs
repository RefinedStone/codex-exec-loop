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
fn native_tui_exits_after_its_controlling_pty_disconnects() {
    let fixture = IsolatedTuiFixture::new();
    let (mut master, slave) = open_pty(100, 30).expect("PTY fixture should open");
    set_nonblocking(&master).expect("PTY master should become nonblocking");

    let mut child = spawn_native_tui(&fixture, &slave);
    drop(slave);

    wait_for_terminal_startup(&mut child, &mut master);
    drop(master);

    let (status, stderr) = child
        .wait_for_exit(DISCONNECT_TIMEOUT)
        .expect("native TUI should exit after its controlling PTY disconnects");
    assert!(
        status.success(),
        "native TUI should shut down cleanly after PTY disconnect: {status}; stderr: {stderr}"
    );
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
        .env("PATH", "/usr/bin:/bin")
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

fn wait_for_terminal_startup(child: &mut ChildGuard, master: &mut File) {
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
        if output
            .windows(b"Akra".len())
            .any(|window| window == b"Akra")
        {
            // Visible Akra copy proves that the first terminal transaction completed. Give the
            // frontend enough time to enter its blocking event poll before disconnecting the PTY.
            thread::sleep(Duration::from_millis(500));
            assert!(
                child.try_wait().is_none(),
                "native TUI exited before PTY disconnect; output: {}",
                String::from_utf8_lossy(&output)
            );
            return;
        }
        assert!(
            Instant::now() < deadline,
            "native TUI did not initialize its terminal before timeout; output: {}",
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
    workspace: PathBuf,
    home: PathBuf,
    akra_home: PathBuf,
    codex_home: PathBuf,
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
        for directory in [&root, &workspace, &home, &akra_home, &codex_home] {
            create_private_directory(directory);
        }
        Self {
            root,
            workspace,
            home,
            akra_home,
            codex_home,
        }
    }
}

impl Drop for IsolatedTuiFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
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
