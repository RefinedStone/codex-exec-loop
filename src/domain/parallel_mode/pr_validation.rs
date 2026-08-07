use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
        if pull_request_number == 0 {
            return Err("PR validation pull request number must be positive".to_string());
        }
        Ok(Self {
            repository,
            pull_request_number,
        })
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
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PrValidationFindingSource(String);

impl PrValidationFindingSource {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        non_empty(value, "PR validation finding source").map(Self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrValidationPhase {
    Registered,
    PreMergeObservation,
    RemediationQueued,
    RemediationRunning,
    PostMergeObservation,
    Settled,
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
    terminal: bool,
}

impl PrValidationRequiredCheck {
    pub fn new(
        kind: PrValidationCheckKind,
        name: impl Into<String>,
        terminal: bool,
    ) -> Result<Self, String> {
        Ok(Self {
            key: PrValidationRequiredCheckKey {
                kind,
                name: non_empty(name, "PR validation required check name")?,
            },
            terminal,
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
    RequiredCheckNotTerminal(PrValidationRequiredCheckKey),
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
                .filter(|check| !check.terminal)
                .map(|check| {
                    PrValidationCompletionBlocker::RequiredCheckNotTerminal(check.key.clone())
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrValidationEvent {
    BeginPreMergeObservation,
    FindingObserved(PrValidationFinding),
    RemediationQueued(PrValidationRemediationCorrelation),
    RemediationStarted { finding_key: PrValidationFindingKey },
    RemediationCompleted { finding_key: PrValidationFindingKey },
    TargetShaChanged(PrValidationTargetShaSnapshot),
    MergeObserved(PrValidationCommitSha),
    BeginPostMergeObservation,
    Settle(PrValidationCompletion),
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
            Self::MergeObserved(_) => "merge_observed",
            Self::BeginPostMergeObservation => "begin_post_merge_observation",
            Self::Settle(_) => "settle",
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
    merge_sha: Option<PrValidationCommitSha>,
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
            merge_sha: None,
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

    pub fn terminal_reason(&self) -> Option<&PrValidationTerminalReason> {
        self.terminal_reason.as_ref()
    }

    pub fn transition(
        &self,
        event: PrValidationEvent,
    ) -> Result<Self, PrValidationTransitionRejection> {
        let mut next = self.clone();
        if let PrValidationEvent::Settle(completion) = &event {
            let expected = next
                .merge_sha
                .as_ref()
                .unwrap_or_else(|| next.target_shas.source_sha());
            if &completion.target_sha != expected {
                return Err(PrValidationTransitionRejection::TargetShaMismatch {
                    expected: expected.clone(),
                    observed: completion.target_sha.clone(),
                });
            }
        }
        if let PrValidationEvent::FindingObserved(finding) = &event {
            if finding.target_sha != *next.target_shas.source_sha() {
                return Err(PrValidationTransitionRejection::TargetShaMismatch {
                    expected: next.target_shas.source_sha().clone(),
                    observed: finding.target_sha.clone(),
                });
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
                next.phase = PrValidationPhase::PreMergeObservation;
            }
            PrValidationEvent::TargetShaChanged(target_shas)
                if next.phase != PrValidationPhase::Settled =>
            {
                next.target_shas = target_shas;
                next.phase = PrValidationPhase::PreMergeObservation;
                next.findings.clear();
                next.remediations.clear();
                next.active_remediation = None;
                next.merge_sha = None;
                next.completion = None;
                next.terminal_reason = None;
            }
            PrValidationEvent::MergeObserved(merge_sha)
                if next.phase == PrValidationPhase::PreMergeObservation =>
            {
                next.merge_sha = Some(merge_sha);
            }
            PrValidationEvent::BeginPostMergeObservation
                if next.phase == PrValidationPhase::PreMergeObservation
                    && next.merge_sha.is_some() =>
            {
                next.phase = PrValidationPhase::PostMergeObservation;
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

fn non_empty(value: impl Into<String>, label: &str) -> Result<String, String> {
    let value = value.into().trim().to_string();
    if value.is_empty() {
        Err(format!("{label} must not be empty"))
    } else {
        Ok(value)
    }
}
