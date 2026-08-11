use std::io::{self, Read, Write};
use std::process::{
    Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Output, Stdio,
};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(windows)]
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

/*
 * Containment guarantees are platform specific:
 *
 * - Linux combines a pre-exec process group with a random inherited marker and a bounded `/proc`
 *   identity sweep. The sweep freezes every discovered member before killing it, so ordinary
 *   `setsid` and double-fork daemonization cannot escape cleanup. PID/start-time pairs and pidfds
 *   prevent a stale observation from targeting a recycled PID. A program that deliberately
 *   removes its marker and daemonizes after its complete parent lineage has exited still requires
 *   a delegated cgroup or PID namespace for a kernel-enforced guarantee.
 * - Other Unix targets retain race-free process-group creation. A descendant that calls `setsid`
 *   leaves that boundary; this limitation is emitted as a spawn diagnostic.
 * - Windows starts the process suspended, assigns it to a kill-on-close Job, and only then resumes
 *   it. No user-mode instruction can run in the spawn-to-assignment interval.
 */

pub(crate) const SUBPROCESS_TIMEOUT_ENV: &str = "CODEX_EXEC_LOOP_SUBPROCESS_TIMEOUT_SECS";
const DEFAULT_SUBPROCESS_TIMEOUT_SECS: u64 = 30;
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const DEFAULT_MAX_STDOUT_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_MAX_STDERR_BYTES: usize = 1024 * 1024;
const DEFAULT_MAX_STDIN_BYTES: usize = 1024 * 1024;
const MAX_DRAIN_BYTES_PER_STREAM_POLL: usize = 256 * 1024;
const STDOUT_OVERFLOW: u8 = 1;
const STDERR_OVERFLOW: u8 = 2;
#[cfg(target_os = "linux")]
const LINUX_CONTAINMENT_ENV: &str = "AKRA_INTERNAL_SUBPROCESS_CONTAINMENT_ID";
#[cfg(target_os = "linux")]
const LINUX_SWEEP_MAX_PASSES: usize = 16;
#[cfg(target_os = "linux")]
const LINUX_SWEEP_TIMEOUT: Duration = Duration::from_millis(500);
#[cfg(target_os = "linux")]
const LINUX_REQUIRED_STABLE_STOPPED_PASSES: usize = 2;
#[cfg(target_os = "linux")]
const LINUX_MAX_ENVIRON_BYTES: u64 = 2 * 1024 * 1024;
#[cfg(target_os = "linux")]
const LINUX_SCAN_MAX_PROCESSES: usize = 32 * 1024;
#[cfg(target_os = "linux")]
const LINUX_SCAN_MAX_ENVIRON_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) struct ManagedChild {
    child: Child,
    #[cfg(unix)]
    process_group_id: Option<libc::pid_t>,
    #[cfg(target_os = "linux")]
    linux_containment: Option<LinuxContainment>,
    #[cfg(windows)]
    job: Option<WindowsJob>,
    child_reaped: bool,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Debug)]
struct LinuxContainment {
    marker: String,
    root: LinuxProcessIdentity,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct LinuxProcessIdentity {
    pid: libc::pid_t,
    start_time_ticks: u64,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Debug)]
struct LinuxProcessInfo {
    identity: LinuxProcessIdentity,
    state: char,
    parent_pid: libc::pid_t,
    process_group_id: libc::pid_t,
}

#[cfg(target_os = "linux")]
impl LinuxProcessInfo {
    fn is_stopped_or_terminal(&self) -> bool {
        matches!(self.state, 'T' | 't' | 'Z' | 'X' | 'x')
    }

    fn is_terminal(&self) -> bool {
        matches!(self.state, 'Z' | 'X' | 'x')
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct LinuxContainmentSnapshot {
    members: Vec<LinuxProcessInfo>,
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct LinuxFreezeReport {
    tracked: std::collections::HashSet<LinuxProcessIdentity>,
    diagnostics: Vec<String>,
    converged: bool,
    already_empty: bool,
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct LinuxTerminationReport {
    tracked: std::collections::HashSet<LinuxProcessIdentity>,
    error: Option<io::Error>,
    verification_required: bool,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug)]
struct LinuxScanLimits {
    max_processes: usize,
    max_environment_bytes: u64,
}

#[cfg(target_os = "linux")]
impl LinuxScanLimits {
    const PRODUCTION: Self = Self {
        max_processes: LINUX_SCAN_MAX_PROCESSES,
        max_environment_bytes: LINUX_SCAN_MAX_ENVIRON_BYTES,
    };
}

#[cfg(target_os = "linux")]
#[derive(Debug)]
struct LinuxScanBudget {
    deadline: Instant,
    limits: LinuxScanLimits,
    observed_processes: usize,
    environment_bytes: u64,
}

#[cfg(target_os = "linux")]
impl LinuxScanBudget {
    fn new(deadline: Instant, limits: LinuxScanLimits) -> Self {
        Self {
            deadline,
            limits,
            observed_processes: 0,
            environment_bytes: 0,
        }
    }

    fn check_deadline(&self, operation: &str) -> io::Result<()> {
        if Instant::now() < self.deadline {
            return Ok(());
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("Linux containment {operation} exceeded its absolute sweep deadline"),
        ))
    }

    fn observe_process(&mut self) -> io::Result<()> {
        self.observed_processes = self.observed_processes.saturating_add(1);
        if self.observed_processes <= self.limits.max_processes {
            return Ok(());
        }
        Err(io::Error::other(format!(
            "Linux containment process scan exceeded its aggregate {}-PID budget",
            self.limits.max_processes
        )))
    }

    fn charge_environment_bytes(&mut self, bytes: usize) -> io::Result<()> {
        let bytes = u64::try_from(bytes).unwrap_or(u64::MAX);
        self.environment_bytes = self.environment_bytes.saturating_add(bytes);
        if self.environment_bytes <= self.limits.max_environment_bytes {
            return Ok(());
        }
        Err(io::Error::other(format!(
            "Linux containment process scan exceeded its aggregate {}-byte environment budget",
            self.limits.max_environment_bytes
        )))
    }
}

impl ManagedChild {
    pub(crate) fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin.take()
    }

    pub(crate) fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }

    pub(crate) fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr.take()
    }

    pub(crate) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let status = self.child.try_wait()?;
        if status.is_some() {
            self.child_reaped = true;
        }
        Ok(status)
    }

    fn terminate_process_tree(&mut self) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        {
            let process_group_id = self.process_group_id.take();
            let containment = self.linux_containment.take();
            if let (Some(process_group_id), Some(containment)) = (process_group_id, containment) {
                let report = terminate_linux_containment(&containment, process_group_id);
                let verification_error = report
                    .verification_required
                    .then(|| {
                        verify_linux_containment_terminated(
                            &containment,
                            process_group_id,
                            &report.tracked,
                        )
                    })
                    .transpose()
                    .err();
                return finish_linux_cleanup(report.error, verification_error, None);
            }
            return Ok(());
        }

        #[cfg(all(unix, not(target_os = "linux")))]
        if let Some(process_group_id) = self.process_group_id.take() {
            return signal_unix_process_group(process_group_id, libc::SIGKILL);
        }

        #[cfg(windows)]
        if let Some(job) = self.job.take() {
            return job.terminate();
        }

        #[allow(unreachable_code)]
        Ok(())
    }

    pub(crate) fn terminate_and_wait(&mut self) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        {
            let target = self
                .process_group_id
                .take()
                .zip(self.linux_containment.take());
            let mut report = target.as_ref().map(|(process_group_id, containment)| {
                terminate_linux_containment(containment, *process_group_id)
            });
            let mut child_error = None;
            if !self.child_reaped {
                // Disarm before syscalls so Drop never targets a recycled PID after an attempted reap.
                self.child_reaped = true;
                if let Err(error) = self.child.kill()
                    && !process_is_already_gone(&error)
                {
                    child_error = Some(format!("failed to kill subprocess leader: {error}"));
                }
                if let Err(error) = self.child.wait() {
                    let message = format!("failed to reap subprocess leader: {error}");
                    child_error = Some(match child_error {
                        Some(previous) => format!("{previous}; {message}"),
                        None => message,
                    });
                }
            }
            let verification_error = match (target.as_ref(), report.as_ref()) {
                (Some((process_group_id, containment)), Some(report))
                    if report.verification_required =>
                {
                    verify_linux_containment_terminated(
                        containment,
                        *process_group_id,
                        &report.tracked,
                    )
                    .err()
                }
                _ => None,
            };
            finish_linux_cleanup(
                report.as_mut().and_then(|report| report.error.take()),
                verification_error,
                child_error,
            )
        }

        #[cfg(not(target_os = "linux"))]
        {
            let tree_error = self.terminate_process_tree().err();
            let mut child_error = None;
            if !self.child_reaped {
                // Disarm before syscalls so Drop never targets a recycled PID after an attempted reap.
                self.child_reaped = true;
                if let Err(error) = self.child.kill()
                    && !process_is_already_gone(&error)
                {
                    child_error = Some(format!("failed to kill subprocess leader: {error}"));
                }
                if let Err(error) = self.child.wait() {
                    let message = format!("failed to reap subprocess leader: {error}");
                    child_error = Some(match child_error {
                        Some(previous) => format!("{previous}; {message}"),
                        None => message,
                    });
                }
            }
            combine_cleanup_errors(tree_error, child_error)
        }
    }

    fn finish_success(&mut self) -> io::Result<()> {
        debug_assert!(self.child_reaped);
        self.terminate_process_tree()
    }
}

#[cfg(target_os = "linux")]
fn finish_linux_cleanup(
    termination_error: Option<io::Error>,
    verification_error: Option<io::Error>,
    child_error: Option<String>,
) -> io::Result<()> {
    let tree_error = match (termination_error, verification_error) {
        (None, None) => None,
        (Some(error), None) | (None, Some(error)) => Some(error),
        (Some(error), Some(verification_error)) => Some(io::Error::new(
            error.kind(),
            format!("{error}; post-termination verification failed: {verification_error}"),
        )),
    };
    combine_cleanup_errors(tree_error, child_error)
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        if let Err(error) = self.terminate_and_wait() {
            tracing::warn!(error = %error, "subprocess containment cleanup failed during drop");
        }
    }
}

fn combine_cleanup_errors(
    tree_error: Option<io::Error>,
    child_error: Option<String>,
) -> io::Result<()> {
    match (tree_error, child_error) {
        (None, None) => Ok(()),
        (Some(error), None) => Err(error),
        (None, Some(message)) => Err(io::Error::other(message)),
        (Some(error), Some(message)) => {
            Err(io::Error::new(error.kind(), format!("{error}; {message}")))
        }
    }
}

fn process_is_already_gone(error: &io::Error) -> bool {
    #[cfg(unix)]
    return error.raw_os_error() == Some(libc::ESRCH)
        || error.kind() == io::ErrorKind::InvalidInput;

    #[cfg(windows)]
    return matches!(error.raw_os_error(), Some(87 | 1168))
        || error.kind() == io::ErrorKind::InvalidInput;

    #[allow(unreachable_code)]
    false
}

#[cfg(all(unix, not(target_os = "linux")))]
fn signal_unix_process_group(process_group_id: libc::pid_t, signal: libc::c_int) -> io::Result<()> {
    let result = unsafe {
        // A negative PID addresses the process group created in the child before exec.
        libc::kill(-process_group_id, signal)
    };
    if result == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}

#[cfg(target_os = "linux")]
fn new_linux_containment_marker() -> io::Result<String> {
    use rand::RngCore;

    let mut random = [0_u8; 16];
    rand::rngs::OsRng
        .try_fill_bytes(&mut random)
        .map_err(|error| {
            io::Error::other(format!(
                "secure Linux subprocess containment marker generation failed: {error}"
            ))
        })?;
    let mut marker = String::with_capacity(random.len() * 2);
    for byte in random {
        use std::fmt::Write;
        write!(&mut marker, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(marker)
}

#[cfg(target_os = "linux")]
fn read_linux_process_info(pid: libc::pid_t) -> io::Result<LinuxProcessInfo> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    parse_linux_process_stat(&stat).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid Linux process stat record for PID {pid}"),
        )
    })
}

#[cfg(target_os = "linux")]
fn parse_linux_process_stat(stat: &str) -> Option<LinuxProcessInfo> {
    let pid = stat[..stat.find(' ')?].parse().ok()?;
    // `comm` is parenthesized and may itself contain spaces or closing parentheses.
    let command_end = stat.rfind(") ")?;
    let fields = stat[command_end + 2..]
        .split_ascii_whitespace()
        .collect::<Vec<_>>();
    let state = fields.first()?.chars().next()?;
    let parent_pid = fields.get(1)?.parse().ok()?;
    let process_group_id = fields.get(2)?.parse().ok()?;
    let start_time_ticks = fields.get(19)?.parse().ok()?;
    Some(LinuxProcessInfo {
        identity: LinuxProcessIdentity {
            pid,
            start_time_ticks,
        },
        state,
        parent_pid,
        process_group_id,
    })
}

#[cfg(target_os = "linux")]
fn linux_process_ids(budget: &mut LinuxScanBudget) -> io::Result<Vec<libc::pid_t>> {
    let mut process_ids = Vec::new();
    for entry in std::fs::read_dir("/proc")? {
        budget.check_deadline("process enumeration")?;
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                tracing::debug!(error = %error, "skipping an unreadable /proc entry");
                continue;
            }
        };
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<libc::pid_t>().ok())
            .filter(|pid| *pid > 0)
        else {
            continue;
        };
        budget.observe_process()?;
        process_ids.push(pid);
    }
    Ok(process_ids)
}

#[cfg(target_os = "linux")]
fn linux_process_has_marker(
    pid: libc::pid_t,
    expected: &[u8],
    budget: &mut LinuxScanBudget,
) -> io::Result<bool> {
    budget.check_deadline("environment scan")?;
    let Ok(file) = std::fs::File::open(format!("/proc/{pid}/environ")) else {
        return Ok(false);
    };
    let mut environment = Vec::new();
    let mut file = file.take(LINUX_MAX_ENVIRON_BYTES.saturating_add(1));
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        budget.check_deadline("environment scan")?;
        let read = match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) if error.raw_os_error() == Some(libc::ESRCH) => return Ok(false),
            Err(error) => {
                tracing::debug!(pid, error = %error, "skipping a process that changed during environment scan");
                return Ok(false);
            }
        };
        budget.charge_environment_bytes(read)?;
        environment.extend_from_slice(&buffer[..read]);
        if environment.len() as u64 > LINUX_MAX_ENVIRON_BYTES {
            return Err(io::Error::other(format!(
                "Linux containment environment for PID {pid} exceeded its {LINUX_MAX_ENVIRON_BYTES}-byte per-process budget"
            )));
        }
    }
    Ok(environment
        .split(|byte| *byte == 0)
        .any(|entry| entry == expected))
}

#[cfg(target_os = "linux")]
fn discover_linux_containment_members(
    containment: &LinuxContainment,
    process_group_id: libc::pid_t,
    tracked: &std::collections::HashSet<LinuxProcessIdentity>,
    deadline: Instant,
    limits: LinuxScanLimits,
) -> io::Result<Vec<LinuxProcessInfo>> {
    use std::collections::{HashMap, HashSet};

    let mut budget = LinuxScanBudget::new(deadline, limits);
    let process_ids = linux_process_ids(&mut budget)?;
    let expected_marker = format!("{LINUX_CONTAINMENT_ENV}={}", containment.marker);
    let mut seed_identities = HashSet::new();
    for identity in tracked
        .iter()
        .copied()
        .chain(std::iter::once(containment.root))
    {
        if read_linux_process_info(identity.pid).is_ok_and(|process| process.identity == identity) {
            seed_identities.insert(identity);
        }
    }
    for pid in process_ids.iter().copied() {
        budget.check_deadline("marker discovery")?;
        if linux_process_has_marker(pid, expected_marker.as_bytes(), &mut budget)?
            && let Ok(process) = read_linux_process_info(pid)
        {
            seed_identities.insert(process.identity);
        }
    }
    if seed_identities.is_empty() {
        // Normal commands have no live leader, group member, or marked descendant after reaping.
        // Avoid parsing every `/proc/*/stat` record and skip the stabilization passes entirely.
        return Ok(Vec::new());
    }

    let mut processes = Vec::new();
    for pid in process_ids {
        budget.check_deadline("process identity scan")?;
        match read_linux_process_info(pid) {
            Ok(process) => processes.push(process),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) if error.raw_os_error() == Some(libc::ESRCH) => {}
            Err(error) => {
                tracing::debug!(pid, error = %error, "skipping a process that changed during /proc scan");
            }
        }
    }
    let by_pid = processes
        .iter()
        .map(|process| (process.identity.pid, process))
        .collect::<HashMap<_, _>>();
    let mut member_pids = HashSet::new();

    for process in &processes {
        if seed_identities.contains(&process.identity) {
            member_pids.insert(process.identity.pid);
        }
    }

    // Include descendants whose environment has been intentionally or accidentally cleared while
    // their observable parent lineage still reaches a marked member.
    loop {
        let previous_len = member_pids.len();
        for process in &processes {
            if member_pids.contains(&process.parent_pid) {
                member_pids.insert(process.identity.pid);
            }
        }
        if member_pids.len() == previous_len {
            break;
        }
    }

    // Process-group membership is safe to add only after a start-time/marker anchored member proves
    // that the original group still exists. This avoids signaling a numerically recycled PGID.
    let group_is_anchored = member_pids.iter().any(|pid| {
        by_pid
            .get(pid)
            .is_some_and(|process| process.process_group_id == process_group_id)
    });
    if group_is_anchored {
        for process in &processes {
            if process.process_group_id == process_group_id {
                member_pids.insert(process.identity.pid);
            }
        }
    }

    Ok(processes
        .into_iter()
        .filter(|process| member_pids.contains(&process.identity.pid))
        .collect())
}

#[cfg(target_os = "linux")]
fn freeze_linux_containment_with_hooks<Discover, Stop>(
    initial_tracked: std::collections::HashSet<LinuxProcessIdentity>,
    max_passes: usize,
    mut discover: Discover,
    mut stop: Stop,
) -> LinuxFreezeReport
where
    Discover: FnMut(
        &std::collections::HashSet<LinuxProcessIdentity>,
    ) -> io::Result<LinuxContainmentSnapshot>,
    Stop: FnMut(LinuxProcessIdentity) -> io::Result<()>,
{
    use std::collections::HashSet;

    let mut tracked = initial_tracked;
    let mut previous_stopped = None::<HashSet<LinuxProcessIdentity>>;
    let mut stable_stopped_passes = 0_usize;
    let mut diagnostics = Vec::new();
    let mut converged = false;
    let mut already_empty = false;

    for _ in 0..max_passes {
        let snapshot = match discover(&tracked) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                diagnostics.push(format!("failed to enumerate Linux descendants: {error}"));
                for identity in tracked.iter().copied() {
                    if let Err(error) = stop(identity) {
                        diagnostics.push(format!(
                            "failed to freeze tracked PID {} at start time {} after enumeration failure: {error}",
                            identity.pid, identity.start_time_ticks
                        ));
                    }
                }
                break;
            }
        };
        if snapshot.members.is_empty() {
            converged = true;
            already_empty = true;
            break;
        }

        let stopped_identities = snapshot
            .members
            .iter()
            .map(|member| member.identity)
            .collect::<HashSet<_>>();
        let mut all_members_stopped = !stopped_identities.is_empty();
        // Revalidate and stop every live identity on every pass. A stop sent during this pass does
        // not count as observed convergence; a later discovery must report the process stopped.
        for member in &snapshot.members {
            tracked.insert(member.identity);
            if !member.is_terminal()
                && let Err(error) = stop(member.identity)
            {
                diagnostics.push(format!(
                    "failed to freeze PID {} at start time {}: {error}",
                    member.identity.pid, member.identity.start_time_ticks
                ));
                all_members_stopped = false;
            }
            if !member.is_stopped_or_terminal() {
                all_members_stopped = false;
            }
        }

        if all_members_stopped {
            if previous_stopped.as_ref() == Some(&stopped_identities) {
                stable_stopped_passes = stable_stopped_passes.saturating_add(1);
            } else {
                previous_stopped = Some(stopped_identities);
                stable_stopped_passes = 1;
            }
            if stable_stopped_passes >= LINUX_REQUIRED_STABLE_STOPPED_PASSES {
                converged = true;
                break;
            }
        } else {
            previous_stopped = None;
            stable_stopped_passes = 0;
        }
    }

    if !converged {
        diagnostics.push(format!(
            "Linux containment freeze did not converge within its {max_passes}-pass budget"
        ));
    }
    LinuxFreezeReport {
        tracked,
        diagnostics,
        converged,
        already_empty,
    }
}

#[cfg(target_os = "linux")]
fn terminate_linux_containment(
    containment: &LinuxContainment,
    process_group_id: libc::pid_t,
) -> LinuxTerminationReport {
    use std::collections::HashSet;

    let deadline = Instant::now() + LINUX_SWEEP_TIMEOUT;
    let freeze = freeze_linux_containment_with_hooks(
        HashSet::from([containment.root]),
        LINUX_SWEEP_MAX_PASSES,
        |tracked| {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "Linux containment freeze exceeded {}ms",
                        LINUX_SWEEP_TIMEOUT.as_millis()
                    ),
                ));
            }
            let members = discover_linux_containment_members(
                containment,
                process_group_id,
                tracked,
                deadline,
                LinuxScanLimits::PRODUCTION,
            )?;
            Ok(LinuxContainmentSnapshot { members })
        },
        |identity| signal_linux_identity(identity, libc::SIGSTOP),
    );
    debug_assert!(freeze.converged || !freeze.diagnostics.is_empty());
    let mut diagnostics = freeze.diagnostics;
    let tracked = freeze.tracked;
    if freeze.already_empty {
        return LinuxTerminationReport {
            tracked,
            error: if diagnostics.is_empty() {
                None
            } else {
                Some(io::Error::other(diagnostics.join("; ")))
            },
            verification_required: false,
        };
    }

    // No final discovery is allowed between convergence and termination: every identity in this
    // set was revalidated and stopped first, and the same stopped set was observed twice.
    let group_is_anchored = tracked.iter().copied().any(|identity| {
        read_linux_process_info(identity.pid).is_ok_and(|process| {
            process.identity == identity && process.process_group_id == process_group_id
        })
    });
    if group_is_anchored {
        let result = unsafe { libc::kill(-process_group_id, libc::SIGKILL) };
        if result != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                diagnostics.push(format!(
                    "failed to terminate Unix process group {process_group_id}: {error}"
                ));
            }
        }
    }
    for identity in tracked.iter().copied() {
        if let Err(error) = signal_linux_identity(identity, libc::SIGKILL) {
            diagnostics.push(format!(
                "failed to terminate PID {} at start time {}: {error}",
                identity.pid, identity.start_time_ticks
            ));
        }
    }

    LinuxTerminationReport {
        tracked,
        error: if diagnostics.is_empty() {
            None
        } else {
            Some(io::Error::other(diagnostics.join("; ")))
        },
        verification_required: true,
    }
}

#[cfg(target_os = "linux")]
fn discover_linux_containment_residue(
    containment: &LinuxContainment,
    process_group_id: libc::pid_t,
    tracked: &std::collections::HashSet<LinuxProcessIdentity>,
    deadline: Instant,
    limits: LinuxScanLimits,
) -> io::Result<Vec<LinuxProcessInfo>> {
    let members = discover_linux_containment_members(
        containment,
        process_group_id,
        tracked,
        deadline,
        limits,
    )?;
    // Numeric PGIDs may be recycled after the managed leader is reaped. The shared discovery
    // path includes group members only while a marker or exact PID/start-time identity anchors
    // that group, so verification cannot terminate an unrelated recycled process group.
    Ok(members
        .into_iter()
        // Zombies/dead tasks cannot execute or fork and may remain until an unrelated subreaper
        // runs. Verification is about live marker and process-group members.
        .filter(|process| !process.is_terminal())
        .collect())
}

#[cfg(target_os = "linux")]
fn verify_linux_containment_terminated(
    containment: &LinuxContainment,
    process_group_id: libc::pid_t,
    tracked: &std::collections::HashSet<LinuxProcessIdentity>,
) -> io::Result<()> {
    use std::collections::HashSet;

    let deadline = Instant::now() + LINUX_SWEEP_TIMEOUT;
    let mut diagnostics = Vec::new();
    let mut last_residue = HashSet::new();
    for _ in 0..LINUX_SWEEP_MAX_PASSES {
        let residue = discover_linux_containment_residue(
            containment,
            process_group_id,
            tracked,
            deadline,
            LinuxScanLimits::PRODUCTION,
        )?;
        if residue.is_empty() {
            return if diagnostics.is_empty() {
                Ok(())
            } else {
                Err(io::Error::other(diagnostics.join("; ")))
            };
        }

        last_residue = residue.iter().map(|process| process.identity).collect();
        for process in &residue {
            if let Err(error) = signal_linux_identity(process.identity, libc::SIGSTOP) {
                diagnostics.push(format!(
                    "failed to stop residual PID {} at start time {} before retrying termination: {error}",
                    process.identity.pid, process.identity.start_time_ticks
                ));
            }
        }
        let group_is_anchored = residue.iter().any(|process| {
            process.process_group_id == process_group_id
                && read_linux_process_info(process.identity.pid).is_ok_and(|current| {
                    current.identity == process.identity
                        && current.process_group_id == process_group_id
                })
        });
        if group_is_anchored {
            let result = unsafe { libc::kill(-process_group_id, libc::SIGKILL) };
            if result != 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) {
                    diagnostics.push(format!(
                        "failed to re-terminate Unix process group {process_group_id}: {error}"
                    ));
                }
            }
        }
        for process in residue {
            if let Err(error) = signal_linux_identity(process.identity, libc::SIGKILL) {
                diagnostics.push(format!(
                    "failed to re-terminate residual PID {} at start time {}: {error}",
                    process.identity.pid, process.identity.start_time_ticks
                ));
            }
        }
    }

    let mut remaining = last_residue.into_iter().collect::<Vec<_>>();
    remaining.sort_by_key(|identity| (identity.pid, identity.start_time_ticks));
    let remaining = remaining
        .into_iter()
        .map(|identity| format!("{}@{}", identity.pid, identity.start_time_ticks))
        .collect::<Vec<_>>()
        .join(", ");
    diagnostics.push(format!(
        "Linux containment post-termination verification did not reach zero marker/process-group members within its {LINUX_SWEEP_MAX_PASSES}-pass budget; remaining identities: {remaining}"
    ));
    Err(io::Error::other(diagnostics.join("; ")))
}

#[cfg(target_os = "linux")]
fn signal_linux_identity(identity: LinuxProcessIdentity, signal: libc::c_int) -> io::Result<()> {
    let descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, identity.pid, 0) };
    if descriptor < 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        tracing::debug!(
            pid = identity.pid,
            error = %error,
            "pidfd_open unavailable; using start-time-verified kill fallback"
        );
        return signal_linux_identity_without_pidfd(identity, signal);
    }
    // `pidfd_open` returns a native `int`; `syscall` widens that value to `c_long`.
    let descriptor = descriptor as libc::c_int;

    if !verify_linux_identity(identity)? {
        unsafe {
            libc::close(descriptor);
        }
        return Ok(());
    }
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            descriptor,
            signal,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    let signal_error = if result == 0 {
        None
    } else {
        Some(io::Error::last_os_error())
    };
    unsafe {
        libc::close(descriptor);
    }
    match signal_error {
        None => Ok(()),
        Some(error) if error.raw_os_error() == Some(libc::ESRCH) => Ok(()),
        Some(error)
            if matches!(
                error.raw_os_error(),
                Some(libc::ENOSYS | libc::EPERM | libc::EINVAL)
            ) =>
        {
            tracing::debug!(
                pid = identity.pid,
                error = %error,
                "pidfd_send_signal unavailable; using start-time-verified kill fallback"
            );
            signal_linux_identity_without_pidfd(identity, signal)
        }
        Some(error) => Err(error),
    }
}

#[cfg(target_os = "linux")]
fn signal_linux_identity_without_pidfd(
    identity: LinuxProcessIdentity,
    signal: libc::c_int,
) -> io::Result<()> {
    if !verify_linux_identity(identity)? {
        return Ok(());
    }
    let result = unsafe { libc::kill(identity.pid, signal) };
    if result == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}

#[cfg(target_os = "linux")]
fn verify_linux_identity(identity: LinuxProcessIdentity) -> io::Result<bool> {
    match read_linux_process_info(identity.pid) {
        Ok(process) => Ok(process.identity == identity),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => Ok(false),
        Err(error) => Err(io::Error::new(
            error.kind(),
            format!(
                "failed to verify Linux PID {} at start time {}: {error}",
                identity.pid, identity.start_time_ticks
            ),
        )),
    }
}

#[cfg(windows)]
struct WindowsJob {
    handle: OwnedHandle,
}

#[cfg(windows)]
impl WindowsJob {
    fn new() -> io::Result<Self> {
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };

        let raw_handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw_handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let handle = unsafe { OwnedHandle::from_raw_handle(raw_handle) };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let size = u32::try_from(std::mem::size_of_val(&limits))
            .expect("Windows job limit structure size must fit u32");
        let succeeded = unsafe {
            SetInformationJobObject(
                handle.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                size,
            )
        };
        if succeeded == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { handle })
    }

    fn assign(&self, child: &Child) -> io::Result<()> {
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;

        let succeeded =
            unsafe { AssignProcessToJobObject(self.handle.as_raw_handle(), child.as_raw_handle()) };
        if succeeded == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn resume(&self, child: &Child) -> io::Result<()> {
        use windows_sys::Win32::Foundation::{HANDLE, NTSTATUS};

        #[link(name = "ntdll")]
        unsafe extern "system" {
            #[link_name = "NtResumeProcess"]
            fn nt_resume_process(process_handle: HANDLE) -> NTSTATUS;
        }

        let status = unsafe { nt_resume_process(child.as_raw_handle()) };
        if status >= 0 {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "NtResumeProcess failed with NTSTATUS 0x{:08x}",
                status as u32
            )))
        }
    }

    fn terminate(&self) -> io::Result<()> {
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;

        let succeeded = unsafe { TerminateJobObject(self.handle.as_raw_handle(), 1) };
        if succeeded == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

enum CapturedPipe {
    Stdout(ChildStdout),
    Stderr(ChildStderr),
}

impl Read for CapturedPipe {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Stdout(pipe) => pipe.read(buffer),
            Self::Stderr(pipe) => pipe.read(buffer),
        }
    }
}

#[cfg(unix)]
impl CapturedPipe {
    fn raw_fd(&self) -> std::os::fd::RawFd {
        match self {
            Self::Stdout(pipe) => pipe.as_raw_fd(),
            Self::Stderr(pipe) => pipe.as_raw_fd(),
        }
    }
}

#[cfg(windows)]
impl CapturedPipe {
    fn raw_handle(&self) -> std::os::windows::io::RawHandle {
        match self {
            Self::Stdout(pipe) => pipe.as_raw_handle(),
            Self::Stderr(pipe) => pipe.as_raw_handle(),
        }
    }
}

struct BoundedPipeReader {
    pipe: Option<CapturedPipe>,
    retained: Vec<u8>,
    max_bytes: usize,
    complete: bool,
}

impl BoundedPipeReader {
    fn new(pipe: Option<CapturedPipe>, max_bytes: usize) -> io::Result<Self> {
        let complete = pipe.is_none();
        let reader = Self {
            pipe,
            retained: Vec::with_capacity(max_bytes.min(8 * 1024)),
            max_bytes,
            complete,
        };
        reader.prepare_nonblocking()?;
        Ok(reader)
    }

    #[cfg(unix)]
    fn prepare_nonblocking(&self) -> io::Result<()> {
        let Some(pipe) = self.pipe.as_ref() else {
            return Ok(());
        };
        let descriptor = pipe.raw_fd();
        let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[cfg(windows)]
    fn prepare_nonblocking(&self) -> io::Result<()> {
        Ok(())
    }

    fn drain_available(&mut self) -> io::Result<bool> {
        if self.complete {
            return Ok(false);
        }

        let mut drained = 0;
        let mut buffer = [0_u8; 8 * 1024];
        while drained < MAX_DRAIN_BYTES_PER_STREAM_POLL {
            #[cfg(windows)]
            match self.windows_available_bytes()? {
                Some(0) => return Ok(false),
                Some(available) => {
                    let available = usize::try_from(available).unwrap_or(usize::MAX);
                    let budget = MAX_DRAIN_BYTES_PER_STREAM_POLL - drained;
                    let read_limit = available.min(budget).min(buffer.len());
                    if read_limit == 0 {
                        return Ok(false);
                    }
                    match self.read_once(&mut buffer[..read_limit])? {
                        Some(read) => drained += read,
                        None => return Ok(false),
                    }
                }
                None => {
                    self.complete = true;
                    return Ok(false);
                }
            }

            #[cfg(unix)]
            match self.read_once(&mut buffer)? {
                Some(read) => drained += read,
                None => return Ok(false),
            }

            if self.retained.len() > self.max_bytes {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn read_once(&mut self, buffer: &mut [u8]) -> io::Result<Option<usize>> {
        let Some(pipe) = self.pipe.as_mut() else {
            self.complete = true;
            return Ok(None);
        };
        match pipe.read(buffer) {
            Ok(0) => {
                self.complete = true;
                Ok(None)
            }
            Ok(read) => {
                let next_len = self.retained.len().saturating_add(read);
                if next_len > self.max_bytes {
                    self.retained.resize(self.max_bytes.saturating_add(1), 0);
                } else {
                    self.retained.extend_from_slice(&buffer[..read]);
                }
                Ok(Some(read))
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
            #[cfg(windows)]
            Err(error) if windows_pipe_is_closed(&error) => {
                self.complete = true;
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    #[cfg(windows)]
    fn windows_available_bytes(&self) -> io::Result<Option<u32>> {
        use windows_sys::Win32::System::Pipes::PeekNamedPipe;

        let Some(pipe) = self.pipe.as_ref() else {
            return Ok(None);
        };
        let mut available = 0_u32;
        let succeeded = unsafe {
            PeekNamedPipe(
                pipe.raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        };
        if succeeded != 0 {
            return Ok(Some(available));
        }
        let error = io::Error::last_os_error();
        if windows_pipe_is_closed(&error) {
            Ok(None)
        } else {
            Err(error)
        }
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn into_retained(self) -> Vec<u8> {
        self.retained
    }
}

#[cfg(windows)]
fn windows_pipe_is_closed(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(109 | 232 | 233))
}

pub(crate) fn configured_subprocess_timeout() -> Duration {
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

pub(crate) fn spawn(command: &mut Command) -> io::Result<ManagedChild> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        #[cfg(target_os = "linux")]
        let containment_marker = {
            // Linux cleanup depends on procfs for start-time identity and escaped-daemon discovery.
            // Refuse to claim containment when the kernel boundary is unavailable.
            read_linux_process_info(libc::pid_t::try_from(std::process::id()).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "current PID does not fit pid_t")
            })?)
            .map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("Linux subprocess containment requires readable procfs: {error}"),
                )
            })?;
            let marker = new_linux_containment_marker()?;
            command.env(LINUX_CONTAINMENT_ENV, &marker);
            marker
        };

        // Creating the group in the child before exec closes the fork-before-setpgid race.
        command.process_group(0);
        let mut child = command.spawn()?;
        let process_group_id = match libc::pid_t::try_from(child.id()) {
            Ok(process_group_id) if process_group_id > 0 => process_group_id,
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "spawned child PID cannot identify a Unix process group",
                ));
            }
        };
        #[cfg(target_os = "linux")]
        let linux_containment = match read_linux_process_info(process_group_id) {
            Ok(process) => Some(LinuxContainment {
                marker: containment_marker,
                root: process.identity,
            }),
            Err(error) => {
                // The child is still unreaped, so its PID and process-group identifier cannot be
                // recycled while this fail-closed cleanup runs.
                let mut cleanup_errors = Vec::new();
                let group_result = unsafe { libc::kill(-process_group_id, libc::SIGKILL) };
                if group_result != 0 {
                    let cleanup_error = io::Error::last_os_error();
                    if cleanup_error.raw_os_error() != Some(libc::ESRCH) {
                        cleanup_errors.push(format!(
                            "failed to kill unidentified process group: {cleanup_error}"
                        ));
                    }
                }
                if let Err(cleanup_error) = child.kill()
                    && !process_is_already_gone(&cleanup_error)
                {
                    cleanup_errors.push(format!(
                        "failed to kill unidentified process leader: {cleanup_error}"
                    ));
                }
                if let Err(cleanup_error) = child.wait() {
                    cleanup_errors.push(format!(
                        "failed to reap unidentified process leader: {cleanup_error}"
                    ));
                }
                let cleanup_diagnostic = if cleanup_errors.is_empty() {
                    String::new()
                } else {
                    format!("; cleanup also failed: {}", cleanup_errors.join("; "))
                };
                return Err(io::Error::new(
                    error.kind(),
                    format!(
                        "failed to capture spawned Linux process identity: {error}{cleanup_diagnostic}"
                    ),
                ));
            }
        };

        #[cfg(not(target_os = "linux"))]
        tracing::debug!(
            platform = std::env::consts::OS,
            containment = "process-group-only",
            limitation = "a descendant that creates a new session can leave this boundary",
            "spawned subprocess with limited Unix descendant containment"
        );
        Ok(ManagedChild {
            child,
            process_group_id: Some(process_group_id),
            #[cfg(target_os = "linux")]
            linux_containment,
            child_reaped: false,
        })
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::CREATE_SUSPENDED;

        let job = WindowsJob::new()?;
        // `std::process::Command` does not expose STARTUPINFOEX or the primary thread handle needed
        // for PROC_THREAD_ATTRIBUTE_JOB_LIST/ResumeThread. CREATE_SUSPENDED plus the process-wide
        // NtResumeProcess operation preserves Command's quoting, environment, and stdio behavior
        // while closing the spawn-before-Job-assignment execution race. This helper owns the
        // creation-flags field (all current callers leave it unset); extending the API is required
        // before a caller can request additional flags without losing this guarantee.
        command.creation_flags(CREATE_SUSPENDED);
        let mut child = command.spawn()?;
        if let Err(error) = job.assign(&child) {
            let kill_error = child
                .kill()
                .err()
                .filter(|error| !process_is_already_gone(error));
            let wait_error = child.wait().err();
            let mut message =
                format!("failed to contain spawned process in a Windows job: {error}");
            if let Some(error) = kill_error {
                message.push_str(&format!("; failed to kill suspended process: {error}"));
            }
            if let Some(error) = wait_error {
                message.push_str(&format!("; failed to reap suspended process: {error}"));
            }
            return Err(io::Error::new(error.kind(), message));
        }
        if let Err(error) = job.resume(&child) {
            let terminate_error = job.terminate().err();
            let kill_error = child
                .kill()
                .err()
                .filter(|error| !process_is_already_gone(error));
            let wait_error = child.wait().err();
            let mut message = format!("failed to resume Job-contained Windows process: {error}");
            if let Some(error) = terminate_error {
                message.push_str(&format!("; failed to terminate Job: {error}"));
            }
            if let Some(error) = kill_error {
                message.push_str(&format!("; failed to kill suspended process: {error}"));
            }
            if let Some(error) = wait_error {
                message.push_str(&format!("; failed to reap suspended process: {error}"));
            }
            return Err(io::Error::other(message));
        }
        Ok(ManagedChild {
            child,
            job: Some(job),
            child_reaped: false,
        })
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
    let child = spawn(command)?;
    wait_with_output_timeout(child, command_label, timeout)
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

pub(crate) fn command_output_with_input_and_timeout(
    command: &mut Command,
    command_label: &str,
    input: &[u8],
    timeout: Duration,
) -> io::Result<Output> {
    command_output_with_input_timeout_and_limits(
        command,
        command_label,
        input,
        timeout,
        DEFAULT_MAX_STDOUT_BYTES,
        DEFAULT_MAX_STDERR_BYTES,
    )
}

pub(crate) fn command_output_with_input_timeout_and_limits(
    command: &mut Command,
    command_label: &str,
    input: &[u8],
    timeout: Duration,
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
) -> io::Result<Output> {
    if input.len() > DEFAULT_MAX_STDIN_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{command_label} stdin exceeded the {DEFAULT_MAX_STDIN_BYTES}-byte limit"),
        ));
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = spawn(command)?;
    let mut stdin = child.take_stdin().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::BrokenPipe,
            format!("{command_label} did not expose its configured stdin pipe"),
        )
    })?;
    let input = input.to_vec();
    let input_writer = match std::thread::Builder::new()
        .name("akra-bounded-subprocess-stdin".to_string())
        .spawn(move || -> io::Result<()> {
            stdin.write_all(&input)?;
            stdin.flush()
        }) {
        Ok(writer) => writer,
        Err(error) => {
            let cleanup_error = child.terminate_and_wait().err();
            return Err(io::Error::new(
                error.kind(),
                match cleanup_error {
                    Some(cleanup_error) => format!(
                        "failed to start bounded stdin writer: {error}; subprocess cleanup failed: {cleanup_error}"
                    ),
                    None => format!("failed to start bounded stdin writer: {error}"),
                },
            ));
        }
    };

    let output = wait_with_output_timeout_and_limits(
        child,
        command_label,
        timeout,
        max_stdout_bytes,
        max_stderr_bytes,
    );
    let input_result = input_writer
        .join()
        .map_err(|_| io::Error::other(format!("{command_label} bounded stdin writer panicked")));
    match output {
        Err(error) => {
            let _ = input_result;
            Err(error)
        }
        Ok(output) => {
            input_result??;
            Ok(output)
        }
    }
}

pub(crate) fn wait_with_output(child: ManagedChild, command_label: &str) -> io::Result<Output> {
    wait_with_output_timeout(child, command_label, configured_subprocess_timeout())
}

pub(crate) fn wait_with_output_timeout(
    child: ManagedChild,
    command_label: &str,
    timeout: Duration,
) -> io::Result<Output> {
    wait_with_output_timeout_and_limits(
        child,
        command_label,
        timeout,
        DEFAULT_MAX_STDOUT_BYTES,
        DEFAULT_MAX_STDERR_BYTES,
    )
}

pub(crate) fn wait_with_output_timeout_and_limits(
    mut child: ManagedChild,
    command_label: &str,
    timeout: Duration,
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
) -> io::Result<Output> {
    /*
     * `Child::wait_with_output` starts draining only after the child exits. A child that fills a
     * pipe before exit therefore deadlocks until the timeout. Drain both pipes concurrently while
     * retaining only bounded output; the child still sees normal pipe flow and a hostile response
     * cannot turn diagnostics capture into unbounded memory growth.
     */
    let mut stdout = match BoundedPipeReader::new(
        child.take_stdout().map(CapturedPipe::Stdout),
        max_stdout_bytes,
    ) {
        Ok(reader) => reader,
        Err(error) => {
            return Err(terminate_after_error(&mut child, error));
        }
    };
    let mut stderr = match BoundedPipeReader::new(
        child.take_stderr().map(CapturedPipe::Stderr),
        max_stderr_bytes,
    ) {
        Ok(reader) => reader,
        Err(error) => {
            return Err(terminate_after_error(&mut child, error));
        }
    };
    let started_at = Instant::now();
    let mut status = None;
    loop {
        let stdout_overflow = match stdout.drain_available() {
            Ok(overflow) => overflow,
            Err(error) => {
                return Err(terminate_after_error(&mut child, error));
            }
        };
        let stderr_overflow = match stderr.drain_available() {
            Ok(overflow) => overflow,
            Err(error) => {
                return Err(terminate_after_error(&mut child, error));
            }
        };
        let overflowed_streams = (u8::from(stdout_overflow) * STDOUT_OVERFLOW)
            | (u8::from(stderr_overflow) * STDERR_OVERFLOW);
        if overflowed_streams != 0 {
            return Err(terminate_after_error(
                &mut child,
                output_limit_error(
                    command_label,
                    overflowed_streams,
                    max_stdout_bytes,
                    max_stderr_bytes,
                ),
            ));
        }
        if status.is_none() {
            status = match child.try_wait() {
                Ok(Some(status)) => Some(status),
                Ok(None) => None,
                Err(error) => {
                    return Err(terminate_after_error(&mut child, error));
                }
            };
        }
        if status.is_some() && stdout.is_complete() && stderr.is_complete() {
            child.finish_success()?;
            return Ok(Output {
                status: status.take().expect("completed child status should exist"),
                stdout: stdout.into_retained(),
                stderr: stderr.into_retained(),
            });
        }
        if started_at.elapsed() >= timeout {
            return Err(terminate_after_error(
                &mut child,
                timeout_error(command_label, timeout),
            ));
        }
        std::thread::sleep(POLL_INTERVAL.min(timeout.saturating_sub(started_at.elapsed())));
    }
}

fn terminate_after_error(child: &mut ManagedChild, primary: io::Error) -> io::Error {
    preserve_primary_error_during_cleanup(primary, child.terminate_and_wait())
}

fn preserve_primary_error_during_cleanup(primary: io::Error, cleanup: io::Result<()>) -> io::Error {
    match cleanup {
        Ok(()) => primary,
        Err(cleanup_error) => io::Error::new(
            primary.kind(),
            format!("{primary}; subprocess containment cleanup also failed: {cleanup_error}"),
        ),
    }
}

fn output_limit_error(
    command_label: &str,
    overflowed_streams: u8,
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
) -> io::Error {
    let streams = match overflowed_streams & (STDOUT_OVERFLOW | STDERR_OVERFLOW) {
        STDOUT_OVERFLOW => format!("stdout exceeded {max_stdout_bytes} bytes"),
        STDERR_OVERFLOW => format!("stderr exceeded {max_stderr_bytes} bytes"),
        _ => format!(
            "stdout exceeded {max_stdout_bytes} bytes and stderr exceeded {max_stderr_bytes} bytes"
        ),
    };
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("command `{command_label}` output limit exceeded: {streams}"),
    )
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
        DEFAULT_MAX_STDIN_BYTES, DEFAULT_SUBPROCESS_TIMEOUT_SECS, command_output,
        command_output_with_input_and_timeout, command_output_with_timeout, format_duration,
        parse_subprocess_timeout_secs, spawn, wait_with_output, wait_with_output_timeout,
        wait_with_output_timeout_and_limits,
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
    fn bounded_input_writer_drains_child_output_before_the_child_reads_stdin() {
        let input = vec![b'i'; 256 * 1024];
        let output = command_output_with_input_and_timeout(
            Command::new("sh").args([
                "-c",
                "head -c 262144 /dev/zero; cat >/dev/null; printf complete >&2",
            ]),
            "large output before stdin fixture",
            &input,
            Duration::from_secs(2),
        )
        .expect("input writing and output draining must make progress concurrently");

        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 262_144);
        assert_eq!(output.stderr, b"complete");
    }

    #[test]
    fn bounded_input_rejects_oversized_payload_before_spawning() {
        let marker = std::env::temp_dir().join(format!(
            "akra-oversized-stdin-marker-{}",
            std::process::id()
        ));
        let error = command_output_with_input_and_timeout(
            Command::new("sh")
                .args(["-c", "printf spawned > \"$1\""])
                .arg("oversized-stdin")
                .arg(&marker),
            "oversized stdin fixture",
            &vec![0_u8; DEFAULT_MAX_STDIN_BYTES + 1],
            Duration::from_secs(1),
        )
        .expect_err("oversized stdin must fail before process creation");

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("stdin exceeded"));
        assert!(!marker.exists());
    }

    #[test]
    fn blocked_input_writer_remains_bounded_by_the_process_timeout() {
        let started_at = Instant::now();
        let error = command_output_with_input_and_timeout(
            Command::new("sh").args(["-c", "sleep 2"]),
            "non-reading stdin fixture",
            &vec![b'i'; DEFAULT_MAX_STDIN_BYTES],
            Duration::from_millis(100),
        )
        .expect_err("a non-reading child must time out without deadlocking the writer");

        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(started_at.elapsed() < Duration::from_secs(1));
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

    #[test]
    fn timeout_duration_format_uses_seconds_only_for_whole_seconds() {
        assert_eq!(format_duration(Duration::from_secs(2)), "2s");
        assert_eq!(format_duration(Duration::from_millis(250)), "250ms");
        assert_eq!(format_duration(Duration::from_millis(1_250)), "1250ms");
    }

    #[test]
    fn wait_with_output_timeout_kills_slow_existing_child() {
        let child = spawn(
            Command::new("sh")
                .args(["-c", "sleep 2"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )
        .expect("child should start");

        let error = wait_with_output_timeout(child, "sh -c sleep 2", Duration::from_millis(50))
            .expect_err("slow child should time out");

        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("timed out after 50ms"));
    }

    #[test]
    fn containment_failure_preserves_the_primary_timeout_error() {
        let primary = super::timeout_error("fork-churn fixture", Duration::from_millis(75));
        let error = super::preserve_primary_error_during_cleanup(
            primary,
            Err(std::io::Error::other(
                "Linux containment freeze did not converge",
            )),
        );

        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(
            error
                .to_string()
                .starts_with("command `fork-churn fixture` timed out after 75ms")
        );
        assert!(
            error
                .to_string()
                .contains("subprocess containment cleanup also failed")
        );
        assert!(error.to_string().contains("did not converge"));
    }

    #[cfg(unix)]
    #[test]
    fn explicit_termination_disarms_process_group_and_child_pid() {
        let mut child = spawn(
            Command::new("sh")
                .args(["-c", "sleep 30"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )
        .expect("long-lived child should start");

        assert!(child.process_group_id.is_some());
        assert!(!child.child_reaped);
        child
            .terminate_and_wait()
            .expect("explicit containment cleanup should succeed");
        assert!(child.process_group_id.is_none());
        assert!(child.child_reaped);

        // A second cleanup pass must be a state-only no-op, never another kill(PGID/PID).
        child
            .terminate_and_wait()
            .expect("repeated containment cleanup should be a no-op");
        assert!(child.process_group_id.is_none());
        assert!(child.child_reaped);
    }

    #[test]
    fn exited_parent_with_inherited_pipe_still_obeys_the_deadline() {
        let child = spawn(
            Command::new("sh")
                .args(["-c", "sleep 1 & printf parent-exited"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )
        .expect("parent with a background child should start");
        let started_at = Instant::now();

        let error = wait_with_output_timeout(
            child,
            "parent exits while a descendant owns the output pipe",
            Duration::from_millis(50),
        )
        .expect_err("an inherited pipe must not bypass the subprocess deadline");

        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(
            started_at.elapsed() < Duration::from_millis(500),
            "reader completion must remain inside the command deadline"
        );
    }

    #[cfg(unix)]
    #[test]
    fn timeout_terminates_long_lived_descendants_in_the_child_process_group() {
        let pid_file = std::env::temp_dir().join(format!(
            "akra-subprocess-descendant-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should follow Unix epoch")
                .as_nanos()
        ));
        let child = spawn(
            Command::new("sh")
                .args([
                    "-c",
                    "sleep 30 & descendant=$!; printf '%s' \"$descendant\" > \"$1\"; wait",
                    "akra-subprocess-tree-test",
                ])
                .arg(&pid_file)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )
        .expect("parent with a long-lived descendant should start");

        let error = wait_with_output_timeout(
            child,
            "parent with a long-lived descendant",
            Duration::from_millis(250),
        )
        .expect_err("the process tree fixture should time out");
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);

        let descendant_pid = std::fs::read_to_string(&pid_file)
            .expect("descendant PID should be published before the timeout")
            .parse::<libc::pid_t>()
            .expect("published descendant PID should be numeric");
        let _ = std::fs::remove_file(&pid_file);
        let gone_deadline = Instant::now() + Duration::from_secs(2);
        while unix_process_exists(descendant_pid) && Instant::now() < gone_deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !unix_process_exists(descendant_pid),
            "timed-out descendant process {descendant_pid} must not survive"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn successful_parent_cleanup_terminates_sets_id_double_fork_daemon() {
        let fixture_id = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should follow Unix epoch")
                .as_nanos()
        );
        let pid_file =
            std::env::temp_dir().join(format!("akra-subprocess-double-fork-{fixture_id}.pid"));
        let release_file =
            std::env::temp_dir().join(format!("akra-subprocess-double-fork-{fixture_id}.release"));
        let executable = std::env::current_exe().expect("current test executable should resolve");
        let child = spawn(
            Command::new(executable)
                .args([
                    "--exact",
                    "subprocess::tests::linux_double_fork_fixture_helper",
                    "--nocapture",
                ])
                .env("AKRA_TEST_DOUBLE_FORK_PID_FILE", &pid_file)
                .env("AKRA_TEST_DOUBLE_FORK_RELEASE_FILE", &release_file)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )
        .expect("the managed fixture leader should remain alive through identity capture");
        std::fs::write(&release_file, b"release")
            .expect("fixture release should be published after managed identity capture");
        let output_result = wait_with_output_timeout(
            child,
            "Linux setsid double-fork fixture",
            Duration::from_secs(3),
        );
        let _ = std::fs::remove_file(&release_file);
        let output = output_result
            .expect("the fixture parent should exit and its escaped daemon should be contained");
        assert!(output.status.success());

        let daemon_pid = std::fs::read_to_string(&pid_file)
            .expect("double-fork daemon PID should be published")
            .parse::<libc::pid_t>()
            .expect("published daemon PID should be numeric");
        let _ = std::fs::remove_file(&pid_file);
        let gone_deadline = Instant::now() + Duration::from_secs(2);
        while unix_process_exists(daemon_pid) && Instant::now() < gone_deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !unix_process_exists(daemon_pid),
            "setsid double-fork daemon {daemon_pid} must not escape successful parent cleanup"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_double_fork_fixture_helper() {
        use std::os::unix::ffi::OsStrExt;

        let Some(pid_file) = std::env::var_os("AKRA_TEST_DOUBLE_FORK_PID_FILE") else {
            return;
        };
        let release_file = std::env::var_os("AKRA_TEST_DOUBLE_FORK_RELEASE_FILE")
            .expect("nested double-fork fixture requires a release path");
        let pid_file = std::ffi::CString::new(pid_file.as_os_str().as_bytes())
            .expect("fixture PID path must not contain NUL");
        let release_file = std::ffi::CString::new(release_file.as_os_str().as_bytes())
            .expect("fixture release path must not contain NUL");

        // After fork, use only async-signal-safe libc operations. The test harness can have other
        // threads whose allocator/stdio locks must not be touched by the forked descendants.
        unsafe {
            let first_child = libc::fork();
            if first_child < 0 {
                libc::_exit(90);
            }
            if first_child == 0 {
                if libc::setsid() < 0 {
                    libc::_exit(91);
                }
                let daemon = libc::fork();
                if daemon < 0 {
                    libc::_exit(92);
                }
                if daemon > 0 {
                    libc::_exit(0);
                }
                libc::close(libc::STDIN_FILENO);
                libc::close(libc::STDOUT_FILENO);
                libc::close(libc::STDERR_FILENO);
                write_fixture_pid(pid_file.as_ptr());
                libc::sleep(30);
                libc::_exit(0);
            }

            // Keep the managed leader alive until its parent has captured PID/start-time identity
            // and the daemon has published its PID. It still exits normally, so cleanup must find
            // the now-reparented daemon by containment marker.
            for _ in 0..2_000 {
                if libc::access(pid_file.as_ptr(), libc::F_OK) == 0
                    && libc::access(release_file.as_ptr(), libc::F_OK) == 0
                {
                    libc::_exit(0);
                }
                libc::usleep(1_000);
            }
            libc::_exit(93);
        }
    }

    #[cfg(target_os = "linux")]
    unsafe fn write_fixture_pid(path: *const libc::c_char) {
        let descriptor = unsafe {
            libc::open(
                path,
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC,
                0o600,
            )
        };
        if descriptor < 0 {
            unsafe { libc::_exit(94) };
        }
        let mut digits = [0_u8; 32];
        let mut index = digits.len();
        let mut pid = unsafe { libc::getpid() } as u32;
        loop {
            index -= 1;
            digits[index] = b'0' + (pid % 10) as u8;
            pid /= 10;
            if pid == 0 {
                break;
            }
        }
        let length = digits.len() - index;
        if unsafe { libc::write(descriptor, digits[index..].as_ptr().cast(), length) }
            != length as isize
        {
            unsafe { libc::_exit(95) };
        }
        unsafe {
            libc::close(descriptor);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_stat_parser_uses_start_time_identity_with_tricky_command_names() {
        let stat = "321 (name with ) parens) S 7 321 321 0 -1 4194304 10 11 12 13 14 15 16 17 18 19 1 0 987654 0";
        let process = super::parse_linux_process_stat(stat).expect("stat record should parse");

        assert_eq!(process.identity.pid, 321);
        assert_eq!(process.state, 'S');
        assert_eq!(process.parent_pid, 7);
        assert_eq!(process.process_group_id, 321);
        assert_eq!(process.identity.start_time_ticks, 987654);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn late_member_at_old_final_scan_is_stopped_before_convergence() {
        use std::cell::RefCell;
        use std::collections::{HashSet, VecDeque};
        use std::rc::Rc;

        let first = linux_test_identity(101, 1_001);
        let late = linux_test_identity(202, 2_002);
        let mut snapshots = VecDeque::from([
            linux_test_snapshot([(first, 'R')]),
            linux_test_snapshot([(first, 'T')]),
            linux_test_snapshot([(first, 'T'), (late, 'R')]),
            linux_test_snapshot([(first, 'T'), (late, 'T')]),
            linux_test_snapshot([(first, 'T'), (late, 'T')]),
        ]);
        let events = Rc::new(RefCell::new(Vec::new()));
        let discovery_events = Rc::clone(&events);
        let stop_events = Rc::clone(&events);

        let report = super::freeze_linux_containment_with_hooks(
            HashSet::from([first]),
            8,
            move |_| {
                discovery_events.borrow_mut().push("discover".to_string());
                Ok(snapshots
                    .pop_front()
                    .expect("scripted discovery should not overrun"))
            },
            move |identity| {
                stop_events
                    .borrow_mut()
                    .push(format!("stop:{}", identity.pid));
                Ok(())
            },
        );

        assert!(report.converged);
        assert!(report.diagnostics.is_empty());
        assert_eq!(
            events
                .borrow()
                .iter()
                .filter(|event| event.as_str() == "discover")
                .count(),
            5,
            "an active late member must reset the stopped-set stabilization window"
        );
        assert!(
            events
                .borrow()
                .iter()
                .any(|event| event == &format!("stop:{}", late.pid)),
            "the member that used to appear only in the unsafe final scan must be stopped"
        );
        assert_eq!(report.tracked, HashSet::from([first, late]));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn continuous_fork_churn_exhausts_the_freeze_budget_as_an_error() {
        use std::collections::{HashSet, VecDeque};

        let identities = [
            linux_test_identity(301, 3_001),
            linux_test_identity(302, 3_002),
            linux_test_identity(303, 3_003),
            linux_test_identity(304, 3_004),
        ];
        let mut snapshots = VecDeque::from([
            linux_test_snapshot([(identities[0], 'R')]),
            linux_test_snapshot([(identities[0], 'T'), (identities[1], 'R')]),
            linux_test_snapshot([
                (identities[0], 'T'),
                (identities[1], 'T'),
                (identities[2], 'R'),
            ]),
            linux_test_snapshot([
                (identities[0], 'T'),
                (identities[1], 'T'),
                (identities[2], 'T'),
                (identities[3], 'R'),
            ]),
        ]);
        let mut stopped = HashSet::new();

        let report = super::freeze_linux_containment_with_hooks(
            HashSet::from([identities[0]]),
            4,
            |_| {
                Ok(snapshots
                    .pop_front()
                    .expect("fork-churn script should match the pass budget"))
            },
            |identity| {
                stopped.insert(identity);
                Ok(())
            },
        );

        assert!(!report.converged);
        assert!(!report.already_empty);
        assert_eq!(report.tracked, HashSet::from(identities));
        assert!(identities.iter().all(|identity| stopped.contains(identity)));
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.contains("did not converge within its 4-pass budget")),
            "pass exhaustion must remain an observable cleanup error: {:?}",
            report.diagnostics
        );
    }

    #[cfg(target_os = "linux")]
    fn linux_test_identity(pid: libc::pid_t, start_time_ticks: u64) -> super::LinuxProcessIdentity {
        super::LinuxProcessIdentity {
            pid,
            start_time_ticks,
        }
    }

    #[cfg(target_os = "linux")]
    fn linux_test_snapshot<const N: usize>(
        members: [(super::LinuxProcessIdentity, char); N],
    ) -> super::LinuxContainmentSnapshot {
        super::LinuxContainmentSnapshot {
            members: members
                .into_iter()
                .map(|(identity, state)| super::LinuxProcessInfo {
                    identity,
                    state,
                    parent_pid: 1,
                    process_group_id: 101,
                })
                .collect(),
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_scan_process_budget_exhaustion_is_an_observable_error() {
        let root = super::read_linux_process_info(
            libc::pid_t::try_from(std::process::id()).expect("test PID should fit pid_t"),
        )
        .expect("test process identity should be readable");
        let containment = super::LinuxContainment {
            marker: "budget-fixture".to_string(),
            root: root.identity,
        };
        let tracked = std::collections::HashSet::from([root.identity]);

        let error = super::discover_linux_containment_members(
            &containment,
            root.process_group_id,
            &tracked,
            Instant::now() + Duration::from_secs(1),
            super::LinuxScanLimits {
                max_processes: 0,
                max_environment_bytes: u64::MAX,
            },
        )
        .expect_err("exhausted PID budget must not look like an empty process tree");

        assert!(error.to_string().contains("aggregate 0-PID budget"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_scan_environment_budget_exhaustion_is_an_observable_error() {
        let mut budget = super::LinuxScanBudget::new(
            Instant::now() + Duration::from_secs(1),
            super::LinuxScanLimits {
                max_processes: usize::MAX,
                max_environment_bytes: 0,
            },
        );
        let pid = libc::pid_t::try_from(std::process::id()).expect("test PID should fit pid_t");

        let error = super::linux_process_has_marker(pid, b"missing-marker", &mut budget)
            .expect_err("exhausted environment budget must not look like a marker miss");

        assert!(
            error
                .to_string()
                .contains("aggregate 0-byte environment budget")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_scan_absolute_deadline_is_an_observable_timeout() {
        let mut budget = super::LinuxScanBudget::new(
            Instant::now(),
            super::LinuxScanLimits {
                max_processes: usize::MAX,
                max_environment_bytes: u64::MAX,
            },
        );

        let error = super::linux_process_ids(&mut budget)
            .expect_err("expired deadline must not look like an empty process tree");

        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("absolute sweep deadline"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn normal_short_lived_commands_use_bounded_containment_fast_path() {
        let started_at = Instant::now();
        for _ in 0..32 {
            let output = command_output_with_timeout(
                &mut Command::new("true"),
                "short-lived containment performance fixture",
                Duration::from_secs(1),
            )
            .expect("short-lived command should complete");
            assert!(output.status.success());
        }
        assert!(
            started_at.elapsed() < Duration::from_secs(3),
            "32 no-descendant commands should not incur repeated full process-tree sweeps"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn kill_fallback_rejects_stale_start_time_before_signaling_pid() {
        let mut process = Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("fallback identity fixture should start");
        let pid = libc::pid_t::try_from(process.id()).expect("fixture PID should fit pid_t");
        let identity = super::read_linux_process_info(pid)
            .expect("fixture identity should be readable")
            .identity;
        let stale_identity = super::LinuxProcessIdentity {
            start_time_ticks: identity.start_time_ticks.saturating_add(1),
            ..identity
        };

        super::signal_linux_identity_without_pidfd(stale_identity, libc::SIGKILL)
            .expect("stale PID identity must be a safe no-op");
        let status_after_stale_signal = process
            .try_wait()
            .expect("fixture status should be readable");
        if status_after_stale_signal.is_none() {
            super::signal_linux_identity_without_pidfd(identity, libc::SIGKILL)
                .expect("current PID identity should be signaled");
            process.wait().expect("signaled fixture should be reaped");
        }
        assert!(
            status_after_stale_signal.is_none(),
            "a mismatched start time must never signal the live process"
        );
    }

    #[cfg(unix)]
    fn unix_process_exists(pid: libc::pid_t) -> bool {
        if unsafe { libc::kill(pid, 0) } == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }

    #[test]
    fn command_output_drains_more_than_a_pipe_buffer_before_child_exit() {
        let output = command_output_with_timeout(
            Command::new("sh").args(["-c", "head -c 262144 /dev/zero"]),
            "large stdout fixture",
            Duration::from_secs(2),
        )
        .expect("concurrent draining should let a large-output child exit");

        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 262_144);
    }

    #[test]
    fn bounded_output_capture_fails_without_retaining_an_oversized_response() {
        let child = spawn(
            Command::new("sh")
                .args(["-c", "head -c 262144 /dev/zero"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )
        .expect("large-output child should start");

        let error = wait_with_output_timeout_and_limits(
            child,
            "bounded large stdout fixture",
            Duration::from_secs(2),
            4 * 1024,
            4 * 1024,
        )
        .expect_err("oversized stdout must fail closed");

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("stdout exceeded 4096 bytes"));
    }
}
