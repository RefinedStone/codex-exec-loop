/*
planning_editor_copy는 draft editor의 status copy DTO를 ratatui `Line`으로 낮추는 마지막 표현 계층이다.
검증 결과, dirty label, close risk는 editor UI state와 presentation copy builder가 이미 계산해서 넘긴다.
여기서 editor state를 다시 조회하지 않기 때문에 popup/inline renderer는 같은 status/key vocabulary를 쓰고,
저장/검증/close guard의 실제 상태 전이는 planning_draft_editor_ui와 controller에 남는다.
*/
use super::super::super::super::super::planning_draft_editor_ui::PlanningDraftEditorCloseRisk;
use super::super::super::super::{AkraTheme, Line, PlanningValidationSeverity, Span};
use super::copy::{PlanningDraftEditorStatusCopy, planning_draft_title_line};

/*
 * Header는 editor surface 전체의 위치 정보다. title은 draft editor 계열 chrome을 공유하고,
 * directory line은 사용자가 어느 staged draft workspace를 편집 중인지 확인하게 한다.
 */
pub(super) fn build_planning_draft_editor_header_lines(
    draft_directory: &str,
) -> Vec<Line<'static>> {
    vec![
        planning_draft_title_line(" / operator inspection"),
        Line::from(format!("draft dir: {draft_directory}")),
    ]
}

/*
 * Status builder는 `PlanningDraftEditorStatusCopy` 하나만 입력으로 받아 footer/status panel을 만든다.
 * surface layer가 모아 둔 copy를 그대로 소비하므로 이 함수는 editor state, validation report,
 * close guard를 다시 조회하지 않고 Line ordering과 색상 선택만 책임진다.
 */
pub(super) fn build_planning_draft_editor_status_lines(
    copy: PlanningDraftEditorStatusCopy,
) -> Vec<Line<'static>> {
    /*
     * 첫 세 줄은 editor의 기본 위치와 promote 가능성이다. draft/file/validation을 항상 같은 순서로
     * 보여 주어 파일을 바꾸거나 저장할 때 status panel의 시각적 anchor가 흔들리지 않게 한다.
     */
    let mut status_lines = vec![
        Line::from(format!("staged draft: {}", copy.draft_name)),
        Line::from(format!(
            "current file: {} ({}/{})",
            copy.active_path, copy.selected_file_position, copy.file_count
        )),
        Line::from(vec![
            Span::styled("validation state: ", AkraTheme::muted()),
            /*
             * validation_ok는 copy projection이 만든 coarse promote gate다. 이 층은 문구와 색상만 고르고,
             * 어떤 rule이 실패했는지는 아래 first_issue line이 담당한다.
             */
            Span::styled(
                if copy.validation_ok {
                    "ok"
                } else {
                    "needs attention"
                },
                if copy.validation_ok {
                    AkraTheme::success()
                } else {
                    AkraTheme::warning()
                },
            ),
        ]),
    ];
    if let Some(issue) = copy.first_issue {
        /*
         * validation issue가 있으면 staged path보다 issue를 우선 표시한다. 사용자가 편집기 안에서 바로
         * 고칠 수 있는 신호가 경로 정보보다 행동 가치가 높기 때문이다.
         */
        status_lines.push(Line::from(vec![
            Span::styled(
                match issue.severity {
                    PlanningValidationSeverity::Error => "error: ",
                    PlanningValidationSeverity::Warning => "warning: ",
                },
                match issue.severity {
                    PlanningValidationSeverity::Error => AkraTheme::danger(),
                    PlanningValidationSeverity::Warning => AkraTheme::warning(),
                },
            ),
            Span::raw(issue.detail),
        ]));
    } else {
        /*
         * 문제가 없을 때는 selected draft file이 어떤 staged path로 쓰일지 보여 준다. validation issue와
         * 상호 배타로 두어 좁은 footer 영역에서 핵심 정보가 과밀해지지 않게 한다.
         */
        status_lines.push(Line::from(format!(
            "staged path: {}",
            copy.staged_path_summary
        )));
    }
    status_lines.push(Line::from(format!("dirty: {}", copy.dirty_label_summary)));
    if copy.has_dirty_labels {
        /*
         * dirty buffer가 있으면 validation report는 마지막 저장 시점 기준이다. 이 note는 사용자가 현재
         * 화면의 validation state를 "방금 타이핑한 내용까지 검증됨"으로 오해하지 않도록 경계를 표시한다.
         */
        status_lines.push(Line::from(
            "validation note: the status above reflects the last saved draft until Ctrl+S re-runs checks",
        ));
    }
    status_lines.push(Line::from(copy.next_action));
    if let Some(risk) = copy.close_risk {
        /*
         * close risk는 dirty buffer와 invalid staged draft를 잃을 수 있는 상황을 설명한다.
         * confirmation_pending이면 이미 사용자가 닫기를 시도한 두 번째 단계라 danger 색상으로 강도를 올린다.
         */
        status_lines.push(Line::from(vec![
            Span::styled(
                if copy.confirmation_pending {
                    "close pending: "
                } else {
                    "close guard: "
                },
                if copy.confirmation_pending {
                    AkraTheme::danger()
                } else {
                    AkraTheme::warning()
                },
            ),
            Span::raw(planning_draft_close_guard_detail(
                risk,
                copy.confirmation_pending,
            )),
        ]));
    }
    status_lines
}

/*
 * Key guide는 editor 조작법을 안정된 네 줄로 유지한다. 첫 세 줄은 항상 같은 편집/저장 단축키이고,
 * 마지막 줄만 close risk 상태에 따라 즉시 닫기, review close, confirm close로 바뀐다.
 */
pub(super) fn build_planning_draft_editor_key_lines(
    close_risk: Option<PlanningDraftEditorCloseRisk>,
    confirmation_pending: bool,
) -> Vec<Line<'static>> {
    vec![
        AkraTheme::key_line("controls: Tab/BackTab switches files  |  arrows move the cursor"),
        AkraTheme::key_line(
            "controls: Enter inserts newline  |  Backspace deletes  |  Ctrl+W deletes the previous word",
        ),
        AkraTheme::key_line(
            "controls: Ctrl+S saves and validates  |  Ctrl+P saves and promotes active planning",
        ),
        planning_draft_editor_close_key_line(close_risk, confirmation_pending),
    ]
}

/*
 * close guard detail은 `PlanningDraftEditorCloseRisk`의 두 risk bit와 confirmation 단계 조합을
 * 사람이 읽는 문장으로 바꾼다. status panel은 위험의 의미를 설명하고, key guide는 같은 close state를
 * 다음 입력 가능 키로 번역하므로 이 조합표가 두 표현의 공통 vocabulary가 된다.
 */
fn planning_draft_close_guard_detail(
    risk: PlanningDraftEditorCloseRisk,
    confirmation_pending: bool,
) -> &'static str {
    match (
        risk.has_dirty_buffers(),
        risk.has_invalid_staged_draft(),
        confirmation_pending,
    ) {
        (true, true, true) => {
            "discard unsaved edits or keep editing; the invalid staged draft will remain on disk"
        }
        (true, false, true) => "discard unsaved edits or press n to keep editing",
        (false, true, true) => {
            "close now or press n to keep editing; the invalid staged draft will remain on disk"
        }
        (true, true, false) => {
            "unsaved edits and an invalid staged draft require confirmation before close"
        }
        (true, false, false) => "unsaved edits require confirmation before close",
        (false, true, false) => "an invalid staged draft requires confirmation before close",
        /*
         * close_risk가 Some인데 두 bit가 모두 false인 경우는 정상 흐름에서는 드물다.
         * 그래도 exhaustiveness를 위해 즉시 닫기 가능 문구를 돌려 UI가 모순된 경고를 표시하지 않게 한다.
         */
        (false, false, _) => "close is available immediately",
    }
}

/*
 * close key line은 같은 close state를 조작 안내로 번역한다. detail 문장은 위험을 설명하고,
 * 이 함수는 사용자가 다음에 누를 수 있는 키를 현재 confirmation 단계에 맞춰 좁게 보여 준다.
 */
fn planning_draft_editor_close_key_line(
    close_risk: Option<PlanningDraftEditorCloseRisk>,
    confirmation_pending: bool,
) -> Line<'static> {
    if confirmation_pending {
        return AkraTheme::key_line(
            "controls: Enter, Esc, or Ctrl+C confirms close  |  n keeps editing",
        );
    }

    if close_risk.is_some() {
        return AkraTheme::key_line("controls: Esc/Ctrl+C reviews close");
    }

    AkraTheme::key_line("controls: Esc/Ctrl+C closes this surface")
}
