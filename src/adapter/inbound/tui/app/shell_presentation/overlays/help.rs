use super::super::{AkraTheme, InlineShellCommand, Line};

// help overlay는 domain/runtime projection이 아니라 shell command catalog의 read-only view다.
// section을 header, command list, key footer로 미리 나눠 inline inspection renderer가
// `InlineShellCommand` registry나 column alignment 규칙을 직접 알지 않게 한다.
pub(crate) struct HelpOverlayView {
    // header는 이 화면이 command 실행 surface가 아니라 prompt 위 inspection임을 알려 준다.
    pub(crate) header_lines: Vec<Line<'static>>,
    // command rows는 autocomplete/suggestion과 같은 registry에서 파생되어 help와 입력 힌트가 갈라지지 않는다.
    pub(crate) command_lines: Vec<Line<'static>>,
    // footer는 overlay lifecycle 조작만 담고, 개별 command 사용법은 command rows에 남긴다.
    pub(crate) key_lines: Vec<Line<'static>>,
}

// controller는 help overlay state만 켜고, 이 builder가 command enum의 단일 source of
// truth를 renderer-facing line DTO로 낮춘다. 새 command를 추가하면 command catalog만
// 갱신해도 help overlay가 같은 순서와 설명을 따라가야 한다.
pub(crate) fn build_help_overlay_view() -> HelpOverlayView {
    let entries = InlineShellCommand::help_entries();
    // usage column은 가장 긴 command usage를 기준으로 잡고 두 칸 padding을 더한다.
    // 이렇게 해야 `:task [prompt]`처럼 긴 command가 있어도 detail 문장이 같은 열에서 시작한다.
    let usage_width = entries
        .iter()
        .map(|entry| entry.usage.len())
        .max()
        .unwrap_or(0)
        .saturating_add(2);

    HelpOverlayView {
        header_lines: vec![
            AkraTheme::title_line("Shell Command Help", " / inline inspection"),
            Line::from("Commands are typed directly into the prompt."),
        ],
        // command section은 registry order를 보존한다. 그 순서가 help 화면의 정보 구조이자
        // completion list와 맞춰야 하는 operator mental model이다.
        command_lines: entries
            .into_iter()
            .map(|entry| {
                Line::from(format!(
                    "{:<width$}{}",
                    entry.usage,
                    entry.detail,
                    width = usage_width
                ))
            })
            .collect(),
        key_lines: vec![AkraTheme::key_line("Esc/Ctrl+C: close")],
    }
}
