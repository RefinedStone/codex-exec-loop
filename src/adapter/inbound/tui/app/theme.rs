use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders};

pub(super) struct AkraTheme;

impl AkraTheme {
    pub(super) const BRAND: Color = Color::Rgb(0, 229, 183);
    pub(super) const ACCENT: Color = Color::Rgb(91, 141, 239);

    pub(super) fn brand() -> Style {
        Style::default()
            .fg(Self::BRAND)
            .add_modifier(Modifier::BOLD)
    }

    pub(super) fn accent() -> Style {
        Style::default().fg(Self::ACCENT)
    }

    pub(super) fn title() -> Style {
        Self::brand()
    }

    pub(super) fn success() -> Style {
        Style::default().fg(Color::Green)
    }

    pub(super) fn warning() -> Style {
        Style::default().fg(Color::Yellow)
    }

    pub(super) fn danger() -> Style {
        Style::default().fg(Color::Red)
    }

    pub(super) fn muted() -> Style {
        Style::default().fg(Color::Gray)
    }

    pub(super) fn subtle() -> Style {
        Style::default().fg(Color::DarkGray)
    }

    pub(super) fn shortcut() -> Style {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    }

    pub(super) fn selected() -> Style {
        Style::default()
            .fg(Color::Black)
            .bg(Self::BRAND)
            .add_modifier(Modifier::BOLD)
    }

    pub(super) fn panel() -> Style {
        Self::accent()
    }

    pub(super) fn panel_block<'a, T>(title: T) -> Block<'a>
    where
        T: Into<Line<'a>>,
    {
        Block::default()
            .borders(Borders::ALL)
            .border_style(Self::panel())
            .title(title)
    }

    #[cfg(test)]
    pub(super) fn panel_inner(area: ratatui::layout::Rect) -> ratatui::layout::Rect {
        Block::default().borders(Borders::ALL).inner(area)
    }

    pub(super) fn title_line(text: &'static str, suffix: &'static str) -> Line<'static> {
        Line::from(vec![
            Span::styled("Akra", Self::brand()),
            Span::raw(" / "),
            Span::styled(text, Self::title()),
            Span::raw(suffix),
        ])
    }

    pub(super) fn key_line(text: impl Into<String>) -> Line<'static> {
        Line::styled(text.into(), Self::shortcut())
    }

    pub(super) fn selected_marker() -> &'static str {
        "> "
    }

    pub(super) fn idle_marker() -> &'static str {
        "  "
    }

    pub(super) fn list_highlight_symbol() -> &'static str {
        "> "
    }
}
