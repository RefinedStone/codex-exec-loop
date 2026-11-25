//! Deliberately narrow native Linux sandbox. There is no PATH lookup, sudo,
//! WSL, shared namespace, privileged bubblewrap or unsandboxed fallback.
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::application::port::outbound::issue_code_validator_runner_port::{
    IssueCodeValidatorNotRun as NotRun, IssueCodeValidatorRunOutcome as Outcome,
    IssueCodeValidatorRunRequest,
};

const BWRAP: &str = "/usr/bin/bwrap";
const MAX_VALIDATOR: u64 = 8 * 1024 * 1024;
const MESSAGE_LIMIT: usize = 4096;
// Only the fixed shell, AFTER namespace setup, can issue this first frame.
// A bubblewrap setup error may also exit with 1; it must not be treated as
// a validator's rejection. Keep stdout bounded including this frame.
const START_FRAME: &[u8] = b"AKRA_VALIDATOR_STARTED_V1\n";
const SHELL_ENTRY: &str = "printf 'AKRA_VALIDATOR_STARTED_V1\\n'; exec /validator";
const SYSTEM_DIRS: &[&str] = &[
    "/usr/bin",
    "/usr/lib",
    "/usr/lib64",
    "/bin",
    "/lib",
    "/lib64",
];

fn system_owned(path: &Path, is_file: bool) -> bool {
    let Ok(canonical) = path.canonicalize() else {
        return false;
    };
    canonical.ancestors().all(|part| {
        fs::symlink_metadata(part).is_ok_and(|metadata| {
            metadata.uid() == 0
                && metadata.mode() & 0o022 == 0
                && if part == canonical && is_file {
                    metadata.is_file() && metadata.mode() & 0o6000 == 0
                } else {
                    metadata.is_dir()
                }
        })
    })
}

fn supported_host() -> bool {
    // WSL's Windows interoperability and container-specific bind mounts are
    // outside this profile. A missing/unreadable release is not proof of support.
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } == 0 {
        return false;
    }
    fs::read_to_string("/proc/sys/kernel/osrelease")
        .is_ok_and(|release| !release.to_ascii_lowercase().contains("microsoft"))
}

fn program() -> Option<PathBuf> {
    if !supported_host() || !system_owned(Path::new(BWRAP), true) {
        return None;
    }
    let cwd = std::env::current_dir().ok()?;
    let program = crate::trusted_executable::validate_absolute(Path::new(BWRAP), &cwd).ok()?;
    crate::trusted_executable::validate_native_executable(&program).ok()?;
    Some(program)
}

struct Workspace(PathBuf);
impl Workspace {
    fn create() -> io::Result<Self> {
        // Use fixed /tmp, not a configurable project/user-supplied temp path.
        let path = Path::new("/tmp").join(format!(
            "akra-validator-{}-{:032x}",
            std::process::id(),
            rand::random::<u128>()
        ));
        DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self(path))
    }
    fn stage(&self, content: &[u8], input: &str) -> io::Result<(PathBuf, File)> {
        let validator = self.0.join("validator");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o500)
            .open(&validator)?
            .write_all(content)?;
        let input_path = self.0.join("input.json");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o400)
            .open(&input_path)?
            .write_all(input.as_bytes())?;
        Ok((validator, File::open(input_path)?))
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn approved_content(request: &IssueCodeValidatorRunRequest) -> Result<Vec<u8>, NotRun> {
    if !Path::new(&request.command).is_absolute() {
        return Err(NotRun::Missing);
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(&request.command)
        .map_err(|_| NotRun::Missing)?;
    let metadata = file.metadata().map_err(|_| NotRun::Missing)?;
    if !metadata.is_file() || metadata.len() > MAX_VALIDATOR {
        return Err(NotRun::Missing);
    }
    let mut content = Vec::new();
    file.take(MAX_VALIDATOR + 1)
        .read_to_end(&mut content)
        .map_err(|_| NotRun::Missing)?;
    if content.len() as u64 > MAX_VALIDATOR
        || format!("{:x}", Sha256::digest(&content)) != request.sha256
    {
        return Err(NotRun::HashMismatch);
    }
    Ok(content)
}

fn sandbox_command(program: &Path, validator: &Path) -> io::Result<Command> {
    let mut command = Command::new(program);
    // Do not use --unshare-all: it can downgrade user/cgroup namespace creation.
    command.args([
        "--unshare-user",
        "--disable-userns",
        "--assert-userns-disabled",
        "--unshare-pid",
        "--unshare-net",
        "--unshare-ipc",
        "--unshare-uts",
        "--unshare-cgroup",
        "--cap-drop",
        "ALL",
        "--die-with-parent",
        "--new-session",
        "--clearenv",
        "--setenv",
        "PATH",
        "/usr/bin:/bin",
        "--setenv",
        "TMPDIR",
        "/tmp",
    ]);
    // No host root, /etc, home, /usr/local, /mnt, /run, /sys or /proc mount.
    // This profile trusts the administrator-installed system executables/libs,
    // not project-specific runtimes or user-installed interpreters.
    for dir in SYSTEM_DIRS {
        let path = Path::new(dir);
        match fs::symlink_metadata(path) {
            Ok(_) if system_owned(path, false) => {
                command.args(["--ro-bind", dir, dir]);
            }
            Ok(_) => return Err(io::Error::other("untrusted system directory")),
            Err(error)
                if error.kind() == io::ErrorKind::NotFound
                    && matches!(*dir, "/lib64" | "/usr/lib64") => {}
            Err(error) => return Err(error),
        }
    }
    mount_workspace(&mut command, validator);
    command
        .env_clear()
        .current_dir("/")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // SAFETY: only async-signal-safe libc calls; no allocation or locks after fork.
    unsafe {
        command.pre_exec(|| {
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                return Err(io::Error::last_os_error());
            }
            for (resource, maximum) in [
                (libc::RLIMIT_AS, 512 * 1024 * 1024),
                (libc::RLIMIT_NPROC, 64),
                (libc::RLIMIT_FSIZE, 16 * 1024 * 1024),
                (libc::RLIMIT_NOFILE, 128),
                (libc::RLIMIT_CORE, 0),
                (libc::RLIMIT_CPU, 120),
            ] {
                let mut old = libc::rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                };
                if libc::getrlimit(resource, &mut old) != 0 {
                    return Err(io::Error::last_os_error());
                }
                let limit = maximum.min(old.rlim_cur).min(old.rlim_max);
                let value = libc::rlimit {
                    rlim_cur: limit,
                    rlim_max: limit,
                };
                if libc::setrlimit(resource, &value) != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    Ok(command)
}

fn mount_workspace(command: &mut Command, validator: &Path) {
    command
        .arg("--ro-bind")
        .arg(validator)
        .arg("/validator")
        .args([
            "--dev",
            "/dev",
            // --dev creates its own tmpfs, including /dev/shm. The root
            // remount below is non-recursive; freeze this mount explicitly.
            // Device bind mounts remain usable, but ordinary files cannot be made.
            "--remount-ro",
            "/dev",
            "--size",
            "16777216",
            "--tmpfs",
            "/tmp",
            "--remount-ro",
            "/",
            "--chdir",
            "/tmp",
            "--",
            "/bin/sh",
            "-c",
            SHELL_ENTRY,
        ]);
}

pub(super) fn run(request: &IssueCodeValidatorRunRequest) -> Outcome {
    let Some(program) = program() else {
        return Outcome::NotRun(NotRun::IsolationUnavailable);
    };
    let content = match approved_content(request) {
        Ok(content) => content,
        Err(reason) => return Outcome::NotRun(reason),
    };
    let Ok(workspace) = Workspace::create() else {
        return Outcome::NotRun(NotRun::Crashed);
    };
    let Ok((validator, input)) = workspace.stage(&content, &request.input_json) else {
        return Outcome::NotRun(NotRun::Crashed);
    };
    let Ok(mut command) = sandbox_command(&program, &validator) else {
        return Outcome::NotRun(NotRun::IsolationUnavailable);
    };
    command.stdin(input);
    collect(&mut command, request.timeout)
}

fn collect(command: &mut Command, timeout: Duration) -> Outcome {
    let Ok(mut child) = crate::subprocess::spawn(command) else {
        return Outcome::NotRun(NotRun::IsolationUnavailable);
    };
    let Some(mut output) = child.take_stdout() else {
        let _ = child.terminate_and_wait();
        return Outcome::NotRun(NotRun::Crashed);
    };
    let mut captured = Vec::new();
    let result = (|| -> io::Result<Option<std::process::ExitStatus>> {
        let fd = output.as_raw_fd();
        // SAFETY: fd is owned by the live ChildStdout.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let deadline = Instant::now() + timeout;
        loop {
            drain(&mut output, &mut captured)?;
            if let Some(status) = child.try_wait()? {
                return Ok(Some(status));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    })();
    // The sandbox has its own PID namespace/init; bwrap also ties its lifetime
    // to its parent. The common contained-child helper verifies host cleanup.
    if child.terminate_and_wait().is_err() {
        return Outcome::NotRun(NotRun::Crashed);
    }
    match result {
        Ok(Some(status)) if drain(&mut output, &mut captured).is_ok() => {
            classify(status.code(), &captured)
        }
        Ok(None) => Outcome::NotRun(NotRun::TimedOut),
        _ => Outcome::NotRun(NotRun::Crashed),
    }
}

fn drain(output: &mut impl Read, captured: &mut Vec<u8>) -> io::Result<()> {
    let mut buffer = [0u8; 8192];
    for _ in 0..8 {
        match output.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                let keep =
                    count.min((MESSAGE_LIMIT + START_FRAME.len()).saturating_sub(captured.len()));
                captured.extend_from_slice(&buffer[..keep]);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn classify(status: Option<i32>, output: &[u8]) -> Outcome {
    let Some(message) = output.strip_prefix(START_FRAME) else {
        return Outcome::NotRun(NotRun::IsolationUnavailable);
    };
    match status {
        Some(0) => Outcome::Passed,
        Some(1) => Outcome::Rejected {
            message: String::from_utf8_lossy(message).trim().to_string(),
        },
        _ => Outcome::NotRun(NotRun::Crashed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_filesystem_is_read_only_before_the_bounded_workspace_is_mounted() {
        let mut command = Command::new(BWRAP);
        mount_workspace(&mut command, Path::new("/staged-validator"));
        let arguments = command
            .get_args()
            .map(|argument| argument.to_str().unwrap())
            .collect::<Vec<_>>();
        let devices = arguments
            .windows(2)
            .position(|arguments| arguments == ["--dev", "/dev"])
            .unwrap();
        let read_only_devices = arguments
            .windows(2)
            .position(|arguments| arguments == ["--remount-ro", "/dev"])
            .unwrap();
        let workspace = arguments
            .windows(4)
            .position(|arguments| arguments == ["--size", "16777216", "--tmpfs", "/tmp"])
            .unwrap();
        let read_only_root = arguments
            .windows(2)
            .position(|arguments| arguments == ["--remount-ro", "/"])
            .unwrap();
        assert!(devices < read_only_devices);
        assert!(read_only_devices < workspace);
        assert!(workspace < read_only_root);
    }

    #[test]
    fn launcher_setup_failure_cannot_become_a_validator_pass_or_rejection() {
        for code in [Some(0), Some(1), Some(2), None] {
            assert_eq!(
                classify(code, b"setup failed"),
                Outcome::NotRun(NotRun::IsolationUnavailable)
            );
        }
        assert_eq!(classify(Some(0), START_FRAME), Outcome::Passed);
        assert_eq!(
            classify(Some(1), b"AKRA_VALIDATOR_STARTED_V1\nrejected"),
            Outcome::Rejected {
                message: "rejected".into()
            }
        );
    }

    #[test]
    fn approved_file_is_bounded_and_hash_checked_before_staging() {
        let workspace = Workspace::create().unwrap();
        let file = workspace.0.join("source");
        fs::write(&file, b"#!/bin/sh\nexit 0\n").unwrap();
        let mut request = IssueCodeValidatorRunRequest {
            command: file.to_string_lossy().into_owned(),
            sha256: "0".repeat(64),
            timeout: Duration::from_secs(5),
            input_json: "{}".into(),
        };
        assert_eq!(approved_content(&request), Err(NotRun::HashMismatch));
        request.sha256 = format!("{:x}", Sha256::digest(fs::read(&file).unwrap()));
        let approved = approved_content(&request).unwrap();
        fs::write(&file, b"changed").unwrap();
        let (staged, _) = workspace.stage(&approved, "{}").unwrap();
        assert_eq!(fs::read(staged).unwrap(), approved);
    }

    #[test]
    fn missing_or_unavailable_sandbox_never_falls_back_to_direct_execution() {
        if program().is_some() {
            return;
        }
        let workspace = Workspace::create().unwrap();
        let command = workspace.0.join("source");
        let marker = workspace.0.join("unsafe-marker");
        let script = format!("#!/bin/sh\ntouch '{}'\n", marker.display());
        fs::write(&command, &script).unwrap();
        let request = IssueCodeValidatorRunRequest {
            command: command.to_string_lossy().into_owned(),
            sha256: format!("{:x}", Sha256::digest(script)),
            timeout: Duration::from_secs(3),
            input_json: "{}".into(),
        };
        assert_eq!(run(&request), Outcome::NotRun(NotRun::IsolationUnavailable));
        assert!(!marker.exists());
    }

    // Required manual capability test on a native Linux machine with system
    // bubblewrap and user namespaces. Never silently skip it and claim support.
    #[test]
    #[ignore = "requires native Linux with bubblewrap and unprivileged user namespaces"]
    fn native_profile_executes_only_inside_the_declared_boundaries() {
        assert!(
            program().is_some(),
            "native Linux sandbox prerequisites unavailable"
        );
        let workspace = Workspace::create().unwrap();
        let outside = workspace.0.join("outside");
        fs::write(&outside, b"synthetic outside value").unwrap();
        let source = workspace.0.join("source");
        let script = format!(
            r#"#!/bin/sh
[ -z "$HOME" ] || exit 3
[ ! -e /proc/self ] || exit 4
[ ! -e '{}' ] || exit 5
[ ! -e /run/user ] || exit 6
[ ! -w /validator ] || exit 7
if (: > /dev/akra-validator-probe) 2>/dev/null; then exit 9; fi
written=0
for path in /dev/shm/akra-validator-a /dev/shm/akra-validator-b; do
    if dd if=/dev/zero of="$path" bs=1048576 count=12 2>/dev/null; then
        written=$((written + 1))
    fi
done
[ "$written" -eq 0 ] || exit 10
printf 'workspace probe' > /tmp/akra-validator-probe || exit 11
printf '' > /dev/null || exit 12
grep -q title || exit 8
echo 'rejected by fixture'
exit 1
"#,
            outside.display()
        );
        fs::write(&source, &script).unwrap();
        let request = IssueCodeValidatorRunRequest {
            command: source.to_string_lossy().into_owned(),
            sha256: format!("{:x}", Sha256::digest(&script)),
            timeout: Duration::from_secs(10),
            input_json: r#"{"title":"T"}"#.into(),
        };
        assert_eq!(
            run(&request),
            Outcome::Rejected {
                message: "rejected by fixture".into()
            }
        );
    }
}

#[cfg(test)]
mod native_tests;
