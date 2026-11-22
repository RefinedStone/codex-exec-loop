//! 보호된 복원 채널만 평문을 직렬화한다. 일반 Gateway wire 타입과 공유하지 않는다.
use crate::application::port::inbound::vault_restore_session_port::{
    VaultRestoreReconnectRequest, VaultRestoreSessionField, VaultRestoreSessionHold,
    VaultRestoreSessionMapping, VaultRestoreSessionPlace, VaultRestoreSessionReview,
};
use crate::domain::issue_work::{IssueProjectScope, IssueProvider, IssueReference};
use serde::{Deserialize, Serialize};
use std::io;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use zeroize::Zeroizing;

pub(super) const MAX_INPUT_BYTES: usize = 16 * 1024;
pub(super) const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const VERSION: u32 = 2;

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
    Review {
        version: u32,
        id: String,
    },
    Reconnect {
        version: u32,
        id: String,
        reviewed_hash: String,
        mappings: Vec<Mapping>,
    },
}
impl Request {
    pub(super) fn header(&self) -> (u32, &str) {
        match self {
            Self::Prepare { version, id, .. }
            | Self::Restore { version, id }
            | Self::Review { version, id }
            | Self::Reconnect { version, id, .. } => (*version, id),
        }
    }
    pub(super) fn into_reconnect(self) -> Option<VaultRestoreReconnectRequest> {
        let Self::Reconnect {
            reviewed_hash,
            mappings,
            ..
        } = self
        else {
            return None;
        };
        if !valid_reviewed_hash(&reviewed_hash) {
            return None;
        }
        Some(VaultRestoreReconnectRequest {
            reviewed_hash,
            mappings: mappings
                .into_iter()
                .map(|mapping| VaultRestoreSessionMapping {
                    marker: mapping.marker,
                    place: VaultRestoreSessionPlace {
                        field: mapping.field.into(),
                        position: mapping.position,
                    },
                })
                .collect(),
        })
    }
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
#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Field {
    Title,
    Body,
}
impl From<Field> for VaultRestoreSessionField {
    fn from(value: Field) -> Self {
        match value {
            Field::Title => Self::Title,
            Field::Body => Self::Body,
        }
    }
}
impl From<VaultRestoreSessionField> for Field {
    fn from(value: VaultRestoreSessionField) -> Self {
        match value {
            VaultRestoreSessionField::Title => Self::Title,
            VaultRestoreSessionField::Body => Self::Body,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Mapping {
    marker: String,
    field: Field,
    #[serde(deserialize_with = "safe_position")]
    position: usize,
}
fn safe_position<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<usize, D::Error> {
    let position = u64::deserialize(deserializer)?;
    if position > 9_007_199_254_740_991 {
        return Err(serde::de::Error::custom("invalid marker position"));
    }
    usize::try_from(position).map_err(|_| serde::de::Error::custom("invalid marker position"))
}
pub(super) fn valid_header(version: u32, id: &str) -> bool {
    version == VERSION
        && !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
}
fn valid_reviewed_hash(value: &str) -> bool {
    let Some((version, digest)) = value.split_once(':') else {
        return false;
    };
    let Ok(number) = version.parse::<u32>() else {
        return false;
    };
    number > 0
        && version == number.to_string()
        && digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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
    ChangedDuringReview,
    MappingRejected,
    RecordUnavailable,
    WriteOutcomeUnknown,
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
            VaultRestoreSessionHold::ChangedDuringReview => Self::ChangedDuringReview,
            VaultRestoreSessionHold::MappingRejected => Self::MappingRejected,
            VaultRestoreSessionHold::RecordUnavailable => Self::RecordUnavailable,
            VaultRestoreSessionHold::WriteOutcomeUnknown => Self::WriteOutcomeUnknown,
        }
    }
}
impl Hold {
    pub(super) fn reconnectable(self) -> bool {
        matches!(
            self,
            Self::PublishedCopyChanged
                | Self::MarkerMoved
                | Self::MarkerDuplicated
                | Self::MarkerMissing
        )
    }
}
#[derive(Serialize)]
pub(super) struct Capabilities {
    pub reconnect: bool,
}
#[derive(Serialize)]
struct Place {
    field: Field,
    position: usize,
}
#[derive(Serialize)]
struct Candidate<'a> {
    field: Field,
    position: usize,
    context: &'a str,
}
#[derive(Serialize)]
struct Marker<'a> {
    marker: &'a str,
    previous: Place,
    candidates: Vec<Candidate<'a>>,
}
#[derive(Serialize)]
pub(super) struct Review<'a> {
    reviewed_hash: &'a str,
    published_title: &'a str,
    published_body: &'a str,
    markers: Vec<Marker<'a>>,
}
impl<'a> From<&'a VaultRestoreSessionReview> for Review<'a> {
    fn from(value: &'a VaultRestoreSessionReview) -> Self {
        Self {
            reviewed_hash: &value.reviewed_hash,
            published_title: value.published.expose_title(),
            published_body: value.published.expose_body(),
            markers: value
                .markers
                .iter()
                .map(|marker| Marker {
                    marker: &marker.marker,
                    previous: Place {
                        field: marker.previous.field.into(),
                        position: marker.previous.position,
                    },
                    candidates: marker
                        .candidates
                        .iter()
                        .map(|candidate| Candidate {
                            field: candidate.place.field.into(),
                            position: candidate.place.position,
                            context: &candidate.context,
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Reply<'a> {
    Prepared { capabilities: Capabilities },
    Held { reason: Hold, can_reconnect: bool },
    Restored { title: &'a str, body: &'a str },
    Review { review: Review<'a> },
    Reconnected,
}
#[derive(Serialize)]
struct Envelope<'a> {
    version: u32,
    id: &'a str,
    result: Reply<'a>,
}
pub(super) struct Encoded {
    pub bytes: Zeroizing<Vec<u8>>,
    pub terminal: bool,
}
/// 모든 응답과 개행을 합쳐 세션 전체 출력이 한도를 넘지 않는다.
pub(super) struct OutputBudget {
    remaining: usize,
}
impl OutputBudget {
    pub(super) fn new() -> Self {
        Self {
            remaining: MAX_OUTPUT_BYTES,
        }
    }
    pub(super) fn encode(
        &mut self,
        id: &str,
        reply: Reply<'_>,
        terminal: bool,
    ) -> io::Result<Encoded> {
        let bytes = encode(id, reply)?;
        let encoded = if bytes.len() <= self.remaining {
            Encoded { bytes, terminal }
        } else {
            drop(bytes);
            Encoded {
                bytes: encode(
                    id,
                    Reply::Held {
                        reason: Hold::SourceUnavailable,
                        can_reconnect: false,
                    },
                )?,
                terminal: true,
            }
        };
        if encoded.bytes.len() > self.remaining {
            return Err(io::Error::other("restore response unavailable"));
        }
        self.remaining -= encoded.bytes.len();
        Ok(encoded)
    }
}
fn encode(id: &str, result: Reply<'_>) -> io::Result<Zeroizing<Vec<u8>>> {
    let mut bytes = Zeroizing::new(
        serde_json::to_vec(&Envelope {
            version: VERSION,
            id,
            result,
        })
        .map_err(|_| io::Error::other("restore response unavailable"))?,
    );
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
