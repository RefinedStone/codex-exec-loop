// 학습 주석: selection view는 planning init overlay UI state가 들고 있는 enum selection을 화면 copy로
// 변환합니다. AkraTheme/Line은 TUI text 출력이고, selection enum들은 controller state machine의 현재 위치입니다.
use super::super::super::super::super::{
    AkraTheme, Line, PlanningInitDetailSelection, PlanningInitModeSelection,
};
// 학습 주석: overlay_option_line은 선택 가능/비활성 option row의 공통 스타일 helper입니다. 이 파일은
// option semantics만 결정하고 marker/style 조립은 공통 helper에 맡깁니다.
use super::super::super::super::option_lines::overlay_option_line;
// 학습 주석: PlanningInitOverlayView는 popup renderer가 읽는 최종 DTO입니다. mode selection과 detail
// selection 모두 같은 header/summary/options/status/key 구조로 내려갑니다.
use super::super::super::PlanningInitOverlayView;
// 학습 주석: planning_setup_title_line은 planning setup popup 전체의 title styling을 통일합니다.
use super::super::copy::planning_setup_title_line;

// 학습 주석: build_mode_selection_overlay_view는 planning init 첫 단계인 "simple vs detail" 선택 화면을 만듭니다.
// controller는 selected_mode만 갱신하고, 이 함수는 그 값을 selected marker/status copy/key guidance로 반영합니다.
pub(super) fn build_mode_selection_overlay_view(
    // 학습 주석: selected_mode는 PlanningInitOverlayUiState의 현재 mode cursor입니다. A/B, arrow input은
    // controller에서 이 enum을 바꾸고 renderer는 여기서 selected row를 다시 계산합니다.
    selected_mode: PlanningInitModeSelection,
) -> PlanningInitOverlayView {
    // 학습 주석: 이 DTO는 아직 planning files를 stage하지 않는 inspection 단계의 화면입니다. 모든 copy는
    // "어느 authoring path로 들어갈지"를 설명하고, 실제 draft 생성은 Enter 처리 후 controller가 수행합니다.
    PlanningInitOverlayView {
        // 학습 주석: header는 planning setup popup의 위치와 현재 step 목적을 알려 줍니다.
        header_lines: vec![
            planning_setup_title_line(" / operator inspection"),
            // 학습 주석: 아직 파일 stage 전이라는 점을 명시해 사용자가 이 선택을 안전한 preview 단계로 이해하게 합니다.
            Line::from("Pick the planning entry path before any files are staged."),
        ],
        // 학습 주석: summary는 두 mode의 공통 invariant를 설명합니다. 어떤 mode든 accepted planning state를
        // 직접 바꾸기 전에 promotable draft를 먼저 만든다는 점이 핵심입니다.
        summary_lines: vec![
            Line::from(
                "Every guided path stages a promotable draft before active planning changes.",
            ),
            // 학습 주석: simple/detail 차이를 UX 수준에서 요약합니다. 자세한 file scaffold는 다음 단계나 editor에서 드러납니다.
            Line::from(
                "Simple mode keeps one generic active direction; detail mode prepares richer direction authoring.",
            ),
        ],
        // 학습 주석: option_lines는 keyboard selection state를 시각적으로 드러내는 영역입니다. disabled flag는
        // 둘 다 false라 이 단계에서는 simple/detail 모두 선택 가능합니다.
        option_lines: vec![
            // 학습 주석: simple mode는 low-ceremony path입니다. 선택되면 controller가 simple draft staging으로 바로 이어갑니다.
            overlay_option_line(
                "A",
                "simple mode",
                "stage one generic direction and an empty task ledger",
                selected_mode == PlanningInitModeSelection::Simple,
                false,
            ),
            // 학습 주석: detail mode는 manual/llm-assisted detail selection 단계로 한 번 더 들어가는 path입니다.
            overlay_option_line(
                "B",
                "detail mode",
                "branch into manual or future llm-assisted authoring",
                selected_mode == PlanningInitModeSelection::Detail,
                false,
            ),
        ],
        // 학습 주석: status는 현재 selected enum을 사람이 읽는 label로 다시 말해 줍니다. option row와
        // 별도로 status area에 같은 정보를 두어 좁은 화면에서도 current selection이 남습니다.
        status_lines: vec![
            Line::from(format!(
                "current selection: {}",
                // 학습 주석: enum variant를 route label로 바꿉니다. 이 label은 controller branch 이름과 일치해야 합니다.
                match selected_mode {
                    PlanningInitModeSelection::Simple => "simple mode",
                    PlanningInitModeSelection::Detail => "detail mode",
                }
            )),
            // 학습 주석: 현재 권장 경로를 명시합니다. simple mode가 기본값인 이유를 status area에서 한 번 더 설명합니다.
            Line::from("simple mode is the low-ceremony path for planning-aware execution."),
        ],
        // 학습 주석: key_lines는 controller의 ModeSelection key map과 맞아야 합니다. A/B/arrow는 selection
        // 이동이고 Enter는 선택 branch 실행, Esc/Ctrl+C는 overlay cancel입니다.
        key_lines: vec![
            AkraTheme::key_line("A/B or arrows move selection."),
            AkraTheme::key_line("Enter continues. Esc/Ctrl+C cancels."),
        ],
    }
}

// 학습 주석: build_detail_selection_overlay_view는 detail mode 안에서 manual editor와 future LLM-assisted
// authoring path를 고르는 화면을 만듭니다. 현재는 manual만 실제로 진행 가능하고 LLM-assisted는 disabled row입니다.
pub(super) fn build_detail_selection_overlay_view(
    // 학습 주석: selected_detail은 detail-selection step의 cursor입니다. disabled option도 cursor로 볼 수
    // 있지만 Enter 처리에서 controller가 unsupported notice를 보여 줍니다.
    selected_detail: PlanningInitDetailSelection,
) -> PlanningInitOverlayView {
    // 학습 주석: detail selection은 mode selection 이후의 두 번째 inspection 화면입니다. 여기서도 아직
    // accepted planning state는 바뀌지 않고, manual 선택 시 staged draft editor로 넘어갑니다.
    PlanningInitOverlayView {
        // 학습 주석: header는 현재 step이 detail-mode draft preparation 방식 선택임을 알려 줍니다.
        header_lines: vec![
            planning_setup_title_line(" / operator inspection"),
            Line::from("Current step: choose how detail-mode drafts should be prepared."),
        ],
        // 학습 주석: summary는 supported path와 visible-but-disabled target UX를 나란히 설명합니다. 사용자가
        // 왜 LLM-assisted option이 보이지만 진행되지 않는지 알 수 있게 합니다.
        summary_lines: vec![
            Line::from("Manual opens the staged draft editor inside the shell."),
            Line::from("LLM-assisted remains visible for the target UX but is still disabled."),
        ],
        // 학습 주석: option_lines는 current cursor와 disabled state를 모두 표현합니다. B row는 선택 표시가
        // 가능하더라도 disabled=true라 visual language가 "아직 진행 불가"를 드러냅니다.
        option_lines: vec![
            // 학습 주석: manual은 현재 지원되는 detail authoring path입니다. Enter는 embedded draft editor를 엽니다.
            overlay_option_line(
                "A",
                "manual",
                "stage the detail scaffold and keep editing inside the shell",
                selected_detail == PlanningInitDetailSelection::Manual,
                false,
            ),
            // 학습 주석: LLM-assisted는 roadmap/target UX를 숨기지 않기 위해 row는 유지하지만 disabled로 표시합니다.
            overlay_option_line(
                "B",
                "llm-assisted",
                "future guided drafting flow (not supported yet)",
                selected_detail == PlanningInitDetailSelection::LlmAssisted,
                true,
            ),
        ],
        // 학습 주석: status는 cursor가 disabled option 위에 있을 때도 명확한 label을 보여 줍니다. controller의
        // unsupported notice와 함께 사용자가 막힌 이유를 이해하게 합니다.
        status_lines: vec![
            Line::from(format!(
                "current selection: {}",
                // 학습 주석: disabled 상태는 label에도 포함해 option row를 놓쳐도 unsupported 상태가 보이게 합니다.
                match selected_detail {
                    PlanningInitDetailSelection::Manual => "manual",
                    PlanningInitDetailSelection::LlmAssisted => "llm-assisted (disabled)",
                }
            )),
            // 학습 주석: supported happy path를 구체적으로 알려 줍니다. Enter on manual은 planning manual editor
            // session을 열고 이후 save/promote flow로 이어집니다.
            Line::from("Enter on manual opens the embedded draft editor."),
        ],
        // 학습 주석: key_lines는 DetailSelection controller key map을 반영합니다. Backspace/Left는 mode
        // selection으로 되돌아가고, Enter는 selected detail branch를 실행합니다.
        key_lines: vec![
            AkraTheme::key_line("A/B or arrows move selection."),
            AkraTheme::key_line("Backspace/Left goes back. Enter continues. Esc/Ctrl+C cancels."),
        ],
    }
}
