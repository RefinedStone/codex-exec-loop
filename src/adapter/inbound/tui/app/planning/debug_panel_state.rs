// 학습 주석: 이 환경 변수는 planner worker의 prompt/response 같은 자세한 디버그 정보를 TUI에 노출할지
// 결정하는 startup-time switch입니다. 기본값은 Normal이라 운영 화면에는 compact 상태만 남습니다.
const PLANNER_VISIBILITY_ENV_VAR: &str = "CODEX_EXEC_LOOP_PLANNER_VISIBILITY";

// 학습 주석: PlannerWorkerStatus는 post-turn evaluation 중 planning refresh/repair worker가 지금 어떤 단계에
// 있는지 나타내는 UI 상태입니다. turn_submission_runtime이 값을 갱신하고 debug/status panel이 label로 표시합니다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::adapter::inbound::tui::app) enum PlannerWorkerStatus {
    // 학습 주석: Idle은 아직 worker 관측 정보가 없거나 새 draft/session으로 reset된 상태입니다.
    #[default]
    Idle,
    // 학습 주석: RefreshRunning은 턴 종료 후 queue/head/proposal 상태를 다시 계산하는 refresh worker가 실행 중임을 뜻합니다.
    RefreshRunning,
    // 학습 주석: RefreshSucceeded는 refresh worker가 runtime snapshot을 정상 갱신했고 auto-follow 판단이 계속 가능함을 뜻합니다.
    RefreshSucceeded,
    // 학습 주석: RefreshFailed는 worker error, repair request, repeated queue head, invalid snapshot처럼 refresh 후
    // 자동 진행을 막아야 하는 상태를 통합해 표시합니다.
    RefreshFailed,
    // 학습 주석: RepairRunning은 planning 파일 변경이나 invalid state 이후 repair worker가 실행 중인 상태입니다.
    RepairRunning,
    // 학습 주석: RepairSucceeded는 repair worker가 문제를 해결해 runtime snapshot을 다시 사용할 수 있게 만든 상태입니다.
    RepairSucceeded,
    // 학습 주석: RepairFailed는 repair worker 응답이 실패했거나 여전히 repair request/block reason이 남은 상태입니다.
    RepairFailed,
}

// 학습 주석: label은 enum variant를 debug/status panel에서 읽는 짧은 operator copy로 변환합니다. 이 mapping을
// 상태 enum 옆에 두어 worker recording code와 presentation code가 문자열을 중복 정의하지 않게 합니다.
impl PlannerWorkerStatus {
    // 학습 주석: 반환값은 static string입니다. status 값만으로 항상 같은 label이 나오므로 allocation 없이
    // footer/debug panel에서 반복적으로 사용할 수 있습니다.
    pub(in crate::adapter::inbound::tui::app) fn label(self) -> &'static str {
        // 학습 주석: refresh와 repair는 같은 worker subsystem이지만 사용자가 보는 조치가 다르므로 label에서 구분합니다.
        match self {
            Self::Idle => "idle",
            Self::RefreshRunning => "refresh running",
            Self::RefreshSucceeded => "refresh ok",
            Self::RefreshFailed => "refresh failed",
            Self::RepairRunning => "repair running",
            Self::RepairSucceeded => "repair ok",
            Self::RepairFailed => "repair failed",
        }
    }
}

// 학습 주석: PlannerVisibility는 planner worker panel의 상세 정보 노출 수준입니다. Normal은 compact status,
// Debug는 prompt/response/notice detail까지 보여 주어 planning worker 문제를 추적할 수 있게 합니다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::adapter::inbound::tui::app) enum PlannerVisibility {
    // 학습 주석: Normal은 기본값입니다. 자동 follow-up 사용자는 worker 내부 prompt를 보지 않고 high-level 상태만 봅니다.
    #[default]
    Normal,
    // 학습 주석: Debug는 worker prompt, raw response, host detail을 panel에 더 많이 노출하는 진단 모드입니다.
    Debug,
}

// 학습 주석: visibility helper들은 startup 환경 변수 값을 앱 내부 enum으로 바꾸고, presentation layer가
// debug detail 표시 여부를 bool로 물을 수 있게 합니다.
impl PlannerVisibility {
    // 학습 주석: from_environment는 app_runtime 초기화 시 한 번 호출되어 NativeTuiApp.planner_visibility를 채웁니다.
    // runtime 중에는 환경 변수를 다시 읽지 않으므로 한 세션의 debug visibility가 안정적으로 유지됩니다.
    pub(in crate::adapter::inbound::tui::app) fn from_environment() -> Self {
        // 학습 주석: var 실패는 env var가 없다는 정상 상황이므로 ok/as_deref로 Option<&str> 형태로 낮춥니다.
        Self::from_env_value(std::env::var(PLANNER_VISIBILITY_ENV_VAR).ok().as_deref())
    }

    // 학습 주석: from_env_value는 testable parser입니다. 실제 환경을 건드리지 않고 None, blank, true/debug/verbose
    // 같은 입력이 어떤 visibility가 되는지 app.rs tests에서 고정합니다.
    pub(in crate::adapter::inbound::tui::app) fn from_env_value(value: Option<&str>) -> Self {
        // 학습 주석: 사용자가 shell env에 넣은 값은 대소문자와 공백이 섞일 수 있으므로 먼저 normalize합니다.
        match value
            // 학습 주석: Option 안의 문자열만 trim해 None은 그대로 Normal fallback으로 내려가게 합니다.
            .map(str::trim)
            // 학습 주석: 빈 문자열은 env var가 없는 것과 같은 의미로 취급합니다.
            .filter(|value| !value.is_empty())
            // 학습 주석: TRUE, Debug처럼 대문자가 섞인 값을 허용하기 위해 ASCII lowercase로 비교합니다.
            .map(|value| value.to_ascii_lowercase())
            // 학습 주석: match arm은 &str literal과 비교하므로 temporary String을 Option<&str>로 빌립니다.
            .as_deref()
        {
            // 학습 주석: debug/verbose/detailed는 사람이 읽는 값이고, 1/true는 CI나 shell flag에서 쓰기 쉬운 값입니다.
            Some("debug") | Some("verbose") | Some("detailed") | Some("1") | Some("true") => {
                Self::Debug
            }
            // 학습 주석: 알 수 없는 값은 안전하게 Normal로 떨어뜨립니다. 잘못된 env var 하나로 noisy debug panel을
            // 켜지 않기 위한 보수적 기본값입니다.
            _ => Self::Normal,
        }
    }

    // 학습 주석: presentation code는 enum variant를 직접 match하지 않고 이 helper로 상세 표시 여부만 묻습니다.
    // 그러면 visibility variant가 늘어나도 copy/layout code의 조건을 한곳에서 조정할 수 있습니다.
    pub(in crate::adapter::inbound::tui::app) fn shows_debug_details(self) -> bool {
        matches!(self, Self::Debug)
    }
}

// 학습 주석: PlannerWorkerPanelState는 마지막 planning worker interaction을 TUI가 표시할 수 있게 보관하는
// 작은 UI state입니다. post_turn_execution이 갱신하고, planning presentation/debug panel과 queue overlay가 읽습니다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::adapter::inbound::tui::app) struct PlannerWorkerPanelState {
    // 학습 주석: status는 현재/마지막 refresh-repair worker 상태입니다. panel label과 success/failure color의 기준입니다.
    pub(in crate::adapter::inbound::tui::app) status: PlannerWorkerStatus,
    // 학습 주석: last_operation_label은 "refresh", "repair"처럼 실행한 worker operation의 사람이 읽는 이름입니다.
    pub(in crate::adapter::inbound::tui::app) last_operation_label: Option<String>,
    // 학습 주석: last_summary는 worker가 채택한 결과나 실패 detail의 compact summary입니다.
    pub(in crate::adapter::inbound::tui::app) last_summary: Option<String>,
    // 학습 주석: last_rejected_summary는 worker가 생성했지만 채택하지 않은 후보/판단을 따로 보여 주는 detail입니다.
    pub(in crate::adapter::inbound::tui::app) last_rejected_summary: Option<String>,
    // 학습 주석: last_queue_summary는 worker 이후 실제 planning queue snapshot에서 계산한 다음 task/idle summary입니다.
    pub(in crate::adapter::inbound::tui::app) last_queue_summary: Option<String>,
    // 학습 주석: last_notice_detail은 summary prefix를 제외한 worker notices입니다. repair/block reason 같은 부가 진단을 담습니다.
    pub(in crate::adapter::inbound::tui::app) last_notice_detail: Option<String>,
    // 학습 주석: last_prompt는 planner worker에 보낸 raw prompt입니다. Debug visibility에서만 주로 의미가 있습니다.
    pub(in crate::adapter::inbound::tui::app) last_prompt: Option<String>,
    // 학습 주석: last_response는 planner worker가 돌려준 raw response입니다. worker 판단을 사후 분석할 때 사용합니다.
    pub(in crate::adapter::inbound::tui::app) last_response: Option<String>,
    // 학습 주석: last_host_detail은 worker가 아니라 TUI host가 수행한 후처리입니다. 예를 들어 proposal promotion,
    // repeated queue-head pause 같은 host-side decision을 worker response와 분리해 기록합니다.
    pub(in crate::adapter::inbound::tui::app) last_host_detail: Option<String>,
}

// 학습 주석: panel state helper는 rendering code가 "패널을 표시할 이유가 있는가"만 묻도록 해 줍니다.
// 각 field를 presentation 곳곳에서 반복 검사하지 않고 여기서 content 존재 조건을 고정합니다.
impl PlannerWorkerPanelState {
    // 학습 주석: has_content는 status가 Idle이 아니거나 last_* detail 중 하나라도 남아 있으면 true입니다.
    // 새 draft/session에서 reset되면 false가 되어 debug panel/footer가 빈 planner section을 숨길 수 있습니다.
    pub(in crate::adapter::inbound::tui::app) fn has_content(&self) -> bool {
        !matches!(self.status, PlannerWorkerStatus::Idle)
            // 학습 주석: operation label만 있어도 running/last operation context를 보여 줄 가치가 있습니다.
            || self.last_operation_label.is_some()
            // 학습 주석: summary는 일반 사용자가 가장 먼저 읽는 worker 결과입니다.
            || self.last_summary.is_some()
            // 학습 주석: rejected summary는 후보가 있었지만 채택되지 않은 이유를 설명합니다.
            || self.last_rejected_summary.is_some()
            // 학습 주석: queue summary는 worker 이후 실제 다음 planning task를 보여 주는 행입니다.
            || self.last_queue_summary.is_some()
            // 학습 주석: notice detail은 block/repair 경고처럼 summary 밖의 진단 정보를 담습니다.
            || self.last_notice_detail.is_some()
            // 학습 주석: prompt/response는 Debug visibility에서만 드러나더라도 content 존재 판단에는 포함합니다.
            || self.last_prompt.is_some()
            || self.last_response.is_some()
            // 학습 주석: host detail은 worker 외부 후처리의 흔적이므로 worker response가 없어도 panel을 열 이유가 됩니다.
            || self.last_host_detail.is_some()
    }
}
