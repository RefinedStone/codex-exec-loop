#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationRuntimeEnvelope {
    pub thread_request: ConversationRuntimeConfigurationRequest,
    pub turn_request: Option<ConversationRuntimeConfigurationRequest>,
    pub applied: ConversationRuntimeConfigurationObservation,
    pub applied_scope: ConversationRuntimeAppliedScope,
    pub launch_environment: ConversationRuntimeLaunchEnvironment,
    pub thread_status: ConversationRuntimeObservedValue<ConversationRuntimeThreadStatus>,
    pub last_model_reroute: Option<ConversationRuntimeModelReroute>,
    pub projection_gap: Option<ConversationRuntimeObservationGap>,
    pub observation_sequence: u64,
}

impl ConversationRuntimeEnvelope {
    pub fn unobserved() -> Self {
        Self::prepared(
            ConversationRuntimeConfigurationRequest::default(),
            ConversationRuntimeConfigurationObservation::default(),
            ConversationRuntimeLaunchEnvironment::unknown(),
            ConversationRuntimeObservedValue::Missing,
        )
    }

    pub fn prepared(
        thread_request: ConversationRuntimeConfigurationRequest,
        applied: ConversationRuntimeConfigurationObservation,
        launch_environment: ConversationRuntimeLaunchEnvironment,
        thread_status: ConversationRuntimeObservedValue<ConversationRuntimeThreadStatus>,
    ) -> Self {
        Self {
            thread_request,
            turn_request: None,
            applied,
            applied_scope: ConversationRuntimeAppliedScope::PreparedThreadResponse,
            launch_environment,
            thread_status,
            last_model_reroute: None,
            projection_gap: None,
            observation_sequence: 0,
        }
    }

    pub fn record_turn_request(&mut self, request: ConversationRuntimeConfigurationRequest) {
        self.turn_request = Some(request);
        self.last_model_reroute = None;
    }

    pub fn apply_settings(&mut self, settings: ConversationRuntimeConfigurationObservation) {
        self.applied.model = settings.model;
        self.applied.model_provider = settings.model_provider;
        self.applied.reasoning_effort = settings.reasoning_effort;
        self.applied.service_tier = settings.service_tier;
        self.applied.cwd = settings.cwd;
        self.applied.approval_policy = settings.approval_policy;
        self.applied.approvals_reviewer = settings.approvals_reviewer;
        self.applied.sandbox = settings.sandbox;
        self.applied.permission_profile = settings.permission_profile;
        self.applied_scope = ConversationRuntimeAppliedScope::ThreadSettingsObservation;
        self.clear_projection_gap(true, true, false);
        // Thread settings do not contain source provenance. Preserve the source
        // observed on thread preparation instead of replacing it with Missing.
        self.observation_sequence = self.observation_sequence.saturating_add(1);
    }

    pub fn apply_model_reroute(&mut self, reroute: ConversationRuntimeModelReroute) {
        self.applied.model = ConversationRuntimeObservedValue::Observed(reroute.to_model.clone());
        self.last_model_reroute = Some(reroute);
        self.clear_projection_gap(false, true, false);
        self.observation_sequence = self.observation_sequence.saturating_add(1);
    }

    pub fn apply_thread_status(
        &mut self,
        status: ConversationRuntimeObservedValue<ConversationRuntimeThreadStatus>,
    ) {
        self.thread_status = status;
        self.clear_projection_gap(false, false, true);
        self.observation_sequence = self.observation_sequence.saturating_add(1);
    }

    pub fn apply_observation_gap(&mut self, gap: ConversationRuntimeObservationGap) {
        if gap.settings_may_be_stale {
            self.applied.model = ConversationRuntimeObservedValue::UnavailableAfterObservationGap;
            self.applied.model_provider =
                ConversationRuntimeObservedValue::UnavailableAfterObservationGap;
            self.applied.reasoning_effort =
                ConversationRuntimeObservedValue::UnavailableAfterObservationGap;
            self.applied.service_tier =
                ConversationRuntimeObservedValue::UnavailableAfterObservationGap;
            self.applied.cwd = ConversationRuntimeObservedValue::UnavailableAfterObservationGap;
            self.applied.approval_policy =
                ConversationRuntimeObservedValue::UnavailableAfterObservationGap;
            self.applied.approvals_reviewer =
                ConversationRuntimeObservedValue::UnavailableAfterObservationGap;
            self.applied.sandbox = ConversationRuntimeObservedValue::UnavailableAfterObservationGap;
            self.applied.permission_profile =
                ConversationRuntimeObservedValue::UnavailableAfterObservationGap;
        } else if gap.model_may_be_stale {
            self.applied.model = ConversationRuntimeObservedValue::UnavailableAfterObservationGap;
        }
        if gap.status_may_be_stale {
            self.thread_status = ConversationRuntimeObservedValue::UnavailableAfterObservationGap;
        }
        self.last_model_reroute = None;
        self.observation_sequence = self.observation_sequence.saturating_add(1);
        self.projection_gap = Some(self.projection_gap.unwrap_or_default().merged_with(gap));
    }

    pub fn apply_correlated_observation(
        &mut self,
        expected_thread_id: Option<&str>,
        expected_turn_id: Option<&str>,
        observation: &ConversationRuntimeEnvelopeObservation,
    ) -> Result<(), ConversationRuntimeEnvelopeObservationRejection> {
        let (observed_thread_id, observed_turn_id) = match observation {
            ConversationRuntimeEnvelopeObservation::SettingsUpdated { thread_id, .. }
            | ConversationRuntimeEnvelopeObservation::ThreadStatusChanged { thread_id, .. }
            | ConversationRuntimeEnvelopeObservation::ProjectionGap { thread_id, .. } => {
                (thread_id.as_str(), None)
            }
            ConversationRuntimeEnvelopeObservation::ModelRerouted {
                thread_id, turn_id, ..
            } => (thread_id.as_str(), Some(turn_id.as_str())),
        };
        if expected_thread_id != Some(observed_thread_id) {
            return Err(ConversationRuntimeEnvelopeObservationRejection::Thread {
                expected_thread_id: expected_thread_id.map(str::to_string),
            });
        }
        if let Some(observed_turn_id) = observed_turn_id
            && expected_turn_id != Some(observed_turn_id)
        {
            return Err(ConversationRuntimeEnvelopeObservationRejection::Turn {
                expected_turn_id: expected_turn_id.map(str::to_string),
            });
        }
        match observation {
            ConversationRuntimeEnvelopeObservation::SettingsUpdated { settings, .. } => {
                self.apply_settings((**settings).clone());
            }
            ConversationRuntimeEnvelopeObservation::ModelRerouted { reroute, .. } => {
                self.apply_model_reroute(reroute.clone());
            }
            ConversationRuntimeEnvelopeObservation::ThreadStatusChanged { status, .. } => {
                self.apply_thread_status(status.clone());
            }
            ConversationRuntimeEnvelopeObservation::ProjectionGap { gap, .. } => {
                self.apply_observation_gap(*gap);
            }
        }
        Ok(())
    }

    fn clear_projection_gap(&mut self, settings: bool, model: bool, status: bool) {
        let Some(mut gap) = self.projection_gap else {
            return;
        };
        if settings {
            gap.settings_may_be_stale = false;
        }
        if model || settings {
            gap.model_may_be_stale = false;
        }
        if status {
            gap.status_may_be_stale = false;
        }
        self.projection_gap = (!gap.is_empty()).then_some(gap);
    }

    pub fn applied_cwd(&self) -> Option<&str> {
        match &self.applied.cwd {
            ConversationRuntimeObservedValue::Observed(value)
            | ConversationRuntimeObservedValue::Defaulted(value) => Some(value),
            ConversationRuntimeObservedValue::Null
            | ConversationRuntimeObservedValue::Missing
            | ConversationRuntimeObservedValue::Malformed(_)
            | ConversationRuntimeObservedValue::UnavailableOnStableResponse
            | ConversationRuntimeObservedValue::UnavailableAfterObservationGap => None,
        }
    }
}

impl Default for ConversationRuntimeEnvelope {
    fn default() -> Self {
        Self::unobserved()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversationRuntimeConfigurationRequest {
    pub model: ConversationRuntimeRequestedValue<String>,
    pub model_provider: ConversationRuntimeRequestedValue<String>,
    pub reasoning_effort: ConversationRuntimeRequestedValue<String>,
    pub service_tier: ConversationRuntimeRequestedValue<String>,
    pub cwd: ConversationRuntimeRequestedValue<String>,
    pub approval_policy: ConversationRuntimeRequestedValue<ConversationRuntimeApprovalPolicy>,
    pub approvals_reviewer: ConversationRuntimeRequestedValue<ConversationRuntimeApprovalsReviewer>,
    pub sandbox: ConversationRuntimeRequestedValue<ConversationRuntimeSandboxPolicy>,
    pub permission_profile: ConversationRuntimeRequestedValue<String>,
    pub source: ConversationRuntimeRequestedValue<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ConversationRuntimeRequestedValue<T> {
    #[default]
    Omitted,
    ExplicitNull,
    Value(T),
}

impl<T> ConversationRuntimeRequestedValue<T> {
    pub fn as_value(&self) -> Option<&T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Omitted | Self::ExplicitNull => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversationRuntimeConfigurationObservation {
    pub model: ConversationRuntimeObservedValue<String>,
    pub model_provider: ConversationRuntimeObservedValue<String>,
    pub reasoning_effort: ConversationRuntimeObservedValue<String>,
    pub service_tier: ConversationRuntimeObservedValue<String>,
    pub cwd: ConversationRuntimeObservedValue<String>,
    pub approval_policy: ConversationRuntimeObservedValue<ConversationRuntimeApprovalPolicy>,
    pub approvals_reviewer: ConversationRuntimeObservedValue<ConversationRuntimeApprovalsReviewer>,
    pub sandbox: ConversationRuntimeObservedValue<ConversationRuntimeSandboxPolicy>,
    pub permission_profile: ConversationRuntimeObservedValue<ConversationRuntimePermissionProfile>,
    pub source: ConversationRuntimeObservedValue<ConversationRuntimeThreadSource>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ConversationRuntimeObservedValue<T> {
    Observed(T),
    Defaulted(T),
    Null,
    #[default]
    Missing,
    Malformed(ConversationRuntimeMalformedValue),
    UnavailableOnStableResponse,
    UnavailableAfterObservationGap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationRuntimeMalformedValue {
    ExpectedString,
    ExpectedObject,
    ExpectedArray,
    ExpectedBoolean,
    MissingDiscriminator,
    InvalidObject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationRuntimeApprovalPolicy {
    Untrusted,
    OnRequest,
    Never,
    Granular {
        sandbox_approval: bool,
        rules: bool,
        mcp_elicitations: bool,
        request_permissions: Option<bool>,
        skill_approval: Option<bool>,
    },
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationRuntimeApprovalsReviewer {
    User,
    AutoReview,
    GuardianSubagent,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationRuntimeSandboxPolicy {
    ReadOnly {
        network_access: Option<bool>,
    },
    WorkspaceWrite {
        network_access: Option<bool>,
        writable_roots: Option<Vec<String>>,
        writable_roots_truncated: bool,
        exclude_tmpdir_env_var: Option<bool>,
        exclude_slash_tmp: Option<bool>,
    },
    DangerFullAccess,
    ExternalSandbox {
        network_access: Option<String>,
    },
    Unknown {
        policy_type: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationRuntimePermissionProfile {
    pub id: String,
    pub extends: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationRuntimeLaunchEnvironment {
    pub process_environment: ConversationRuntimeProcessEnvironment,
    pub shell_environment: ConversationRuntimeShellEnvironment,
    pub api_key_auth: bool,
}

impl ConversationRuntimeLaunchEnvironment {
    pub const fn unknown() -> Self {
        Self {
            process_environment: ConversationRuntimeProcessEnvironment::Unknown,
            shell_environment: ConversationRuntimeShellEnvironment::Unknown,
            api_key_auth: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationRuntimeProcessEnvironment {
    Scrubbed,
    InheritedAll,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationRuntimeShellEnvironment {
    None,
    Core,
    All,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationRuntimeThreadStatus {
    NotLoaded,
    Idle,
    SystemError,
    Active {
        waiting_on_approval: bool,
        waiting_on_user_input: bool,
        unknown_flags: Vec<String>,
        unknown_flags_truncated: bool,
    },
    Unknown(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationRuntimeThreadSource {
    Cli,
    Vscode,
    Exec,
    AppServer,
    Custom,
    SubAgentReview,
    SubAgentCompact,
    SubAgentMemoryConsolidation,
    SubAgentThreadSpawn,
    SubAgentOther,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationRuntimeAppliedScope {
    PreparedThreadResponse,
    ThreadSettingsObservation,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConversationRuntimeObservationGap {
    pub settings_may_be_stale: bool,
    pub model_may_be_stale: bool,
    pub status_may_be_stale: bool,
}

impl ConversationRuntimeObservationGap {
    pub const fn settings() -> Self {
        Self {
            settings_may_be_stale: true,
            model_may_be_stale: true,
            status_may_be_stale: false,
        }
    }

    pub const fn model() -> Self {
        Self {
            settings_may_be_stale: false,
            model_may_be_stale: true,
            status_may_be_stale: false,
        }
    }

    pub const fn status() -> Self {
        Self {
            settings_may_be_stale: false,
            model_may_be_stale: false,
            status_may_be_stale: true,
        }
    }

    pub const fn merged_with(self, other: Self) -> Self {
        Self {
            settings_may_be_stale: self.settings_may_be_stale || other.settings_may_be_stale,
            model_may_be_stale: self.model_may_be_stale || other.model_may_be_stale,
            status_may_be_stale: self.status_may_be_stale || other.status_may_be_stale,
        }
    }

    pub const fn is_empty(self) -> bool {
        !self.settings_may_be_stale && !self.model_may_be_stale && !self.status_may_be_stale
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationRuntimeModelReroute {
    pub from_model: String,
    pub to_model: String,
    pub reason: ConversationRuntimeModelRerouteReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationRuntimeModelRerouteReason {
    HighRiskCyberActivity,
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationRuntimeEnvelopeObservation {
    SettingsUpdated {
        thread_id: String,
        settings: Box<ConversationRuntimeConfigurationObservation>,
    },
    ModelRerouted {
        thread_id: String,
        turn_id: String,
        reroute: ConversationRuntimeModelReroute,
    },
    ThreadStatusChanged {
        thread_id: String,
        status: ConversationRuntimeObservedValue<ConversationRuntimeThreadStatus>,
    },
    ProjectionGap {
        thread_id: String,
        gap: ConversationRuntimeObservationGap,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationRuntimeEnvelopeObservationRejection {
    Thread { expected_thread_id: Option<String> },
    Turn { expected_turn_id: Option<String> },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepared_envelope() -> ConversationRuntimeEnvelope {
        ConversationRuntimeEnvelope::prepared(
            ConversationRuntimeConfigurationRequest {
                model: ConversationRuntimeRequestedValue::Value("requested-model".to_string()),
                cwd: ConversationRuntimeRequestedValue::Value("/requested".to_string()),
                ..ConversationRuntimeConfigurationRequest::default()
            },
            ConversationRuntimeConfigurationObservation {
                model: ConversationRuntimeObservedValue::Observed("applied-model".to_string()),
                source: ConversationRuntimeObservedValue::Observed(
                    ConversationRuntimeThreadSource::AppServer,
                ),
                ..ConversationRuntimeConfigurationObservation::default()
            },
            ConversationRuntimeLaunchEnvironment {
                process_environment: ConversationRuntimeProcessEnvironment::Scrubbed,
                shell_environment: ConversationRuntimeShellEnvironment::Core,
                api_key_auth: false,
            },
            ConversationRuntimeObservedValue::Observed(ConversationRuntimeThreadStatus::Idle),
        )
    }

    #[test]
    fn requested_and_applied_values_remain_distinct() {
        let envelope = prepared_envelope();

        assert_eq!(
            envelope.thread_request.model.as_value().map(String::as_str),
            Some("requested-model")
        );
        assert_eq!(
            envelope.applied.model,
            ConversationRuntimeObservedValue::Observed("applied-model".to_string())
        );
        assert_eq!(
            envelope.applied.cwd,
            ConversationRuntimeObservedValue::Missing
        );
    }

    #[test]
    fn settings_replace_applied_fields_without_erasing_thread_source() {
        let mut envelope = prepared_envelope();

        envelope.apply_settings(ConversationRuntimeConfigurationObservation {
            model: ConversationRuntimeObservedValue::Observed("settings-model".to_string()),
            cwd: ConversationRuntimeObservedValue::Null,
            ..ConversationRuntimeConfigurationObservation::default()
        });

        assert_eq!(
            envelope.applied.model,
            ConversationRuntimeObservedValue::Observed("settings-model".to_string())
        );
        assert_eq!(envelope.applied.cwd, ConversationRuntimeObservedValue::Null);
        assert_eq!(
            envelope.applied.source,
            ConversationRuntimeObservedValue::Observed(ConversationRuntimeThreadSource::AppServer)
        );
    }

    #[test]
    fn reroute_changes_only_applied_model_and_retains_request() {
        let mut envelope = prepared_envelope();

        envelope.apply_model_reroute(ConversationRuntimeModelReroute {
            from_model: "applied-model".to_string(),
            to_model: "rerouted-model".to_string(),
            reason: ConversationRuntimeModelRerouteReason::HighRiskCyberActivity,
        });

        assert_eq!(
            envelope.thread_request.model.as_value().map(String::as_str),
            Some("requested-model")
        );
        assert_eq!(
            envelope.applied.model,
            ConversationRuntimeObservedValue::Observed("rerouted-model".to_string())
        );
        assert_eq!(envelope.observation_sequence, 1);
        assert_eq!(
            envelope
                .last_model_reroute
                .as_ref()
                .map(|reroute| reroute.from_model.as_str()),
            Some("applied-model")
        );
    }

    #[test]
    fn observation_gap_marks_only_affected_truth_and_later_snapshots_restore_it() {
        let mut envelope = prepared_envelope();
        envelope.apply_observation_gap(
            ConversationRuntimeObservationGap::settings()
                .merged_with(ConversationRuntimeObservationGap::status()),
        );

        assert_eq!(
            envelope.applied.model,
            ConversationRuntimeObservedValue::UnavailableAfterObservationGap
        );
        assert_eq!(
            envelope.applied.cwd,
            ConversationRuntimeObservedValue::UnavailableAfterObservationGap
        );
        assert_eq!(
            envelope.thread_status,
            ConversationRuntimeObservedValue::UnavailableAfterObservationGap
        );
        assert_eq!(
            envelope.applied.source,
            ConversationRuntimeObservedValue::Observed(ConversationRuntimeThreadSource::AppServer)
        );

        envelope.apply_settings(ConversationRuntimeConfigurationObservation {
            model: ConversationRuntimeObservedValue::Observed("restored-model".to_string()),
            cwd: ConversationRuntimeObservedValue::Observed("/restored".to_string()),
            ..ConversationRuntimeConfigurationObservation::default()
        });
        assert_eq!(
            envelope.projection_gap,
            Some(ConversationRuntimeObservationGap::status())
        );
        assert_eq!(
            envelope.applied_scope,
            ConversationRuntimeAppliedScope::ThreadSettingsObservation
        );

        envelope.apply_thread_status(ConversationRuntimeObservedValue::Observed(
            ConversationRuntimeThreadStatus::Idle,
        ));
        assert_eq!(envelope.projection_gap, None);
        assert_eq!(envelope.observation_sequence, 3);
    }
}
