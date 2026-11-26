use std::io;
use std::mem::{size_of, size_of_val};
use std::os::windows::io::AsRawHandle;
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_BROKEN_PIPE, ERROR_INSUFFICIENT_BUFFER,
    GetLastError, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::SECURITY_CAPABILITIES;
use windows_sys::Win32::Storage::FileSystem::ReadFile;
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, IsProcessInJob, JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_JOB_MEMORY,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicAccountingInformation,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject,
};
use windows_sys::Win32::System::Pipes::{CreatePipe, PeekNamedPipe};
use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
    DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess,
    GetExitCodeProcess, GetProcessMitigationPolicy, InitializeProcThreadAttributeList,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY,
    PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_JOB_LIST, PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY,
    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, PROCESS_INFORMATION, ProcessChildProcessPolicy,
    ProcessSystemCallDisablePolicy, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW,
    TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};

use super::handles::Handle;
use super::security::{Identity, wide};
use super::storage::Workspace;
use crate::application::port::outbound::issue_code_validator_runner_port::{
    IssueCodeValidatorCancellation, IssueCodeValidatorNotRun as NotRun,
    IssueCodeValidatorRunOutcome as Outcome,
};

const MESSAGE_LIMIT: usize = 4096;
const MAX_DURATION: Duration = Duration::from_secs(60);
// Published UpdateProcThreadAttribute values (not all are exported by windows-sys).
const ALL_APPLICATION_PACKAGES_OPT_OUT: u32 = 1;
const CHILD_PROCESS_RESTRICTED: u32 = 1;
const WIN32K_SYSTEM_CALL_DISABLE_ALWAYS_ON: u64 = 1 << 28;

pub(super) fn run(
    identity: &Identity,
    workspace: &Workspace,
    timeout: Duration,
    cancellation: &IssueCodeValidatorCancellation,
) -> io::Result<Outcome> {
    let stdout = Pipe::new()?;
    let stderr = Pipe::new()?;
    let stdin = inherited_copy(workspace.input()?.as_raw_handle())?;
    let job = Job::new()?;
    let child = create_suspended(identity, workspace, &job, &stdin, &stdout, &stderr)?;
    drop(stdin);
    drop(stdout.child);
    drop(stderr.child);
    let result = (|| {
        verify_and_resume(identity, &child, &job)?;
        wait_bounded(
            &child,
            &job,
            &stdout.parent,
            &stderr.parent,
            timeout,
            cancellation,
        )
    })();
    // Timeout and user cancel both end here: the owned Job is terminated and observed empty.
    // Even successful exit is not publishable until the owned Job is empty.
    let cleanup = job.quiesce(child.process.raw());
    cleanup?;
    result
}

struct Child {
    process: Handle,
    thread: Handle,
}
impl Drop for Child {
    fn drop(&mut self) {
        // SAFETY: this is our process; termination also covers all pre-resume errors.
        unsafe {
            TerminateProcess(self.process.raw(), 0xefff);
            WaitForSingleObject(self.process.raw(), 5_000);
        }
    }
}

fn create_suspended(
    identity: &Identity,
    workspace: &Workspace,
    job: &Job,
    stdin: &Handle,
    stdout: &Pipe,
    stderr: &Pipe,
) -> io::Result<Child> {
    let application = wide(workspace.executable.as_os_str())?;
    let mut command = wide(format!("\"{}\"", workspace.executable.display()).as_ref())?;
    let directory = wide(workspace.path.as_os_str())?;
    let environment = environment(workspace)?;
    let capabilities = SECURITY_CAPABILITIES {
        AppContainerSid: identity.sid(),
        Capabilities: null_mut(),
        CapabilityCount: 0,
        Reserved: 0,
    };
    let packages = ALL_APPLICATION_PACKAGES_OPT_OUT;
    let children = CHILD_PROCESS_RESTRICTED;
    let mitigation = WIN32K_SYSTEM_CALL_DISABLE_ALWAYS_ON;
    let inherited = [stdin.raw(), stdout.child.raw(), stderr.child.raw()];
    let jobs = [job.0.raw()];
    // Values passed to UpdateProcThreadAttribute outlive this list (declaration order).
    let mut attributes = Attributes::new(6)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, &capabilities)?;
    attributes.set(
        PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY,
        &packages,
    )?;
    attributes.set(PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY, &children)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY, &mitigation)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, &inherited)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_JOB_LIST, &jobs)?;
    // SAFETY: zero is valid initial state for all pointer/integer fields in these Win32 structs.
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = stdin.raw();
    startup.StartupInfo.hStdOutput = stdout.child.raw();
    startup.StartupInfo.hStdError = stderr.child.raw();
    startup.lpAttributeList = attributes.pointer;
    // SAFETY: the process handles are populated only when process creation succeeds.
    let mut information: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: every buffer/attribute value is stable, terminated and live for the call;
    // only the three explicitly listed stdio handles can be inherited.
    if unsafe {
        CreateProcessW(
            application.as_ptr(),
            command.as_mut_ptr(),
            null(),
            null(),
            1,
            CREATE_SUSPENDED
                | CREATE_NO_WINDOW
                | CREATE_UNICODE_ENVIRONMENT
                | EXTENDED_STARTUPINFO_PRESENT,
            environment.as_ptr().cast(),
            directory.as_ptr(),
            &startup.StartupInfo,
            &mut information,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let process = Handle::new(information.hProcess)?;
    let thread = match Handle::new(information.hThread) {
        Ok(thread) => thread,
        Err(error) => {
            // SAFETY: creation succeeded but thread ownership was not established.
            unsafe {
                TerminateProcess(process.raw(), 0xefff);
                WaitForSingleObject(process.raw(), 5_000);
            }
            return Err(error);
        }
    };
    Ok(Child { process, thread })
}

fn environment(workspace: &Workspace) -> io::Result<Vec<u16>> {
    let mut system = [0u16; 32_768];
    // SAFETY: the buffer length is correctly supplied; no inherited SystemRoot is trusted.
    let length = unsafe { GetWindowsDirectoryW(system.as_mut_ptr(), system.len() as u32) } as usize;
    if length == 0 || length >= system.len() {
        return Err(io::Error::other("Windows system directory is unavailable"));
    }
    let system = String::from_utf16(&system[..length]).map_err(io::Error::other)?;
    let temp = workspace
        .path
        .to_str()
        .ok_or_else(|| io::Error::other("invalid workspace path"))?;
    let local = crate::private_fs::windows_local_app_data_path().map_err(io::Error::other)?;
    let local = local
        .to_str()
        .ok_or_else(|| io::Error::other("invalid local application data path"))?;
    let mut block = Vec::new();
    for entry in [
        format!("LOCALAPPDATA={local}"),
        format!("SystemRoot={system}"),
        format!("TEMP={temp}"),
        format!("TMP={temp}"),
    ] {
        block.extend(wide(entry.as_ref())?);
    }
    block.push(0);
    Ok(block)
}

fn verify_mitigation(process: HANDLE, policy: i32) -> io::Result<()> {
    let mut flags = 0u32;
    // Both documented policy structures contain one DWORD Flags; bit 0 enforces the restriction.
    // SAFETY: the output is one aligned DWORD with its exact size.
    if unsafe {
        GetProcessMitigationPolicy(
            process,
            policy,
            (&mut flags as *mut u32).cast(),
            size_of::<u32>(),
        )
    } == 0
        || flags & 1 == 0
    {
        return Err(io::Error::other(
            "required process mitigation is unavailable",
        ));
    }
    Ok(())
}

struct Attributes {
    pointer: LPPROC_THREAD_ATTRIBUTE_LIST,
    _storage: Vec<usize>,
}
impl Attributes {
    fn new(count: u32) -> io::Result<Self> {
        let mut size = 0usize;
        // SAFETY: the documented null first call reports the required allocation size.
        let first = unsafe { InitializeProcThreadAttributeList(null_mut(), count, 0, &mut size) };
        // SAFETY: GetLastError reads this thread's immediately preceding API error.
        if first != 0
            || unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER
            || size == 0
            || size > 64 * 1024
        {
            return Err(io::Error::other("process attributes are unavailable"));
        }
        let mut storage = vec![0usize; size.div_ceil(size_of::<usize>())];
        let pointer = storage.as_mut_ptr().cast();
        // SAFETY: usize storage has sufficient alignment and the requested capacity.
        if unsafe { InitializeProcThreadAttributeList(pointer, count, 0, &mut size) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            pointer,
            _storage: storage,
        })
    }
    fn set<T>(&mut self, attribute: u32, value: &T) -> io::Result<()> {
        // SAFETY: callers keep each immutable value alive until this list is deleted.
        if unsafe {
            UpdateProcThreadAttribute(
                self.pointer,
                0,
                attribute as usize,
                (value as *const T).cast(),
                size_of::<T>(),
                null_mut(),
                null(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: initialization succeeded and storage is still live.
        unsafe { DeleteProcThreadAttributeList(self.pointer) };
    }
}

struct Job(Handle);
impl Job {
    fn new() -> io::Result<Self> {
        // SAFETY: NULL security attributes produce a noninheritable, unnamed Job.
        let job = Self(Handle::new(unsafe { CreateJobObjectW(null(), null()) })?);
        // SAFETY: zero initializes all limits not explicitly enabled.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
            | JOB_OBJECT_LIMIT_JOB_MEMORY;
        limits.BasicLimitInformation.ActiveProcessLimit = 1;
        limits.JobMemoryLimit = 128 * 1024 * 1024;
        // SAFETY: handle and information buffer are valid and size matches the selected class.
        if unsafe {
            SetInformationJobObject(
                job.0.raw(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }
    fn verify_member(&self, process: HANDLE) -> io::Result<()> {
        let mut member = 0;
        // SAFETY: both handles identify live objects owned by this invocation.
        if unsafe { IsProcessInJob(process, self.0.raw(), &mut member) } == 0
            || member == 0
            || self.active()? != 1
        {
            return Err(io::Error::other(
                "validator Job association was not established",
            ));
        }
        Ok(())
    }
    fn active(&self) -> io::Result<u32> {
        // SAFETY: zero is a valid initial state for this output structure.
        let mut state: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
        let mut returned = 0u32;
        // SAFETY: output has exactly the selected structure size.
        if unsafe {
            QueryInformationJobObject(
                self.0.raw(),
                JobObjectBasicAccountingInformation,
                (&mut state as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                size_of_val(&state) as u32,
                &mut returned,
            )
        } == 0
            || returned as usize != size_of_val(&state)
        {
            return Err(io::Error::other("validator Job state is unavailable"));
        }
        Ok(state.ActiveProcesses)
    }
    fn quiesce(&self, process: HANDLE) -> io::Result<()> {
        if self.active()? != 0 {
            // SAFETY: only this invocation's private Job can be terminated.
            if unsafe { TerminateJobObject(self.0.raw(), 0xefff) } == 0 {
                return Err(io::Error::last_os_error());
            }
        }
        // SAFETY: process remains owned until after cleanup.
        if unsafe { WaitForSingleObject(process, 5_000) } != WAIT_OBJECT_0 {
            return Err(io::Error::other("validator termination was not observed"));
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.active()? != 0 {
            if Instant::now() >= deadline {
                return Err(io::Error::other("validator Job did not empty"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: failure cleanup is fail-closed; the owned handle also has KILL_ON_JOB_CLOSE.
        unsafe { TerminateJobObject(self.0.raw(), 0xefff) };
    }
}

struct Pipe {
    parent: Handle,
    child: Handle,
}
impl Pipe {
    fn new() -> io::Result<Self> {
        let mut read = null_mut();
        let mut write = null_mut();
        // SAFETY: noninheritable handles, bounded kernel pipe buffer, valid outputs.
        if unsafe { CreatePipe(&mut read, &mut write, null(), 4096) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let parent = Handle::new(read)?;
        let write = Handle::new(write)?;
        let child = inherited_copy(write.raw())?;
        Ok(Self { parent, child })
    }
}

fn inherited_copy(handle: HANDLE) -> io::Result<Handle> {
    let mut copy = null_mut();
    // SAFETY: GetCurrentProcess is only used for DuplicateHandle, never inherited.
    let current = unsafe { GetCurrentProcess() };
    // SAFETY: the source handle is live; this duplicate is the only inheritable copy.
    if unsafe {
        DuplicateHandle(
            current,
            handle,
            current,
            &mut copy,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Handle::new(copy)
}

fn drain(pipe: &Handle, output: &mut Vec<u8>, retain: bool) -> io::Result<()> {
    // A flooding child cannot starve deadline checks. Allocation never follows its output size.
    for _ in 0..4 {
        let mut available = 0;
        // SAFETY: a private readable pipe and valid out-pointer; no peek buffer.
        if unsafe {
            PeekNamedPipe(
                pipe.raw(),
                null_mut(),
                0,
                null_mut(),
                &mut available,
                null_mut(),
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) {
                Ok(())
            } else {
                Err(error)
            };
        }
        if available == 0 {
            return Ok(());
        }
        let mut bytes = [0u8; 4096];
        let mut read = 0;
        let count = available.min(bytes.len() as u32);
        // SAFETY: we read only bytes just reported available into a fixed output buffer.
        if unsafe { ReadFile(pipe.raw(), bytes.as_mut_ptr(), count, &mut read, null_mut()) } == 0 {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) {
                Ok(())
            } else {
                Err(error)
            };
        }
        if read > count {
            return Err(io::Error::other("invalid validator pipe length"));
        }
        if retain {
            let keep = (read as usize).min(MESSAGE_LIMIT.saturating_sub(output.len()));
            output.extend_from_slice(&bytes[..keep]);
        }
    }
    Ok(())
}

fn wait_bounded(
    child: &Child,
    job: &Job,
    stdout: &Handle,
    stderr: &Handle,
    timeout: Duration,
    cancellation: &IssueCodeValidatorCancellation,
) -> io::Result<Outcome> {
    let deadline = Instant::now() + timeout.min(MAX_DURATION);
    let mut message = Vec::new();
    let mut discarded = Vec::new();
    loop {
        drain(stdout, &mut message, true)?;
        drain(stderr, &mut discarded, false)?;
        // SAFETY: querying our process wait state does not execute child code.
        match unsafe { WaitForSingleObject(child.process.raw(), 0) } {
            WAIT_OBJECT_0 => {
                let mut code = 0;
                // SAFETY: the process is signalled and the output pointer is valid.
                if unsafe { GetExitCodeProcess(child.process.raw(), &mut code) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                #[cfg(test)]
                eprintln!("native validator exit code: {code:#010x}");
                job.quiesce(child.process.raw())?;
                drain(stdout, &mut message, true)?;
                return Ok(match code {
                    0 => Outcome::Passed,
                    1 => Outcome::Rejected {
                        message: bounded_message(&message),
                    },
                    _ => Outcome::NotRun(NotRun::Crashed),
                });
            }
            WAIT_TIMEOUT => {}
            _ => return Err(io::Error::other("validator process wait failed")),
        }
        if Instant::now() >= deadline {
            return Ok(Outcome::NotRun(NotRun::TimedOut));
        }
        if cancellation.is_cancelled() {
            return Ok(Outcome::NotRun(NotRun::Cancelled));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn bounded_message(bytes: &[u8]) -> String {
    let mut value = String::from_utf8_lossy(bytes).trim().to_owned();
    let mut end = value.len().min(MESSAGE_LIMIT);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value
}

fn verify_and_resume(identity: &Identity, child: &Child, job: &Job) -> io::Result<()> {
    identity.verify_process(child.process.raw())?;
    job.verify_member(child.process.raw())?;
    verify_mitigation(child.process.raw(), ProcessChildProcessPolicy)?;
    verify_mitigation(child.process.raw(), ProcessSystemCallDisablePolicy)?;
    // No instruction in the approved image runs before all required postconditions.
    // SAFETY: this thread was created suspended and has not previously been resumed.
    if unsafe { ResumeThread(child.thread.raw()) } != 1 {
        return Err(io::Error::other("validator thread could not be resumed"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{profile::Profile, storage, tests::Fixture};
    use super::*;

    #[test]
    fn native_windows_validator_rejected_token_never_resumes_and_owned_job_empties() {
        let fixture = Fixture::create();
        let request = fixture.request("pass");
        let content = storage::approved_content(&request).unwrap();
        let identity = Identity::create().unwrap();
        let wrong_identity = Identity::create().unwrap();
        let mut profile = Profile::create(&identity).unwrap();
        let mut workspace = Workspace::create(&identity, &content, &request.input_json).unwrap();
        let stdout = Pipe::new().unwrap();
        let stderr = Pipe::new().unwrap();
        let stdin = inherited_copy(workspace.input().unwrap().as_raw_handle()).unwrap();
        let job = Job::new().unwrap();
        let child =
            create_suspended(&identity, &workspace, &job, &stdin, &stdout, &stderr).unwrap();
        drop(stdin);
        drop(stdout.child);
        drop(stderr.child);
        assert_eq!(job.active().unwrap(), 1);
        assert!(verify_and_resume(&wrong_identity, &child, &job).is_err());
        job.quiesce(child.process.raw()).unwrap();
        assert_eq!(job.active().unwrap(), 0);
        let mut output = Vec::new();
        drain(&stdout.parent, &mut output, true).unwrap();
        assert!(
            output.is_empty(),
            "the fixture never reached its first instruction"
        );
        drop(child);
        workspace.cleanup().unwrap();
        profile.cleanup().unwrap();
        let local = crate::private_fs::windows_local_app_data_path().unwrap();
        assert!(!local.join("Temp").join(&identity.name).exists());
        assert!(!local.join("Packages").join(&identity.name).exists());
    }
}
