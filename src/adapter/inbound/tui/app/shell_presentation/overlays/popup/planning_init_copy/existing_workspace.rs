use super::super::super::super::super::{AkraTheme, Line};
use super::super::super::PlanningInitOverlayView;
use super::super::copy::{PlanningExistingWorkspaceCopy, planning_setup_title_line};

// 이미 accepted planning state가 있는 workspace에서는 init flow가 새 bootstrap scaffold를
// 다시 만들면 안 된다. 이 builder는 copy DTO만 소비해 "현재 runtime을 관리하라"는
// guard 화면을 만들고, app/runtime snapshot 선택 정책은 위 projection layer에 남긴다.
pub(super) fn build_existing_workspace_overlay_view(
    copy: PlanningExistingWorkspaceCopy,
) -> PlanningInitOverlayView {
    // status 영역은 복구 가능한 next action을 먼저 보여 준 뒤 runtime failure를 덧붙인다.
    // 실패가 있더라도 queue/directions로 진입할 수 있어야 하므로 diagnostic이 action copy를
    // 밀어내지 않게 같은 vector 뒤쪽에 추가한다.
    let mut status_lines = vec![
        Line::from("Enter opens queue inspection for the existing planning workspace."),
        Line::from("Press D to maintain directions."),
    ];
    if let Some(failure_summary) = copy.failure_summary.as_deref() {
        status_lines.push(Line::from(format!("planning failure: {failure_summary}")));
    }

    PlanningInitOverlayView {
        // header는 사용자가 아직 planning setup modal 안에 있지만, 현재 branch가 creation이
        // 아니라 existing workspace management임을 즉시 알려 준다.
        header_lines: vec![
            planning_setup_title_line(" / existing workspace"),
            Line::from(
                "This workspace already has accepted planning state. Manage the current runtime instead of restaging a bootstrap scaffold.",
            ),
        ],
        // summary는 왜 queue inspection이 primary action인지 설명한다. accepted planning은
        // hidden planning worker session의 structured payload를 통해 DB task authority를 갱신하므로,
        // 사용자는 scaffold 생성보다 runtime queue 상태를 봐야 한다.
        summary_lines: vec![Line::from(
            "Hidden planning worker sessions update DB task authority through structured payloads.",
        )],
        // option lines는 선택지가 아니라 현재 workspace fact sheet다. path, planning substate,
        // queue summary, idle policy를 한 곳에 두어 사용자가 어느 runtime을 관리할지 확인한다.
        option_lines: vec![
            Line::from(format!("workspace: {}", copy.workspace_directory)),
            Line::from(format!("planning state: {}", copy.plan_state_label)),
            Line::from(format!("queue state: {}", copy.queue_summary)),
            Line::from(format!("queue idle policy: {}", copy.queue_idle_policy)),
        ],
        status_lines,
        // key lines는 init wizard의 promote/edit vocabulary를 쓰지 않는다. Enter/Q/D/Esc가
        // 기존 runtime 검사, directions 관리, 닫기라는 관리 surface의 shortcut임을 고정한다.
        key_lines: vec![
            AkraTheme::key_line("Enter opens queue inspection."),
            AkraTheme::key_line("Q opens queue inspection. D opens directions maintenance."),
            AkraTheme::key_line("Esc/Ctrl+C closes this surface."),
        ],
    }
}
