/*
 * planning controller의 status text helper는 TUI 입력 처리와 application planning service 사이의
 * operator-facing audit boundary다. workspace 변경, doctor 검사, editor state transition은 다른
 * 모듈이 수행하고, 이 파일은 그 결과를 footer/status line이 즉시 보여 줄 수 있는 한 줄 문장으로
 * 고정한다. 그래서 controller action은 service result를 그대로 노출하지 않고 이 작은 copy layer를 통과한다.
 */
use super::*;

/*
 * reset 명령 usage는 parse 실패, 잘못된 confirm token, 초과 인자 모두에서 같은 회복 경로로 나가야 한다.
 * 한 상수로 묶어 `handle_reset_shell_command`가 실패 종류별로 다른 문법을 암시하지 않게 한다.
 */
const RESET_USAGE_TEXT: &str =
    "usage: :reset <queue|directions|all>  |  add `confirm` for directions or all";

/*
 * planning draft editor는 overlay 위의 in-memory buffer와 이미 staged 된 draft를 동시에 다룬다.
 * close warning은 "저장하지 않은 buffer 손실"과 "검증 실패 draft가 디스크에 남음"을 분리해서 알려야,
 * 사용자가 Esc/Enter를 한 번 더 누를 때 어떤 상태가 사라지고 어떤 상태가 남는지 판단할 수 있다.
 */
pub(super) fn planning_manual_editor_close_warning_status(
    risk: PlanningDraftEditorCloseRisk,
) -> String {
    match (risk.has_dirty_buffers(), risk.has_invalid_staged_draft()) {
        (true, true) => "planning draft editor close pending; press Esc again or Enter to discard unsaved edits and leave the invalid staged draft for later review, or press n to keep editing".to_string(),
        (true, false) => "planning draft editor close pending; press Esc again or Enter to discard unsaved edits, or press n to keep editing".to_string(),
        (false, true) => "planning draft editor close pending; press Esc again or Enter to close and leave the invalid staged draft for later review, or press n to keep editing".to_string(),
        (false, false) => "planning draft editor close pending".to_string(),
    }
}

/*
 * close warning이 두 번째 입력으로 확정되면 controller는 overlay를 닫고 이 문구를
 * `ConversationInputEvent::StatusMessageShown`으로 보낸다. 이 helper는 cleanup을 수행하지 않고,
 * 이미 발생한 close transition을 사용자가 볼 수 있는 audit trail로 남긴다.
 */
pub(super) fn planning_manual_editor_closed_status(risk: PlanningDraftEditorCloseRisk) -> String {
    match (risk.has_dirty_buffers(), risk.has_invalid_staged_draft()) {
        (true, true) => "planning draft editor closed; unsaved in-memory edits were discarded and the staged draft still needs validation".to_string(),
        (true, false) => {
            "planning draft editor closed; unsaved in-memory edits were discarded".to_string()
        }
        (false, true) => "planning draft editor closed; invalid staged draft remains in drafts for review".to_string(),
        (false, false) => "planning draft editor closed".to_string(),
    }
}

/*
 * directions editor도 같은 draft editor state machine을 쓰지만 operator가 보는 대상은 planning queue가
 * 아니라 direction authority다. 별도 helper로 둬 동일한 risk matrix를 유지하면서 copy의 주어만
 * directions maintenance 흐름에 맞춘다.
 */
pub(super) fn directions_manual_editor_close_warning_status(
    risk: PlanningDraftEditorCloseRisk,
) -> String {
    match (risk.has_dirty_buffers(), risk.has_invalid_staged_draft()) {
        (true, true) => "directions editor close pending; press Esc again or Enter to discard unsaved edits and leave the invalid staged draft for later review, or press n to keep editing".to_string(),
        (true, false) => "directions editor close pending; press Esc again or Enter to discard unsaved edits, or press n to keep editing".to_string(),
        (false, true) => "directions editor close pending; press Esc again or Enter to close and leave the invalid staged draft for later review, or press n to keep editing".to_string(),
        (false, false) => "directions editor close pending".to_string(),
    }
}

/*
 * directions editor가 닫힌 뒤에는 `present_directions_maintenance_overview`가 다시 summary를 로드한다.
 * 이 문구는 방금 떠난 editor에서 어떤 draft 위험이 남았는지 overview status line으로 이어 주어,
 * 사용자가 directions maintenance 흐름에서 맥락을 잃지 않게 한다.
 */
pub(super) fn directions_manual_editor_closed_status(risk: PlanningDraftEditorCloseRisk) -> String {
    match (risk.has_dirty_buffers(), risk.has_invalid_staged_draft()) {
        (true, true) => "directions editor closed; unsaved in-memory edits were discarded and the staged draft still needs validation".to_string(),
        (true, false) => {
            "directions editor closed; unsaved in-memory edits were discarded".to_string()
        }
        (false, true) => "directions editor closed; invalid staged draft remains in drafts for review".to_string(),
        (false, false) => "directions editor closed".to_string(),
    }
}

/*
 * planning doctor report는 application service가 workspace, queue, proposal, health를 검사한 projection이다.
 * TUI controller는 report 구조 전체를 footer에 밀어 넣지 않고 이 compact status line만 보낸다.
 * absent workspace일 때만 다음 행동까지 붙여 첫 실행 사용자가 `:planning`으로 넘어갈 수 있게 한다.
 */
pub(super) fn planning_doctor_status_text(report: &PlanningDoctorReport) -> String {
    let mut parts = vec![format!(
        "planning state: {}",
        report.planning_state().label()
    )];

    // queue-idle policy는 worker scheduling 정책이므로 workspace state 뒤에 붙는 보조 진단이다.
    if let Some(queue_idle_policy) = report.queue_idle_policy() {
        parts.push(format!("queue-idle: {queue_idle_policy}"));
    }
    // queue summary는 현재 task authority가 어떤 작업을 먼저 실행할지 status line에 압축한다.
    if let Some(queue_summary) = report.queue_summary() {
        parts.push(format!("queue: {queue_summary}"));
    }
    // proposal summary는 directions/detail authoring 쪽 후보 상태를 doctor 결과에 연결한다.
    if let Some(proposal_summary) = report.proposal_summary() {
        parts.push(format!("proposals: {proposal_summary}"));
    }
    /*
     * issue가 있으면 health보다 우선한다. 사용자는 정상성 요약보다 고쳐야 하는 원인을 먼저 봐야 하고,
     * controller는 이 한 줄을 footer에도 그대로 흘려보낸다.
     */
    if let Some(issue) = report.issue() {
        parts.push(format!("issue: {issue}"));
    } else if let Some(health) = report.health() {
        parts.push(format!("health: {health}"));
    }
    // note는 hard failure가 아닌 보충 설명이라 마지막에 붙여 앞선 판단을 흐리지 않게 한다.
    if let Some(note) = report.note() {
        parts.push(format!("note: {note}"));
    }
    if report.planning_state() == PlanningDoctorState::Absent {
        parts.push("next action: run :planning to stage the default planning scaffold".to_string());
    }

    parts.join(" / ")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/*
 * `:reset` shell command는 target마다 destructive 정도가 다르다. controller가 parse, preview,
 * confirm, execute 결정을 분리할 수 있도록 target과 confirm 여부를 함께 넘기는 작은 command DTO를 둔다.
 */
pub(super) struct ParsedResetShellCommand {
    // queue/directions/all 중 실제 workspace reset service에 전달할 대상이다.
    pub(super) target: PlanningResetTarget,
    // directions/all처럼 authority를 크게 바꾸는 reset은 preview 후 confirm token을 요구한다.
    pub(super) confirmed: bool,
}

/*
 * parse_reset_shell_argument는 TUI shell 문자열을 application의 `PlanningResetTarget`으로 바꾸는
 * adapter boundary다. 여기서 허용 문법을 좁게 유지해야 `reset_workspace` service가 UI 전용 문자열
 * 해석을 알 필요가 없고, controller는 preview/confirm/execute 흐름만 조립하면 된다.
 */
pub(super) fn parse_reset_shell_argument(
    argument: Option<&str>,
) -> Result<ParsedResetShellCommand, String> {
    let Some(argument) = argument.map(str::trim).filter(|value| !value.is_empty()) else {
        return Err(RESET_USAGE_TEXT.to_string());
    };
    let mut parts = argument.split_whitespace();
    let Some(target) = parts.next() else {
        return Err(RESET_USAGE_TEXT.to_string());
    };
    let confirmation = parts.next();
    let confirmed = match confirmation {
        None => false,
        Some(value) if value.eq_ignore_ascii_case("confirm") => true,
        Some(_) => {
            return Err(RESET_USAGE_TEXT.to_string());
        }
    };
    if parts.next().is_some() {
        return Err(RESET_USAGE_TEXT.to_string());
    }
    let target = match target.to_ascii_lowercase().as_str() {
        "queue" => PlanningResetTarget::Queue,
        "directions" => PlanningResetTarget::Directions,
        "all" => PlanningResetTarget::All,
        _ => {
            return Err(RESET_USAGE_TEXT.to_string());
        }
    };
    Ok(ParsedResetShellCommand { target, confirmed })
}

/*
 * reset preview text는 destructive action을 바로 실행하지 않는 안전 장치다. queue reset은 derived
 * state 정리라 즉시 실행할 수 있지만, directions/all은 authority 문서와 prompt artifacts를 바꾸므로
 * controller가 이 문구를 먼저 보여 주고 `confirm` 입력을 기다린다.
 */
pub(super) fn planning_reset_preview_text(target: PlanningResetTarget) -> String {
    match target {
        PlanningResetTarget::Queue => {
            "reset queue preview: rewrites DB task authority and clears derived queue state"
                .to_string()
        }
        PlanningResetTarget::Directions => "reset directions preview: rewrites DB direction authority, recreates the default queue-idle prompt, removes direction detail docs and prompt artifacts, and clears derived queue state / rerun `:reset directions confirm` to continue".to_string(),
        PlanningResetTarget::All => "reset all preview: replaces the full active planning scaffold, clears derived queue state, and refreshes the planning authority / rerun `:reset all confirm` to continue".to_string(),
    }
}

/*
 * reset result text는 `PlanningWorkspaceResetResult`의 path 목록을 그대로 노출하지 않고 rewritten/removed
 * 개수만 status line에 남긴다. 상세 파일 목록은 service result와 log의 영역이고, TUI footer는 reset이
 * 어느 범위에 적용됐는지 빠르게 확인하는 데 집중한다.
 */
pub(super) fn planning_reset_status_text(result: &PlanningWorkspaceResetResult) -> String {
    format!(
        "planning reset applied / target: {} / rewritten: {} / removed: {}",
        result.target.label(),
        result.rewritten_paths.len(),
        result.removed_paths.len(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_shell_argument_maps_to_shared_application_targets() {
        /*
         * TUI shell spelling is an inbound grammar detail. The parser must emit
         * PlanningResetTarget so CLI, admin, Telegram, and TUI stay on the same
         * destructive reset vocabulary.
         */
        for (raw, expected, confirmed) in [
            ("queue", PlanningResetTarget::Queue, false),
            ("directions", PlanningResetTarget::Directions, false),
            ("directions confirm", PlanningResetTarget::Directions, true),
            ("all", PlanningResetTarget::All, false),
            ("all confirm", PlanningResetTarget::All, true),
        ] {
            let parsed = parse_reset_shell_argument(Some(raw)).expect("reset target should parse");
            assert_eq!(parsed.target, expected);
            assert_eq!(parsed.confirmed, confirmed);
        }

        assert!(parse_reset_shell_argument(Some("tasks")).is_err());
        assert!(parse_reset_shell_argument(Some("queue now")).is_err());
        assert!(parse_reset_shell_argument(None).is_err());
    }
}
