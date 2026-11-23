use crate::domain::provider_credentials::{CredentialResult, ProviderCredentialError as Error};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{atomic::AtomicBool, mpsc};
use std::time::{Duration, Instant};
use zeroize::{Zeroize, Zeroizing};

pub(super) fn safe_environment(
    command: &mut Command,
    environment: impl Fn(&str) -> Option<OsString>,
) {
    command.env_clear();
    for name in [
        "SystemRoot",
        "WINDIR",
        "USERPROFILE",
        "LOCALAPPDATA",
        "APPDATA",
        "HOME",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
        "XAUTHORITY",
    ] {
        if let Some(value) = environment(name).filter(|value| {
            value.to_str().is_some_and(|value| {
                !value.is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control)
            })
        }) {
            command.env(name, value);
        }
    }
    #[cfg(unix)]
    command.env("PATH", "/usr/bin:/bin").current_dir("/");
}
pub(super) fn prompt_command(
    program: &Path,
    environment: impl Fn(&str) -> Option<OsString>,
) -> Command {
    let mut command = Command::new(program);
    safe_environment(&mut command, environment);
    command
        .arg("provider-credential-prompt")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
}
/// The reader never accumulates more than the fixed cap. Stderr is never captured or formatted.
/// Keep stdin open for the owned helper's lifetime; closing it signals owner death.
pub(super) fn capture(
    command: &mut Command,
    input: Vec<u8>,
    keep_input_open: bool,
    cancelled: &AtomicBool,
    deadline: Instant,
    cap: usize,
) -> CredentialResult<Zeroizing<Vec<u8>>> {
    super::check_live(cancelled, deadline)?;
    let mut child = crate::subprocess::spawn(command).map_err(|_| Error::Unavailable)?;
    let mut stdin = child.take_stdin().ok_or(Error::Unavailable)?;
    let mut stdout = child.take_stdout().ok_or(Error::Unavailable)?;
    let (input_tx, input_rx) = mpsc::sync_channel(1);
    let writer = std::thread::spawn(move || {
        let result = stdin
            .write_all(&input)
            .and_then(|_| stdin.flush())
            .map(|_| stdin);
        let _ = input_tx.send(result.map_err(|_| Error::Unavailable));
    });
    let (output_tx, output_rx) = mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let mut output = Zeroizing::new(Vec::new());
        let mut chunk = Zeroizing::new([0u8; 4096]);
        let result = loop {
            match stdout.read(&mut *chunk) {
                Ok(0) => break Ok(output),
                Ok(count) if output.len().saturating_add(count) <= cap => {
                    output.extend_from_slice(&chunk[..count]);
                    chunk.zeroize();
                }
                Ok(_) => break Err(Error::TooLarge),
                Err(_) => break Err(Error::Unavailable),
            }
        };
        let _ = output_tx.send(result);
    });
    let mut held_input = None;
    let mut written = false;
    let mut output = None;
    let result = (|| {
        loop {
            super::check_live(cancelled, deadline)?;
            if !written {
                match input_rx.try_recv() {
                    Ok(Ok(stdin)) => {
                        written = true;
                        if keep_input_open {
                            held_input = Some(stdin);
                        }
                    }
                    Ok(Err(error)) => return Err(error),
                    Err(mpsc::TryRecvError::Disconnected) => return Err(Error::Unavailable),
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }
            if output.is_none() {
                match output_rx.try_recv() {
                    Ok(result) => output = Some(result?),
                    Err(mpsc::TryRecvError::Disconnected) => return Err(Error::Unavailable),
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }
            match child.try_wait().map_err(|_| Error::Unavailable)? {
                Some(status) if !status.success() => return Err(Error::Cancelled),
                Some(_) if written && output.is_some() => return Ok(output.take().unwrap()),
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    })();
    drop(held_input);
    // ManagedChild retains ownership in the cleanup worker even if an OS kill/wait fails or stalls.
    // A failed reap does not prove that inherited pipes are closed; never join blocked pipe workers.
    finish_io_workers(
        move || child.terminate_and_wait().map_err(|_| Error::Unavailable),
        writer,
        reader,
        Duration::from_secs(2),
    )?;
    result
}
fn finish_io_workers(
    cleanup: impl FnOnce() -> CredentialResult<()> + Send + 'static,
    writer: std::thread::JoinHandle<()>,
    reader: std::thread::JoinHandle<()>,
    timeout: Duration,
) -> CredentialResult<()> {
    let deadline = Instant::now() + timeout;
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = tx.send(cleanup());
    });
    match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(Ok(())) => {}
        // Dropping JoinHandles detaches; their zeroizing buffers are erased when the pipes eventually close.
        _ => return Err(Error::Unavailable),
    }
    while !writer.is_finished() || !reader.is_finished() {
        if Instant::now() >= deadline {
            return Err(Error::Unavailable);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    if writer.join().is_err() || reader.join().is_err() {
        return Err(Error::Unavailable);
    }
    Ok(())
}
#[cfg(test)]
mod cleanup_tests {
    use super::*;
    fn blocked_worker() -> (
        std::thread::JoinHandle<()>,
        mpsc::Sender<()>,
        mpsc::Receiver<()>,
    ) {
        let (release_tx, release_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = release_rx.recv();
            let _ = done_tx.send(());
        });
        (worker, release_tx, done_rx)
    }
    #[test]
    fn cleanup_error_never_joins_workers_that_still_hold_private_pipes() {
        let (writer, release_writer, writer_done) = blocked_worker();
        let (reader, release_reader, reader_done) = blocked_worker();
        assert_eq!(
            finish_io_workers(
                || Err(Error::Unavailable),
                writer,
                reader,
                Duration::from_millis(50)
            ),
            Err(Error::Unavailable)
        );
        drop(release_writer);
        drop(release_reader);
        writer_done.recv_timeout(Duration::from_secs(1)).unwrap();
        reader_done.recv_timeout(Duration::from_secs(1)).unwrap();
    }
    #[test]
    fn a_stalled_reap_and_a_successful_reap_with_stalled_io_are_both_bounded() {
        for stalled_cleanup in [false, true] {
            let (writer, release_writer, writer_done) = blocked_worker();
            let (reader, release_reader, reader_done) = blocked_worker();
            let (release_cleanup, wait_cleanup) = mpsc::channel::<()>();
            let result = finish_io_workers(
                move || {
                    if stalled_cleanup {
                        let _ = wait_cleanup.recv();
                    }
                    Ok(())
                },
                writer,
                reader,
                Duration::from_millis(25),
            );
            assert_eq!(result, Err(Error::Unavailable));
            drop(release_cleanup);
            drop(release_writer);
            drop(release_reader);
            writer_done.recv_timeout(Duration::from_secs(1)).unwrap();
            reader_done.recv_timeout(Duration::from_secs(1)).unwrap();
        }
    }
}
