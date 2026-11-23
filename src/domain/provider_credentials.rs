//! Provider credentials are owned by one Gateway profile. Public values never contain a token.
use serde::{Deserialize, Serialize};
use std::fmt;
use zeroize::Zeroizing;

pub const MAX_LINEAR_UTF16_UNITS: usize = 4096;
pub const MAX_LINEAR_SECRET_BYTES: usize = 3 * MAX_LINEAR_UTF16_UNITS;
/// An explicit transport/storage bound for Jira; oversized legacy values must be retained.
pub const MAX_JIRA_SECRET_BYTES: usize = 64 * 1024;
pub const SECRET_CHUNK_BYTES: usize = 2048;
pub const MAX_SECRET_PARTS: usize = MAX_JIRA_SECRET_BYTES / SECRET_CHUNK_BYTES;
pub const MAX_SAFE_REVISION: u64 = 9_007_199_254_740_991;
pub const MAX_OPERATIONS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    Jira,
    Linear,
}
impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Jira => "jira",
            Self::Linear => "linear",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JiraDeployment {
    Cloud,
    Server,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectionConfiguration {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment: Option<JiraDeployment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}
impl ConnectionConfiguration {
    pub fn normalized(&self, provider: Provider) -> CredentialResult<Self> {
        match provider {
            Provider::Linear if self == &Self::default() => Ok(self.clone()),
            Provider::Linear => Err(ProviderCredentialError::InvalidRequest),
            Provider::Jira => {
                let raw = self
                    .site_url
                    .as_deref()
                    .ok_or(ProviderCredentialError::InvalidRequest)?;
                if raw.len() > 2048 {
                    return Err(ProviderCredentialError::InvalidRequest);
                }
                let site_url = crate::domain::work_context::normalize_jira_site(raw.trim())
                    .ok_or(ProviderCredentialError::InvalidRequest)?;
                let deployment = self
                    .deployment
                    .ok_or(ProviderCredentialError::InvalidRequest)?;
                let email = match deployment {
                    JiraDeployment::Cloud => {
                        let value = self.email.as_deref().unwrap_or_default().trim();
                        if value.is_empty()
                            || value.len() > 320
                            || value.chars().any(char::is_control)
                        {
                            return Err(ProviderCredentialError::InvalidRequest);
                        }
                        Some(value.to_owned())
                    }
                    JiraDeployment::Server => None,
                };
                Ok(Self {
                    site_url: Some(site_url),
                    deployment: Some(deployment),
                    email,
                })
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectionMetadata {
    pub configuration: ConnectionConfiguration,
    pub account_id: String,
    pub account_name: String,
}
impl ConnectionMetadata {
    pub fn validate(&self, provider: Provider) -> CredentialResult<()> {
        if self.configuration.normalized(provider)? != self.configuration
            || !plain(&self.account_id, 256)
            || !plain(&self.account_name, 1024)
        {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectionRef {
    pub profile_id: String,
    pub provider: Provider,
    pub connection_id: String,
    pub revision: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectionSummary {
    pub reference: ConnectionRef,
    pub metadata: ConnectionMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectRequest {
    pub operation_id: String,
    pub profile_id: String,
    pub provider: Provider,
    pub expected_revision: u64,
    pub configuration: ConnectionConfiguration,
}
pub type ProviderCredentialPromptRequest = ConnectRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DisconnectRequest {
    pub operation_id: String,
    pub profile_id: String,
    pub provider: Provider,
    pub expected_revision: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CredentialOperationRequest {
    pub operation_id: String,
    pub profile_id: String,
    pub provider: Provider,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyImportOffer {
    pub operation_id: String,
    pub profile_id: String,
    pub provider: Provider,
    pub expected_revision: u64,
    pub configuration: ConnectionConfiguration,
    /// SHA-256 of the encrypted legacy snapshot and its public connection metadata, never token.
    pub snapshot_id: String,
}
impl LegacyImportOffer {
    pub fn connect_request(&self) -> ConnectRequest {
        ConnectRequest {
            operation_id: self.operation_id.clone(),
            profile_id: self.profile_id.clone(),
            provider: self.provider,
            expected_revision: self.expected_revision,
            configuration: self.configuration.clone(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LegacyCleanupRequest {
    pub operation_id: String,
    pub profile_id: String,
    pub provider: Provider,
    pub snapshot_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CredentialOperationKind {
    Connect,
    LegacyImport,
    Disconnect,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CredentialOperationStatus {
    AwaitingInput,
    Prepared,
    Verifying,
    Storing,
    Committed,
    Cancelled,
    Superseded,
    Failed,
    OutcomeUnknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationReceipt {
    pub operation_id: String,
    pub profile_id: String,
    pub provider: Provider,
    pub expected_revision: u64,
    pub kind: CredentialOperationKind,
    pub status: CredentialOperationStatus,
    pub connection: Option<ConnectionRef>,
    pub cleanup_pending: bool,
    pub legacy_snapshot_id: Option<String>,
    pub legacy_cleanup_allowed: bool,
    pub error: Option<ProviderCredentialError>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderConnectionStatus {
    Disconnected,
    Connected,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderCredentialCapabilities {
    pub native_input: bool,
    pub legacy_import: bool,
    pub keyring: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderCredentialState {
    pub profile_id: String,
    pub provider: Provider,
    pub revision: u64,
    pub status: ProviderConnectionStatus,
    pub active_connection: Option<ConnectionSummary>,
    pub operation: Option<OperationReceipt>,
    pub capabilities: ProviderCredentialCapabilities,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderCredentialError {
    InvalidRequest,
    Unsupported,
    Locked,
    Unavailable,
    Conflict,
    Busy,
    Cancelled,
    OutcomeUnknown,
    Corrupt,
    TooLarge,
    CapacityExceeded,
    NotConnected,
    NotFound,
    Unauthorized,
}
pub type CredentialResult<T> = Result<T, ProviderCredentialError>;
impl fmt::Display for ProviderCredentialError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRequest => "invalid provider credential request",
            Self::Unsupported => "provider credentials are unsupported",
            Self::Locked => "provider key store is locked or access was denied",
            Self::Unavailable => "provider credentials are unavailable",
            Self::Conflict => "provider connection changed",
            Self::Busy => "another provider credential operation is pending",
            Self::Cancelled => "provider credential operation was cancelled",
            Self::OutcomeUnknown => "provider credential outcome is not yet known",
            Self::Corrupt => "provider credential state is invalid",
            Self::TooLarge => "provider credential exceeds the supported size",
            Self::CapacityExceeded => "provider credential operation history is full",
            Self::NotConnected => "provider is not connected",
            Self::NotFound => "provider credential operation was not found",
            Self::Unauthorized => "provider credential owner is unavailable",
        })
    }
}
impl std::error::Error for ProviderCredentialError {}

/// No Debug, Clone, or Serialize. Only native/private ingress constructs this value.
pub struct SecretToken(Zeroizing<Vec<u8>>);
impl SecretToken {
    pub fn from_bytes(provider: Provider, bytes: Vec<u8>) -> CredentialResult<Self> {
        let bytes = Zeroizing::new(bytes);
        let text =
            std::str::from_utf8(&bytes).map_err(|_| ProviderCredentialError::InvalidRequest)?;
        let maximum = match provider {
            Provider::Jira => MAX_JIRA_SECRET_BYTES,
            Provider::Linear => MAX_LINEAR_SECRET_BYTES,
        };
        if bytes.len() > maximum
            || (provider == Provider::Linear
                && text.encode_utf16().count() > MAX_LINEAR_UTF16_UNITS)
        {
            return Err(ProviderCredentialError::TooLarge);
        }
        let text = text.trim();
        if text.is_empty() || text.bytes().any(|byte| byte <= 0x20 || byte == 0x7f) {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        Ok(Self(Zeroizing::new(text.as_bytes().to_vec())))
    }
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).expect("validated credential Unicode")
    }
}

pub fn valid_profile_id(value: &str) -> bool {
    hex(value, 64)
}
pub fn valid_snapshot_id(value: &str) -> bool {
    hex(value, 64)
}
pub fn valid_connection_id(value: &str) -> bool {
    hex(value, 32)
}
pub fn valid_operation_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
}
pub fn valid_revision(value: u64) -> bool {
    value <= MAX_SAFE_REVISION
}
fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn plain(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}

/// Internal, secret-free persistence. Never expose this storage document through Gateway RPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoredConnection {
    pub summary: ConnectionSummary,
    pub operation_id: String,
    pub part_count: u16,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SecretPartState {
    Prepared,
    Submitted,
    Verified,
    Deleted,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoredOperation {
    pub receipt: OperationReceipt,
    pub configuration: ConnectionConfiguration,
    pub owner_id: String,
    pub candidate: Option<StoredConnection>,
    pub parts: Vec<SecretPartState>,
    pub legacy_cleaned: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CredentialLedger {
    pub profile_id: String,
    pub provider: Provider,
    pub row_version: u64,
    pub revision: u64,
    pub tombstone: bool,
    pub active: Option<StoredConnection>,
    pub operations: Vec<StoredOperation>,
}
impl CredentialLedger {
    pub fn empty(profile_id: String, provider: Provider) -> Self {
        Self {
            profile_id,
            provider,
            row_version: 0,
            revision: 0,
            tombstone: false,
            active: None,
            operations: Vec::new(),
        }
    }
}

impl CredentialOperationStatus {
    pub fn is_pending(self) -> bool {
        matches!(
            self,
            Self::AwaitingInput | Self::Prepared | Self::Verifying | Self::Storing
        )
    }
}

impl CredentialLedger {
    /// Validate every retained record before a read or CAS, including records not being changed.
    pub fn validate(&self) -> CredentialResult<()> {
        use std::collections::HashSet;
        if !valid_profile_id(&self.profile_id)
            || !valid_revision(self.row_version)
            || !valid_revision(self.revision)
            || self.operations.len() > MAX_OPERATIONS
        {
            return Err(ProviderCredentialError::Corrupt);
        }
        let mut operations = HashSet::new();
        let mut connections = HashSet::new();
        for operation in &self.operations {
            let receipt = &operation.receipt;
            if !valid_operation_id(&receipt.operation_id)
                || !operations.insert(receipt.operation_id.as_str())
                || receipt.profile_id != self.profile_id
                || receipt.provider != self.provider
                || !valid_revision(receipt.expected_revision)
                || receipt.expected_revision > self.revision
                || !valid_operation_id(&operation.owner_id)
                || (receipt.kind != CredentialOperationKind::Disconnect
                    && operation
                        .configuration
                        .normalized(self.provider)
                        .ok()
                        .as_ref()
                        != Some(&operation.configuration))
                || (receipt.kind == CredentialOperationKind::Disconnect
                    && operation.configuration != ConnectionConfiguration::default())
            {
                return Err(ProviderCredentialError::Corrupt);
            }
            if receipt.kind == CredentialOperationKind::LegacyImport {
                if !receipt
                    .legacy_snapshot_id
                    .as_deref()
                    .is_some_and(valid_snapshot_id)
                    || (receipt.legacy_cleanup_allowed
                        && receipt.status != CredentialOperationStatus::Committed)
                    || (operation.legacy_cleaned
                        && receipt.status != CredentialOperationStatus::Committed)
                    || (receipt.legacy_cleanup_allowed
                        != (receipt.status == CredentialOperationStatus::Committed
                            && !operation.legacy_cleaned))
                {
                    return Err(ProviderCredentialError::Corrupt);
                }
            } else if receipt.legacy_snapshot_id.is_some()
                || receipt.legacy_cleanup_allowed
                || operation.legacy_cleaned
            {
                return Err(ProviderCredentialError::Corrupt);
            }
            if let Some(candidate) = &operation.candidate {
                validate_stored_connection(candidate, &self.profile_id, self.provider)?;
                if candidate.operation_id != receipt.operation_id
                    || candidate.summary.reference.revision
                        != receipt
                            .expected_revision
                            .checked_add(1)
                            .ok_or(ProviderCredentialError::Corrupt)?
                    || !connections.insert(candidate.summary.reference.connection_id.as_str())
                    || operation.parts.len() != usize::from(candidate.part_count)
                    || candidate.summary.metadata.configuration != operation.configuration
                    || receipt.kind == CredentialOperationKind::Disconnect
                {
                    return Err(ProviderCredentialError::Corrupt);
                }
                if receipt.status == CredentialOperationStatus::Committed
                    && receipt.connection.as_ref() != Some(&candidate.summary.reference)
                {
                    return Err(ProviderCredentialError::Corrupt);
                }
            } else if !operation.parts.is_empty()
                || receipt.connection.is_some()
                || (receipt.kind != CredentialOperationKind::Disconnect
                    && matches!(
                        receipt.status,
                        CredentialOperationStatus::Storing | CredentialOperationStatus::Committed
                    ))
            {
                return Err(ProviderCredentialError::Corrupt);
            }
            if receipt.status != CredentialOperationStatus::Committed
                && receipt.connection.is_some()
            {
                return Err(ProviderCredentialError::Corrupt);
            }
        }
        if let Some(active) = &self.active {
            validate_stored_connection(active, &self.profile_id, self.provider)?;
            if active.summary.reference.revision != self.revision
                || !self.operations.iter().any(|operation| {
                    operation.candidate.as_ref() == Some(active)
                        && operation.receipt.status == CredentialOperationStatus::Committed
                        && operation
                            .parts
                            .iter()
                            .all(|part| *part == SecretPartState::Verified)
                })
            {
                return Err(ProviderCredentialError::Corrupt);
            }
        }
        if self.row_version == 0
            && (self.revision != 0
                || self.tombstone
                || self.active.is_some()
                || !self.operations.is_empty())
        {
            return Err(ProviderCredentialError::Corrupt);
        }
        Ok(())
    }
}

fn validate_stored_connection(
    connection: &StoredConnection,
    profile_id: &str,
    provider: Provider,
) -> CredentialResult<()> {
    let reference = &connection.summary.reference;
    let max_parts = match provider {
        Provider::Linear => MAX_LINEAR_SECRET_BYTES.div_ceil(SECRET_CHUNK_BYTES),
        Provider::Jira => MAX_SECRET_PARTS,
    };
    if reference.profile_id != profile_id
        || reference.provider != provider
        || !valid_connection_id(&reference.connection_id)
        || reference.revision == 0
        || !valid_revision(reference.revision)
        || !valid_operation_id(&connection.operation_id)
        || connection.part_count == 0
        || usize::from(connection.part_count) > max_parts
    {
        return Err(ProviderCredentialError::Corrupt);
    }
    connection
        .summary
        .metadata
        .validate(provider)
        .map_err(|_| ProviderCredentialError::Corrupt)
}

// These capsules live only inside the OS key store. The fingerprint never enters the ledger
// or a public DTO. Even a maximum chunk stays below Windows' 2,560-byte blob limit.
const CAPSULE_MAGIC: &[u8; 8] = b"AKRAPC01";
const CAPSULE_HEADER: usize = 80;

fn capsule_binding(connection: &StoredConnection) -> CredentialResult<[u8; 32]> {
    use sha2::{Digest, Sha256};
    let metadata = serde_json::to_vec(connection).map_err(|_| ProviderCredentialError::Corrupt)?;
    let mut digest = Sha256::new();
    digest.update(b"akra-provider-credential-binding-v1\0");
    digest.update(metadata);
    Ok(digest.finalize().into())
}

fn secret_fingerprint(binding: &[u8; 32], token: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(b"akra-provider-credential-secret-v1\0");
    digest.update(binding);
    digest.update(token);
    digest.finalize().into()
}

pub(crate) fn encode_secret_capsules(
    connection: &StoredConnection,
    token: &SecretToken,
) -> CredentialResult<Vec<Zeroizing<Vec<u8>>>> {
    if token.expose().len().div_ceil(SECRET_CHUNK_BYTES) != usize::from(connection.part_count) {
        return Err(ProviderCredentialError::Corrupt);
    }
    let binding = capsule_binding(connection)?;
    let fingerprint = Zeroizing::new(secret_fingerprint(&binding, token.expose()));
    let mut capsules = Vec::with_capacity(usize::from(connection.part_count));
    for (index, chunk) in token.expose().chunks(SECRET_CHUNK_BYTES).enumerate() {
        let mut capsule = Zeroizing::new(Vec::with_capacity(CAPSULE_HEADER + chunk.len()));
        capsule.extend_from_slice(CAPSULE_MAGIC);
        capsule.extend_from_slice(&binding);
        capsule.extend_from_slice(&fingerprint[..]);
        capsule.extend_from_slice(&(index as u16).to_le_bytes());
        capsule.extend_from_slice(&connection.part_count.to_le_bytes());
        capsule.extend_from_slice(&(token.expose().len() as u32).to_le_bytes());
        capsule.extend_from_slice(chunk);
        capsules.push(capsule);
    }
    Ok(capsules)
}

pub(crate) fn capsule_has_binding(
    connection: &StoredConnection,
    index: u16,
    capsule: &[u8],
) -> CredentialResult<bool> {
    if capsule.len() <= CAPSULE_HEADER || capsule.len() > CAPSULE_HEADER + SECRET_CHUNK_BYTES {
        return Ok(false);
    }
    Ok(&capsule[..8] == CAPSULE_MAGIC
        && capsule[8..40] == capsule_binding(connection)?
        && capsule[72..74] == index.to_le_bytes()
        && capsule[74..76] == connection.part_count.to_le_bytes())
}

pub(crate) fn decode_secret_capsules(
    connection: &StoredConnection,
    capsules: &[Zeroizing<Vec<u8>>],
) -> CredentialResult<SecretToken> {
    if capsules.len() != usize::from(connection.part_count) || capsules.is_empty() {
        return Err(ProviderCredentialError::Corrupt);
    }
    let mut token = Zeroizing::new(Vec::new());
    let mut fingerprint = Zeroizing::new([0u8; 32]);
    let mut total = 0usize;
    for (index, capsule) in capsules.iter().enumerate() {
        if !capsule_has_binding(connection, index as u16, capsule)? {
            return Err(ProviderCredentialError::Corrupt);
        }
        let declared = u32::from_le_bytes(
            capsule[76..80]
                .try_into()
                .map_err(|_| ProviderCredentialError::Corrupt)?,
        ) as usize;
        if index == 0 {
            total = declared;
            fingerprint.copy_from_slice(&capsule[40..72]);
            if total == 0
                || total > MAX_JIRA_SECRET_BYTES
                || total.div_ceil(SECRET_CHUNK_BYTES) != capsules.len()
            {
                return Err(ProviderCredentialError::Corrupt);
            }
        } else if total != declared || capsule[40..72] != fingerprint[..] {
            return Err(ProviderCredentialError::Corrupt);
        }
        let expected_chunk = (total - index * SECRET_CHUNK_BYTES).min(SECRET_CHUNK_BYTES);
        if capsule.len() - CAPSULE_HEADER != expected_chunk {
            return Err(ProviderCredentialError::Corrupt);
        }
        token.extend_from_slice(&capsule[CAPSULE_HEADER..]);
    }
    let actual = Zeroizing::new(secret_fingerprint(&capsule_binding(connection)?, &token));
    if actual[..] != fingerprint[..] || token.len() != total {
        return Err(ProviderCredentialError::Corrupt);
    }
    SecretToken::from_bytes(connection.summary.reference.provider, token.to_vec())
        .map_err(|_| ProviderCredentialError::Corrupt)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderCredentialChanged {
    pub provider: Provider,
    pub profile_id: String,
    pub revision: u64,
}

#[cfg(test)]
mod tests;
