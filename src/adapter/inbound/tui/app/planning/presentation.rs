use super::super::NativeTuiApp;
use super::debug_panel_state::PlanningWorkerPanelState;
use super::status_projection::compact_queue_framing_summary;
use crate::domain::text::compact_whitespace_detail;

// Planning worker debug panel은 runtime worker를 제어하지 않고 마지막 관측 snapshot만 읽는 presentation surface다.
// 빈 Vec은 "panel을 그리지 않음"이라는 renderer contract라서 visibility gate와 content gate를 여기서 함께 확정한다.
pub(crate) fn build_planning_worker_panel_lines(
    app: &NativeTuiApp,
    max_detail_len: usize,
) -> Vec<String> {
    build_planning_worker_panel_lines_for_state(
        app.planning_worker_shows_debug_details(),
        &app.planning_worker_panel_state,
        max_detail_len,
    )
}

fn build_planning_worker_panel_lines_for_state(
    show_debug_details: bool,
    planning_worker: &PlanningWorkerPanelState,
    max_detail_len: usize,
) -> Vec<String> {
    // planning worker detail은 operator가 켠 경우에만 노출된다. 기본 shell 화면은 worker internals 대신
    // user-facing planning status surfaces를 우선한다.
    if !show_debug_details {
        return Vec::new();
    }

    // toggle이 켜져 있어도 관측된 내용이 없으면 placeholder panel을 만들지 않는다.
    // debug area height가 빈 diagnostic 때문에 흔들리는 일을 피한다.
    if !planning_worker.has_content() {
        return Vec::new();
    }

    // 첫 줄은 status를 anchor로 두고, queue framing이 있으면 같은 line에 붙여 worker state와 queue context를 함께 읽게 한다.
    let mut first_line = format!("planning worker status: {}", planning_worker.status.label());
    if let Some(operation_label) = planning_worker.last_operation_label.as_deref() {
        first_line.push_str(&format!(
            "  |  planning worker operation: {}",
            compact_whitespace_detail(operation_label, max_detail_len)
        ));
    }
    if let Some(queue_summary) = planning_worker.last_queue_summary.as_deref() {
        first_line.push_str(&format!(
            "  |  planning worker queue: {}",
            compact_queue_framing_summary(queue_summary, max_detail_len)
        ));
    }

    // diagnostic hierarchy는 가장 안정적인 status/queue에서 시작해 점점 구체적인 detail로 내려간다.
    // 관측되지 않은 optional field는 생략해 panel 높이가 실제 정보량만 반영하게 한다.
    let mut lines = vec![first_line];
    // summary는 worker가 최근 판단한 작업 설명이고, multiline payload는 shell 한 줄 panel에 맞게 접는다.
    if let Some(summary) = planning_worker.last_summary.as_deref() {
        lines.push(format!(
            "planning worker detail: {}",
            compact_whitespace_detail(summary, max_detail_len)
        ));
    }
    // notice는 진행 설명보다 operator attention이 필요한 planning-worker-side diagnostic이라 별도 label로 분리한다.
    if let Some(notice_detail) = planning_worker.last_notice_detail.as_deref() {
        lines.push(format!(
            "planning worker notice: {}",
            compact_whitespace_detail(notice_detail, max_detail_len)
        ));
    }
    // host detail은 worker 판단이 아니라 실행 환경의 문제를 추적하기 위한 channel이다.
    if let Some(host_detail) = planning_worker.last_host_detail.as_deref() {
        lines.push(format!(
            "planning worker host detail: {}",
            compact_whitespace_detail(host_detail, max_detail_len)
        ));
    }
    // rejected summary는 정상 summary와 섞지 않아 planning worker가 candidate를 버린 이유를 바로 찾게 한다.
    if let Some(rejected_summary) = planning_worker.last_rejected_summary.as_deref() {
        lines.push(format!(
            "planning worker rejected: {}",
            compact_whitespace_detail(rejected_summary, max_detail_len)
        ));
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::super::debug_panel_state::{PlanningWorkerPanelState, PlanningWorkerStatus};
    use super::build_planning_worker_panel_lines_for_state;

    #[test]
    fn planning_worker_panel_is_hidden_without_debug_visibility() {
        let state = PlanningWorkerPanelState {
            status: PlanningWorkerStatus::RefreshSucceeded,
            last_summary: Some("accepted task".to_string()),
            ..Default::default()
        };

        assert!(build_planning_worker_panel_lines_for_state(false, &state, 40).is_empty());
    }

    #[test]
    fn planning_worker_panel_skips_empty_debug_state() {
        assert!(
            build_planning_worker_panel_lines_for_state(
                true,
                &PlanningWorkerPanelState::default(),
                40
            )
            .is_empty()
        );
    }

    #[test]
    fn planning_worker_panel_renders_operation_queue_and_diagnostics() {
        let state = PlanningWorkerPanelState {
            status: PlanningWorkerStatus::RepairFailed,
            last_operation_label: Some("repair runtime projection after invalid files".to_string()),
            last_queue_summary: Some("ready task with\nmultiline framing".to_string()),
            last_summary: Some("worker accepted a focused repair candidate".to_string()),
            last_notice_detail: Some("validation still reports one blocking error".to_string()),
            last_host_detail: Some("host rejected stale candidate revision".to_string()),
            last_rejected_summary: Some("candidate rewrote unrelated task".to_string()),
            ..Default::default()
        };

        let lines = build_planning_worker_panel_lines_for_state(true, &state, 36);

        assert_eq!(lines.len(), 5);
        assert!(lines[0].contains("planning worker status: repair failed"));
        assert!(lines[0].contains("planning worker operation: repair runtime projection"));
        assert!(lines[0].contains("planning worker queue: ready task"));
        assert!(lines[1].contains("planning worker detail: worker accepted"));
        assert!(lines[2].contains("planning worker notice: validation still"));
        assert!(lines[3].contains("planning worker host detail: host rejected"));
        assert!(lines[4].contains("planning worker rejected: candidate rewrote"));
    }
}
