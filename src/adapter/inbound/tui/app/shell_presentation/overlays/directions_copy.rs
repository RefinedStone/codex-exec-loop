/*
 * directions_copy는 directions maintenance overlay의 wording boundary다. controller는
 * DirectionsMaintenanceOverlayStep과 summary facts를 고르고, projection은 selectable rows를 만든다.
 * 이 파일은 그 facts를 operator-facing 문장으로 바꿔 staged draft workflow, detail-doc repair,
 * queue-idle prompt maintenance가 화면마다 같은 설명을 쓰게 한다.
 */
use super::super::super::{AkraTheme, DetailDocConfirmChoice, Line};
use super::super::option_lines::overlay_option_line;
use super::DirectionsMaintenanceOverlayView;

// 모든 step이 같은 title prefix를 공유해 overview, selection, confirm, editor가 하나의 maintenance 흐름으로 읽힌다.
fn directions_title_line(suffix: &'static str) -> Line<'static> {
    AkraTheme::title_line("Directions Maintenance", suffix)
}

/*
 * Overview는 directions authority scan과 queue-idle prompt health를 한 화면에 압축한다. 이 copy는
 * accepted DB authority가 promote 전까지 바뀌지 않는다는 authoring contract를 반복해서 보여 주어,
 * operator가 raw file을 직접 고치기보다 staged draft/editor 흐름으로 들어가게 한다.
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
         * Summary는 이 overlay가 inspector이면서 editor 진입점임을 설명한다. staged draft를 만들 뿐
         * accepted state를 바로 바꾸지 않으므로 runtime prompt와 worker queue는 promote 전까지 기존
         * direction authority를 계속 본다.
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
         * Parse error가 있으면 direction 단위 repair target을 신뢰할 수 없다. 그래서 detail-doc
         * repair action은 막고, prompt editor/reload/manual repair 쪽으로 operator를 유도해 invalid
         * catalog를 기준으로 새 supporting file을 만들지 않게 한다.
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
        // Status는 service가 계산한 health snapshot을 그대로 노출해 doctor/admin 경로와 같은 문제 수치를 보여 준다.
        status_lines: vec![
            Line::from(format!(
                "directions: {total_direction_count} total / {missing_doc_count} missing docs / {broken_doc_count} broken docs"
            )),
            Line::from(format!(
                "queue-idle: policy {queue_idle_policy} / prompt {queue_idle_prompt_status} / {queue_idle_prompt}"
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
 * Detail-doc selection은 이미 projection이 고른 missing/broken mapping만 받아 화면 shell을 만든다.
 * copy layer가 목록을 다시 계산하지 않기 때문에 selection movement, disabled action 판단, render rows가
 * 같은 controller state를 source of truth로 유지한다.
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
         * Generated path copy는 supporting_files helper와 같은 convention을 사용자에게 드러낸다.
         * file 생성과 catalog mapping 갱신이 함께 staged 된다는 점을 말해, runtime prompt가 promote
         * 후에만 새 detail doc을 참조한다는 경계를 유지한다.
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
 * Confirm overlay는 selected direction snapshot을 staged file mutation으로 넘기기 전 마지막 checkpoint다.
 * 여기서 Yes/No만 남기면 accidental doc generation이 accepted catalog에 직접 반영되지 않고,
 * controller가 pending direction id와 choice를 같은 상태에서 읽는 staged-edit workflow가 분명해진다.
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
        // 확인 문구는 사람이 보는 제목과 deterministic repair path를 함께 보여 staged 변경 추적을 쉽게 한다.
        summary_lines: vec![
            Line::from(format!("direction: {direction_title}")),
            Line::from(format!(
                "default repair path: .codex-exec-loop/planning/directions/{direction_id}.md"
            )),
        ],
        // selected_choice는 controller state에서 온 값이라 키 이동, highlight, Enter behavior가 같은 source를 본다.
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
 * Manual editor step은 dedicated draft editor가 실제 buffer rendering과 save/validate 상태를 맡는다.
 * 이 fallback view는 shell overlay contract를 채우는 최소 copy만 제공해, directions maintenance router가
 * step마다 같은 DirectionsMaintenanceOverlayView shape를 유지하게 한다.
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
