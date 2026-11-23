//! Fixed Windows self-helper. Only inherited pipes carry its request and receipt.
//! No public RPC, argv field, or environment variable can confirm a request.
use crate::application::port::outbound::vault_native_confirmation_port::{
    VaultNativeConfirmationOutcome as Outcome, VaultNativeConfirmationRequest,
    helper_protocol::{self as protocol, Decision, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES},
};
use std::io::{self, Read, Write};
use std::os::windows::io::AsRawHandle;
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(10);
const FIRST_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Called before diagnostics and daemon initialization in the Gateway binary.
pub fn run_if_requested() -> Option<ExitCode> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.first().is_none_or(|arg| arg != protocol::COMMAND) {
        return None;
    }
    if args.len() != 1 {
        return Some(ExitCode::FAILURE);
    }
    let input = std::io::stdin();
    let output = std::io::stdout();
    if !is_pipe(&input) || !is_pipe(&output) {
        return Some(ExitCode::FAILURE);
    }
    Some(ExitCode::from(run_with_pipes(
        input,
        output,
        crate::composition::desktop_restore::native_confirmation_dialog,
    )))
}

fn is_pipe(pipe: &impl AsRawHandle) -> bool {
    use windows_sys::Win32::Storage::FileSystem::{FILE_TYPE_PIPE, GetFileType};
    unsafe { GetFileType(pipe.as_raw_handle()) == FILE_TYPE_PIPE }
}

pub(crate) fn run_with_pipes<R, W, P>(mut input: R, mut output: W, prompt: P) -> u8
where
    R: Read + AsRawHandle,
    W: Write,
    P: FnOnce(&VaultNativeConfirmationRequest) -> Outcome + Send + 'static,
{
    let Some(request) = read_request(&mut input, Instant::now() + FIRST_REQUEST_TIMEOUT) else {
        return 1;
    };
    let Some(target) = request.target() else {
        return 1;
    };
    let deadline = Instant::now() + Duration::from_millis(u64::from(request.lifetime_millis));
    // The main thread owns EOF/deadline checks while a worker shows the native dialog.
    // Returning from this fixed process entry point closes a stuck dialog with the process.
    let (sent, received) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sent.send(prompt(&target));
    });
    let decision = loop {
        if Instant::now() >= deadline {
            break Decision::Cancelled;
        }
        match available_bytes(&input) {
            Ok(Some(0)) => {}
            Ok(None) => break Decision::Cancelled,
            Ok(Some(_)) | Err(_) => break Decision::Unavailable,
        }
        match received.try_recv() {
            Ok(outcome) => {
                // An approval racing parent EOF or the deadline is never emitted.
                if Instant::now() >= deadline || !matches!(available_bytes(&input), Ok(Some(0))) {
                    break Decision::Cancelled;
                }
                break match outcome {
                    Outcome::Confirmed => Decision::Confirmed,
                    Outcome::Cancelled => Decision::Cancelled,
                    Outcome::Unavailable => Decision::Unavailable,
                    #[cfg(test)]
                    Outcome::Unsupported => Decision::Unavailable,
                };
            }
            Err(mpsc::TryRecvError::Disconnected) => break Decision::Unavailable,
            Err(mpsc::TryRecvError::Empty) => std::thread::sleep(POLL),
        }
    };
    let Some(bytes) = protocol::encode(
        &protocol::Receipt::new(&request, decision),
        MAX_RESPONSE_BYTES,
    ) else {
        return 1;
    };
    if output
        .write_all(&bytes)
        .and_then(|()| output.flush())
        .is_ok()
    {
        0
    } else {
        1
    }
}

fn read_request(
    input: &mut (impl Read + AsRawHandle),
    deadline: Instant,
) -> Option<protocol::Request> {
    let mut bytes = Vec::new();
    loop {
        if Instant::now() >= deadline {
            return None;
        }
        let available = available_bytes(input).ok()?? as usize;
        if available > MAX_REQUEST_BYTES.saturating_sub(bytes.len()) {
            return None;
        }
        if available == 0 {
            std::thread::sleep(POLL);
            continue;
        }
        let mut part = [0_u8; 1024];
        let limit = available.min(part.len());
        let count = input.read(&mut part[..limit]).ok()?;
        if count == 0 {
            return None;
        }
        bytes.extend_from_slice(&part[..count]);
        if bytes.contains(&b'\n') {
            let request = protocol::decode_request(&bytes)?;
            // Reject pipelined requests before any dialog starts.
            if !matches!(available_bytes(input), Ok(Some(0))) {
                return None;
            }
            return Some(request);
        }
    }
}

fn available_bytes(pipe: &impl AsRawHandle) -> io::Result<Option<u32>> {
    use windows_sys::Win32::System::Pipes::PeekNamedPipe;
    let mut available = 0;
    let success = unsafe {
        PeekNamedPipe(
            pipe.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            &mut available,
            std::ptr::null_mut(),
        )
    };
    if success != 0 {
        return Ok(Some(available));
    }
    let error = io::Error::last_os_error();
    if matches!(error.raw_os_error(), Some(109 | 232 | 233)) {
        Ok(None)
    } else {
        Err(error)
    }
}
