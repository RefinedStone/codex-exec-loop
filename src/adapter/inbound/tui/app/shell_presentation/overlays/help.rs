// 학습 주석: help overlay는 domain data가 아니라 shell presentation data입니다. theme helper와 Line type,
// 그리고 InlineShellCommand의 command catalog만 가져와 renderer-ready copy를 만듭니다.
use super::super::{AkraTheme, InlineShellCommand, Line};

// 학습 주석: HelpOverlayView는 inline help inspection renderer가 필요한 줄 묶음을 미리 나눈 DTO입니다.
// builder가 section을 나눠 주기 때문에 renderer는 layout 배치에만 집중하고 command catalog를 직접 알 필요가 없습니다.
pub(crate) struct HelpOverlayView {
    // 학습 주석: header_lines는 overlay 제목과 짧은 설명입니다. command list와 분리해 상단 chrome에 배치됩니다.
    pub(crate) header_lines: Vec<Line<'static>>,
    // 학습 주석: command_lines는 InlineShellCommand registry에서 파생된 실제 도움말 rows입니다.
    pub(crate) command_lines: Vec<Line<'static>>,
    // 학습 주석: key_lines는 overlay 자체를 닫는 조작 안내입니다. command rows와 별도 footer로 렌더링됩니다.
    pub(crate) key_lines: Vec<Line<'static>>,
}

// 학습 주석: build_help_overlay_view는 inline command catalog를 help overlay의 view model로 변환합니다.
// shell_controller는 overlay state만 켜고, shell_rendering은 이 함수의 결과를 받아 화면에 배치합니다.
pub(crate) fn build_help_overlay_view() -> HelpOverlayView {
    // 학습 주석: help_entries는 command enum의 단일 source of truth에서 usage/detail을 뽑아 옵니다.
    // 여기에 의존하면 help overlay와 autocomplete/suggestion copy가 서로 다른 목록으로 갈라지지 않습니다.
    let entries = InlineShellCommand::help_entries();
    // 학습 주석: usage_width는 왼쪽 command usage column의 폭입니다. 가장 긴 usage에 2칸 padding을 더해
    // detail 문장이 줄마다 같은 위치에서 시작하게 합니다.
    let usage_width = entries
        // 학습 주석: command entry들을 빌려 순회하므로 아래에서 entries를 다시 into_iter로 소비할 수 있습니다.
        .iter()
        // 학습 주석: 각 command의 renderable usage 문자열 길이만 비교합니다.
        .map(|entry| entry.usage.len())
        // 학습 주석: 가장 긴 usage를 기준으로 column width를 정합니다.
        .max()
        // 학습 주석: command catalog가 비어도 panic하지 않고 0폭으로 fallback합니다.
        .unwrap_or(0)
        // 학습 주석: saturating_add는 극단적인 usize overflow 대신 최대값에 머물게 하는 defensive padding입니다.
        .saturating_add(2);

    // 학습 주석: 여기서 반환하는 Line들은 모두 'static owned text입니다. renderer가 borrow lifetime을 신경 쓰지 않고
    // popup paragraph/list에 넣을 수 있습니다.
    HelpOverlayView {
        // 학습 주석: header는 theme title helper를 써서 다른 overlay title과 같은 스타일 contract를 공유합니다.
        header_lines: vec![
            // 학습 주석: subtitle의 "inline inspection"은 이 help가 full-screen route가 아니라 prompt 위 overlay임을 말합니다.
            AkraTheme::title_line("Shell Command Help", " / inline inspection"),
            // 학습 주석: 사용자는 별도 command palette가 아니라 prompt에 직접 `:command`를 입력합니다.
            Line::from("Commands are typed directly into the prompt."),
        ],
        // 학습 주석: command section은 catalog order를 보존합니다. InlineShellCommand 쪽 순서가 help 화면의 정보 구조입니다.
        command_lines: entries
            // 학습 주석: entries를 소비해 각 help entry를 owned Line으로 변환합니다.
            .into_iter()
            // 학습 주석: usage column은 left align하고, detail은 계산된 폭 뒤에 이어 붙입니다.
            .map(|entry| {
                // 학습 주석: format의 `<width$`가 command usage를 column width만큼 채워 help rows를 세로로 정렬합니다.
                Line::from(format!(
                    "{:<width$}{}",
                    entry.usage,
                    entry.detail,
                    width = usage_width
                ))
            })
            // 학습 주석: renderer가 Vec<Line>을 그대로 paragraph/list로 넘길 수 있게 수집합니다.
            .collect(),
        // 학습 주석: footer에는 overlay close shortcut만 둡니다. command 실행법은 header와 command rows가 담당합니다.
        key_lines: vec![AkraTheme::key_line("Esc/Ctrl+C: close")],
    }
}
