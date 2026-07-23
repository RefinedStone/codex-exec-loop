use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use crate::core::app::{CoreEffectCompletion, CoreInput};
use crate::core::runtime::CoreInputSender;
use crate::panic_observation::catch_redacted_worker_unwind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RedactedWorkerPanic;

pub(crate) struct RedactedWorkerHandle<Output> {
    worker: JoinHandle<()>,
    outcome_receiver: mpsc::Receiver<Result<Output, RedactedWorkerPanic>>,
}

impl<Output> RedactedWorkerHandle<Output> {
    pub(crate) fn join(self) -> Result<Output, RedactedWorkerPanic> {
        if self.worker.join().is_err() {
            return Err(RedactedWorkerPanic);
        }
        self.outcome_receiver
            .recv()
            .unwrap_or(Err(RedactedWorkerPanic))
    }
}

pub(crate) fn spawn_effect_completion_worker<Work>(
    input_sender: CoreInputSender,
    panic_completion: CoreEffectCompletion,
    work: Work,
) -> JoinHandle<()>
where
    Work: FnOnce() -> CoreEffectCompletion + Send + 'static,
{
    spawn_effect_completion_worker_with_recovery(input_sender, panic_completion, || {}, work)
}

pub(crate) fn spawn_effect_completion_worker_with_recovery<Work, Recover>(
    input_sender: CoreInputSender,
    panic_completion: CoreEffectCompletion,
    recover: Recover,
    work: Work,
) -> JoinHandle<()>
where
    Work: FnOnce() -> CoreEffectCompletion + Send + 'static,
    Recover: FnOnce() + Send + 'static,
{
    spawn_guarded_completion_job(panic_completion, recover, work, move |completion| {
        let _ = input_sender.send(CoreInput::EffectCompleted(completion));
    })
}

pub(crate) fn spawn_worker_with_panic_fallback<Work, Recover>(
    work: Work,
    recover: Recover,
) -> JoinHandle<()>
where
    Work: FnOnce() + Send + 'static,
    Recover: FnOnce() + Send + 'static,
{
    spawn_guarded_job(work, move |result| {
        if result.is_err() {
            let _ = catch_redacted_worker_unwind(recover);
        }
    })
}

pub(crate) fn spawn_joinable_redacted_worker<Work, Output>(
    work: Work,
) -> RedactedWorkerHandle<Output>
where
    Work: FnOnce() -> Output + Send + 'static,
    Output: Send + 'static,
{
    let (outcome_sender, outcome_receiver) = mpsc::channel();
    let worker = spawn_guarded_job(work, move |result| {
        let _ = outcome_sender.send(result);
    });
    RedactedWorkerHandle {
        worker,
        outcome_receiver,
    }
}

fn spawn_guarded_completion_job<Completion, Work, Recover, Publish>(
    panic_completion: Completion,
    recover: Recover,
    work: Work,
    publish: Publish,
) -> JoinHandle<()>
where
    Completion: Send + 'static,
    Work: FnOnce() -> Completion + Send + 'static,
    Recover: FnOnce() + Send + 'static,
    Publish: FnOnce(Completion) + Send + 'static,
{
    spawn_guarded_job(work, move |result| {
        let completion = match result {
            Ok(completion) => completion,
            Err(_) => {
                let _ = catch_redacted_worker_unwind(recover);
                panic_completion
            }
        };
        publish(completion);
    })
}

fn spawn_guarded_job<Output, Work, Settle>(work: Work, settle: Settle) -> JoinHandle<()>
where
    Output: Send + 'static,
    Work: FnOnce() -> Output + Send + 'static,
    Settle: FnOnce(Result<Output, RedactedWorkerPanic>) + Send + 'static,
{
    thread::spawn(move || {
        let result = catch_redacted_worker_unwind(work).map_err(|_| RedactedWorkerPanic);
        let _ = catch_redacted_worker_unwind(|| settle(result));
    })
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    fn assert_exactly_one_completion(
        work: impl FnOnce() -> Result<&'static str, &'static str> + Send + 'static,
        expected: Result<&'static str, &'static str>,
    ) {
        let (completion_tx, completion_rx) = mpsc::channel();
        let worker = spawn_guarded_completion_job(
            Err("panic"),
            || {},
            work,
            move |completion| {
                completion_tx
                    .send(completion)
                    .expect("completion receiver should remain connected");
            },
        );

        worker
            .join()
            .expect("guarded worker should settle its panic");
        assert_eq!(
            completion_rx
                .recv()
                .expect("guarded worker should publish one completion"),
            expected
        );
        assert!(matches!(
            completion_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn guarded_completion_job_publishes_success_failure_and_panic_exactly_once() {
        assert_exactly_one_completion(|| Ok("ready"), Ok("ready"));
        assert_exactly_one_completion(|| Err("provider failed"), Err("provider failed"));
        assert_exactly_one_completion(|| panic!("SENSITIVE-PANIC-PAYLOAD"), Err("panic"));
    }

    #[test]
    fn recovery_panic_cannot_replace_or_omit_the_prebuilt_completion() {
        let (completion_sender, completion_receiver) = mpsc::channel();
        let worker = spawn_guarded_completion_job(
            "typed panic completion",
            || panic!("SENSITIVE-RECOVERY-PANIC"),
            || panic!("SENSITIVE-WORK-PANIC"),
            move |completion| {
                completion_sender
                    .send(completion)
                    .expect("completion receiver should remain connected");
            },
        );

        worker
            .join()
            .expect("recovery panic must remain inside the redacted boundary");
        assert_eq!(completion_receiver.recv(), Ok("typed panic completion"));
        assert!(matches!(
            completion_receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn detached_and_joinable_workers_share_the_redacted_panic_boundary() {
        let (recovery_sender, recovery_receiver) = mpsc::channel();
        let detached = spawn_worker_with_panic_fallback(
            || -> () { panic!("SENSITIVE-OUTER-WORKER-PANIC") },
            move || {
                recovery_sender
                    .send(())
                    .expect("recovery receiver should remain connected");
            },
        );
        detached
            .join()
            .expect("detached worker helper should settle its panic");
        assert_eq!(recovery_receiver.recv(), Ok(()));
        assert!(matches!(
            recovery_receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected)
        ));

        let joinable =
            spawn_joinable_redacted_worker(|| -> () { panic!("SENSITIVE-STREAM-WORKER-PANIC") });
        assert_eq!(joinable.join(), Err(RedactedWorkerPanic));
    }
}
