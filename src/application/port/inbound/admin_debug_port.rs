use std::fmt;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_STEP_INTERVAL: Duration = Duration::from_millis(2_400);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminDebugScenario {
    PostMergeSuccess,
    CheckFailureRecovery,
    OptionalSkipped,
    RequiredMissing,
    RateLimitRecovery,
    ProviderOutageRetry,
    ClosedUnmergedAttested,
    RestartVerifying,
    PollClaimRace,
    DuplicateLateReview,
}

impl AdminDebugScenario {
    pub const ALL: [Self; 10] = [
        Self::PostMergeSuccess,
        Self::CheckFailureRecovery,
        Self::OptionalSkipped,
        Self::RequiredMissing,
        Self::RateLimitRecovery,
        Self::ProviderOutageRetry,
        Self::ClosedUnmergedAttested,
        Self::RestartVerifying,
        Self::PollClaimRace,
        Self::DuplicateLateReview,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Self::PostMergeSuccess => "post_merge_success",
            Self::CheckFailureRecovery => "check_failure_recovery",
            Self::OptionalSkipped => "optional_skipped",
            Self::RequiredMissing => "required_missing",
            Self::RateLimitRecovery => "rate_limit_recovery",
            Self::ProviderOutageRetry => "provider_outage_retry",
            Self::ClosedUnmergedAttested => "closed_unmerged_attested",
            Self::RestartVerifying => "restart_verifying",
            Self::PollClaimRace => "poll_claim_race",
            Self::DuplicateLateReview => "duplicate_late_review",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::PostMergeSuccess => "1 · Post-Merge 성공",
            Self::CheckFailureRecovery => "2 · 실패 → Queue → 재검증",
            Self::OptionalSkipped => "3 · Optional skipped",
            Self::RequiredMissing => "4 · Required missing/skipped",
            Self::RateLimitRecovery => "5 · Rate limit 복구",
            Self::ProviderOutageRetry => "6 · Provider outage + RetryNow",
            Self::ClosedUnmergedAttested => "7 · Closed + distributor attestation",
            Self::RestartVerifying => "8 · Verifying 중 재시작",
            Self::PollClaimRace => "9 · Two-process claim race",
            Self::DuplicateLateReview => "10 · Duplicate + late review",
        }
    }

    pub fn from_key(value: &str) -> Option<Self> {
        match value.trim() {
            "post_merge_success" => Some(Self::PostMergeSuccess),
            "check_failure_recovery" => Some(Self::CheckFailureRecovery),
            "optional_skipped" => Some(Self::OptionalSkipped),
            "required_missing" => Some(Self::RequiredMissing),
            "rate_limit_recovery" => Some(Self::RateLimitRecovery),
            "provider_outage_retry" => Some(Self::ProviderOutageRetry),
            "closed_unmerged_attested" => Some(Self::ClosedUnmergedAttested),
            "restart_verifying" => Some(Self::RestartVerifying),
            "poll_claim_race" => Some(Self::PollClaimRace),
            "duplicate_late_review" => Some(Self::DuplicateLateReview),
            _ => None,
        }
    }

    pub(crate) fn stages(self) -> &'static [AdminDebugStage] {
        match self {
            Self::PostMergeSuccess => &[
                AdminDebugStage::Ready,
                AdminDebugStage::Delivering,
                AdminDebugStage::Reviewing,
                AdminDebugStage::Complete,
            ],
            Self::CheckFailureRecovery => &[
                AdminDebugStage::Ready,
                AdminDebugStage::Reviewing,
                AdminDebugStage::Blocked,
                AdminDebugStage::Intake,
                AdminDebugStage::Dispatching,
                AdminDebugStage::Working,
                AdminDebugStage::Reviewing,
                AdminDebugStage::Complete,
            ],
            Self::OptionalSkipped => &[
                AdminDebugStage::Ready,
                AdminDebugStage::Reviewing,
                AdminDebugStage::Complete,
            ],
            Self::RequiredMissing => &[
                AdminDebugStage::Ready,
                AdminDebugStage::Reviewing,
                AdminDebugStage::Blocked,
            ],
            Self::RateLimitRecovery | Self::ProviderOutageRetry => &[
                AdminDebugStage::Ready,
                AdminDebugStage::Blocked,
                AdminDebugStage::Recovering,
                AdminDebugStage::Reviewing,
                AdminDebugStage::Complete,
            ],
            Self::ClosedUnmergedAttested => &[
                AdminDebugStage::Ready,
                AdminDebugStage::Blocked,
                AdminDebugStage::Delivering,
                AdminDebugStage::Reviewing,
                AdminDebugStage::Complete,
            ],
            Self::RestartVerifying => &[
                AdminDebugStage::Ready,
                AdminDebugStage::Reviewing,
                AdminDebugStage::Recovering,
                AdminDebugStage::Reviewing,
                AdminDebugStage::Complete,
            ],
            Self::PollClaimRace => &[
                AdminDebugStage::Ready,
                AdminDebugStage::Dispatching,
                AdminDebugStage::Reviewing,
                AdminDebugStage::Complete,
            ],
            Self::DuplicateLateReview => &[
                AdminDebugStage::Ready,
                AdminDebugStage::Reviewing,
                AdminDebugStage::QueuePressure,
                AdminDebugStage::Reviewing,
                AdminDebugStage::Complete,
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminDebugStage {
    Ready,
    Intake,
    QueuePressure,
    Dispatching,
    Working,
    Reviewing,
    Blocked,
    Recovering,
    Delivering,
    Cleanup,
    Complete,
}

impl AdminDebugStage {
    pub fn key(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Intake => "intake",
            Self::QueuePressure => "queue_pressure",
            Self::Dispatching => "dispatching",
            Self::Working => "working",
            Self::Reviewing => "reviewing",
            Self::Blocked => "blocked",
            Self::Recovering => "recovering",
            Self::Delivering => "delivering",
            Self::Cleanup => "cleanup",
            Self::Complete => "complete",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "준비",
            Self::Intake => "작업 접수",
            Self::QueuePressure => "대기열 압력",
            Self::Dispatching => "요원 배정",
            Self::Working => "병렬 작업",
            Self::Reviewing => "리뷰",
            Self::Blocked => "차단",
            Self::Recovering => "복구",
            Self::Delivering => "배포",
            Self::Cleanup => "정리",
            Self::Complete => "완료",
        }
    }

    pub fn summary(self) -> &'static str {
        match self {
            Self::Ready => "세 개의 슬롯과 워커 프로필이 시나리오 입력을 기다립니다.",
            Self::Intake => "검증된 작업 세 건이 애플리케이션 큐에 접수되었습니다.",
            Self::QueuePressure => "슬롯 용량보다 많은 작업이 들어와 분배 대기열이 증가했습니다.",
            Self::Dispatching => "애플리케이션 하네스가 작업과 워커 임대를 연결하고 있습니다.",
            Self::Working => "세 워커가 각 작업 공간에서 병렬 구현을 진행합니다.",
            Self::Reviewing => "완료 보고가 검토 게이트와 검증 단계로 이동했습니다.",
            Self::Blocked => "검증 실패가 한 레인을 차단하고 복구 판단을 요청했습니다.",
            Self::Recovering => "차단된 레인이 새 세대의 임대로 복구되고 있습니다.",
            Self::Delivering => "검토된 변경이 push, PR, rebase 통합 순서로 전달됩니다.",
            Self::Cleanup => "병합된 레인의 worktree와 임대를 정리합니다.",
            Self::Complete => "모든 레인이 통합되어 다음 작업을 기다립니다.",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminDebugScenarioOption {
    pub key: &'static str,
    pub label: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminDebugStageRecord {
    pub index: usize,
    pub stage: AdminDebugStage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminDebugHarnessProjection {
    pub enabled: bool,
    pub playing: bool,
    pub scenario: AdminDebugScenario,
    pub stage: AdminDebugStage,
    pub stage_index: usize,
    pub stage_count: usize,
    pub revision: u64,
    pub run_id: u64,
    pub step_interval_ms: u64,
    pub scenarios: Vec<AdminDebugScenarioOption>,
    pub history: Vec<AdminDebugStageRecord>,
}

impl AdminDebugHarnessProjection {
    pub fn progress_percent(&self) -> u8 {
        if self.stage_count <= 1 {
            return 100;
        }
        ((self.stage_index * 100) / (self.stage_count - 1)) as u8
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminDebugHarnessCommand {
    Play,
    Pause,
    Step,
    Reset,
    SelectScenario(AdminDebugScenario),
}

#[derive(Debug, Clone, Copy)]
pub struct AdminDebugHarnessConfig {
    pub enabled: bool,
    pub step_interval: Duration,
}

impl AdminDebugHarnessConfig {
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            step_interval: DEFAULT_STEP_INTERVAL,
        }
    }

    pub fn disabled() -> Self {
        Self {
            enabled: false,
            step_interval: DEFAULT_STEP_INTERVAL,
        }
    }

    #[cfg(test)]
    pub(crate) fn enabled_with_interval(step_interval: Duration) -> Self {
        Self {
            enabled: true,
            step_interval,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdminDebugHarnessError {
    Disabled,
}

impl fmt::Display for AdminDebugHarnessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => formatter.write_str("admin debug harness is disabled"),
        }
    }
}

impl std::error::Error for AdminDebugHarnessError {}

pub trait AdminDebugPort: Send + Sync {
    fn projection(&self) -> AdminDebugHarnessProjection;

    fn execute(
        &self,
        command: AdminDebugHarnessCommand,
    ) -> Result<AdminDebugHarnessProjection, AdminDebugHarnessError>;
}

impl<T> AdminDebugPort for Arc<T>
where
    T: AdminDebugPort + ?Sized,
{
    fn projection(&self) -> AdminDebugHarnessProjection {
        self.as_ref().projection()
    }

    fn execute(
        &self,
        command: AdminDebugHarnessCommand,
    ) -> Result<AdminDebugHarnessProjection, AdminDebugHarnessError> {
        self.as_ref().execute(command)
    }
}
