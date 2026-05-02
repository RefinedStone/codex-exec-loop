// 학습 주석: existing workspace view는 themed key lines와 plain styled lines를 섞어 공통 planning init DTO를 채웁니다.
use super::super::super::super::super::{AkraTheme, Line};
// 학습 주석: 최종 반환 type은 popup renderer가 소비하는 planning init overlay view입니다.
use super::super::super::PlanningInitOverlayView;
// 학습 주석: copy는 runtime snapshot에서 추출된 presentation data이고, title helper는 planning setup 계열 styling을 맞춥니다.
use super::super::copy::{PlanningExistingWorkspaceCopy, planning_setup_title_line};

// 학습 주석: 이 builder는 "새 bootstrap scaffold를 만들지 말고 기존 planning workspace를 관리하라"는
// init overlay variant를 조립합니다. input copy만 소비해 app/runtime state에는 직접 접근하지 않습니다.
pub(super) fn build_existing_workspace_overlay_view(
    // 학습 주석: copy에는 workspace path, plan/queue state, idle policy, optional failure summary가 들어 있습니다.
    copy: PlanningExistingWorkspaceCopy,
) -> PlanningInitOverlayView {
    // 학습 주석: status lines는 primary next actions로 시작하고, runtime failure가 있으면 같은 status area에
    // 추가 diagnostic을 붙입니다.
    let mut status_lines = vec![
        // 학습 주석: existing workspace에서 Enter는 bootstrap promote가 아니라 queue inspection으로 연결됩니다.
        Line::from("Enter opens queue inspection for the existing planning workspace."),
        // 학습 주석: D shortcut은 directions maintenance로 빠르게 들어가는 복구/관리 path입니다.
        Line::from("Press D to maintain directions."),
    ];
    // 학습 주석: failure summary가 있을 때만 status 영역에 planning failure line을 추가합니다.
    if let Some(failure_summary) = copy.failure_summary.as_deref() {
        status_lines.push(Line::from(format!("planning failure: {failure_summary}")));
    }

    PlanningInitOverlayView {
        // 학습 주석: header는 planning setup flow 안에서 existing workspace branch에 들어왔음을 알려 줍니다.
        header_lines: vec![
            planning_setup_title_line(" / existing workspace"),
            // 학습 주석: 사용자가 새 scaffold를 restage하지 않고 현재 runtime을 관리해야 한다는 핵심 안내입니다.
            Line::from(
                "This workspace already has accepted planning state. Manage the current runtime instead of restaging a bootstrap scaffold.",
            ),
        ],
        // 학습 주석: summary는 hidden planner sessions가 structured payload로 DB task authority를 업데이트한다는
        // runtime architecture를 짧게 설명합니다.
        summary_lines: vec![Line::from(
            "Hidden planner sessions update DB task authority through structured payloads.",
        )],
        // 학습 주석: option lines는 copy에서 온 runtime/workspace facts를 나열해 현재 상태를 점검하게 합니다.
        option_lines: vec![
            // 학습 주석: workspace line은 어떤 directory의 planning state를 관리하는지 보여 줍니다.
            Line::from(format!("workspace: {}", copy.workspace_directory)),
            // 학습 주석: plan state line은 runtime substate를 보여 줍니다.
            Line::from(format!("planning state: {}", copy.plan_state_label)),
            // 학습 주석: queue state line은 current queue summary 또는 unavailable fallback을 보여 줍니다.
            Line::from(format!("queue state: {}", copy.queue_summary)),
            // 학습 주석: idle policy line은 queue가 비었을 때 planner가 어떤 정책으로 반응하는지 설명합니다.
            Line::from(format!("queue idle policy: {}", copy.queue_idle_policy)),
        ],
        status_lines,
        // 학습 주석: key lines는 existing workspace branch에서 가능한 navigation shortcuts를 theme style로 고정합니다.
        key_lines: vec![
            // 학습 주석: Enter와 Q가 같은 queue inspection path를 여는 점을 primary/secondary 형태로 모두 노출합니다.
            AkraTheme::key_line("Enter opens queue inspection."),
            // 학습 주석: directions maintenance shortcut은 existing workspace 복구/관리의 다른 축입니다.
            AkraTheme::key_line("Q opens queue inspection. D opens directions maintenance."),
            // 학습 주석: close shortcut은 새 scaffold를 만들지 않고 overlay를 빠져나가는 escape path입니다.
            AkraTheme::key_line("Esc/Ctrl+C closes this surface."),
        ],
    }
}
