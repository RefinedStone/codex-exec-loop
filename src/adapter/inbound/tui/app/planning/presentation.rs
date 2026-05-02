// 학습 주석: `NativeTuiApp`은 planner worker panel state와 debug detail toggle을 가진 상위 TUI 상태입니다.
// 이 presentation helper는 app state를 읽기만 하고 mutation이나 worker control은 하지 않습니다.
use super::super::NativeTuiApp;
// 학습 주석: queue framing summary는 planner가 마지막으로 본 queue 상태를 짧은 한 줄 copy로 줄이는 helper입니다.
// planner detail panel에서 너무 긴 queue 설명이 shell layout을 밀어내지 않게 합니다.
use super::status_projection::compact_queue_framing_summary;
// 학습 주석: domain text compaction은 multiline/detail 문자열을 UI-safe single-line snippet으로 바꿉니다.
// presentation layer가 같은 truncation 규칙을 재구현하지 않고 domain helper를 재사용합니다.
use crate::domain::text::compact_whitespace_detail;

// 학습 주석: 이 함수는 planner worker panel state를 debug panel용 plain string lines로 projection합니다.
// 반환값이 비어 있으면 caller는 planner panel 자체를 그리지 않으므로, visibility gating과 content gating을
// 이 boundary에서 함께 처리합니다.
pub(crate) fn build_planner_panel_lines(app: &NativeTuiApp, max_detail_len: usize) -> Vec<String> {
    // 학습 주석: planner debug details toggle이 꺼져 있으면 worker state가 있어도 화면에 노출하지 않습니다.
    // 운영자가 요청했을 때만 내부 planner 상태가 TUI에 나타나게 하는 privacy/noise gate입니다.
    if !app.planner_shows_debug_details() {
        // 학습 주석: 빈 vector는 "그릴 줄 없음"이라는 renderer contract입니다.
        return Vec::new();
    }

    // 학습 주석: planner_worker_panel_state는 planner runtime에서 마지막으로 관측한 status/detail snapshot입니다.
    // 여기서는 immutable reference로 읽어 presentation copy만 만듭니다.
    let planner = &app.planner_worker_panel_state;
    // 학습 주석: toggle이 켜져 있어도 아직 status/detail이 없으면 빈 panel을 그리지 않습니다. shell의 debug
    // area가 의미 없는 placeholder로 흔들리지 않게 하는 content gate입니다.
    if !planner.has_content() {
        // 학습 주석: content가 없는 경우도 renderer에는 빈 vector로 표현합니다.
        return Vec::new();
    }

    // 학습 주석: 첫 줄은 항상 planner status label로 시작합니다. 이후 queue summary가 있으면 같은 줄에 붙여
    // 가장 중요한 상태와 queue framing을 한눈에 보게 합니다.
    let mut first_line = format!("planner status: {}", planner.status.label());
    // 학습 주석: last_queue_summary는 Option이라 planner가 아직 queue를 관측하지 않았을 수 있습니다. 있을 때만
    // compact helper로 줄여 status line의 보조 segment로 붙입니다.
    if let Some(queue_summary) = planner.last_queue_summary.as_deref() {
        first_line.push_str(&format!(
            "  |  planner queue: {}",
            compact_queue_framing_summary(queue_summary, max_detail_len)
        ));
    }

    // 학습 주석: lines는 status/queue가 들어간 첫 줄로 시작합니다. 아래 optional details는 관측된 값만
    // 뒤에 append되어 panel 높이가 실제 정보량에 맞게 늘어납니다.
    let mut lines = vec![first_line];
    // 학습 주석: last_summary는 planner가 최근 작업을 어떻게 요약했는지 보여 줍니다. multiline summary는
    // compact_whitespace_detail로 접어 shell transcript 흐름을 깨지 않게 합니다.
    if let Some(summary) = planner.last_summary.as_deref() {
        lines.push(format!(
            "planner detail: {}",
            compact_whitespace_detail(summary, max_detail_len)
        ));
    }
    // 학습 주석: notice detail은 사용자에게 알려야 할 planner-side diagnostic입니다. summary와 별도 label을
    // 써서 일반 진행 설명과 주의/알림성 메시지를 구분합니다.
    if let Some(notice_detail) = planner.last_notice_detail.as_deref() {
        lines.push(format!(
            "planner notice: {}",
            compact_whitespace_detail(notice_detail, max_detail_len)
        ));
    }
    // 학습 주석: host detail은 planner worker를 실행한 host/runtime 쪽 정보입니다. worker 자체의 판단과
    // 실행 환경 문제를 분리해서 추적할 수 있게 별도 줄로 둡니다.
    if let Some(host_detail) = planner.last_host_detail.as_deref() {
        lines.push(format!(
            "planner host detail: {}",
            compact_whitespace_detail(host_detail, max_detail_len)
        ));
    }
    // 학습 주석: rejected summary는 planner가 받아들이지 않은 candidate/output을 설명합니다. 정상 summary와
    // 분리해 rejection 원인을 debug panel에서 바로 찾게 합니다.
    if let Some(rejected_summary) = planner.last_rejected_summary.as_deref() {
        lines.push(format!(
            "planner rejected: {}",
            compact_whitespace_detail(rejected_summary, max_detail_len)
        ));
    }
    // 학습 주석: caller는 이 vector를 그대로 debug panel line으로 렌더링합니다. 여기서 순서를 고정해 status,
    // summary, notice, host, rejection 순으로 읽히는 diagnostic hierarchy를 유지합니다.
    lines
}
