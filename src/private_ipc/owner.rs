//! Verify the OS account on the connected stream before sending credential bytes.
use anyhow::{Result, bail};

#[cfg(unix)]
pub(crate) type PrivateIpcClientConnection = tokio::net::UnixStream;
#[cfg(windows)]
pub(crate) type PrivateIpcClientConnection = tokio::net::windows::named_pipe::NamedPipeClient;

pub(crate) async fn connect_owner_checked(
    endpoint: &str,
    deadline: tokio::time::Instant,
) -> Result<PrivateIpcClientConnection> {
    #[cfg(unix)]
    let stream = tokio::time::timeout_at(deadline, tokio::net::UnixStream::connect(endpoint))
        .await
        .map_err(|_| anyhow::anyhow!("private IPC connection timed out"))?
        .map_err(|_| anyhow::anyhow!("private IPC connection is unavailable"))?;
    #[cfg(windows)]
    let stream = {
        loop {
            if tokio::time::Instant::now() >= deadline {
                bail!("private IPC connection timed out");
            }
            match tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint) {
                Ok(stream) => break stream,
                Err(error) if error.raw_os_error() == Some(231) => {
                    tokio::time::sleep_until(
                        (tokio::time::Instant::now() + std::time::Duration::from_millis(10))
                            .min(deadline),
                    )
                    .await;
                }
                Err(_) => bail!("private IPC connection is unavailable"),
            }
        }
    };
    verify_server_owner(&stream)?;
    Ok(stream)
}

#[cfg(unix)]
fn verify_unix_owner(stream: &tokio::net::UnixStream) -> Result<()> {
    let credentials = stream
        .peer_cred()
        .map_err(|_| anyhow::anyhow!("private IPC peer identity is unavailable"))?;
    // SAFETY: geteuid takes no arguments and returns the current effective identity.
    require_same_uid(credentials.uid(), unsafe { libc::geteuid() })
}
#[cfg(unix)]
fn require_same_uid(actual: u32, expected: u32) -> Result<()> {
    if actual != expected {
        bail!("private IPC peer belongs to another OS account");
    }
    Ok(())
}

pub(crate) fn verify_client_owner(stream: &super::ClientConnection) -> Result<()> {
    #[cfg(unix)]
    {
        verify_unix_owner(stream)
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        let mut process = 0;
        // SAFETY: the connected server handle and the process-id output remain valid.
        if unsafe {
            windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId(
                stream.as_raw_handle().cast(),
                &mut process,
            )
        } == 0
        {
            bail!("private IPC peer identity is unavailable");
        }
        windows::verify_process_owner(process)
    }
}

pub(crate) fn verify_server_owner(stream: &PrivateIpcClientConnection) -> Result<()> {
    #[cfg(unix)]
    {
        verify_unix_owner(stream)
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        let mut process = 0;
        // SAFETY: the connected client handle and the process-id output remain valid.
        if unsafe {
            windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId(
                stream.as_raw_handle().cast(),
                &mut process,
            )
        } == 0
        {
            bail!("private IPC peer identity is unavailable");
        }
        windows::verify_process_owner(process)
    }
}

/// A secret helper never writes to a terminal or an ordinary redirected file.
pub(crate) fn require_inherited_pipes() -> Result<()> {
    #[cfg(unix)]
    {
        for descriptor in [libc::STDIN_FILENO, libc::STDOUT_FILENO] {
            let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
            // SAFETY: fstat initializes the supplied storage on success.
            if unsafe { libc::fstat(descriptor, metadata.as_mut_ptr()) } != 0 {
                bail!("credential helper requires inherited private pipes");
            }
            // SAFETY: fstat succeeded.
            let mode = unsafe { metadata.assume_init() }.st_mode & libc::S_IFMT;
            if mode != libc::S_IFIFO && mode != libc::S_IFSOCK {
                bail!("credential helper requires inherited private pipes");
            }
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::Storage::FileSystem::{FILE_TYPE_PIPE, GetFileType};
        use windows_sys::Win32::System::Console::{
            GetStdHandle, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        };
        for kind in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE] {
            // SAFETY: GetStdHandle borrows the process's configured standard handle.
            let handle = unsafe { GetStdHandle(kind) };
            if handle.is_null()
                || handle == INVALID_HANDLE_VALUE
                || unsafe { GetFileType(handle) } != FILE_TYPE_PIPE
            {
                bail!("credential helper requires inherited private pipes");
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::Foundation::{HANDLE, LocalFree};
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{
        GetTokenInformation, IsValidSid, TOKEN_QUERY, TOKEN_USER, TokenUser,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    pub(super) fn verify_process_owner(process_id: u32) -> Result<()> {
        if process_id == 0 {
            bail!("private IPC peer identity is unavailable");
        }
        // SAFETY: OpenProcess returns a newly owned handle or null.
        let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
        if raw.is_null() {
            bail!("private IPC peer identity is unavailable");
        }
        // SAFETY: raw is a successful newly owned process handle.
        let process = unsafe { OwnedHandle::from_raw_handle(raw.cast()) };
        let actual = process_sid(process.as_raw_handle().cast())?;
        require_same_sid(
            &actual,
            &crate::private_fs::windows_current_user_sid_string()?,
        )
    }

    fn require_same_sid(actual: &str, expected: &str) -> Result<()> {
        if actual != expected {
            bail!("private IPC peer belongs to another OS account");
        }
        Ok(())
    }

    fn process_sid(process: HANDLE) -> Result<String> {
        let mut raw_token = std::ptr::null_mut();
        // SAFETY: process is valid and output points to owned-handle storage.
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut raw_token) } == 0 {
            bail!("private IPC peer identity is unavailable");
        }
        // SAFETY: OpenProcessToken succeeded and transferred this handle.
        let token = unsafe { OwnedHandle::from_raw_handle(raw_token.cast()) };
        let mut required = 0;
        // SAFETY: documented null-buffer probe, with valid required-size storage.
        unsafe {
            GetTokenInformation(
                token.as_raw_handle().cast(),
                TokenUser,
                std::ptr::null_mut(),
                0,
                &mut required,
            );
        }
        if required < std::mem::size_of::<TOKEN_USER>() as u32 || required > 64 * 1024 {
            bail!("private IPC peer identity is unavailable");
        }
        let mut buffer = vec![0usize; (required as usize).div_ceil(std::mem::size_of::<usize>())];
        // SAFETY: aligned buffer is at least required bytes.
        if unsafe {
            GetTokenInformation(
                token.as_raw_handle().cast(),
                TokenUser,
                buffer.as_mut_ptr().cast(),
                required,
                &mut required,
            )
        } == 0
        {
            bail!("private IPC peer identity is unavailable");
        }
        // SAFETY: successful TokenUser query returned TOKEN_USER in the aligned buffer.
        let sid = unsafe { (*(buffer.as_ptr().cast::<TOKEN_USER>())).User.Sid };
        if sid.is_null() || unsafe { IsValidSid(sid) } == 0 {
            bail!("private IPC peer identity is unavailable");
        }
        let mut text: *mut u16 = std::ptr::null_mut();
        // SAFETY: sid remains valid while buffer is alive.
        if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
            bail!("private IPC peer identity is unavailable");
        }
        // SAFETY: ConvertSidToStringSidW returns a NUL-terminated allocation.
        let length = unsafe { (0..).take_while(|&index| *text.add(index) != 0).count() };
        let value = String::from_utf16(unsafe { std::slice::from_raw_parts(text, length) });
        // SAFETY: the allocation was returned by ConvertSidToStringSidW.
        unsafe {
            LocalFree(text.cast());
        }
        value.map_err(|_| anyhow::anyhow!("private IPC peer identity is unavailable"))
    }

    #[test]
    fn process_owner_checks_current_user_and_rejects_a_different_sid() {
        verify_process_owner(std::process::id()).unwrap();
        assert!(require_same_sid("S-1-5-21-1", "S-1-5-21-2").is_err());
        assert!(verify_process_owner(0).is_err());
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[tokio::test]
    async fn owner_check_accepts_self_and_rejects_another_uid() {
        let (first, _second) = tokio::net::UnixStream::pair().unwrap();
        verify_server_owner(&first).unwrap();
        assert!(require_same_uid(1, 2).is_err());
    }
}
