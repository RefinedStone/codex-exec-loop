//! Windows confirmation is a contained self-helper, never an executable from PATH.
use super::*;
use crate::application::port::outbound::vault_native_confirmation_port::VaultNativeConfirmationPurpose as Purpose;
use crate::application::port::outbound::vault_native_confirmation_port::helper_protocol::{
    self as protocol, MAX_LIFETIME_MILLIS, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES,
};
use std::io::{self, Read, Write};
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const POLL: Duration = Duration::from_millis(10);

pub(super) fn confirm(
    request: &VaultNativeConfirmationRequest,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> Outcome {
    let Ok(program) = trusted_self() else {
        return Outcome::Unavailable;
    };
    let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
        return Outcome::Cancelled;
    };
    let wire = protocol::Request::from_target(
        request,
        format!("{:032x}", rand::random::<u128>()),
        remaining
            .as_millis()
            .clamp(1, u128::from(MAX_LIFETIME_MILLIS)) as u32,
    );
    if wire.target().is_none() {
        return Outcome::Unavailable;
    }
    if cancelled.load(Ordering::SeqCst) || Instant::now() >= deadline {
        return Outcome::Cancelled;
    }
    let Ok(mut child) = crate::subprocess::spawn_without_console(&mut command_for(&program)) else {
        return Outcome::Unavailable;
    };
    let (Some(input), Some(output)) = (child.take_stdin(), child.take_stdout()) else {
        let _ = finish_child(child, None);
        return Outcome::Unavailable;
    };
    exchange(child, input, output, wire, cancelled, deadline)
}

fn trusted_self() -> anyhow::Result<std::path::PathBuf> {
    let executable = std::env::current_exe()?;
    let cwd = std::env::current_dir()?;
    let program = crate::trusted_executable::validate_absolute(&executable, &cwd)?;
    crate::trusted_executable::validate_native_executable(&program)?;
    Ok(program)
}

fn command_for(program: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .arg(protocol::COMMAND)
        .env_clear()
        .current_dir(program.parent().unwrap_or(program))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
}

/// Tests can supply a synthetic child pipe; production only takes the helper's stdout.
pub(crate) fn exchange<R: Read + AsRawHandle>(
    mut child: crate::subprocess::ManagedChild,
    input: ChildStdin,
    mut output: R,
    request: protocol::Request,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> Outcome {
    let Some(bytes) = protocol::encode(&request, MAX_REQUEST_BYTES) else {
        let _ = finish_child(child, None);
        return Outcome::Unavailable;
    };
    let (sent, receive) = mpsc::channel();
    // A stuck child's stdin must not turn even this bounded write into an unbounded wait.
    let writer = std::thread::spawn(move || {
        let mut input = input;
        let result = input.write_all(&bytes).and_then(|()| input.flush());
        let _ = sent.send((input, result));
    });
    let mut held_input = None;
    let mut response = Vec::new();
    let mut eof = false;
    let mut exit = None;
    let outcome = loop {
        if cancelled.load(Ordering::SeqCst) || Instant::now() >= deadline {
            break Outcome::Cancelled;
        }
        if held_input.is_none() {
            match receive.try_recv() {
                Ok((input, Ok(()))) => held_input = Some(input),
                Ok((_, Err(_))) | Err(mpsc::TryRecvError::Disconnected) => {
                    break Outcome::Unavailable;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if !eof {
            match read_available(&mut output, &mut response) {
                Ok(closed) => eof = closed,
                Err(_) => break Outcome::Unavailable,
            }
        }
        if response.contains(&b'\n') && protocol::decode_receipt(&response, &request).is_none() {
            break Outcome::Unavailable;
        }
        if exit.is_none() {
            match child.try_wait() {
                Ok(status) => exit = status,
                Err(_) => break Outcome::Unavailable,
            }
        }
        if let Some(status) = exit {
            if !status.success() {
                break Outcome::Unavailable;
            }
            if eof && held_input.is_some() {
                if cancelled.load(Ordering::SeqCst) || Instant::now() >= deadline {
                    break Outcome::Cancelled;
                }
                break protocol::decode_receipt(&response, &request)
                    .unwrap_or(Outcome::Unavailable);
            }
        }
        std::thread::sleep(POLL);
    };
    drop(held_input); // Notify the helper of EOF before terminating the contained process tree.
    drop(receive);
    if !finish_child(child, Some(writer)) {
        return Outcome::Unavailable;
    }
    if cancelled.load(Ordering::SeqCst) || Instant::now() >= deadline {
        Outcome::Cancelled
    } else {
        outcome
    }
}

fn finish_child(
    mut child: crate::subprocess::ManagedChild,
    writer: Option<std::thread::JoinHandle<()>>,
) -> bool {
    let cleanup = std::thread::spawn(move || {
        let cleaned = child.terminate_and_wait().is_ok();
        drop(child);
        cleaned
    });
    finish_workers(cleanup, writer, Duration::from_secs(2))
}

fn finish_workers(
    cleanup: std::thread::JoinHandle<bool>,
    writer: Option<std::thread::JoinHandle<()>>,
    lifetime: Duration,
) -> bool {
    let deadline = Instant::now() + lifetime;
    while (!cleanup.is_finished() || writer.as_ref().is_some_and(|worker| !worker.is_finished()))
        && Instant::now() < deadline
    {
        std::thread::sleep(POLL);
    }
    // Dropping an unfinished JoinHandle detaches only the cleanup worker. It keeps ownership of
    // the contained child; the caller never waits indefinitely or treats uncertain cleanup as approval.
    let cleaned = cleanup.is_finished() && cleanup.join().unwrap_or(false);
    let written = match writer {
        Some(writer) => writer.is_finished() && writer.join().is_ok(),
        None => true,
    };
    cleaned && written
}

fn read_available<R: Read + AsRawHandle>(
    output: &mut R,
    retained: &mut Vec<u8>,
) -> io::Result<bool> {
    let Some(available) = available_bytes(output)? else {
        return Ok(true);
    };
    if available == 0 {
        return Ok(false);
    }
    let remaining = MAX_RESPONSE_BYTES.saturating_sub(retained.len());
    if available as usize > remaining {
        return Err(io::Error::other(
            "native confirmation response exceeds its bound",
        ));
    }
    let mut bytes = [0_u8; MAX_RESPONSE_BYTES];
    let count = output.read(&mut bytes[..available as usize])?;
    retained.extend_from_slice(&bytes[..count]);
    Ok(count == 0)
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

/// Called only in the fixed private helper after its bounded request has been validated.
pub(crate) fn show_dialog(request: &VaultNativeConfirmationRequest) -> Outcome {
    use windows_sys::Win32::UI::WindowsAndMessaging::{IDCANCEL, IDOK, MessageBoxW};
    let (title, text) = dialog_text(request);
    let title = title.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let text = text.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let decision = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            dialog_flags(),
        )
    };
    match decision {
        IDOK => Outcome::Confirmed,
        IDCANCEL => Outcome::Cancelled,
        _ => Outcome::Unavailable,
    }
}

fn dialog_flags() -> u32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MB_DEFBUTTON2, MB_ICONWARNING, MB_OKCANCEL, MB_SETFOREGROUND,
    };
    MB_OKCANCEL | MB_DEFBUTTON2 | MB_ICONWARNING | MB_SETFOREGROUND
}

fn dialog_text(request: &VaultNativeConfirmationRequest) -> (&'static str, String) {
    match request.purpose {
        Purpose::ViewOriginal => (
            "Akra 원문 열람",
            format!(
                "이 이슈의 가려진 원문을 로컬 열람 세션으로 전달할까요?\n\n{}\n프로젝트: {}\n\n최대 5분 동안 열람할 수 있습니다. 같은 OS 계정의 화면 캡처나 접근성 도구는 원문을 볼 수 있습니다.\n\n계속하려면 확인을 선택하세요. 기본 선택은 취소입니다.",
                request.issue_url, request.workspace_dir,
            ),
        ),
        Purpose::RewriteBinding => (
            "Akra 복원 연결 저장",
            format!(
                "선택한 표식을 이 이슈의 현재 게시본에 다시 연결할까요?\n\n{}\n프로젝트: {}\n\n이 장치의 복원 연결을 저장합니다. 원격 이슈의 제목과 본문은 바꾸지 않습니다. 저장이 시작된 뒤 창을 닫으면 저장은 완료됐을 수 있습니다.\n\n연결을 저장하려면 확인을 선택하세요. 기본 선택은 취소입니다.",
                request.issue_url, request.workspace_dir,
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_native_confirmation_uses_only_fixed_argv_and_no_inherited_overrides() {
        let command = command_for(Path::new(r"C:\Program Files\Akra\akra-gateway.exe"));
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![std::ffi::OsStr::new(protocol::COMMAND)]
        );
        assert_eq!(command.get_envs().count(), 0);
        assert_eq!(
            command.get_current_dir(),
            Some(Path::new(r"C:\Program Files\Akra"))
        );
    }

    #[test]
    fn windows_native_cleanup_and_writer_waits_are_bounded_and_never_approve_unknown_cleanup() {
        for block_cleanup in [true, false] {
            let (release, wait) = mpsc::channel();
            let cleanup;
            let writer;
            if block_cleanup {
                cleanup = std::thread::spawn(move || {
                    let _ = wait.recv();
                    true
                });
                writer = Some(std::thread::spawn(|| {}));
            } else {
                cleanup = std::thread::spawn(|| true);
                writer = Some(std::thread::spawn(move || {
                    let _ = wait.recv();
                }));
            }
            let started = Instant::now();
            assert!(!finish_workers(cleanup, writer, Duration::from_millis(50)));
            assert!(started.elapsed() < Duration::from_secs(1));
            release.send(()).unwrap();
        }
    }

    #[test]
    fn windows_dialog_defaults_to_cancel_and_distinguishes_view_from_write() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MB_DEFBUTTON2, MB_OKCANCEL};
        assert_eq!(dialog_flags() & 0x0f, MB_OKCANCEL);
        assert_eq!(dialog_flags() & 0x0300, MB_DEFBUTTON2);
        let mut request = VaultNativeConfirmationRequest {
            issue_url: "https://github.com/acme/project/issues/1".into(),
            workspace_dir: r"C:\work\한글 project".into(),
            purpose: Purpose::ViewOriginal,
        };
        let view = dialog_text(&request);
        request.purpose = Purpose::RewriteBinding;
        let rewrite = dialog_text(&request);
        assert_ne!(view.0, rewrite.0);
        assert!(view.1.contains("원문"));
        assert!(rewrite.1.contains("연결을 저장"));
        assert!(rewrite.1.contains("저장은 완료됐을 수 있습니다"));
        for text in [view.1, rewrite.1] {
            assert!(text.contains(&request.issue_url));
            assert!(text.contains(&request.workspace_dir));
            assert!(text.contains("기본 선택은 취소"));
        }
    }
}
