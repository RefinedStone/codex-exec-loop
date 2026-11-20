//! 자동화 정책의 효과값이다 (#2540, plan 28 §7.3/9).
//! 설정을 해석한 값일 뿐 실행 허가, 장부 모드 변경, 이슈 판정을 수행하지 않는다.
use serde::{Deserialize, Serialize};

use crate::domain::issue_work::IssueProvider;

use super::IssueAutomationAutonomy;

mod validation;

pub use validation::IssueAutomationPolicyValueError;
pub(crate) use validation::{
    validate_gate_action, validate_glob, validate_repository, validate_text_value,
};

pub const ISSUE_AUTOMATION_POLICY_SCHEMA_VERSION: u32 = 1;
pub const MAX_ISSUE_AUTOMATION_TIMEOUT_SECONDS: u64 = 30 * 24 * 60 * 60;

pub const BUILTIN_ISSUE_PROTECTION_PATHS: &[&str] = &[
    ".github/workflows/**",
    "scripts/release/**",
    ".akra/issue-automation.toml",
    ".akra/issue-policy.toml",
    "**/AGENTS.md",
    "**/.env",
    "**/.env.*",
    "**/credentials*",
    "**/*.pem",
    "**/*.key",
];

/// 원문, 출처, 요청 수준은 포함하지 않는다. mode는 사용자 상한으로 자른 효과값이다.
/// 해시에는 이 값의 정규화된 JSON만 사용한다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueAutomationPolicy {
    pub schema_version: u32,
    pub binding: IssueAutomationPolicyBinding,
    pub authority: IssueAutomationAuthority,
    pub labels: IssueAutomationLabels,
    pub select: IssueAutomationSelection,
    pub classify: IssueAutomationClassification,
    pub gates: IssueAutomationPolicyGates,
    pub timeouts: IssueAutomationTimeouts,
    pub budget: IssueAutomationBudget,
    pub feedback: IssueAutomationFeedback,
    pub close: IssueAutomationClose,
    pub protection: IssueAutomationProtection,
    pub human_required: IssueAutomationHumanRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueAutomationPolicyBinding {
    pub github: IssueAutomationGitHubBinding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueAutomationGitHubBinding {
    /// 사람이 고른 owner/name이다. 안정 repository ID나 저장된 binding을 대신하지 않는다.
    pub repository: Option<String>,
    pub mode: IssueAutomationAutonomy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueAutomationAuthority {
    pub autonomy_cap: IssueAutomationAutonomy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueAutomationLabels {
    pub auto: String,
    pub gated: String,
    pub manual: String,
    pub hold: String,
}

impl Default for IssueAutomationLabels {
    fn default() -> Self {
        Self {
            auto: "akra:auto".to_owned(),
            gated: "akra:gated".to_owned(),
            manual: "akra:manual".to_owned(),
            hold: "akra:hold".to_owned(),
        }
    }
}

impl IssueAutomationLabels {
    pub fn selection_labels(&self) -> Vec<String> {
        vec![self.auto.clone(), self.gated.clone(), self.manual.clone()]
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueAutomationSelectionMode {
    #[default]
    LabelsAny,
    AllOpen,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueAutomationTrustedActor {
    pub provider: IssueProvider,
    pub server_id: String,
    pub actor_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueAutomationSelection {
    pub mode: IssueAutomationSelectionMode,
    /// LabelsAny에서 빈 목록은 선택 대상 없음이다. AllOpen은 별도의 명시적 선택이다.
    pub labels_any: Vec<String>,
    pub trusted_actors: Vec<IssueAutomationTrustedActor>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueAutomationRiskClass {
    Low,
    #[default]
    Normal,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueAutomationRiskRule {
    pub class: IssueAutomationRiskClass,
    #[serde(default)]
    pub labels_any: Vec<String>,
    #[serde(default)]
    pub paths_any: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueAutomationClassification {
    /// 첫 일치 규칙을 위한 순서다. 정규화 때 순서를 바꾸거나 규칙을 합치지 않는다.
    pub rule: Vec<IssueAutomationRiskRule>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueAutomationGateAction {
    #[default]
    Human,
    TimeoutDefault,
    TimeoutArbiter,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueAutomationGate {
    Start,
    Spec,
    Plan,
    Delivery,
    Merge,
    Close,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct IssueAutomationGateSet {
    pub start: IssueAutomationGateAction,
    pub spec: IssueAutomationGateAction,
    pub plan: IssueAutomationGateAction,
    pub delivery: IssueAutomationGateAction,
    pub merge: IssueAutomationGateAction,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct IssueAutomationRiskGates {
    pub low: IssueAutomationGateSet,
    pub normal: IssueAutomationGateSet,
    pub high: IssueAutomationGateSet,
}

/// Off/Observe에는 실행 관문 설정을 두지 않는다. close는 별도 close.when 값이다.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct IssueAutomationPolicyGates {
    pub propose: IssueAutomationRiskGates,
    pub gated: IssueAutomationRiskGates,
    pub auto: IssueAutomationRiskGates,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueAutomationTimeouts {
    pub start_seconds: u64,
    /// 0은 사전 알림 없음이다. 실제 타이머를 만들지는 않는다.
    pub remind_before_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueAutomationBudget {
    /// 0은 실행을 허용하지 않는 상한이다.
    pub max_running: u32,
    pub max_started_per_day: u32,
    pub max_attempts: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct IssueAutomationFeedback {
    pub comment: bool,
    pub labels: bool,
    pub transition: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueAutomationCloseWhen {
    #[default]
    Never,
    Integration,
    ProviderLink,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct IssueAutomationClose {
    pub when: IssueAutomationCloseWhen,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueAutomationProtection {
    pub paths: Vec<String>,
}

/// 이 조건은 어느 파일에서도 제거할 수 없다. 판정은 다음 조각에서 이 값을 소비한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueAutomationHumanRule {
    HighRisk,
    HeldOrManual,
    UntrustedActor,
    ProtectedPath,
    LargeDiff,
    MissingAcceptance,
    AttemptsExhausted,
    DisclosureConfirmation,
    BudgetExhausted,
    ArbiterUncertain,
}

impl IssueAutomationHumanRule {
    pub const ALL: [Self; 10] = [
        Self::HighRisk,
        Self::HeldOrManual,
        Self::UntrustedActor,
        Self::ProtectedPath,
        Self::LargeDiff,
        Self::MissingAcceptance,
        Self::AttemptsExhausted,
        Self::DisclosureConfirmation,
        Self::BudgetExhausted,
        Self::ArbiterUncertain,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueAutomationAdditionalHumanRule {
    #[serde(default)]
    pub labels_any: Vec<String>,
    #[serde(default)]
    pub paths_any: Vec<String>,
    pub gates: Vec<IssueAutomationGate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueAutomationHumanRequired {
    pub rules: Vec<IssueAutomationHumanRule>,
    /// 라벨 또는 경로가 맞으면 지정 관문은 사람 필수다. 이름으로 기존 조건을 덮어쓰지 않는다.
    pub additional: Vec<IssueAutomationAdditionalHumanRule>,
}

impl IssueAutomationPolicy {
    pub fn builtin() -> Self {
        let mut policy = Self {
            schema_version: ISSUE_AUTOMATION_POLICY_SCHEMA_VERSION,
            binding: IssueAutomationPolicyBinding {
                github: IssueAutomationGitHubBinding {
                    repository: None,
                    mode: IssueAutomationAutonomy::Observe,
                },
            },
            authority: IssueAutomationAuthority {
                autonomy_cap: IssueAutomationAutonomy::Propose,
            },
            labels: IssueAutomationLabels::default(),
            select: IssueAutomationSelection {
                mode: IssueAutomationSelectionMode::LabelsAny,
                labels_any: IssueAutomationLabels::default().selection_labels(),
                trusted_actors: Vec::new(),
            },
            classify: IssueAutomationClassification { rule: Vec::new() },
            gates: IssueAutomationPolicyGates::default(),
            timeouts: IssueAutomationTimeouts {
                start_seconds: 24 * 60 * 60,
                remind_before_seconds: 2 * 60 * 60,
            },
            budget: IssueAutomationBudget {
                max_running: 2,
                max_started_per_day: 5,
                max_attempts: 3,
            },
            feedback: IssueAutomationFeedback::default(),
            close: IssueAutomationClose::default(),
            protection: IssueAutomationProtection {
                paths: BUILTIN_ISSUE_PROTECTION_PATHS
                    .iter()
                    .map(|path| (*path).to_string())
                    .collect(),
            },
            human_required: IssueAutomationHumanRequired {
                rules: IssueAutomationHumanRule::ALL.to_vec(),
                additional: Vec::new(),
            },
        };
        policy.normalize();
        policy
    }

    pub(crate) fn normalize(&mut self) {
        self.binding.github.mode = self.binding.github.mode.min(self.authority.autonomy_cap);
        normalize_set(&mut self.select.labels_any);
        self.select.trusted_actors.sort_by(|left, right| {
            (
                provider_name(left.provider),
                &left.server_id,
                &left.actor_id,
            )
                .cmp(&(
                    provider_name(right.provider),
                    &right.server_id,
                    &right.actor_id,
                ))
        });
        self.select.trusted_actors.dedup();
        for rule in &mut self.classify.rule {
            normalize_set(&mut rule.labels_any);
            normalize_set(&mut rule.paths_any);
        }
        normalize_set(&mut self.protection.paths);
        normalize_set(&mut self.human_required.rules);
        for rule in &mut self.human_required.additional {
            normalize_set(&mut rule.labels_any);
            normalize_set(&mut rule.paths_any);
            normalize_set(&mut rule.gates);
        }
        normalize_set(&mut self.human_required.additional);
    }

    pub fn validate(&self) -> Result<(), IssueAutomationPolicyValueError> {
        validation::validate_policy(self)
    }
}

fn normalize_set<T: Ord>(values: &mut Vec<T>) {
    values.sort();
    values.dedup();
}

fn provider_name(provider: IssueProvider) -> &'static str {
    match provider {
        IssueProvider::GitHub => "github",
        IssueProvider::Jira => "jira",
    }
}

#[cfg(test)]
mod tests;
