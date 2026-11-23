use super::*;
use crate::domain::provider_credentials::*;
use std::collections::VecDeque;
use std::sync::Mutex;
struct PipeReader {
    bytes: VecDeque<u8>,
    close: mpsc::Receiver<()>,
}
impl Read for PipeReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if let Some(byte) = self.bytes.pop_front() {
            buffer[0] = byte;
            return Ok(1);
        }
        let _ = self.close.recv();
        Ok(0)
    }
}
#[derive(Clone)]
struct Output(Arc<Mutex<Vec<u8>>>);
impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
struct Input;
impl NativePromptPort for Input {
    fn prompt(
        &self,
        _: &ConnectRequest,
        cancelled: &AtomicBool,
        _: Instant,
    ) -> CredentialResult<SecretToken> {
        if cancelled.load(Ordering::SeqCst) {
            return Err(Error::Cancelled);
        }
        SecretToken::from_bytes(Provider::Linear, b"synthetic-private-canary".to_vec())
    }
    fn confirm_migration(
        &self,
        _: &LegacyImportOffer,
        _: &AtomicBool,
        _: Instant,
    ) -> CredentialResult<bool> {
        Ok(false)
    }
}
fn request() -> PromptRequest {
    PromptRequest::Token {
        version: 1,
        request: ConnectRequest {
            operation_id: "prompt-test".into(),
            profile_id: "a".repeat(64),
            provider: Provider::Linear,
            expected_revision: 0,
            configuration: ConnectionConfiguration::default(),
        },
    }
}
#[test]
fn helper_returns_one_raw_secret_frame_without_a_json_secret_field() {
    let (close_tx, close_rx) = mpsc::channel();
    let reader = PipeReader {
        bytes: frames::encode_header(&request()).unwrap().into(),
        close: close_rx,
    };
    let output = Output(Arc::new(Mutex::new(Vec::new())));
    serve(
        reader,
        output.clone(),
        Arc::new(Input),
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .unwrap();
    close_tx.send(()).unwrap();
    let bytes = output.0.lock().unwrap();
    let newline = bytes.iter().position(|&b| b == b'\n').unwrap();
    assert!(!String::from_utf8_lossy(&bytes[..newline]).contains("synthetic-private-canary"));
    let mut reader = std::io::Cursor::new(bytes.as_slice());
    assert!(matches!(
        frames::read_header::<_, PromptResponse>(&mut reader).unwrap(),
        PromptResponse::Submitted { .. }
    ));
    assert_eq!(
        frames::read_secret(&mut reader, Provider::Linear)
            .unwrap()
            .as_str(),
        "synthetic-private-canary"
    );
    frames::require_eof(&mut reader).unwrap();
}
#[test]
fn closed_owner_never_produces_a_successful_secret_reply() {
    let input = std::io::Cursor::new(frames::encode_header(&request()).unwrap());
    let output = Output(Arc::new(Mutex::new(Vec::new())));
    struct WaitForOwner;
    impl NativePromptPort for WaitForOwner {
        fn prompt(
            &self,
            _: &ConnectRequest,
            cancelled: &AtomicBool,
            deadline: Instant,
        ) -> CredentialResult<SecretToken> {
            while !cancelled.load(Ordering::SeqCst) && Instant::now() < deadline {
                std::thread::yield_now();
            }
            Err(Error::Cancelled)
        }
        fn confirm_migration(
            &self,
            _: &LegacyImportOffer,
            _: &AtomicBool,
            _: Instant,
        ) -> CredentialResult<bool> {
            Ok(false)
        }
    }
    let result = serve(
        input,
        output.clone(),
        Arc::new(WaitForOwner),
        Duration::from_secs(1),
        Duration::from_millis(50),
    );
    assert!(result.is_err());
    assert!(output.0.lock().unwrap().is_empty());
}
