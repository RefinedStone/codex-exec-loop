/*
 * 학습 주석: directions_copy는 directions maintenance overlay의 copy deck이다. controller는
 * DirectionsMaintenanceOverlayView라는 작은 view-model만 만들고, 실제 문장과 key hint는 이 파일에
 * 모아 두어 상태 전환 로직이 UI 문구 변경에 끌려가지 않게 한다.
 */
use super::super::super::{AkraTheme, DetailDocConfirmChoice, Line};
use super::super::option_lines::overlay_option_line;
use super::DirectionsMaintenanceOverlayView;

// 학습 주석: 모든 directions overlay가 같은 title prefix를 써서 shell chrome 안에서 한 기능군으로 보이게 한다.
fn directions_title_line(suffix: &'static str) -> Line<'static> {
    AkraTheme::title_line("Directions Maintenance", suffix)
}

/*
 * 학습 주석: overview overlay는 directions catalog와 queue-idle prompt health를 한 화면에 압축한다.
 * 여기서 action copy는 "accepted state는 promote 전까지 변하지 않는다"는 authoring architecture를
 * 반복해서 보여 주며, operator가 raw file을 직접 고치기 전에 staged draft/editor 흐름으로 들어가게 한다.
 */
pub(super) fn build_overview_overlay_view(
    missing_doc_count: usize,
    broken_doc_count: usize,
    total_direction_count: usize,
    queue_idle_policy: &str,
    queue_idle_prompt_status: &str,
    queue_idle_prompt: &str,
    parse_error_summary: Option<&str>,
) -> DirectionsMaintenanceOverlayView {
    DirectionsMaintenanceOverlayView {
        header_lines: vec![
            directions_title_line(" / shell inspection"),
            Line::from(
                "Review operator-owned planning directions and queue-idle policy without editing raw files first.",
            ),
        ],
        /*
         * 학습 주석: summary는 이 overlay가 inspector이면서도 editor 진입점임을 설명한다. accepted DB
         * authority를 바로 바꾸지 않고 staged draft를 만들기 때문에, runtime prompt는 promote 전까지
         * 기존 accepted state를 계속 본다.
         */
        summary_lines: vec![
            Line::from(
                "Use Enter or `p` to create/edit the queue-idle prompt, or `d` to create a detail-doc mapping.",
            ),
            Line::from(
                "Accepted planning state does not change until you promote the staged draft.",
            ),
        ],
        /*
         * 학습 주석: parse error가 있으면 repair/detail actions를 막는다. controller가 invalid catalog를
         * direction 단위로 신뢰할 수 없으므로, 먼저 prompt editor나 reload/manual repair 쪽으로 operator를
         * 유도하는 presentation contract다.
         */
        option_lines: vec![
            overlay_option_line(
                "Enter",
                "edit prompt",
                "stage the queue-idle review prompt markdown and create or repair prompt_path if needed",
                false,
                false,
            ),
            overlay_option_line(
                "D",
                "repair detail docs",
                "choose one direction with a missing or broken doc mapping and stage a markdown file",
                false,
                parse_error_summary.is_some() || (missing_doc_count == 0 && broken_doc_count == 0),
            ),
            overlay_option_line(
                "P",
                "edit queue-idle prompt",
                "stage the queue-idle review prompt markdown and create or repair prompt_path if needed",
                false,
                parse_error_summary.is_some(),
            ),
        ],
        // 학습 주석: status는 health snapshot을 그대로 노출해 doctor/admin 경로와 같은 문제 수치를 보여 준다.
        status_lines: vec![
            Line::from(format!(
                "directions: {total_direction_count} total / {missing_doc_count} missing docs / {broken_doc_count} broken docs"
            )),
            Line::from(format!(
                "queue idle: policy {queue_idle_policy} / prompt {queue_idle_prompt_status} / {queue_idle_prompt}"
            )),
            Line::from(match parse_error_summary {
                Some(error) => format!("directions parse error: {error}"),
                None => "directions parsing: ok".to_string(),
            }),
        ],
        key_lines: vec![
            AkraTheme::key_line(
                "Enter/p: edit queue-idle prompt    d: create or repair detail doc",
            ),
            AkraTheme::key_line("r: reload summary    Esc/Ctrl+C: close"),
        ],
    }
}

/*
 * 학습 주석: detail-doc selection은 validation 결과에서 missing/broken mapping만 option으로 받은 뒤,
 * operator에게 어느 direction의 supporting file을 만들지 고르게 한다. 이 파일은 선택 목록을 계산하지
 * 않고 copy shell만 책임져 presentation layer의 역할을 좁게 유지한다.
 */
pub(super) fn build_detail_doc_selection_overlay_view(
    option_lines: Vec<Line<'static>>,
    selected_direction_title: Option<&str>,
) -> DirectionsMaintenanceOverlayView {
    DirectionsMaintenanceOverlayView {
        header_lines: vec![
            directions_title_line(" / detail docs"),
            Line::from(
                "Choose a direction whose detail-doc mapping should be created or repaired.",
            ),
        ],
        /*
         * 학습 주석: generated path copy는 supporting_files helper와 같은 convention을 사용자에게 드러낸다.
         * file 생성과 catalog mapping 갱신이 함께 staged 된다는 점을 설명해 runtime prompt가 promote 후에만
         * 새 detail doc을 참조한다는 경계를 유지한다.
         */
        summary_lines: vec![
            Line::from(
                "Generated docs follow `.codex-exec-loop/planning/directions/<direction-id>.md`.",
            ),
            Line::from(
                "The file and `detail_doc_path` mapping are staged first and only become active after promote.",
            ),
        ],
        option_lines,
        status_lines: vec![Line::from(format!(
            "selected: {}",
            selected_direction_title.unwrap_or("none")
        ))],
        key_lines: vec![
            AkraTheme::key_line("Up/Down or j/k: move selection"),
            AkraTheme::key_line("Enter: continue    Backspace/Left: back    Esc/Ctrl+C: close"),
        ],
    }
}

/*
 * 학습 주석: confirm overlay는 selected direction을 실제 staged file mutation으로 넘기기 전 마지막
 * checkpoint다. 여기서 Yes/No만 고르게 만들어, accidental doc generation이 accepted directions catalog에
 * 직접 반영되지 않는 staged-edit workflow를 화면에서도 명확히 한다.
 */
pub(super) fn build_detail_doc_confirm_overlay_view(
    direction_title: &str,
    direction_id: &str,
    selected_choice: DetailDocConfirmChoice,
) -> DirectionsMaintenanceOverlayView {
    DirectionsMaintenanceOverlayView {
        header_lines: vec![
            directions_title_line(" / confirm detail doc"),
            Line::from(
                "Open a staged detail document for the selected direction and repair the mapping if needed?",
            ),
        ],
        // 학습 주석: 확인 문구는 사람이 보는 제목과 결정적 생성 경로를 함께 보여 주어 staged 변경을 추적하기 쉽게 한다.
        summary_lines: vec![
            Line::from(format!("direction: {direction_title}")),
            Line::from(format!(
                "default repair path: .codex-exec-loop/planning/directions/{direction_id}.md"
            )),
        ],
        // 학습 주석: selected_choice는 controller state에서 온 값이라 키 이동과 render가 같은 선택 source를 본다.
        option_lines: vec![
            overlay_option_line(
                "1",
                "yes",
                "stage a markdown detail doc for creation or repair",
                selected_choice == DetailDocConfirmChoice::Yes,
                false,
            ),
            overlay_option_line(
                "2",
                "no",
                "return without changing accepted or staged support files",
                selected_choice == DetailDocConfirmChoice::No,
                false,
            ),
        ],
        status_lines: vec![Line::from("confirmation: generate a staged doc file now")],
        key_lines: vec![
            AkraTheme::key_line("Up/Down or j/k: change selection"),
            AkraTheme::key_line("Enter: act    Backspace/Left: back    Esc/Ctrl+C: close"),
        ],
    }
}

/*
 * 학습 주석: manual editor overlay는 directions draft editor가 화면을 장악할 때 shell overlay frame에
 * 남기는 최소 copy다. 실제 editing/rendering은 dedicated editor view가 맡고, 이 함수는 방향성 문구와
 * escape key만 제공해 presentation 책임을 분리한다.
 */
pub(super) fn build_manual_editor_overlay_view() -> DirectionsMaintenanceOverlayView {
    DirectionsMaintenanceOverlayView {
        header_lines: vec![
            directions_title_line(" / staged editor"),
            Line::from("Edit the staged directions draft and save to re-run validation."),
        ],
        summary_lines: vec![Line::from(
            "This state renders through the dedicated draft editor view.",
        )],
        option_lines: vec![Line::from(
            "Use Tab to switch files and Ctrl+S to save + validate.",
        )],
        status_lines: vec![Line::from("editor ready")],
        key_lines: vec![AkraTheme::key_line("Esc/Ctrl+C: close")],
    }
}
