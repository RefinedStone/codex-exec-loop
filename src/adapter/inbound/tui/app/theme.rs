// 학습 주석: TUI theme는 ratatui의 Style/Color/Modifier를 직접 반환합니다. presentation builder가
// 각자 색을 고르지 않고 여기의 의미 토큰을 호출하게 해 popup, status panel, list 선택 표시가 일관됩니다.
use ratatui::style::{Color, Modifier, Style};
// 학습 주석: title/key line helper는 styled text 조각을 조립하므로 Line/Span을 함께 씁니다.
use ratatui::text::{Line, Span};
// 학습 주석: panel chrome은 ratatui Block/Borders를 반환해 shell_rendering 쪽 모든 panel frame이
// 같은 border style과 title 처리 규칙을 공유하게 합니다.
use ratatui::widgets::{Block, Borders};

// 학습 주석: `AkraTheme`는 상태를 갖지 않는 namespace struct입니다. 인스턴스를 만들 필요 없이
// `AkraTheme::warning()`처럼 의미 기반 style token을 호출하도록 의도를 드러냅니다.
pub(super) struct AkraTheme;

// 학습 주석: 이 impl은 TUI adapter의 작은 design system입니다. domain/application 상태를 알지 않고,
// renderer가 "성공", "위험", "선택됨", "panel" 같은 표현 의미를 색/수식어로 낮추는 역할만 합니다.
impl AkraTheme {
    // 학습 주석: BRAND는 Akra 이름, primary selection background, title accent에 쓰는 대표 색입니다.
    // selected foreground가 black인 이유는 이 밝은 cyan-green 배경 위에서 대비를 확보하기 위해서입니다.
    pub(super) const BRAND: Color = Color::Rgb(0, 229, 183);
    // 학습 주석: ACCENT는 panel border와 secondary emphasis에 쓰는 파란 계열 보조 색입니다. BRAND와
    // 역할을 나눠 frame chrome과 active selection이 같은 색으로 과포화되지 않게 합니다.
    pub(super) const ACCENT: Color = Color::Rgb(91, 141, 239);

    // 학습 주석: brand style은 제품명과 선택 highlight처럼 가장 강한 identity 신호에 씁니다.
    pub(super) fn brand() -> Style {
        Style::default()
            .fg(Self::BRAND)
            .add_modifier(Modifier::BOLD)
    }

    // 학습 주석: accent는 brand보다 낮은 강도의 구조 강조입니다. panel border와 보조 label에서 사용됩니다.
    pub(super) fn accent() -> Style {
        Style::default().fg(Self::ACCENT)
    }

    // 학습 주석: title은 현재 brand와 같은 표현을 씁니다. 별도 함수로 둔 이유는 나중에 title hierarchy를
    // 바꾸더라도 호출부가 `brand` 의미와 `title` 의미를 섞어 쓰지 않게 하기 위해서입니다.
    pub(super) fn title() -> Style {
        Self::brand()
    }

    // 학습 주석: success/warning/danger는 planner worker status, validation, runtime notices 같은
    // 상태성 메시지에 쓰는 신호 색입니다. 색 선택을 호출부에 흩뜨리지 않아 semantic state와 rendering을 분리합니다.
    pub(super) fn success() -> Style {
        Style::default().fg(Color::Green)
    }

    pub(super) fn warning() -> Style {
        Style::default().fg(Color::Yellow)
    }

    pub(super) fn danger() -> Style {
        Style::default().fg(Color::Red)
    }

    // 학습 주석: muted/subtle은 부가 설명, placeholder, inactive 상태를 구분합니다. muted는 읽을 수 있는
    // 보조 텍스트, subtle은 더 뒤로 물러난 비활성/힌트에 가까운 톤입니다.
    pub(super) fn muted() -> Style {
        Style::default().fg(Color::Gray)
    }

    pub(super) fn subtle() -> Style {
        Style::default().fg(Color::DarkGray)
    }

    // 학습 주석: shortcut은 key help line에 쓰는 스타일입니다. 노란색+bold 조합으로 본문 상태 텍스트와
    // 키 조작 안내를 빠르게 구분하게 합니다.
    pub(super) fn shortcut() -> Style {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    }

    // 학습 주석: tool style은 live tool activity나 worker/system activity처럼 user prose와 다른
    // machine action을 표시할 때 쓰는 분리 색입니다.
    pub(super) fn tool() -> Style {
        Style::default().fg(Color::Magenta)
    }

    // 학습 주석: selected는 lists/tables에서 현재 focus row를 표시하는 강한 inverse token입니다.
    // foreground/background를 함께 지정해야 terminal theme와 무관하게 선택된 행이 안정적으로 보입니다.
    pub(super) fn selected() -> Style {
        Style::default()
            .fg(Color::Black)
            .bg(Self::BRAND)
            .add_modifier(Modifier::BOLD)
    }

    // 학습 주석: panel은 bordered block chrome의 색상 토큰입니다. panel title content는 caller가 넘기고,
    // border tone은 theme가 고정합니다.
    pub(super) fn panel() -> Style {
        Self::accent()
    }

    // 학습 주석: shell_rendering의 대부분 panel은 이 helper를 통해 Block을 만듭니다. borders, border style,
    // title 연결을 한곳에 두면 planning/session/parallel overlays가 같은 frame grammar를 공유합니다.
    pub(super) fn panel_block<'a, T>(title: T) -> Block<'a>
    where
        // 학습 주석: title은 raw &str일 수도 있고 styled Line일 수도 있습니다. Into<Line>을 받으면
        // `title_line`처럼 복합 Span title도 같은 helper로 처리됩니다.
        T: Into<Line<'a>>,
    {
        Block::default()
            .borders(Borders::ALL)
            .border_style(Self::panel())
            .title(title)
    }

    #[cfg(test)]
    // 학습 주석: 테스트는 실제 render frame과 같은 border thickness를 계산해야 합니다. production에서
    // 노출하지 않는 inner-area helper를 cfg(test)로 열어 snapshot/geometry 테스트가 theme contract를 공유하게 합니다.
    pub(super) fn panel_inner(area: ratatui::layout::Rect) -> ratatui::layout::Rect {
        Block::default().borders(Borders::ALL).inner(area)
    }

    // 학습 주석: main shell title은 `Akra / <section><suffix>` 형식을 공유합니다. 브랜드 span, separator,
    // section title을 helper로 묶어 popup frame이 section별로 같은 heading grammar를 사용하게 합니다.
    pub(super) fn title_line(text: &'static str, suffix: &'static str) -> Line<'static> {
        Line::from(vec![
            Span::styled("Akra", Self::brand()),
            Span::raw(" / "),
            Span::styled(text, Self::title()),
            Span::raw(suffix),
        ])
    }

    // 학습 주석: key line은 caller가 만든 조작 안내 문자열을 shortcut style 한 줄로 감쌉니다. String-like
    // 입력을 받아 동적 copy와 static copy를 같은 API로 처리합니다.
    pub(super) fn key_line(text: impl Into<String>) -> Line<'static> {
        Line::styled(text.into(), Self::shortcut())
    }

    // 학습 주석: selected/idle marker는 text list 앞의 고정 너비 prefix입니다. 두 함수가 같은 폭을
    // 반환해야 list row text가 선택 여부에 따라 흔들리지 않습니다.
    pub(super) fn selected_marker() -> &'static str {
        "> "
    }

    pub(super) fn idle_marker() -> &'static str {
        "  "
    }

    // 학습 주석: ratatui List의 highlight symbol도 selected marker와 같은 glyph를 씁니다. 수동 prefix와
    // widget-level highlight가 서로 다른 모양이 되지 않도록 별도 helper로 노출합니다.
    pub(super) fn list_highlight_symbol() -> &'static str {
        "> "
    }
}
