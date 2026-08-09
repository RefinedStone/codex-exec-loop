use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

#[cfg(test)]
#[path = "pr_validation_projection_tests.rs"]
mod projection_tests;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PrValidationRecordKey(String);

impl PrValidationRecordKey {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        non_empty(value, "PR validation record key").map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrValidationTarget {
    repository: String,
    pull_request_number: u64,
}

impl PrValidationTarget {
    pub fn new(repository: impl Into<String>, pull_request_number: u64) -> Result<Self, String> {
        let repository = non_empty(repository, "PR validation repository")?;
        let mut segments = repository.split('/');
        if repository.len() > 200
            || segments.clone().count() != 2
            || segments.any(|segment| {
                segment.is_empty()
                    || !segment.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
                    })
            })
        {
            return Err(
                "PR validation repository must be a canonical GitHub owner/name".to_string(),
            );
        }
        if pull_request_number == 0 {
            return Err("PR validation pull request number must be positive".to_string());
        }
        Ok(Self {
            repository,
            pull_request_number,
        })
    }

    pub fn canonical_url(&self) -> String {
        format!(
            "https://github.com/{}/pull/{}",
            self.repository, self.pull_request_number
        )
    }

    pub fn repository(&self) -> &str {
        &self.repository
    }

    pub fn pull_request_number(&self) -> u64 {
        self.pull_request_number
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PrValidationCommitSha(String);

impl PrValidationCommitSha {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(
                "PR validation commit SHA must contain exactly 40 hexadecimal characters"
                    .to_string(),
            );
        }
        Ok(Self(value.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrValidationTargetShaSnapshot {
    source_sha: PrValidationCommitSha,
    base_sha: PrValidationCommitSha,
}

impl PrValidationTargetShaSnapshot {
    pub fn new(source_sha: PrValidationCommitSha, base_sha: PrValidationCommitSha) -> Self {
        Self {
            source_sha,
            base_sha,
        }
    }

    pub fn source_sha(&self) -> &PrValidationCommitSha {
        &self.source_sha
    }

    pub fn base_sha(&self) -> &PrValidationCommitSha {
        &self.base_sha
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntegrationMethod {
    GithubRebaseMerge,
    DistributorCherryPick,
}

impl IntegrationMethod {
    pub fn label(self) -> &'static str {
        match self {
            Self::GithubRebaseMerge => "github_rebase_merge",
            Self::DistributorCherryPick => "distributor_cherry_pick",
        }
    }
}

/// Durable proof that one reviewed source revision reached the integration branch.
/// Validation completion remains a separate lifecycle transition bound to `evidence_sha`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrationAttestation {
    method: IntegrationMethod,
    source_sha: PrValidationCommitSha,
    base_before_sha: Option<PrValidationCommitSha>,
    evidence_sha: PrValidationCommitSha,
    pull_request_number: Option<u64>,
    github_merge_sha: Option<PrValidationCommitSha>,
    integrated_at: DateTime<Utc>,
    remote_verified_at: DateTime<Utc>,
}

impl IntegrationAttestation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        method: IntegrationMethod,
        source_sha: PrValidationCommitSha,
        base_before_sha: Option<PrValidationCommitSha>,
        evidence_sha: PrValidationCommitSha,
        pull_request_number: Option<u64>,
        github_merge_sha: Option<PrValidationCommitSha>,
        integrated_at: DateTime<Utc>,
        remote_verified_at: DateTime<Utc>,
    ) -> Result<Self, String> {
        if pull_request_number == Some(0) {
            return Err("integration attestation pull request number must be positive".to_string());
        }
        if remote_verified_at < integrated_at {
            return Err(
                "integration attestation remote verification cannot predate integration"
                    .to_string(),
            );
        }
        match method {
            IntegrationMethod::GithubRebaseMerge
                if github_merge_sha.as_ref() != Some(&evidence_sha) =>
            {
                return Err(
                    "GitHub merge attestation must bind its merge SHA to the evidence SHA"
                        .to_string(),
                );
            }
            IntegrationMethod::DistributorCherryPick if github_merge_sha.is_some() => {
                return Err(
                    "distributor cherry-pick attestation cannot claim a GitHub merge SHA"
                        .to_string(),
                );
            }
            _ => {}
        }
        Ok(Self {
            method,
            source_sha,
            base_before_sha,
            evidence_sha,
            pull_request_number,
            github_merge_sha,
            integrated_at,
            remote_verified_at,
        })
    }

    pub fn method(&self) -> IntegrationMethod {
        self.method
    }

    pub fn source_sha(&self) -> &PrValidationCommitSha {
        &self.source_sha
    }

    pub fn base_before_sha(&self) -> Option<&PrValidationCommitSha> {
        self.base_before_sha.as_ref()
    }

    pub fn evidence_sha(&self) -> &PrValidationCommitSha {
        &self.evidence_sha
    }

    pub fn pull_request_number(&self) -> Option<u64> {
        self.pull_request_number
    }

    pub fn github_merge_sha(&self) -> Option<&PrValidationCommitSha> {
        self.github_merge_sha.as_ref()
    }

    pub fn integrated_at(&self) -> DateTime<Utc> {
        self.integrated_at
    }

    pub fn remote_verified_at(&self) -> DateTime<Utc> {
        self.remote_verified_at
    }

    fn has_same_authority_identity(&self, other: &Self) -> bool {
        self.method == other.method
            && self.source_sha == other.source_sha
            && self.base_before_sha == other.base_before_sha
            && self.evidence_sha == other.evidence_sha
            && self.pull_request_number == other.pull_request_number
            && self.github_merge_sha == other.github_merge_sha
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PrValidationFindingSource(String);

impl PrValidationFindingSource {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        non_empty(value, "PR validation finding source").map(Self)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PrValidationFindingKey {
    source: PrValidationFindingSource,
    provider_event_id: String,
}

impl PrValidationFindingKey {
    pub fn new(
        source: PrValidationFindingSource,
        provider_event_id: impl Into<String>,
    ) -> Result<Self, String> {
        Ok(Self {
            source,
            provider_event_id: non_empty(provider_event_id, "PR validation provider event id")?,
        })
    }

    pub fn source(&self) -> &PrValidationFindingSource {
        &self.source
    }

    pub fn provider_event_id(&self) -> &str {
        &self.provider_event_id
    }
}

// JSON object keys must serialize as strings. Length-prefixing the source keeps the persisted
// identity reversible even when provider IDs contain punctuation.
impl Serialize for PrValidationFindingKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!(
            "{}:{}{}",
            self.source.as_str().len(),
            self.source.as_str(),
            self.provider_event_id
        ))
    }
}

impl<'de> Deserialize<'de> for PrValidationFindingKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        let (length, identity) = encoded
            .split_once(':')
            .ok_or_else(|| serde::de::Error::custom("invalid PR validation finding key"))?;
        let source_length = length
            .parse::<usize>()
            .map_err(|_| serde::de::Error::custom("invalid PR validation finding source length"))?;
        if source_length > identity.len() || !identity.is_char_boundary(source_length) {
            return Err(serde::de::Error::custom(
                "invalid PR validation finding source boundary",
            ));
        }
        let (source, provider_event_id) = identity.split_at(source_length);
        Self::new(
            PrValidationFindingSource::new(source).map_err(serde::de::Error::custom)?,
            provider_event_id,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrValidationFinding {
    key: PrValidationFindingKey,
    target_sha: PrValidationCommitSha,
    summary: String,
}

impl PrValidationFinding {
    pub fn new(
        key: PrValidationFindingKey,
        target_sha: PrValidationCommitSha,
        summary: impl Into<String>,
    ) -> Result<Self, String> {
        Ok(Self {
            key,
            target_sha,
            summary: non_empty(summary, "PR validation finding summary")?,
        })
    }

    pub fn key(&self) -> &PrValidationFindingKey {
        &self.key
    }

    pub fn target_sha(&self) -> &PrValidationCommitSha {
        &self.target_sha
    }

    pub fn summary(&self) -> &str {
        &self.summary
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrValidationRemediationCorrelation {
    finding_key: PrValidationFindingKey,
    remediation_key: PrValidationRecordKey,
}

impl PrValidationRemediationCorrelation {
    pub fn new(
        finding_key: PrValidationFindingKey,
        remediation_key: PrValidationRecordKey,
    ) -> Self {
        Self {
            finding_key,
            remediation_key,
        }
    }

    pub fn finding_key(&self) -> &PrValidationFindingKey {
        &self.finding_key
    }

    pub fn remediation_key(&self) -> &PrValidationRecordKey {
        &self.remediation_key
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrValidationPhase {
    Registered,
    PreMergeObservation,
    RemediationQueued,
    RemediationRunning,
    PostMergeObservation,
    Settled,
    Blocked,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PrValidationCheckKind {
    CheckRun,
    WorkflowRun,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PrValidationRequiredCheckKey {
    kind: PrValidationCheckKind,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrValidationRequiredCheck {
    key: PrValidationRequiredCheckKey,
    successful: bool,
}

impl PrValidationRequiredCheck {
    pub fn new(
        kind: PrValidationCheckKind,
        name: impl Into<String>,
        successful: bool,
    ) -> Result<Self, String> {
        Ok(Self {
            key: PrValidationRequiredCheckKey {
                kind,
                name: non_empty(name, "PR validation required check name")?,
            },
            successful,
        })
    }

    pub fn key(&self) -> &PrValidationRequiredCheckKey {
        &self.key
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PrValidationProviderKey(String);

impl PrValidationProviderKey {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        non_empty(value, "PR validation provider key").map(Self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum PrValidationProviderCompletionState {
    Pending,
    Watchable,
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrValidationProviderCompletion {
    provider: PrValidationProviderKey,
    state: PrValidationProviderCompletionState,
}

impl PrValidationProviderCompletion {
    pub fn pending(provider: PrValidationProviderKey) -> Self {
        Self {
            provider,
            state: PrValidationProviderCompletionState::Pending,
        }
    }

    pub fn watchable(provider: PrValidationProviderKey) -> Self {
        Self {
            provider,
            state: PrValidationProviderCompletionState::Watchable,
        }
    }

    pub fn terminal(provider: PrValidationProviderKey) -> Self {
        Self {
            provider,
            state: PrValidationProviderCompletionState::Terminal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrValidationCatchUpState {
    NotObserved,
    UnseenRelevantEvents,
    NoUnseenRelevantEvents,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrValidationCompletionBlocker {
    ProviderNotTerminal(PrValidationProviderKey),
    ProviderHasNoCompletionContract(PrValidationProviderKey),
    RequiredCheckNotSuccessful(PrValidationRequiredCheckKey),
    FinalCatchUpNotObserved,
    FinalCatchUpHasUnseenRelevantEvents,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrValidationCompletion {
    target_sha: PrValidationCommitSha,
    providers: Vec<PrValidationProviderCompletion>,
    required_checks: Vec<PrValidationRequiredCheck>,
    catch_up: PrValidationCatchUpState,
}

impl PrValidationCompletion {
    pub fn new(
        target_sha: PrValidationCommitSha,
        providers: Vec<PrValidationProviderCompletion>,
        required_checks: Vec<PrValidationRequiredCheck>,
        catch_up: PrValidationCatchUpState,
    ) -> Self {
        Self {
            target_sha,
            providers,
            required_checks,
            catch_up,
        }
    }

    pub fn blockers(&self) -> Vec<PrValidationCompletionBlocker> {
        let mut blockers = Vec::new();
        for provider in &self.providers {
            match provider.state {
                PrValidationProviderCompletionState::Pending => blockers.push(
                    PrValidationCompletionBlocker::ProviderNotTerminal(provider.provider.clone()),
                ),
                PrValidationProviderCompletionState::Watchable => blockers.push(
                    PrValidationCompletionBlocker::ProviderHasNoCompletionContract(
                        provider.provider.clone(),
                    ),
                ),
                PrValidationProviderCompletionState::Terminal => {}
            }
        }
        blockers.extend(
            self.required_checks
                .iter()
                .filter(|check| !check.successful)
                .map(|check| {
                    PrValidationCompletionBlocker::RequiredCheckNotSuccessful(check.key.clone())
                }),
        );
        match self.catch_up {
            PrValidationCatchUpState::NotObserved => {
                blockers.push(PrValidationCompletionBlocker::FinalCatchUpNotObserved)
            }
            PrValidationCatchUpState::UnseenRelevantEvents => {
                blockers.push(PrValidationCompletionBlocker::FinalCatchUpHasUnseenRelevantEvents)
            }
            PrValidationCatchUpState::NoUnseenRelevantEvents => {}
        }
        blockers
    }

    pub fn is_complete(&self) -> bool {
        self.blockers().is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrValidationTerminalReason {
    AllConfiguredSourcesComplete,
    PullRequestClosedWithoutMerge,
    IntegrationEvidenceMissing,
    IntegrationAuthorityConflict,
    ObservationFailed,
    RemediationAdmissionFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrValidationRecoveryAction {
    None,
    PollAgain,
    RunCorrelatedRemediation,
    CompleteCorrelatedRemediation,
    ReopenOrReplacePullRequest,
    RestoreIntegrationEvidence,
    RerunWithFreshRecord,
    RestoreQueueAndRerun,
}

impl PrValidationRecoveryAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::PollAgain => "poll validation again after provider activity",
            Self::RunCorrelatedRemediation => "run the remediation correlated to the finding",
            Self::CompleteCorrelatedRemediation => "complete the correlated remediation task",
            Self::ReopenOrReplacePullRequest => {
                "reopen the pull request or register validation for its replacement"
            }
            Self::RestoreIntegrationEvidence => {
                "restore trusted integration evidence and rerun validation"
            }
            Self::RerunWithFreshRecord => {
                "rerun validation to create a fresh Akra validation record"
            }
            Self::RestoreQueueAndRerun => {
                "restore the planning queue and rerun validation with a fresh Akra record"
            }
        }
    }
}

impl PrValidationTerminalReason {
    pub fn label(&self) -> &'static str {
        match self {
            Self::AllConfiguredSourcesComplete => "all configured validation sources completed",
            Self::PullRequestClosedWithoutMerge => "pull request closed without merge",
            Self::IntegrationEvidenceMissing => "integration evidence is missing",
            Self::IntegrationAuthorityConflict => {
                "integration authority conflicts with trusted evidence"
            }
            Self::ObservationFailed => "trusted validation observation failed",
            Self::RemediationAdmissionFailed => "validation remediation admission failed",
        }
    }

    pub fn recovery_action(&self) -> PrValidationRecoveryAction {
        match self {
            Self::AllConfiguredSourcesComplete => PrValidationRecoveryAction::None,
            Self::PullRequestClosedWithoutMerge => {
                PrValidationRecoveryAction::ReopenOrReplacePullRequest
            }
            Self::IntegrationEvidenceMissing => {
                PrValidationRecoveryAction::RestoreIntegrationEvidence
            }
            Self::IntegrationAuthorityConflict => PrValidationRecoveryAction::RerunWithFreshRecord,
            Self::ObservationFailed => PrValidationRecoveryAction::RerunWithFreshRecord,
            Self::RemediationAdmissionFailed => PrValidationRecoveryAction::RestoreQueueAndRerun,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrValidationEvent {
    BeginPreMergeObservation,
    FindingObserved(PrValidationFinding),
    RemediationQueued(PrValidationRemediationCorrelation),
    RemediationStarted {
        finding_key: PrValidationFindingKey,
    },
    RemediationCompleted {
        finding_key: PrValidationFindingKey,
    },
    TargetShaChanged(PrValidationTargetShaSnapshot),
    IntegrationAttested(IntegrationAttestation),
    MergeObserved(PrValidationCommitSha),
    BeginPostMergeObservation,
    ObservationCheckpointed {
        delivery_revision: u64,
        cursor: Option<String>,
        evidence_fingerprint: String,
    },
    Settle(PrValidationCompletion),
    Block(PrValidationTerminalReason),
    Fail(PrValidationTerminalReason),
}

impl PrValidationEvent {
    fn label(&self) -> &'static str {
        match self {
            Self::BeginPreMergeObservation => "begin_pre_merge_observation",
            Self::FindingObserved(_) => "finding_observed",
            Self::RemediationQueued(_) => "remediation_queued",
            Self::RemediationStarted { .. } => "remediation_started",
            Self::RemediationCompleted { .. } => "remediation_completed",
            Self::TargetShaChanged(_) => "target_sha_changed",
            Self::IntegrationAttested(_) => "integration_attested",
            Self::MergeObserved(_) => "merge_observed",
            Self::BeginPostMergeObservation => "begin_post_merge_observation",
            Self::ObservationCheckpointed { .. } => "observation_checkpointed",
            Self::Settle(_) => "settle",
            Self::Block(_) => "block",
            Self::Fail(_) => "fail",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrValidationTransitionRejection {
    InvalidTransition {
        phase: PrValidationPhase,
        event: &'static str,
    },
    TargetShaMismatch {
        expected: PrValidationCommitSha,
        observed: PrValidationCommitSha,
    },
    IntegrationSourceShaMismatch {
        expected: PrValidationCommitSha,
        observed: PrValidationCommitSha,
    },
    IntegrationPullRequestMismatch {
        expected: u64,
        observed: u64,
    },
    IntegrationAuthorityConflict {
        expected: IntegrationAttestation,
        observed: IntegrationAttestation,
    },
    FindingNotObserved(PrValidationFindingKey),
    RemediationNotQueued(PrValidationFindingKey),
    CompletionIncomplete(Vec<PrValidationCompletionBlocker>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrValidationRecord {
    key: PrValidationRecordKey,
    target: PrValidationTarget,
    target_shas: PrValidationTargetShaSnapshot,
    phase: PrValidationPhase,
    findings: BTreeMap<PrValidationFindingKey, PrValidationFinding>,
    remediations: BTreeMap<PrValidationFindingKey, PrValidationRemediationCorrelation>,
    active_remediation: Option<PrValidationFindingKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    integration_attestation: Option<IntegrationAttestation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    merge_sha: Option<PrValidationCommitSha>,
    observation_revision: u64,
    observation_cursor: Option<String>,
    evidence_fingerprint: Option<String>,
    post_merge_checkpoint_revision: Option<u64>,
    completion: Option<PrValidationCompletion>,
    terminal_reason: Option<PrValidationTerminalReason>,
}

impl PrValidationRecord {
    pub fn register(
        key: PrValidationRecordKey,
        target: PrValidationTarget,
        target_shas: PrValidationTargetShaSnapshot,
    ) -> Self {
        Self {
            key,
            target,
            target_shas,
            phase: PrValidationPhase::Registered,
            findings: BTreeMap::new(),
            remediations: BTreeMap::new(),
            active_remediation: None,
            integration_attestation: None,
            merge_sha: None,
            observation_revision: 0,
            observation_cursor: None,
            evidence_fingerprint: None,
            post_merge_checkpoint_revision: None,
            completion: None,
            terminal_reason: None,
        }
    }

    pub fn key(&self) -> &PrValidationRecordKey {
        &self.key
    }

    pub fn target(&self) -> &PrValidationTarget {
        &self.target
    }

    pub fn target_shas(&self) -> &PrValidationTargetShaSnapshot {
        &self.target_shas
    }

    pub fn phase(&self) -> PrValidationPhase {
        self.phase
    }

    pub fn finding_keys(&self) -> Vec<PrValidationFindingKey> {
        self.findings.keys().cloned().collect()
    }

    pub fn remediation_for(
        &self,
        finding_key: &PrValidationFindingKey,
    ) -> Option<&PrValidationRemediationCorrelation> {
        self.remediations.get(finding_key)
    }

    pub fn remediation_for_task(
        &self,
        task_id: &str,
    ) -> Option<&PrValidationRemediationCorrelation> {
        self.remediations
            .values()
            .find(|correlation| correlation.remediation_key.as_str() == task_id)
    }

    pub fn merge_sha(&self) -> Option<&PrValidationCommitSha> {
        self.integration_attestation
            .as_ref()
            .and_then(IntegrationAttestation::github_merge_sha)
            .or(self.merge_sha.as_ref())
    }

    pub fn integration_attestation(&self) -> Option<&IntegrationAttestation> {
        self.integration_attestation.as_ref()
    }

    pub fn evidence_sha(&self) -> Option<&PrValidationCommitSha> {
        self.integration_attestation
            .as_ref()
            .map(IntegrationAttestation::evidence_sha)
            .or(self.merge_sha.as_ref())
    }

    pub fn with_migrated_legacy_attestation(
        &self,
        observed_at: DateTime<Utc>,
    ) -> Result<Self, String> {
        let Some(legacy_merge_sha) = self.merge_sha.as_ref() else {
            return Ok(self.clone());
        };
        if let Some(attestation) = self.integration_attestation.as_ref() {
            if attestation.method() != IntegrationMethod::GithubRebaseMerge
                || attestation.evidence_sha() != legacy_merge_sha
            {
                return Err(
                    "legacy merge SHA conflicts with the persisted integration attestation"
                        .to_string(),
                );
            }
            let mut migrated = self.clone();
            migrated.merge_sha = None;
            return Ok(migrated);
        }
        let attestation = IntegrationAttestation::new(
            IntegrationMethod::GithubRebaseMerge,
            self.target_shas.source_sha.clone(),
            Some(self.target_shas.base_sha.clone()),
            legacy_merge_sha.clone(),
            Some(self.target.pull_request_number),
            Some(legacy_merge_sha.clone()),
            observed_at,
            observed_at,
        )?;
        let mut migrated = self.clone();
        migrated.integration_attestation = Some(attestation);
        migrated.merge_sha = None;
        Ok(migrated)
    }

    pub fn observation_revision(&self) -> u64 {
        self.observation_revision
    }

    pub fn observation_cursor(&self) -> Option<&str> {
        self.observation_cursor.as_deref()
    }

    pub fn evidence_fingerprint(&self) -> Option<&str> {
        self.evidence_fingerprint.as_deref()
    }

    pub fn has_post_merge_checkpoint(&self) -> bool {
        self.post_merge_checkpoint_revision.is_some()
    }

    pub fn terminal_reason(&self) -> Option<&PrValidationTerminalReason> {
        self.terminal_reason.as_ref()
    }

    pub fn operator_summary(&self) -> PrValidationOperatorSummary {
        let state = match self.phase {
            PrValidationPhase::Settled => PrValidationOperatorState::Terminal,
            PrValidationPhase::Blocked => PrValidationOperatorState::Blocked,
            PrValidationPhase::Failed => PrValidationOperatorState::Failed,
            PrValidationPhase::RemediationQueued | PrValidationPhase::RemediationRunning => {
                PrValidationOperatorState::Remediation
            }
            PrValidationPhase::PreMergeObservation | PrValidationPhase::PostMergeObservation
                if !self.findings.is_empty() =>
            {
                PrValidationOperatorState::Blocking
            }
            PrValidationPhase::Registered
            | PrValidationPhase::PreMergeObservation
            | PrValidationPhase::PostMergeObservation => PrValidationOperatorState::Pending,
        };
        let reason = self.operator_reason(state);
        let recovery = self
            .terminal_reason
            .as_ref()
            .map(PrValidationTerminalReason::recovery_action)
            .unwrap_or_else(|| state.recovery_action());
        let blocker =
            (!matches!(state, PrValidationOperatorState::Terminal)).then(|| reason.clone());
        let correlations = self
            .remediations
            .values()
            .take(4)
            .map(|correlation| PrValidationOperatorCorrelation {
                finding: format!(
                    "{}:{}",
                    correlation.finding_key.source.as_str(),
                    opaque_reference(&correlation.finding_key.provider_event_id)
                ),
                remediation_akra_id: bounded_identifier(correlation.remediation_key.as_str()),
            })
            .collect();
        PrValidationOperatorSummary {
            akra_id: bounded_identifier(self.key.as_str()),
            canonical_pr_url: self.target.canonical_url(),
            pull_request_number: self.target.pull_request_number,
            state,
            phase: self.phase,
            target_short_sha: self.target_shas.source_sha.as_str()[..12].to_string(),
            integration_method: self
                .integration_attestation
                .as_ref()
                .map(IntegrationAttestation::method)
                .or_else(|| {
                    self.merge_sha
                        .as_ref()
                        .map(|_| IntegrationMethod::GithubRebaseMerge)
                }),
            evidence_short_sha: self
                .evidence_sha()
                .map(|evidence_sha| evidence_sha.as_str()[..12].to_string()),
            merge_short_sha: self
                .merge_sha()
                .map(|merge_sha| merge_sha.as_str()[..12].to_string()),
            finding_count: self.findings.len(),
            remediation_count: self.remediations.len(),
            reason,
            blocker,
            recovery,
            recovery_action: recovery.label().to_string(),
            correlations,
            observation_revision: self.observation_revision,
            post_merge_checkpoint_observed: self.post_merge_checkpoint_revision.is_some(),
        }
    }

    fn operator_reason(&self, state: PrValidationOperatorState) -> String {
        if let Some(reason) = self.terminal_reason.as_ref() {
            return reason.label().to_string();
        }
        match state {
            PrValidationOperatorState::Pending => {
                if self.evidence_sha().is_some() {
                    "integration is attested; validation sources are not complete".to_string()
                } else {
                    "validation sources or final catch-up are not complete".to_string()
                }
            }
            PrValidationOperatorState::Blocking => self
                .findings
                .keys()
                .next()
                .map(|key| format!("{} finding requires remediation", key.source.as_str()))
                .unwrap_or_else(|| "validation finding requires remediation".to_string()),
            PrValidationOperatorState::Remediation => self
                .active_remediation
                .as_ref()
                .or_else(|| self.remediations.keys().next())
                .map(|key| format!("{} finding has correlated remediation", key.source.as_str()))
                .unwrap_or_else(|| "correlated remediation is pending".to_string()),
            PrValidationOperatorState::Terminal => {
                "all configured validation sources completed".to_string()
            }
            PrValidationOperatorState::Blocked => "validation is blocked".to_string(),
            PrValidationOperatorState::Failed => "validation failed".to_string(),
        }
    }

    pub fn transition(
        &self,
        event: PrValidationEvent,
    ) -> Result<Self, PrValidationTransitionRejection> {
        let mut next = self.clone();
        if let PrValidationEvent::Settle(completion) = &event {
            let expected = next
                .evidence_sha()
                .unwrap_or_else(|| next.target_shas.source_sha());
            if &completion.target_sha != expected {
                return Err(PrValidationTransitionRejection::TargetShaMismatch {
                    expected: expected.clone(),
                    observed: completion.target_sha.clone(),
                });
            }
        }
        if let PrValidationEvent::FindingObserved(finding) = &event
            && finding.target_sha != *next.target_shas.source_sha()
        {
            return Err(PrValidationTransitionRejection::TargetShaMismatch {
                expected: next.target_shas.source_sha().clone(),
                observed: finding.target_sha.clone(),
            });
        }
        if let PrValidationEvent::IntegrationAttested(attestation) = &event {
            if attestation.source_sha() != next.target_shas.source_sha() {
                return Err(
                    PrValidationTransitionRejection::IntegrationSourceShaMismatch {
                        expected: next.target_shas.source_sha().clone(),
                        observed: attestation.source_sha().clone(),
                    },
                );
            }
            if let Some(observed) = attestation.pull_request_number()
                && observed != next.target.pull_request_number
            {
                return Err(
                    PrValidationTransitionRejection::IntegrationPullRequestMismatch {
                        expected: next.target.pull_request_number,
                        observed,
                    },
                );
            }
            if let Some(expected) = next.integration_attestation.as_ref() {
                if expected.has_same_authority_identity(attestation) {
                    return Ok(next);
                }
                return Err(
                    PrValidationTransitionRejection::IntegrationAuthorityConflict {
                        expected: expected.clone(),
                        observed: attestation.clone(),
                    },
                );
            }
            if let Some(legacy_merge_sha) = next.merge_sha.as_ref()
                && (attestation.method() != IntegrationMethod::GithubRebaseMerge
                    || attestation.evidence_sha() != legacy_merge_sha)
            {
                let legacy = IntegrationAttestation::new(
                    IntegrationMethod::GithubRebaseMerge,
                    next.target_shas.source_sha.clone(),
                    Some(next.target_shas.base_sha.clone()),
                    legacy_merge_sha.clone(),
                    Some(next.target.pull_request_number),
                    Some(legacy_merge_sha.clone()),
                    attestation.integrated_at(),
                    attestation.remote_verified_at(),
                )
                .expect("legacy GitHub merge identity is structurally valid");
                return Err(
                    PrValidationTransitionRejection::IntegrationAuthorityConflict {
                        expected: legacy,
                        observed: attestation.clone(),
                    },
                );
            }
        }

        match event {
            PrValidationEvent::BeginPreMergeObservation
                if next.phase == PrValidationPhase::Registered =>
            {
                next.phase = PrValidationPhase::PreMergeObservation;
            }
            PrValidationEvent::FindingObserved(finding)
                if matches!(
                    next.phase,
                    PrValidationPhase::PreMergeObservation
                        | PrValidationPhase::PostMergeObservation
                ) =>
            {
                next.findings.insert(finding.key.clone(), finding);
            }
            PrValidationEvent::RemediationQueued(correlation)
                if matches!(
                    next.phase,
                    PrValidationPhase::PreMergeObservation
                        | PrValidationPhase::PostMergeObservation
                ) =>
            {
                if !next.findings.contains_key(&correlation.finding_key) {
                    return Err(PrValidationTransitionRejection::FindingNotObserved(
                        correlation.finding_key,
                    ));
                }
                next.remediations
                    .insert(correlation.finding_key.clone(), correlation);
                next.phase = PrValidationPhase::RemediationQueued;
            }
            PrValidationEvent::RemediationStarted { finding_key }
                if next.phase == PrValidationPhase::RemediationQueued =>
            {
                if !next.remediations.contains_key(&finding_key) {
                    return Err(PrValidationTransitionRejection::RemediationNotQueued(
                        finding_key,
                    ));
                }
                next.active_remediation = Some(finding_key);
                next.phase = PrValidationPhase::RemediationRunning;
            }
            PrValidationEvent::RemediationCompleted { finding_key }
                if next.phase == PrValidationPhase::RemediationRunning
                    && next.active_remediation.as_ref() == Some(&finding_key) =>
            {
                next.active_remediation = None;
                next.phase = if next.evidence_sha().is_some() {
                    PrValidationPhase::PostMergeObservation
                } else {
                    PrValidationPhase::PreMergeObservation
                };
            }
            PrValidationEvent::TargetShaChanged(target_shas)
                if !matches!(
                    next.phase,
                    PrValidationPhase::Settled
                        | PrValidationPhase::Blocked
                        | PrValidationPhase::Failed
                ) =>
            {
                next.target_shas = target_shas;
                next.phase = PrValidationPhase::PreMergeObservation;
                next.findings.clear();
                next.remediations.clear();
                next.active_remediation = None;
                next.integration_attestation = None;
                next.merge_sha = None;
                next.observation_cursor = None;
                next.evidence_fingerprint = None;
                next.post_merge_checkpoint_revision = None;
                next.completion = None;
                next.terminal_reason = None;
            }
            PrValidationEvent::IntegrationAttested(attestation)
                if matches!(
                    next.phase,
                    PrValidationPhase::Registered
                        | PrValidationPhase::PreMergeObservation
                        | PrValidationPhase::RemediationQueued
                        | PrValidationPhase::RemediationRunning
                        | PrValidationPhase::PostMergeObservation
                ) =>
            {
                next.integration_attestation = Some(attestation);
                next.merge_sha = None;
                if matches!(
                    next.phase,
                    PrValidationPhase::Registered | PrValidationPhase::PreMergeObservation
                ) {
                    next.phase = PrValidationPhase::PostMergeObservation;
                }
            }
            PrValidationEvent::MergeObserved(merge_sha)
                if next.phase == PrValidationPhase::PreMergeObservation
                    && next.integration_attestation.is_none() =>
            {
                next.merge_sha = Some(merge_sha);
            }
            PrValidationEvent::BeginPostMergeObservation
                if next.phase == PrValidationPhase::PreMergeObservation
                    && next.evidence_sha().is_some() =>
            {
                next.phase = PrValidationPhase::PostMergeObservation;
            }
            PrValidationEvent::ObservationCheckpointed {
                delivery_revision,
                cursor,
                evidence_fingerprint,
            } if !matches!(
                next.phase,
                PrValidationPhase::Settled | PrValidationPhase::Blocked | PrValidationPhase::Failed
            ) && delivery_revision > next.observation_revision =>
            {
                next.observation_revision = delivery_revision;
                next.observation_cursor = cursor;
                next.evidence_fingerprint = Some(evidence_fingerprint);
                if next.phase == PrValidationPhase::PostMergeObservation {
                    next.post_merge_checkpoint_revision = Some(delivery_revision);
                }
            }
            PrValidationEvent::Settle(completion)
                if next.phase == PrValidationPhase::PostMergeObservation =>
            {
                let blockers = completion.blockers();
                if !blockers.is_empty() {
                    return Err(PrValidationTransitionRejection::CompletionIncomplete(
                        blockers,
                    ));
                }
                next.completion = Some(completion);
                next.terminal_reason =
                    Some(PrValidationTerminalReason::AllConfiguredSourcesComplete);
                next.phase = PrValidationPhase::Settled;
            }
            PrValidationEvent::Block(reason)
                if !matches!(
                    next.phase,
                    PrValidationPhase::Settled
                        | PrValidationPhase::Blocked
                        | PrValidationPhase::Failed
                ) =>
            {
                next.terminal_reason = Some(reason);
                next.phase = PrValidationPhase::Blocked;
            }
            PrValidationEvent::Fail(reason)
                if !matches!(
                    next.phase,
                    PrValidationPhase::Settled
                        | PrValidationPhase::Blocked
                        | PrValidationPhase::Failed
                ) =>
            {
                next.terminal_reason = Some(reason);
                next.phase = PrValidationPhase::Failed;
            }
            invalid => {
                return Err(PrValidationTransitionRejection::InvalidTransition {
                    phase: next.phase,
                    event: invalid.label(),
                });
            }
        }
        Ok(next)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrValidationOperatorState {
    Pending,
    Blocking,
    Remediation,
    Terminal,
    Blocked,
    Failed,
}

impl PrValidationOperatorState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Blocking => "blocking",
            Self::Remediation => "remediation",
            Self::Terminal => "terminal",
            Self::Blocked => "blocked",
            Self::Failed => "failed",
        }
    }

    fn recovery_action(self) -> PrValidationRecoveryAction {
        match self {
            Self::Pending => PrValidationRecoveryAction::PollAgain,
            Self::Blocking => PrValidationRecoveryAction::RunCorrelatedRemediation,
            Self::Remediation => PrValidationRecoveryAction::CompleteCorrelatedRemediation,
            Self::Terminal => PrValidationRecoveryAction::None,
            Self::Blocked => PrValidationRecoveryAction::ReopenOrReplacePullRequest,
            Self::Failed => PrValidationRecoveryAction::RerunWithFreshRecord,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrValidationOperatorCorrelation {
    pub finding: String,
    pub remediation_akra_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrValidationOperatorSummary {
    pub akra_id: String,
    pub canonical_pr_url: String,
    pub pull_request_number: u64,
    pub state: PrValidationOperatorState,
    pub phase: PrValidationPhase,
    pub target_short_sha: String,
    pub integration_method: Option<IntegrationMethod>,
    pub evidence_short_sha: Option<String>,
    pub merge_short_sha: Option<String>,
    pub finding_count: usize,
    pub remediation_count: usize,
    pub reason: String,
    pub blocker: Option<String>,
    pub recovery: PrValidationRecoveryAction,
    pub recovery_action: String,
    pub correlations: Vec<PrValidationOperatorCorrelation>,
    pub observation_revision: u64,
    pub post_merge_checkpoint_observed: bool,
}

impl PrValidationOperatorSummary {
    pub fn phase_label(&self) -> &'static str {
        match self.phase {
            PrValidationPhase::Registered => "registered",
            PrValidationPhase::PreMergeObservation => "pre_merge_observation",
            PrValidationPhase::RemediationQueued => "remediation_queued",
            PrValidationPhase::RemediationRunning => "remediation_running",
            PrValidationPhase::PostMergeObservation => "post_merge_observation",
            PrValidationPhase::Settled => "settled",
            PrValidationPhase::Blocked => "blocked",
            PrValidationPhase::Failed => "failed",
        }
    }

    pub fn next_action(&self) -> &str {
        &self.recovery_action
    }

    pub fn compact_label(&self) -> String {
        let integration = self
            .integration_method
            .zip(self.evidence_short_sha.as_deref())
            .map(|(method, evidence)| format!(" · {} {evidence}", method.label()))
            .unwrap_or_default();
        format!(
            "PR #{} {}{} · findings {} · remediation {}",
            self.pull_request_number,
            self.state.label(),
            integration,
            self.finding_count,
            self.remediation_count
        )
    }
}

fn opaque_reference(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    format!("{:x}", digest)[..12].to_string()
}

fn bounded_identifier(value: &str) -> String {
    if value.len() <= 80
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/' | b'#')
        })
    {
        value.to_string()
    } else {
        format!("opaque:{}", opaque_reference(value))
    }
}

fn non_empty(value: impl Into<String>, label: &str) -> Result<String, String> {
    let value = value.into().trim().to_string();
    if value.is_empty() {
        Err(format!("{label} must not be empty"))
    } else {
        Ok(value)
    }
}
