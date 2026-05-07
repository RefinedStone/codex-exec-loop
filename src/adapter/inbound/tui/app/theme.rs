use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders};

// TUI semantic style token을 모아 둔 stateless namespace다.
// presentation builder는 직접 색을 고르지 않고 danger, selected, key-line 같은 의미를 요청한다.
pub(super) struct AkraTheme;

impl AkraTheme {
    // brand는 가장 강한 identity/selection signal이다. selected row는 이 배경 위로 invert되어 terminal palette와 무관한 contrast를 얻는다.
    pub(super) const BRAND: Color = Color::Rgb(0, 229, 183);
    // accent는 secondary chrome emphasis다. border와 auxiliary label을 brand/selection tier보다 낮은 시각 계층에 둔다.
    pub(super) const ACCENT: Color = Color::Rgb(91, 141, 239);

    // Akra label이나 active route marker처럼 강한 identity text에 쓰는 style이다.
    pub(super) fn brand() -> Style {
        Style::default()
            .fg(Self::BRAND)
            .add_modifier(Modifier::BOLD)
    }

    // selected row와 경쟁하면 안 되는 structural emphasis에는 accent만 적용한다.
    pub(super) fn accent() -> Style {
        Style::default().fg(Self::ACCENT)
    }

    // title은 현재 brand와 같지만 별도 함수다. heading hierarchy가 바뀌어도 caller의 의미 요청은 유지된다.
    pub(super) fn title() -> Style {
        Self::brand()
    }

    // outcome token은 validation, planner worker status, runtime notice, startup diagnostic이 함께 쓰는 상태 색 vocabulary다.
    pub(super) fn success() -> Style {
        Style::default().fg(Color::Green)
    }

    pub(super) fn warning() -> Style {
        Style::default().fg(Color::Yellow)
    }

    pub(super) fn danger() -> Style {
        Style::default().fg(Color::Red)
    }

    // muted는 secondary detail용으로 여전히 읽혀야 하고, subtle은 placeholder와 inactive hint처럼 더 낮은 강조에 쓴다.
    pub(super) fn muted() -> Style {
        Style::default().fg(Color::Gray)
    }

    pub(super) fn subtle() -> Style {
        Style::default().fg(Color::DarkGray)
    }

    // keyboard affordance는 별도 token을 써 command help가 status prose와 빠르게 구분되게 한다.
    pub(super) fn shortcut() -> Style {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    }

    // tool/machine activity는 user prose나 assistant text와 다른 source임을 색으로 분리한다.
    pub(super) fn tool() -> Style {
        Style::default().fg(Color::Magenta)
    }

    // selection은 fg/bg를 모두 지정한다. terminal 기본 palette가 focus row를 애매하게 만들지 못하게 한다.
    pub(super) fn selected() -> Style {
        Style::default()
            .fg(Color::Black)
            .bg(Self::BRAND)
            .add_modifier(Modifier::BOLD)
    }

    // panel chrome은 accent-only다. title content는 caller가 title_line이나 custom Line으로 별도 의미를 입힌다.
    pub(super) fn panel() -> Style {
        Self::accent()
    }

    // shell panel, popup, inline inspection surface가 공유하는 frame grammar다.
    pub(super) fn panel_block<'a, T>(title: T) -> Block<'a>
    where
        T: Into<Line<'a>>,
    {
        Block::default()
            .borders(Borders::ALL)
            .border_style(Self::panel())
            .title(title)
    }

    // shared heading grammar다. brand, section title, suffix가 surface마다 예측 가능한 span 순서로 배치된다.
    pub(super) fn title_line(text: &'static str, suffix: &'static str) -> Line<'static> {
        Line::from(vec![
            Span::styled("Akra", Self::brand()),
            Span::raw(" / "),
            Span::styled(text, Self::title()),
            Span::raw(suffix),
        ])
    }

    // static/dynamic command help를 표준 keyboard affordance style로 감싼다.
    pub(super) fn key_line(text: impl Into<String>) -> Line<'static> {
        Line::styled(text.into(), Self::shortcut())
    }

    // manual list prefix는 같은 width를 유지해야 selected row가 text column을 흔들지 않는다.
    pub(super) fn selected_marker() -> &'static str {
        "> "
    }

    pub(super) fn idle_marker() -> &'static str {
        "  "
    }

    // widget-level list highlight도 manual projection row와 같은 glyph를 써 focus vocabulary를 맞춘다.
    pub(super) fn list_highlight_symbol() -> &'static str {
        "> "
    }
}
