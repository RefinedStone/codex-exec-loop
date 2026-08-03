use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, TryRecvError};
use std::time::Duration;

use crossterm::event::{self, Event, MouseEventKind};

use super::core_effect_worker::{RedactedWorkerHandle, spawn_joinable_redacted_worker};

const TERMINAL_INPUT_POLL_INTERVAL: Duration = Duration::from_millis(8);

enum NativeTerminalEventIngressMessage {
    Event(Event),
    Error(io::Error),
}

trait NativeTerminalEventSource: Send + 'static {
    fn poll(&mut self, timeout: Duration) -> io::Result<bool>;
    fn read(&mut self) -> io::Result<Event>;
}

struct CrosstermNativeTerminalEventSource;

impl NativeTerminalEventSource for CrosstermNativeTerminalEventSource {
    fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
        event::poll(timeout)
    }

    fn read(&mut self) -> io::Result<Event> {
        event::read()
    }
}

/// Composition-owned terminal input ingress for the native fullscreen client.
///
/// The inbound TUI receives only this opaque mailbox. Composition owns the
/// joinable task capability through the same redacted panic boundary as other
/// native runtime work, so a terminal reader cannot leak process/thread handles
/// into UI state or bypass the application runtime.
pub(crate) struct NativeTerminalEventIngress {
    receiver: Receiver<NativeTerminalEventIngressMessage>,
    stop: Arc<AtomicBool>,
    reader: Option<RedactedWorkerHandle<()>>,
}

impl NativeTerminalEventIngress {
    pub(crate) fn open() -> Self {
        Self::open_with_source(CrosstermNativeTerminalEventSource)
    }

    fn open_with_source(mut source: impl NativeTerminalEventSource) -> Self {
        let (sender, receiver) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let reader_stop = Arc::clone(&stop);
        let reader = spawn_joinable_redacted_worker(move || {
            while !reader_stop.load(Ordering::Acquire) {
                match source.poll(TERMINAL_INPUT_POLL_INTERVAL) {
                    Ok(false) => continue,
                    Ok(true) => match source.read() {
                        Ok(event) => {
                            // Crossterm enables all-motion reporting with mouse capture. Akra has
                            // no hover semantics, so forwarding plain motion only fills the queue
                            // ahead of the next click or key and recreates perceived input lag.
                            if is_unobserved_pointer_motion(&event) {
                                continue;
                            }
                            if sender
                                .send(NativeTerminalEventIngressMessage::Event(event))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(error) => {
                            let _ = sender.send(NativeTerminalEventIngressMessage::Error(error));
                            break;
                        }
                    },
                    Err(error) => {
                        let _ = sender.send(NativeTerminalEventIngressMessage::Error(error));
                        break;
                    }
                }
            }
        });
        Self {
            receiver,
            stop,
            reader: Some(reader),
        }
    }

    pub(crate) fn try_read(&self) -> io::Result<Option<Event>> {
        match self.receiver.try_recv() {
            Ok(message) => message.into_result().map(Some),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) if self.stop.load(Ordering::Acquire) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(disconnected_reader_error()),
        }
    }

    pub(crate) fn wait(&self, timeout: Duration) -> io::Result<Option<Event>> {
        match self.receiver.recv_timeout(timeout) {
            Ok(message) => message.into_result().map(Some),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) if self.stop.load(Ordering::Acquire) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(disconnected_reader_error()),
        }
    }
}

impl NativeTerminalEventIngressMessage {
    fn into_result(self) -> io::Result<Event> {
        match self {
            Self::Event(event) => Ok(event),
            Self::Error(error) => Err(error),
        }
    }
}

impl Drop for NativeTerminalEventIngress {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn disconnected_reader_error() -> io::Error {
    io::Error::other("terminal event reader stopped unexpectedly")
}

fn is_unobserved_pointer_motion(event: &Event) -> bool {
    matches!(
        event,
        Event::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Moved,
            ..
        })
    )
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};

    use super::*;

    struct ScriptedEventSource {
        events: VecDeque<io::Result<Event>>,
        poll_count: Arc<AtomicUsize>,
    }

    struct PanickingEventSource;

    impl NativeTerminalEventSource for PanickingEventSource {
        fn poll(&mut self, _timeout: Duration) -> io::Result<bool> {
            panic!("synthetic terminal reader panic")
        }

        fn read(&mut self) -> io::Result<Event> {
            unreachable!("poll panics before read")
        }
    }

    impl NativeTerminalEventSource for ScriptedEventSource {
        fn poll(&mut self, timeout: Duration) -> io::Result<bool> {
            self.poll_count.fetch_add(1, Ordering::Relaxed);
            if self.events.is_empty() {
                thread::sleep(timeout);
            }
            Ok(!self.events.is_empty())
        }

        fn read(&mut self) -> io::Result<Event> {
            self.events.pop_front().unwrap_or_else(|| {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "script exhausted",
                ))
            })
        }
    }

    #[test]
    fn reader_collects_input_independently_of_frame_consumption() {
        let poll_count = Arc::new(AtomicUsize::new(0));
        let source = ScriptedEventSource {
            events: VecDeque::from([
                Ok(Event::Key(KeyEvent::from(KeyCode::Char('a')))),
                Ok(Event::Key(KeyEvent::from(KeyCode::Char('b')))),
            ]),
            poll_count: Arc::clone(&poll_count),
        };
        let ingress = NativeTerminalEventIngress::open_with_source(source);

        assert!(matches!(
            ingress.wait(Duration::from_secs(1)).expect("first event"),
            Some(Event::Key(KeyEvent {
                code: KeyCode::Char('a'),
                ..
            }))
        ));
        assert!(matches!(
            ingress.wait(Duration::from_secs(1)).expect("second event"),
            Some(Event::Key(KeyEvent {
                code: KeyCode::Char('b'),
                ..
            }))
        ));
        assert!(poll_count.load(Ordering::Relaxed) >= 2);
    }

    #[test]
    fn reader_errors_cross_the_redacted_boundary_without_panicking() {
        let source = ScriptedEventSource {
            events: VecDeque::from([Err(io::Error::new(io::ErrorKind::BrokenPipe, "pty closed"))]),
            poll_count: Arc::new(AtomicUsize::new(0)),
        };
        let ingress = NativeTerminalEventIngress::open_with_source(source);

        let error = ingress
            .wait(Duration::from_secs(1))
            .expect_err("reader failure should be delivered");
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn reader_panic_is_not_misreported_as_a_clean_terminal_eof() {
        let ingress = NativeTerminalEventIngress::open_with_source(PanickingEventSource);

        let error = ingress
            .wait(Duration::from_secs(1))
            .expect_err("reader panic should disconnect the mailbox as a failure");
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(
            error.to_string(),
            "terminal event reader stopped unexpectedly"
        );
    }

    #[test]
    fn unused_all_motion_reports_never_queue_ahead_of_real_input() {
        let moved = || {
            Ok(Event::Mouse(MouseEvent {
                kind: MouseEventKind::Moved,
                column: 40,
                row: 12,
                modifiers: KeyModifiers::NONE,
            }))
        };
        let mut events = (0..10_000).map(|_| moved()).collect::<VecDeque<_>>();
        events.push_back(Ok(Event::Key(KeyEvent::from(KeyCode::Enter))));
        let ingress = NativeTerminalEventIngress::open_with_source(ScriptedEventSource {
            events,
            poll_count: Arc::new(AtomicUsize::new(0)),
        });

        assert!(matches!(
            ingress.wait(Duration::from_secs(1)).expect("real input"),
            Some(Event::Key(KeyEvent {
                code: KeyCode::Enter,
                ..
            }))
        ));
        assert!(ingress.try_read().expect("empty mailbox").is_none());
    }
}
