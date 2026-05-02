// 학습 주석: status panel facade는 하위 copy/layout 모듈이 만든 ratatui text를 shell_presentation 상위
// namespace로 다시 내보냅니다. Line은 footer, inline tail, live agent panel wrapper의 공통 반환 타입입니다.
use ratatui::text::Line;

// 학습 주석: planning runtime snapshot은 plan indicator label을 만들 때 필요한 application projection입니다.
// 이 facade가 helper를 다시 노출해 popup existing workspace copy도 status panel의 label 규칙을 재사용합니다.
use crate::application::service::planning::PlanningRuntimeSnapshot;

// 학습 주석: ConversationViewModel은 current live agent/status lines를 뽑는 입력입니다. facade는 concrete
// tail_shared module path를 숨기고 shell_presentation이 conversation-level helper만 보게 합니다.
use super::ConversationViewModel;
// 학습 주석: NativeTuiApp은 inline tail, parallel mode summary, plan indicator처럼 app-wide state가 필요한
// status panel projection의 root input입니다.
use super::NativeTuiApp;
#[cfg(test)]
// 학습 주석: footer contract tests는 shell core context를 직접 넣어 footer copy를 검증합니다. production
// facade는 app에서 context를 구성하므로 이 import는 test-only로 제한됩니다.
use super::ShellCorePresentationContext;

#[cfg(test)]
// 학습 주석: footer_copy는 test에서만 직접 wrapper를 노출합니다. production render path는 inline tail
// builder가 footer copy를 내부적으로 조립하므로 상위 module surface를 넓히지 않습니다.
#[path = "status_panels/footer_copy.rs"]
mod footer_copy;
// 학습 주석: live_status_layout은 inline main buffer 하단 tail의 실제 layout DTO를 만듭니다. 이 facade는
// renderer가 하위 파일 구조를 몰라도 `build_inline_tail_view`만 호출하게 하는 경계입니다.
#[path = "status_panels/live_status_layout.rs"]
mod live_status_layout;
// 학습 주석: plan_indicator는 planning runtime snapshot을 compact footer indicator로 낮춥니다. popup과
// footer가 같은 state label을 쓰도록 facade에서 helper를 다시 노출합니다.
#[path = "status_panels/plan_indicator.rs"]
mod plan_indicator;
// 학습 주석: tail_copy는 inline tail의 문장과 status ribbon copy를 만듭니다. live_status_layout이 이 copy를
// 배치하고, facade는 최종 InlineTailView만 외부에 공개합니다.
#[path = "status_panels/tail_copy.rs"]
mod tail_copy;
// 학습 주석: tail_shared는 footer, overlays, tests가 함께 쓰는 작은 status helper를 담습니다. facade는
// 필요한 함수만 골라 다시 내보내 module coupling을 얇게 유지합니다.
#[path = "status_panels/tail_shared.rs"]
mod tail_shared;

// 학습 주석: InlineTailView는 shell_rendering 쪽에서 하단 live tail을 그릴 때 필요한 최종 DTO입니다.
// visibility를 `super::super`로 제한해 shell presentation/rendering 경계 밖으로 새지 않게 합니다.
pub(in super::super) use live_status_layout::InlineTailView;
#[cfg(test)]
// 학습 주석: PlanModeIndicatorView는 footer contract tests가 plan indicator 조합을 직접 주입하기 위해
// test-only로 공개합니다. production code는 current_plan_mode_indicator wrapper를 통해서만 얻습니다.
pub(super) use plan_indicator::PlanModeIndicatorView;

#[cfg(test)]
// 학습 주석: footer copy contract는 의도적으로 많은 independent status slices를 조합합니다. test helper는
// 그 조합 지점을 직접 호출해야 하므로 argument 수 경고를 이 boundary에서만 허용합니다.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_shell_footer_lines_with_context(
    // 학습 주석: context는 shell title, conversation state, action availability 같은 footer 기본 재료입니다.
    context: &ShellCorePresentationContext<'_>,
    // 학습 주석: plan_mode_indicator는 planning status를 footer에 어떻게 붙일지 결정하는 compact view입니다.
    plan_mode_indicator: PlanModeIndicatorView,
    // 학습 주석: parallel_mode_summary_line은 supervisor/slot 상태를 footer ribbon에 섞는 summary copy입니다.
    parallel_mode_summary_line: String,
    // 학습 주석: parallel_mode_alert_line은 missing worktree나 blocked orchestration처럼 attention이 필요한
    // parallel-mode 상태를 optional second-line copy로 전달합니다.
    parallel_mode_alert_line: Option<String>,
    // 학습 주석: github_review_recent_changes_summary는 PR review polling 결과를 footer에 붙이는 optional status입니다.
    github_review_recent_changes_summary: Option<String>,
    // 학습 주석: planning_summary_line은 queue/runtime state의 한 줄 요약입니다. None이면 footer는 planning
    // summary 영역을 생략합니다.
    planning_summary_line: Option<String>,
    // 학습 주석: planning_notice_line은 invalid snapshot, repair, paused auto-follow 같은 부가 planning notice입니다.
    planning_notice_line: Option<String>,
    // 학습 주석: planner_panel_lines는 worker prompt/response/debug panel 요약을 footer copy에 포함할 때 쓰입니다.
    planner_panel_lines: Vec<String>,
) -> Vec<Line<'static>> {
    // 학습 주석: 실제 조합 로직은 footer_copy에 남겨 둡니다. 이 wrapper는 tests가 하위 module path에 직접
    // 의존하지 않도록 하는 안정된 facade입니다.
    footer_copy::build_shell_footer_lines_with_context(
        context,
        plan_mode_indicator,
        parallel_mode_summary_line,
        parallel_mode_alert_line,
        github_review_recent_changes_summary,
        planning_summary_line,
        planning_notice_line,
        planner_panel_lines,
    )
}

// 학습 주석: build_inline_tail_view는 shell renderer가 호출하는 production status panel entrypoint입니다.
// app state와 content width를 받아 prompt/tail/cursor 정보를 포함한 InlineTailView를 만듭니다.
pub(crate) fn build_inline_tail_view(app: &NativeTuiApp, content_width: u16) -> InlineTailView {
    // 학습 주석: live_status_layout이 width-aware layout과 cursor offset 계산을 소유합니다. facade는 그
    // 구현 위치를 숨겨 shell_presentation.rs가 단일 함수만 알게 합니다.
    live_status_layout::build_inline_tail_view(app, content_width)
}

#[cfg(test)]
// 학습 주석: tests가 기존 line-only assertions를 유지할 수 있도록 InlineTailView에서 lines만 꺼내는 adapter입니다.
// production renderer는 cursor/layout metadata까지 필요하므로 build_inline_tail_view를 직접 씁니다.
pub(super) fn build_inline_tail_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    build_inline_tail_view(app, 0).lines
}

// 학습 주석: current_live_agent_lines는 conversation의 streaming/tool activity를 live tail에 표시할 text로
// 낮춥니다. shell overlays와 main tail이 같은 helper를 쓰도록 facade에서 다시 노출합니다.
pub(super) fn current_live_agent_lines(
    // 학습 주석: conversation은 현재 turn state, tool activity, streaming agent message를 포함한 view model입니다.
    conversation: &ConversationViewModel,
) -> Option<Vec<Line<'static>>> {
    // 학습 주석: tail_shared가 live-agent copy policy를 소유합니다. None이면 표시할 live activity가 없다는 뜻입니다.
    tail_shared::current_live_agent_lines(conversation)
}

#[cfg(test)]
// 학습 주석: overlays/base contract tests가 parallel summary copy를 직접 검증할 수 있도록 test-only facade를 둡니다.
pub(super) fn parallel_mode_summary_line(app: &NativeTuiApp) -> String {
    // 학습 주석: tail_shared는 app-wide parallel mode snapshot을 compact footer sentence로 변환합니다.
    tail_shared::parallel_mode_summary_line(app)
}

#[cfg(test)]
// 학습 주석: parallel alert copy는 normal summary와 달리 optional입니다. tests가 None/Some branch를 직접
// 검증할 수 있도록 facade에서만 test 공개합니다.
pub(super) fn parallel_mode_alert_line(app: &NativeTuiApp) -> Option<String> {
    // 학습 주석: tail_shared가 missing/blocked parallel details를 alert line으로 축약합니다.
    tail_shared::parallel_mode_alert_line(app)
}

#[cfg(test)]
// 학습 주석: current plan indicator는 footer rendering의 중요한 branch라 tests에서 직접 만들 수 있게 합니다.
pub(super) fn current_plan_mode_indicator(app: &NativeTuiApp) -> PlanModeIndicatorView {
    // 학습 주석: plan_indicator가 app state의 planning mode/runtime snapshot을 footer view로 변환합니다.
    plan_indicator::current_plan_mode_indicator(app)
}

// 학습 주석: plan_runtime_substate_label은 planning runtime snapshot의 세부 state를 짧은 label로 바꾸는
// shared helper입니다. popup existing workspace와 footer indicator가 같은 label vocabulary를 쓰게 합니다.
pub(super) fn plan_runtime_substate_label(snapshot: &PlanningRuntimeSnapshot) -> &'static str {
    // 학습 주석: 세부 mapping은 plan_indicator에 유지해 status panel label 정책이 한 파일에 모이도록 합니다.
    plan_indicator::plan_runtime_substate_label(snapshot)
}
