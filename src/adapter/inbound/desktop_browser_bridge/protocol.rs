use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

pub(super) const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
pub(super) const IO_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BridgeError {
    pub code: String,
    pub message: String,
}

impl BridgeError {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_owned(),
            message: message.to_owned(),
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(untagged, deny_unknown_fields)]
pub(super) enum Reply {
    Result { result: Value },
    Error { error: BridgeError },
}

impl Reply {
    pub fn error(code: &str, message: &str) -> Self {
        Self::Error {
            error: BridgeError::new(code, message),
        }
    }

    pub fn result(result: Value) -> Self {
        Self::Result { result }
    }

    pub fn control(self, id: &str) -> Value {
        match self {
            Self::Result { result } => json!({"id":id,"result":result}),
            Self::Error { error } => json!({"id":id,"error":error}),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "camelCase")]
pub(super) enum ControlRequest {
    Prepare {
        id: String,
    },
    Bind {
        id: String,
        #[serde(rename = "registrationId")]
        registration_id: String,
        #[serde(rename = "threadId")]
        thread_id: String,
    },
    Revoke {
        id: String,
        #[serde(rename = "registrationId")]
        registration_id: String,
    },
    Answer {
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(flatten)]
        reply: Reply,
    },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(super) enum ClientRequest {
    Call {
        #[serde(rename = "registrationId")]
        registration_id: String,
        token: String,
        #[serde(rename = "threadId")]
        thread_id: String,
        tool: String,
        arguments: Value,
    },
    Watch {
        #[serde(rename = "registrationId")]
        registration_id: String,
        token: String,
    },
}

pub(super) fn known_tool(tool: &str) -> bool {
    matches!(
        tool,
        "browser_status"
            | "browser_navigate"
            | "browser_back"
            | "browser_forward"
            | "browser_reload"
            | "browser_snapshot"
            | "browser_click"
            | "browser_type"
            | "browser_press"
            | "browser_select"
            | "browser_scroll"
            | "browser_wait"
            | "browser_screenshot"
            | "browser_console"
            | "browser_network_failures"
            | "browser_handoff"
    )
}

/// Keeps partial input across select cancellation. Never allocates an unbounded line.
pub(super) struct FrameReader<R> {
    reader: BufReader<R>,
    partial: Vec<u8>,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader: BufReader::new(reader),
            partial: Vec::new(),
        }
    }

    pub async fn read(&mut self) -> std::io::Result<Option<Value>> {
        loop {
            let available = self.reader.fill_buf().await?;
            if available.is_empty() {
                return if self.partial.is_empty() {
                    Ok(None)
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "incomplete frame",
                    ))
                };
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let count = newline.map_or(available.len(), |index| index + 1);
            if self.partial.len() + count > MAX_FRAME_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "frame limit exceeded",
                ));
            }
            self.partial.extend_from_slice(&available[..count]);
            self.reader.consume(count);
            if newline.is_some() {
                let parsed = serde_json::from_slice(&self.partial).map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid frame")
                });
                self.partial.clear();
                return parsed.map(Some);
            }
        }
    }
}

pub(super) async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    value: &impl Serialize,
) -> anyhow::Result<()> {
    let mut frame = serde_json::to_vec(value)?;
    anyhow::ensure!(frame.len() < MAX_FRAME_BYTES, "output frame limit exceeded");
    frame.push(b'\n');
    tokio::time::timeout(IO_DEADLINE, async {
        writer.write_all(&frame).await?;
        writer.flush().await
    })
    .await??;
    Ok(())
}
