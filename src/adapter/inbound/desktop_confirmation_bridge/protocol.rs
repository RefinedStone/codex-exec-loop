use crate::application::port::inbound::stored_command_port::issue_confirmation::{
    IssueConfirmationBinding, IssueConfirmationIntent,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

pub(crate) const MAX_FRAME_BYTES: usize = 64 * 1024;
pub(crate) const IO_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum ControlRequest {
    Prepare {
        id: String,
    },
    Approve {
        id: String,
        registration_id: String,
        intent: Box<IssueConfirmationIntent>,
    },
    Revoke {
        id: String,
        registration_id: String,
    },
}

/// Public IPC can verify a decision, but can never mint one. Only inherited stdin approves.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ClientRequest {
    Bind {
        registration_id: String,
        token: String,
        binding: IssueConfirmationBinding,
    },
    Watch {
        registration_id: String,
        token: String,
        binding: IssueConfirmationBinding,
    },
    Consume {
        registration_id: String,
        token: String,
        proof_id: String,
        intent: Box<IssueConfirmationIntent>,
    },
}

impl ClientRequest {
    pub fn credentials(&self) -> (&str, &str) {
        match self {
            Self::Bind {
                registration_id,
                token,
                ..
            }
            | Self::Watch {
                registration_id,
                token,
                ..
            }
            | Self::Consume {
                registration_id,
                token,
                ..
            } => (registration_id, token),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BridgeError {
    pub code: String,
    pub message: String,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub(crate) enum Reply {
    Result { result: Value },
    Error { error: BridgeError },
}
impl Reply {
    pub fn denied() -> Self {
        Self::Error {
            error: BridgeError {
                code: "confirmation_unavailable".to_owned(),
                message: "Human confirmation is unavailable or does not match this action"
                    .to_owned(),
            },
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
    pub fn into_result(self) -> anyhow::Result<Value> {
        match self {
            Self::Result { result } => Ok(result),
            Self::Error { .. } => anyhow::bail!("human confirmation unavailable"),
        }
    }
}

pub(crate) fn nonce(value: &str, bytes: usize) -> bool {
    value.len() == bytes * 2
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(crate) fn valid_binding(binding: &IssueConfirmationBinding) -> bool {
    nonce(&binding.registration_id, 16)
        && nonce(&binding.connection_id, 16)
        && nonce(&binding.attachment_id, 16)
        && !binding.client_id.is_empty()
        && binding.client_id.len() <= 128
        && !binding.client_id.chars().any(char::is_control)
        && !binding.workspace.is_empty()
        && binding.workspace.len() <= 4096
        && !binding.workspace.chars().any(char::is_control)
}
pub(crate) fn valid_intent(intent: &IssueConfirmationIntent) -> bool {
    valid_binding(&intent.binding)
        && intent.target.validate().is_ok()
        && intent.target.workspace() == intent.binding.workspace
        && !intent.key.trim().is_empty()
        && intent.key.chars().count() <= 128
        && !intent.key.chars().any(char::is_control)
        && nonce(&intent.fingerprint, 16)
}

/// Retains a bounded partial frame across select cancellation.
pub(crate) struct FrameReader<R> {
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
                let value = serde_json::from_slice(&self.partial).map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid frame")
                });
                self.partial.clear();
                return value.map(Some);
            }
        }
    }
}
pub(crate) async fn write_frame<W: AsyncWrite + Unpin>(
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
