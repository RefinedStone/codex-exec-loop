//! 녹화 옵션: `AKRA_GATEWAY_RECORD=1` 일 때만 세션의 텍스트 메시지를 JSONL 로 남긴다 (P1 §4.6).
//! 녹화 파일은 실제 대화를 담으므로 `$AKRA_HOME/recordings/` 의 사용자 전용 디렉터리에만 쓰고, 저장소에
//! 커밋하지 않는다. 턴 없이 클라이언트를 시험할 때 재생 자료로 쓴다 (D-13).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use serde_json::json;

pub(super) const RECORD_ENV_VAR: &str = "AKRA_GATEWAY_RECORD";
const RECORDINGS_DIRECTORY_NAME: &str = "recordings";
const MAX_NAME_ATTEMPTS: u32 = 100;

#[derive(Debug, Clone, Copy)]
pub(super) enum RecordDirection {
    /// 클라이언트가 보낸 메시지이다.
    Client,
    /// 클라이언트로 보낸 메시지이다 (upstream 응답과 `akra/*` 응답).
    Gateway,
}

impl RecordDirection {
    fn label(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Gateway => "gateway",
        }
    }
}

/// 세션마다 녹화 파일을 하나씩 만든다.
pub(super) struct GatewayRecorder {
    directory: PathBuf,
    run_label: String,
    next_session: AtomicU64,
}

impl GatewayRecorder {
    /// 환경 변수가 `1` 일 때만 녹화기를 만든다.
    pub(super) fn from_env(akra_home: &Path) -> Result<Option<Self>> {
        if std::env::var(RECORD_ENV_VAR).ok().as_deref() != Some("1") {
            return Ok(None);
        }
        Self::create(&akra_home.join(RECORDINGS_DIRECTORY_NAME)).map(Some)
    }

    fn create(directory: &Path) -> Result<Self> {
        crate::private_directory::ensure_private_directory(directory)?;
        Ok(Self {
            directory: directory.to_path_buf(),
            // 같은 초 안에 다시 시작해도 이전 실행의 파일과 겹치지 않게 밀리초와 PID 를 붙인다.
            run_label: format!(
                "{}-{}",
                chrono::Utc::now().format("%Y%m%dT%H%M%S%3fZ"),
                std::process::id()
            ),
            next_session: AtomicU64::new(1),
        })
    }

    pub(super) fn open_session(&self) -> Result<SessionRecording> {
        let session = self.next_session.fetch_add(1, Ordering::SeqCst);
        let mut options = OpenOptions::new();
        options.create_new(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        // 이름이 겹치면(같은 밀리초에 다시 시작한 실행 등) 기존 녹화를 덮지 않고 접미사를 붙인다.
        for attempt in 0..MAX_NAME_ATTEMPTS {
            let suffix = if attempt == 0 {
                String::new()
            } else {
                format!("-{attempt}")
            };
            let path = self.directory.join(format!(
                "gateway-{}-{session:04}{suffix}.jsonl",
                self.run_label
            ));
            match options.open(&path) {
                Ok(file) => {
                    return Ok(SessionRecording {
                        file: Mutex::new(file),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("failed to create recording `{}`", path.display())
                    });
                }
            }
        }
        anyhow::bail!("could not find a free recording file name for session {session}")
    }
}

pub(super) struct SessionRecording {
    file: Mutex<File>,
}

impl SessionRecording {
    /// 녹화 실패는 중계를 멈추지 않는다. 경고만 남긴다.
    pub(super) fn record(&self, direction: RecordDirection, text: &str) {
        let line = json!({
            "at": chrono::Utc::now().to_rfc3339(),
            "direction": direction.label(),
            "text": text,
        });
        let mut file = self
            .file
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Err(error) = writeln!(file, "{line}") {
            tracing::warn!(error = %error, "gateway recording write failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::{GatewayRecorder, RecordDirection};

    #[test]
    fn each_session_writes_its_own_jsonl_file_with_directions() {
        let directory = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-gateway-recorder-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let recorder = GatewayRecorder::create(&directory).unwrap();

        let first = recorder.open_session().unwrap();
        first.record(
            RecordDirection::Client,
            r#"{"id":1,"method":"thread/list"}"#,
        );
        first.record(RecordDirection::Gateway, r#"{"id":1,"result":{}}"#);
        let _second = recorder.open_session().unwrap();

        let mut files: Vec<_> = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        files.sort();
        assert_eq!(files.len(), 2);
        let lines: Vec<Value> = std::fs::read_to_string(&files[0])
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["direction"], "client");
        assert_eq!(lines[0]["text"], r#"{"id":1,"method":"thread/list"}"#);
        assert_eq!(lines[1]["direction"], "gateway");
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_restart_with_the_same_run_label_never_overwrites_an_earlier_recording() {
        let directory = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-gateway-recorder-restart-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let first = GatewayRecorder::create(&directory).unwrap();
        let mut second = GatewayRecorder::create(&directory).unwrap();
        // Simulate a restart inside the same millisecond by the same process.
        second.run_label = first.run_label.clone();

        first
            .open_session()
            .unwrap()
            .record(RecordDirection::Client, "first");
        second
            .open_session()
            .unwrap()
            .record(RecordDirection::Client, "second");

        let contents: Vec<String> = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect();
        assert_eq!(contents.len(), 2, "both sessions must have their own file");
        assert!(contents.iter().any(|text| text.contains("\"first\"")));
        assert!(contents.iter().any(|text| text.contains("\"second\"")));
        let _ = std::fs::remove_dir_all(&directory);
    }
}
