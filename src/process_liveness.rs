use anyhow::{Context, Result};

pub(crate) fn process_is_alive(pid: u32) -> Result<bool> {
    if pid == 0 {
        return Ok(false);
    }
    platform::process_is_alive(pid)
}

/// Returns a stable identity for one OS process lifetime when the platform can provide it.
/// `None` means that the process exited or that this platform cannot prove a birth identity.
pub(crate) fn process_start_identity(pid: u32) -> Result<Option<String>> {
    if pid == 0 {
        return Ok(None);
    }
    platform::process_start_identity(pid)
}

/// Returns the birth identity required when creating a new durable process owner.
///
/// Nullable identities remain readable for legacy on-disk ownership records, but a new owner
/// must never be written with PID-only fencing. This also makes unsupported platforms fail closed
/// instead of silently creating an ownership record that is vulnerable to PID reuse.
pub(crate) fn required_process_start_identity(pid: u32) -> Result<String> {
    resolve_required_process_start_identity(process_start_identity(pid))
        .with_context(|| format!("cannot establish birth identity for process {pid}"))
}

fn resolve_required_process_start_identity(probe: Result<Option<String>>) -> Result<String> {
    probe?.context("the operating system did not provide a process birth identity")
}

#[cfg(target_os = "linux")]
fn parse_linux_proc_starttime(stat: &str) -> Result<u64> {
    use anyhow::{Context, anyhow};

    let command_end = stat
        .rfind(')')
        .ok_or_else(|| anyhow!("Linux process stat has no command terminator"))?;
    let fields = stat
        .get(command_end.saturating_add(1)..)
        .context("Linux process stat command boundary is invalid")?
        .split_ascii_whitespace()
        .collect::<Vec<_>>();
    // The tail begins at field 3 (`state`), so field 22 (`starttime`) is index 19.
    fields
        .get(19)
        .context("Linux process stat has no starttime field")?
        .parse::<u64>()
        .context("Linux process stat starttime is invalid")
}

#[cfg(unix)]
mod platform {
    use anyhow::{Context, Result};

    pub(super) fn process_is_alive(pid: u32) -> Result<bool> {
        let pid = i32::try_from(pid).context("process id exceeds the Unix pid range")?;
        // SAFETY: signal 0 performs an existence/permission probe and does not signal the process.
        if unsafe { libc::kill(pid, 0) } == 0 {
            return Ok(true);
        }

        let error = std::io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::ESRCH) => Ok(false),
            Some(libc::EPERM) => Ok(true),
            _ => Err(error).context("failed to probe Unix process liveness"),
        }
    }

    pub(super) fn process_start_identity(pid: u32) -> Result<Option<String>> {
        #[cfg(target_os = "linux")]
        {
            let path = format!("/proc/{pid}/stat");
            let stat = match std::fs::read_to_string(&path) {
                Ok(stat) => stat,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => {
                    return Err(error).with_context(|| format!("failed to read {path}"));
                }
            };
            let starttime = super::parse_linux_proc_starttime(&stat)?;
            let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
                .context("failed to read the Linux boot identity")?;
            let boot_id = boot_id.trim();
            if boot_id.is_empty() || boot_id.chars().any(char::is_control) {
                anyhow::bail!("Linux boot identity is invalid");
            }
            Ok(Some(format!("linux-proc-starttime:{boot_id}:{starttime}")))
        }

        #[cfg(target_vendor = "apple")]
        {
            let pid = i32::try_from(pid).context("process id exceeds the macOS pid range")?;
            let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
            let size = std::mem::size_of::<libc::proc_bsdinfo>();
            let size_i32 = i32::try_from(size).context("macOS process info size overflowed")?;
            // SAFETY: proc_pidinfo writes at most `size` bytes to the valid output buffer.
            let written = unsafe {
                libc::proc_pidinfo(
                    pid,
                    libc::PROC_PIDTBSDINFO,
                    0,
                    info.as_mut_ptr().cast(),
                    size_i32,
                )
            };
            if written <= 0 {
                return Ok(None);
            }
            if usize::try_from(written).ok() != Some(size) {
                anyhow::bail!("macOS returned a partial process birth identity");
            }
            // SAFETY: proc_pidinfo reported that it initialized the full structure.
            let info = unsafe { info.assume_init() };
            Ok(Some(format!(
                "macos-bsd-starttime:{}:{}",
                info.pbi_start_tvsec, info.pbi_start_tvusec
            )))
        }

        #[cfg(not(any(target_os = "linux", target_vendor = "apple")))]
        {
            Ok(None)
        }
    }
}

#[cfg(windows)]
mod platform {
    use anyhow::Result;
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_INVALID_PARAMETER, FILETIME, GetLastError, HANDLE, STILL_ACTIVE,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    struct ProcessHandle(HANDLE);

    impl Drop for ProcessHandle {
        fn drop(&mut self) {
            // SAFETY: this wrapper is constructed only from a successful owned OpenProcess handle.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    pub(super) fn process_is_alive(pid: u32) -> Result<bool> {
        // SAFETY: OpenProcess does not borrow memory and returns an owned handle on success.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            // Access denial does not prove that the process exited. Unknown failures also fail
            // closed so a live owner is never reclaimed on an ambiguous Windows probe.
            return Ok(!matches!(
                unsafe { GetLastError() },
                ERROR_INVALID_PARAMETER
            ));
        }
        let handle = ProcessHandle(handle);
        let mut exit_code = 0_u32;
        // SAFETY: the process handle and output pointer remain valid for the duration of the call.
        if unsafe { GetExitCodeProcess(handle.0, &mut exit_code) } == 0 {
            return Ok(true);
        }
        Ok(exit_code == STILL_ACTIVE as u32)
    }

    pub(super) fn process_start_identity(pid: u32) -> Result<Option<String>> {
        // SAFETY: OpenProcess does not borrow memory and returns an owned handle on success.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return Ok(None);
        }
        let handle = ProcessHandle(handle);
        let mut creation = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut exit = creation;
        let mut kernel = creation;
        let mut user = creation;
        // SAFETY: the process handle and all FILETIME pointers are valid for this call.
        if unsafe { GetProcessTimes(handle.0, &mut creation, &mut exit, &mut kernel, &mut user) }
            == 0
        {
            return Ok(None);
        }
        let value = (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
        Ok(Some(format!("windows-filetime:{value}")))
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    use anyhow::Result;

    pub(super) fn process_is_alive(_pid: u32) -> Result<bool> {
        // An unsupported probe must fail closed: ambiguous ownership is treated as live.
        Ok(true)
    }

    pub(super) fn process_start_identity(_pid: u32) -> Result<Option<String>> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;

    use super::{
        process_is_alive, process_start_identity, resolve_required_process_start_identity,
    };

    #[test]
    fn current_process_is_alive_and_pid_zero_is_not_claimable() {
        assert!(process_is_alive(std::process::id()).expect("current process probe"));
        assert!(!process_is_alive(0).expect("zero pid probe"));
    }

    #[cfg(any(target_os = "linux", target_vendor = "apple", windows))]
    #[test]
    fn current_process_has_a_stable_start_identity() {
        let first = process_start_identity(std::process::id())
            .expect("current process identity probe")
            .expect("supported OS should expose process start identity");
        let second = process_start_identity(std::process::id())
            .expect("second current process identity probe")
            .expect("supported OS should expose process start identity");
        assert_eq!(first, second);
    }

    #[test]
    fn new_process_owners_fail_closed_when_birth_identity_is_ambiguous() {
        assert_eq!(
            resolve_required_process_start_identity(Ok(Some("stable-owner".to_string())))
                .expect("proven birth identity should be accepted"),
            "stable-owner"
        );
        let missing = resolve_required_process_start_identity(Ok(None))
            .expect_err("missing birth identity must reject a new durable owner");
        assert!(missing.to_string().contains("did not provide"));

        let failed = resolve_required_process_start_identity(Err(anyhow!("probe failed")))
            .expect_err("failed birth identity probe must reject a new durable owner");
        assert!(failed.to_string().contains("probe failed"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_proc_stat_parser_handles_spaces_and_parentheses_in_command() {
        let mut fields = vec!["S"; 20];
        fields[19] = "424242";
        let stat = format!("123 (worker ) with spaces) {}", fields.join(" "));
        assert_eq!(super::parse_linux_proc_starttime(&stat).unwrap(), 424242);
    }
}
