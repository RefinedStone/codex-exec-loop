use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/*
 * `ParallelModeReadinessState`는 여러 capability probe를 operator-facing gate verdict로 접은 값이다.
 * 개별 capability가 여러 개여도 사용자는 "지금 병렬 실행을 시작해도 되는가"를 먼저 보므로, domain은
 * Ready/Degraded/Blocked/Repairing 네 단계로 축약한 결론을 제공한다. supervisor, pool reconciliation,
 * TUI command guard는 이 verdict를 공유해 각 화면이 capability 우선순위를 다시 구현하지 않게 한다.
 */
pub enum ParallelModeReadinessState {
    Ready,
    Degraded,
    Blocked,
    Repairing,
}

impl ParallelModeReadinessState {
    // TUI badge, command status, logs가 같은 readiness vocabulary를 쓰도록 domain에 label을 고정한다.
    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::Blocked => "blocked",
            Self::Repairing => "repairing",
        }
    }

    // degraded는 경고를 보여 주되 lane orchestration 자체는 허용하는 상태다.
    pub fn allows_parallel_mode(self) -> bool {
        matches!(self, Self::Ready | Self::Degraded)
    }

    /*
     * capability 목록에서 가장 보수적인 전체 verdict를 계산한다. Blocked는 즉시 중단할 조건이고,
     * Repairing은 아직 복구 중인 degraded state로 축약한다. 이 규칙을 domain에 두면 readiness service와
     * tests가 capability row 순서에 의존하지 않는다.
     */
    pub fn derive_from_capabilities(capabilities: &[ParallelModeCapabilitySnapshot]) -> Self {
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
// capability key는 serialized snapshot에도 노출되므로 snake_case를 public contract로 고정한다.
#[serde(rename_all = "snake_case")]
pub enum ParallelModeCapabilityKey {
    // 현재 workspace가 git repository 안에 있는지 확인한다.
    GitRepository,
    // linked worktree를 만들거나 현재 worktree 상태를 읽을 수 있는지 확인한다.
    GitWorktree,
    // Akra가 요구하는 branch naming/lane context를 만족하는지 확인한다.
    AkraBranch,
    // worker branch push에 필요한 remote가 설정되어 있는지 확인한다.
    PushRemote,
    // GitHub automation에 필요한 gh binary가 있는지 확인한다.
    GhBinary,
    // PR 생성/조회/merge readiness에 필요한 gh auth 상태를 확인한다.
    GhAuth,
    // planning authority와 queue projection이 parallel dispatch에 사용할 수 있는지 확인한다.
    Planning,
    // repo-scoped planning authority shadow store가 inspect/recover 가능한지 확인한다.
    AuthorityStore,
}

impl ParallelModeCapabilityKey {
    // 화면 폭이 좁은 TUI에서도 읽히도록 짧은 operational label을 유지한다.
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
// state도 persisted/transported snapshot의 값이라 snake_case를 유지한다.
#[serde(rename_all = "snake_case")]
pub enum ParallelModeCapabilityState {
    // capability가 병렬 모드 전제 조건을 충족한다.
    Ready,
    // 병렬 모드는 가능하지만 operator에게 degraded fact를 알려야 한다.
    Degraded,
    // 병렬 모드 실행을 막는 hard blocker다.
    Blocked,
    // repair/recovery가 진행 중이거나 필요해 degraded verdict로 접히는 상태다.
    Repairing,
}

impl ParallelModeCapabilityState {
    // summary 문자열과 TUI badge가 같은 state label을 쓰도록 변환을 한곳에 모은다.
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
/*
 * Capability snapshot은 readiness screen 한 줄과 machine-readable diagnostics를 동시에 담는 domain
 * value다. application service가 probe 사실을 이 타입으로 만들고, supervisor/TUI는 `key`, `state`,
 * `detail`, `next_action`만 읽어 presentation을 구성한다.
 */
pub struct ParallelModeCapabilitySnapshot {
    // capability row의 identity라 diffing, lookup, copy generation의 기준이 된다.
    pub key: ParallelModeCapabilityKey,
    // 전체 readiness 계산과 badge 색상 결정을 모두 이끄는 상태다.
    pub state: ParallelModeCapabilityState,
    // 현재 감지된 사실을 설명한다. 사용자가 고칠 일을 단정하는 문구는 next_action으로 분리한다.
    pub detail: String,
    // 조치가 필요한 경우에만 채워져 summary copy의 우선순위를 높인다.
    pub next_action: Option<String>,
}

impl ParallelModeCapabilitySnapshot {
    // borrowed/static copy를 받아도 snapshot은 최종 표시 문구를 소유한다.
    pub fn new(
        key: ParallelModeCapabilityKey,
        state: ParallelModeCapabilityState,
        detail: impl Into<String>,
        next_action: Option<String>,
    ) -> Self {
        Self {
            key,
            state,
            detail: detail.into(),
            next_action,
        }
    }

    // logs, status pane, future API diagnostics가 같은 압축 형식을 쓰도록 한 줄 summary를 제공한다.
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
/*
 * Readiness snapshot은 inbound presentation code에 넘기는 domain boundary object다. service는 probe
 * 결과를 모두 모아 미리 verdict를 계산하고, TUI는 이 snapshot을 저장했다가 `:parallel on`, Ctrl+R,
 * supervisor popup rendering에서 같은 gate 판단을 재사용한다.
 */
pub struct ParallelModeReadinessSnapshot {
    // capability 결과가 어떤 repository/worktree 검사에서 나온 것인지 묶어 준다.
    pub workspace_path: String,
    // 미리 계산된 verdict라 adapter가 우선순위 규칙을 다시 구현하지 않는다.
    pub readiness: ParallelModeReadinessState,
    // 최상위 verdict 뒤에 있는 전체 진단 근거다.
    pub capabilities: Vec<ParallelModeCapabilitySnapshot>,
    // compact view에서 가장 우선해 보여 줄 사용자 메시지다.
    pub top_alert: Option<String>,
}

impl ParallelModeReadinessSnapshot {
    // service가 workspace context, verdict, capability evidence, top alert를 한 번에 고정한다.
    pub fn new(
        workspace_path: impl Into<String>,
        readiness: ParallelModeReadinessState,
        capabilities: Vec<ParallelModeCapabilitySnapshot>,
        top_alert: Option<String>,
    ) -> Self {
        Self {
            workspace_path: workspace_path.into(),
            readiness,
            capabilities,
            top_alert,
        }
    }

    // presentation은 enum을 직접 match하지 않고 readiness label helper를 쓴다.
    pub fn readiness_label(&self) -> &'static str {
        self.readiness.label()
    }

    // command guard와 supervisor builder가 같은 enable rule을 공유한다.
    pub fn allows_parallel_mode(&self) -> bool {
        self.readiness.allows_parallel_mode()
    }

    // targeted lookup은 presentation code가 안정 계약이 아닌 row 순서에 기대지 않게 한다.
    pub fn capability(
        &self,
        key: ParallelModeCapabilityKey,
    ) -> Option<&ParallelModeCapabilitySnapshot> {
        self.capabilities
            .iter()
            .find(|capability| capability.key == key)
    }
}
