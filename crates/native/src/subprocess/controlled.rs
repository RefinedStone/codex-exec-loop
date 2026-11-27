//! Bounded capture with cooperative cancellation. Uses the same ManagedChild
//! containment and pipe readers as ordinary host subprocesses.
use super::*;

#[derive(Debug)]
pub enum ControlledTermination {
    Exited(ExitStatus),
    TimedOut,
    Cancelled,
    OutputLimit,
    IoFailure,
}

pub struct ControlledOutput {
    pub termination: ControlledTermination,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_complete: bool,
    pub stderr_complete: bool,
}

pub fn wait_with_output_controlled(
    mut child: ManagedChild,
    deadline: Instant,
    cancelled: &dyn Fn() -> bool,
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
) -> ControlledOutput {
    let stdout = BoundedPipeReader::new(
        child.take_stdout().map(CapturedPipe::Stdout),
        max_stdout_bytes,
    );
    let stderr = BoundedPipeReader::new(
        child.take_stderr().map(CapturedPipe::Stderr),
        max_stderr_bytes,
    );
    let (Ok(mut stdout), Ok(mut stderr)) = (stdout, stderr) else {
        let _ = child.terminate_and_wait();
        return ControlledOutput {
            termination: ControlledTermination::IoFailure,
            stdout: Vec::new(),
            stderr: Vec::new(),
            stdout_complete: false,
            stderr_complete: false,
        };
    };
    let mut status = None;
    let mut termination = loop {
        if cancelled() {
            break ControlledTermination::Cancelled;
        }
        if Instant::now() >= deadline {
            break ControlledTermination::TimedOut;
        }
        let overflow = match (stdout.drain_available(), stderr.drain_available()) {
            (Ok(a), Ok(b)) => a || b,
            _ => break ControlledTermination::IoFailure,
        };
        if overflow {
            break ControlledTermination::OutputLimit;
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(next) => status = next,
                Err(_) => break ControlledTermination::IoFailure,
            }
        }
        if let Some(completed) = status
            && stdout.is_complete()
            && stderr.is_complete()
        {
            break ControlledTermination::Exited(completed);
        }
        if child
            .wait_for_progress(
                POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
            )
            .is_err()
        {
            break ControlledTermination::IoFailure;
        }
    };
    let cleanup = if matches!(termination, ControlledTermination::Exited(_)) {
        child.finish_success()
    } else {
        child.terminate_and_wait()
    };
    if cleanup.is_err() {
        termination = ControlledTermination::IoFailure;
    }
    let completed = matches!(termination, ControlledTermination::Exited(_));
    let stdout_complete = completed && stdout.is_complete();
    let stderr_complete = completed && stderr.is_complete();
    let mut stdout = stdout.into_retained();
    let mut stderr = stderr.into_retained();
    stdout.truncate(max_stdout_bytes);
    stderr.truncate(max_stderr_bytes);
    ControlledOutput {
        termination,
        stdout,
        stderr,
        stdout_complete,
        stderr_complete,
    }
}

#[cfg(test)]
mod tests;
