// 학습 주석: Style은 planning indicator의 위험/정상 색상을 담고, Span은 footer/status line 안에서 특정 조각만 스타일링하는 단위입니다.
use ratatui::style::Style;
use ratatui::text::Span;

// 학습 주석: PlanningRuntimeSnapshot은 planning workspace의 현재 실행 가능 상태를 담은 application projection입니다. status panel은
// 이 snapshot을 직접 표시하지 않고, 짧은 primary/detail label로 압축합니다.
use crate::application::service::planning::{
    PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus,
};

// 학습 주석: AkraTheme는 footer indicator 색상 토큰을 제공하고, ConversationState/NativeTuiApp은 현재 conversation에 cached snapshot이
// 있는지 아니면 workspace에서 새 snapshot을 읽어야 하는지 판단하는 입력입니다.
use super::super::{AkraTheme, ConversationState, NativeTuiApp};

// 학습 주석: PlanModeIndicatorView는 planning runtime 상태를 footer에 붙일 수 있는 표시 모델로 줄인 값입니다. Copy가 가능한 것은
// label이 모두 static 문자열이고 style도 값 타입이라 footer 조립 중 소유권 부담 없이 전달할 수 있기 때문입니다.
#[derive(Clone, Copy)]
pub(in super::super) struct PlanModeIndicatorView {
    // 학습 주석: primary_label은 사용자가 먼저 보는 큰 상태입니다. workspace setup/invalid/ready처럼 planning surface의 단계만 말합니다.
    primary_label: &'static str,
    // 학습 주석: detail_label은 primary label 뒤의 보조 상태입니다. queue 준비, pause, idle 같은 runtime substate를 붙입니다.
    detail_label: Option<&'static str>,
    // 학습 주석: style은 primary label에만 적용됩니다. invalid만 danger이고 나머지는 accent로 두어 footer가 과도하게 경고색을 쓰지 않습니다.
    style: Style,
}

// 학습 주석: current_plan_mode_indicator는 현재 app 상태에서 가장 신뢰할 수 있는 planning snapshot을 고릅니다. Ready conversation은
// 이미 view model 안에 runtime snapshot을 들고 있고, Loading/Failed는 conversation snapshot이 없으므로 workspace에서 다시 읽습니다.
pub(super) fn current_plan_mode_indicator(app: &NativeTuiApp) -> PlanModeIndicatorView {
    match &app.conversation_state {
        // 학습 주석: Ready 상태에서는 conversation model이 턴 실행 후 갱신한 planning_runtime_snapshot을 그대로 사용합니다. 이 경로가
        // footer와 auto-follow 판단이 같은 snapshot을 보게 하는 정상 경로입니다.
        ConversationState::Ready(conversation) => {
            plan_mode_indicator_from_snapshot(&conversation.planning_runtime_snapshot)
        }
        // 학습 주석: Loading/Failed 상태에서는 conversation cache가 없거나 믿을 수 없으므로 현재 workspace directory를 기준으로
        // runtime snapshot을 직접 로드합니다. startup 직후에도 planning indicator가 빈 값으로 떨어지지 않게 하는 fallback입니다.
        ConversationState::Loading | ConversationState::Failed(_) => {
            let workspace_directory = app.current_workspace_directory();
            let snapshot = app.load_planning_runtime_snapshot(&workspace_directory);
            plan_mode_indicator_from_snapshot(&snapshot)
        }
    }
}

// 학습 주석: plan_runtime_substate_label은 primary workspace status보다 더 작고 실행에 가까운 상태를 한 단어로 만듭니다. footer에서
// "Plan ready / paused"처럼 붙어 사용자에게 다음 자동 진행 가능성을 빠르게 알려 줍니다.
pub(super) fn plan_runtime_substate_label(snapshot: &PlanningRuntimeSnapshot) -> &'static str {
    // 학습 주석: invalid는 다른 어떤 queue/pause 상태보다 우선합니다. workspace 파일이 깨져 있으면 실행 가능 여부 자체가 무효입니다.
    if snapshot.workspace_status() == PlanningRuntimeWorkspaceStatus::Invalid {
        "invalid"
    // 학습 주석: pause reason은 auto-follow가 정책적으로 멈춘 상태입니다. queue head가 있더라도 pause가 있으면 사용자가 알아야 합니다.
    } else if snapshot.auto_followup_pause_reason().is_some() {
        "paused"
    // 학습 주석: actionable queue head가 있으면 다음 planning task로 진행할 수 있으므로 ready로 표시합니다.
    } else if snapshot.has_actionable_queue_head() {
        "ready"
    // 학습 주석: 위 조건이 모두 아니면 workspace는 열려 있지만 지금 자동으로 처리할 queue item이 없는 idle 상태입니다.
    } else {
        "idle"
    }
}

// 학습 주석: plan_mode_prefixed_spans는 기존 footer 문구 뒤에 planning indicator span을 붙입니다. leading_text는 일반 raw span으로
// 남기고 planning primary label에만 style을 줘서 footer 전체 색상이 바뀌지 않게 합니다.
pub(super) fn plan_mode_prefixed_spans(
    // 학습 주석: leading_text는 이미 다른 status panel에서 만든 기본 footer 문장입니다. 이 함수는 그 문장을 보존하고 뒤에 plan 상태만 추가합니다.
    leading_text: String,
    // 학습 주석: indicator는 snapshot에서 추출된 표시 모델입니다. span 조립 함수는 snapshot 구조를 몰라도 됩니다.
    indicator: PlanModeIndicatorView,
) -> Vec<Span<'static>> {
    // 학습 주석: separator를 raw span으로 분리해 기존 footer copy와 plan indicator의 시각적 경계를 안정적으로 유지합니다.
    let mut spans = vec![Span::raw(leading_text), Span::raw("  |  ")];
    // 학습 주석: primary label만 styled span으로 넣습니다. invalid일 때 danger 색상이 "Plan invalid"에 집중됩니다.
    spans.push(Span::styled(indicator.primary_label, indicator.style));
    // 학습 주석: detail label은 raw text로 붙입니다. substate까지 경고색을 칠하면 footer가 과하게 강조되므로 primary label과 분리합니다.
    if let Some(detail_label) = indicator.detail_label {
        spans.push(Span::raw(format!(" / {detail_label}")));
    }
    spans
}

// 학습 주석: plan_mode_indicator_from_snapshot은 application runtime snapshot을 TUI 표시 모델로 변환하는 핵심 매핑입니다. 이 함수가
// workspace status, queue substate, color policy를 한곳에 모아 footer copy가 여러 파일에 흩어지지 않게 합니다.
fn plan_mode_indicator_from_snapshot(snapshot: &PlanningRuntimeSnapshot) -> PlanModeIndicatorView {
    PlanModeIndicatorView {
        // 학습 주석: primary label은 workspace lifecycle에 맞춥니다. task 유무는 detail substate에서 다루므로 ReadyNoTask와
        // ReadyWithTask는 같은 "Plan ready"로 압축합니다.
        primary_label: match snapshot.workspace_status() {
            PlanningRuntimeWorkspaceStatus::Uninitialized => "Plan setup",
            PlanningRuntimeWorkspaceStatus::Invalid => "Plan invalid",
            PlanningRuntimeWorkspaceStatus::ReadyNoTask
            | PlanningRuntimeWorkspaceStatus::ReadyWithTask => "Plan ready",
        },
        // 학습 주석: detail은 항상 붙입니다. primary가 setup/ready/invalid를 말하고, detail이 idle/ready/paused/invalid를 보완합니다.
        detail_label: Some(plan_runtime_substate_label(snapshot)),
        // 학습 주석: 색상 정책은 단순하게 유지합니다. invalid만 danger로 올리고 setup/ready/idle/paused는 accent로 두어 footer가
        // 에러 상태와 일반 상태를 명확히 구분합니다.
        style: if snapshot.workspace_status() == PlanningRuntimeWorkspaceStatus::Invalid {
            AkraTheme::danger()
        } else {
            AkraTheme::accent()
        },
    }
}
