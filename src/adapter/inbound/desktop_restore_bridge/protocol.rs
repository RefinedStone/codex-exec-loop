//! 보호된 복원 채널만 평문을 직렬화한다. 일반 Gateway wire 타입과 공유하지 않는다.
use crate::application::port::inbound::vault_restore_session_port::VaultRestoreSessionHold;
use crate::domain::issue_work::{IssueProjectScope, IssueProvider, IssueReference};
use serde::{Deserialize, Serialize};
use std::io;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use zeroize::Zeroizing;

pub(super) const MAX_INPUT_BYTES: usize = 16 * 1024;
pub(super) const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
#[derive(Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Request {
    Prepare {
        version: u32,
        id: String,
        issue_url: String,
        issue: Coordinates,
    },
    Restore {
        version: u32,
        id: String,
    },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Coordinates {
    provider: String,
    server_id: String,
    repository_id: String,
    issue_id: String,
    project_scope: ProjectScope,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectScope {
    workspace_dir: String,
}
impl Coordinates {
    pub(super) fn into_reference(self) -> Option<IssueReference> {
        if self.provider != "github"
            || self.server_id != "github.com"
            || !node_id(&self.repository_id)
            || !node_id(&self.issue_id)
            || !plain_text(&self.project_scope.workspace_dir, 4096)
        {
            return None;
        }
        Some(IssueReference {
            provider: IssueProvider::GitHub,
            server_id: self.server_id,
            repository_id: self.repository_id,
            issue_id: self.issue_id,
            project_scope: IssueProjectScope {
                workspace_dir: self.project_scope.workspace_dir,
            },
        })
    }
}
pub(super) fn valid_header(version: u32, id: &str) -> bool {
    version == 1
        && !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
}
fn plain_text(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.trim() == value
        && !value.chars().any(char::is_control)
}
fn node_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-=".contains(&byte))
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Hold {
    PlatformUnsupported,
    ConfirmationCancelled,
    SourceUnavailable,
    Locked,
    LatestReadFailed,
    NotBound,
    PublishedCopyChanged,
    MarkerMoved,
    MarkerDuplicated,
    MarkerMissing,
    RecordUnavailable,
}
impl From<VaultRestoreSessionHold> for Hold {
    fn from(reason: VaultRestoreSessionHold) -> Self {
        match reason {
            VaultRestoreSessionHold::PlatformUnsupported => Self::PlatformUnsupported,
            VaultRestoreSessionHold::ConfirmationCancelled => Self::ConfirmationCancelled,
            VaultRestoreSessionHold::SourceUnavailable => Self::SourceUnavailable,
            VaultRestoreSessionHold::Locked => Self::Locked,
            VaultRestoreSessionHold::LatestReadFailed => Self::LatestReadFailed,
            VaultRestoreSessionHold::NotBound => Self::NotBound,
            VaultRestoreSessionHold::PublishedCopyChanged => Self::PublishedCopyChanged,
            VaultRestoreSessionHold::MarkerMoved => Self::MarkerMoved,
            VaultRestoreSessionHold::MarkerDuplicated => Self::MarkerDuplicated,
            VaultRestoreSessionHold::MarkerMissing => Self::MarkerMissing,
            VaultRestoreSessionHold::RecordUnavailable => Self::RecordUnavailable,
        }
    }
}
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Reply<'a> {
    Prepared,
    Held { reason: Hold },
    Restored { title: &'a str, body: &'a str },
}
#[derive(Serialize)]
struct Envelope<'a> {
    version: u32,
    id: &'a str,
    result: Reply<'a>,
}
pub(super) fn encode(id: &str, result: Reply<'_>) -> io::Result<Zeroizing<Vec<u8>>> {
    let mut bytes = Zeroizing::new(
        serde_json::to_vec(&Envelope {
            version: 1,
            id,
            result,
        })
        .map_err(|_| io::Error::other("restore response unavailable"))?,
    );
    if bytes.len() >= MAX_OUTPUT_BYTES {
        return encode(
            id,
            Reply::Held {
                reason: Hold::SourceUnavailable,
            },
        );
    }
    bytes.push(b'\n');
    Ok(bytes)
}
pub(super) async fn write<W: AsyncWrite + Unpin>(output: &mut W, bytes: &[u8]) -> io::Result<()> {
    output.write_all(bytes).await?;
    output.flush().await
}
pub(super) struct Frames<R> {
    reader: BufReader<R>,
    partial: Vec<u8>,
}
impl<R: AsyncRead + Unpin> Frames<R> {
    pub(super) fn new(input: R) -> Self {
        Self {
            reader: BufReader::new(input),
            partial: Vec::new(),
        }
    }
    pub(super) async fn unexpected_input(&mut self) -> io::Result<()> {
        // 한 바이트 또는 EOF만으로 취소한다. 다음 JSON 완성을 기다리지 않는다.
        self.reader.fill_buf().await.map(|_| ())
    }
    pub(super) fn buffered_input(&self) -> bool {
        !self.reader.buffer().is_empty()
    }
    pub(super) async fn read(&mut self) -> io::Result<Option<Request>> {
        loop {
            let available = self.reader.fill_buf().await?;
            if available.is_empty() {
                return if self.partial.is_empty() {
                    Ok(None)
                } else {
                    Err(io::Error::other("incomplete restore request"))
                };
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let count = newline.map_or(available.len(), |index| index + 1);
            if self.partial.len() + count > MAX_INPUT_BYTES {
                return Err(io::Error::other("restore request limit exceeded"));
            }
            self.partial.extend_from_slice(&available[..count]);
            self.reader.consume(count);
            if newline.is_some() {
                let parsed = serde_json::from_slice(&self.partial)
                    .map_err(|_| io::Error::other("invalid restore request"));
                self.partial.clear();
                return parsed.map(Some);
            }
        }
    }
}
