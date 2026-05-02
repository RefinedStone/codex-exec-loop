use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/*
 * 학습 주석: ReadinessState는 parallel mode 전체의 gate verdict입니다.
 * 개별 capability가 여러 개여도 사용자는 "지금 병렬 실행을 시작해도 되는가"를 먼저 보므로,
 * domain은 Ready/Degraded/Blocked/Repairing 네 단계로 축약한 결론을 제공합니다.
 */
pub enum ParallelModeReadinessState {
    Ready,
    Degraded,
    Blocked,
    Repairing,
}

impl ParallelModeReadinessState {
    // 학습 주석: label은 TUI copy와 로그가 같은 readiness vocabulary를 쓰도록 domain 쪽에 둡니다.
    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::Blocked => "blocked",
            Self::Repairing => "repairing",
        }
    }

    // 학습 주석: degraded는 경고를 보여 주되 lane orchestration 자체는 허용하는 상태입니다.
    pub fn allows_parallel_mode(self) -> bool {
        matches!(self, Self::Ready | Self::Degraded)
    }

    // 학습 주석: capability 목록에서 가장 보수적인 전체 verdict를 계산합니다.
    pub fn derive_from_capabilities(capabilities: &[ParallelModeCapabilitySnapshot]) -> Self {
        // 학습 주석: blocked는 즉시 중단할 조건이고, repair/degraded는 끝까지 훑은 뒤 degraded로 축약합니다.
        let mut degraded = false;
        for capability in capabilities {
            match capability.state {
                ParallelModeCapabilityState::Blocked => return Self::Blocked,
                ParallelModeCapabilityState::Degraded | ParallelModeCapabilityState::Repairing => {
                    degraded = true;
                }
                ParallelModeCapabilityState::Ready => {}
            }
        }

        if degraded {
            Self::Degraded
        } else {
            Self::Ready
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
// 학습 주석: capability key는 serialized snapshot에도 노출되므로 snake_case를 public contract로 고정합니다.
#[serde(rename_all = "snake_case")]
// 학습 주석: 각 key는 parallel mode를 시작하기 전 반드시 확인해야 하는 독립 전제 조건입니다.
pub enum ParallelModeCapabilityKey {
    GitRepository,
    GitWorktree,
    AkraBranch,
    PushRemote,
    GhBinary,
    GhAuth,
    Planning,
    AuthorityStore,
}

impl ParallelModeCapabilityKey {
    // 학습 주석: label은 화면 폭이 좁은 TUI에서도 읽히도록 짧은 operational label로 유지합니다.
    pub fn label(self) -> &'static str {
        match self {
            Self::GitRepository => "git repo",
            Self::GitWorktree => "git worktree",
            Self::AkraBranch => "akra branch",
            Self::PushRemote => "push",
            Self::GhBinary => "gh binary",
            Self::GhAuth => "gh auth",
            Self::Planning => "planning",
            Self::AuthorityStore => "authority store",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
// 학습 주석: key와 마찬가지로 state도 persisted/transported snapshot의 값이라 snake_case를 유지합니다.
#[serde(rename_all = "snake_case")]
// 학습 주석: capability state는 개별 전제 조건의 진단 결과이고, readiness state의 재료가 됩니다.
pub enum ParallelModeCapabilityState {
    Ready,
    Degraded,
    Blocked,
    Repairing,
}

impl ParallelModeCapabilityState {
    // 학습 주석: summary 문자열과 TUI badge가 같은 state label을 쓰도록 변환을 한곳에 모읍니다.
    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::Blocked => "blocked",
            Self::Repairing => "repairing",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// 학습 주석: CapabilitySnapshot은 readiness screen 한 줄과 machine-readable diagnostics를 동시에 담는 domain value입니다.
pub struct ParallelModeCapabilitySnapshot {
    // 학습 주석: key는 capability row의 identity라 diffing, lookup, copy generation의 기준이 됩니다.
    pub key: ParallelModeCapabilityKey,
    // 학습 주석: state는 전체 readiness 계산과 badge 색상 결정을 모두 이끕니다.
    pub state: ParallelModeCapabilityState,
    // 학습 주석: detail은 현재 감지된 사실을 설명하고, 사용자가 고칠 일을 단정하지 않습니다.
    pub detail: String,
    // 학습 주석: next_action은 조치가 필요한 경우에만 채워져 summary copy의 우선순위를 높입니다.
    pub next_action: Option<String>,
}

impl ParallelModeCapabilitySnapshot {
    pub fn new(
        key: ParallelModeCapabilityKey,
        state: ParallelModeCapabilityState,
        detail: impl Into<String>,
        next_action: Option<String>,
    ) -> Self {
        Self {
            key,
            state,
            // 학습 주석: 호출자는 borrowed/static copy를 넘겨도 되고, snapshot은 최종 표시 문구를 소유합니다.
            detail: detail.into(),
            next_action,
        }
    }

    // 학습 주석: summary는 로그, status pane, 향후 API diagnostics가 같은 압축 형식을 쓰도록 한 줄로 유지합니다.
    pub fn summary(&self) -> String {
        match &self.next_action {
            Some(next_action) => format!(
                "{}: {} / cause: {} / next action: {}",
                self.key.label(),
                self.state.label(),
                self.detail,
                next_action
            ),
            None => format!(
                "{}: {} / detail: {}",
                self.key.label(),
                self.state.label(),
                self.detail
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
// 학습 주석: ReadinessSnapshot은 inbound presentation code에 넘기는 domain boundary object입니다.
pub struct ParallelModeReadinessSnapshot {
    // 학습 주석: workspace_path는 capability 결과가 어떤 repository/worktree 검사에서 나온 것인지 묶어 줍니다.
    pub workspace_path: String,
    // 학습 주석: readiness는 미리 계산된 verdict라 adapter가 우선순위 규칙을 다시 구현하지 않습니다.
    pub readiness: ParallelModeReadinessState,
    // 학습 주석: capabilities는 최상위 verdict 뒤에 있는 전체 진단 근거를 보존합니다.
    pub capabilities: Vec<ParallelModeCapabilitySnapshot>,
    // 학습 주석: top_alert는 compact view에서 가장 우선해 보여 줄 사용자 메시지 자리입니다.
    pub top_alert: Option<String>,
}

impl ParallelModeReadinessSnapshot {
    pub fn new(
        workspace_path: impl Into<String>,
        readiness: ParallelModeReadinessState,
        capabilities: Vec<ParallelModeCapabilitySnapshot>,
        top_alert: Option<String>,
    ) -> Self {
        Self {
            // 학습 주석: presentation layer는 cwd에서 다시 계산하지 않고 표시용 context로 그대로 사용합니다.
            workspace_path: workspace_path.into(),
            readiness,
            capabilities,
            top_alert,
        }
    }

    pub fn readiness_label(&self) -> &'static str {
        self.readiness.label()
    }

    pub fn allows_parallel_mode(&self) -> bool {
        self.readiness.allows_parallel_mode()
    }

    // 학습 주석: targeted lookup은 presentation code가 안정 계약이 아닌 row 순서에 기대지 않게 합니다.
    pub fn capability(
        &self,
        key: ParallelModeCapabilityKey,
    ) -> Option<&ParallelModeCapabilitySnapshot> {
        self.capabilities
            .iter()
            .find(|capability| capability.key == key)
    }
}
