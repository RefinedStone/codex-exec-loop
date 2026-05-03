use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders};

// Stateless namespace for the TUI's semantic style tokens.
// Presentation builders ask for meaning such as danger, selected, or key-line instead of hard-coding colors.
pub(super) struct AkraTheme;

impl AkraTheme {
    // Brand owns the strongest identity/selection signal; selected rows invert onto this background for stable contrast.
    pub(super) const BRAND: Color = Color::Rgb(0, 229, 183);
    // Accent is secondary chrome emphasis, keeping borders and auxiliary labels below the brand/selection tier.
    pub(super) const ACCENT: Color = Color::Rgb(91, 141, 239);

    // Strong identity text such as Akra labels and active route markers.
    pub(super) fn brand() -> Style {
        Style::default()
            .fg(Self::BRAND)
            .add_modifier(Modifier::BOLD)
    }

    // Structural emphasis that should not compete with selected rows.
    pub(super) fn accent() -> Style {
        Style::default().fg(Self::ACCENT)
    }

    // Title is intentionally separate from brand so heading hierarchy can change without rewriting call sites.
    pub(super) fn title() -> Style {
        Self::brand()
    }

    // Outcome tokens are shared by validation, planner worker status, runtime notices, and startup diagnostics.
    pub(super) fn success() -> Style {
        Style::default().fg(Color::Green)
    }

    pub(super) fn warning() -> Style {
        Style::default().fg(Color::Yellow)
    }

    pub(super) fn danger() -> Style {
        Style::default().fg(Color::Red)
    }

    // Muted remains readable for secondary detail; subtle is for placeholders and inactive hints.
    pub(super) fn muted() -> Style {
        Style::default().fg(Color::Gray)
    }

    pub(super) fn subtle() -> Style {
        Style::default().fg(Color::DarkGray)
    }

    // Keyboard affordances use a distinct token so command help scans apart from status prose.
    pub(super) fn shortcut() -> Style {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    }

    // Tool/machine activity is colored separately from user prose and assistant text.
    pub(super) fn tool() -> Style {
        Style::default().fg(Color::Magenta)
    }

    // Selection is a complete fg/bg pair so terminal palette defaults cannot make focus rows ambiguous.
    pub(super) fn selected() -> Style {
        Style::default()
            .fg(Color::Black)
            .bg(Self::BRAND)
            .add_modifier(Modifier::BOLD)
    }

    // Panel chrome stays accent-only; title content is supplied by callers through title_line or custom Lines.
    pub(super) fn panel() -> Style {
        Self::accent()
    }

    // Common frame grammar for shell panels, popups, and inline inspection surfaces.
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
    // Geometry tests use the same border contract as production without exposing layout helpers at runtime.
    pub(super) fn panel_inner(area: ratatui::layout::Rect) -> ratatui::layout::Rect {
        Block::default().borders(Borders::ALL).inner(area)
    }

    // Shared heading grammar: brand, section title, and suffix occupy predictable spans across surfaces.
    pub(super) fn title_line(text: &'static str, suffix: &'static str) -> Line<'static> {
        Line::from(vec![
            Span::styled("Akra", Self::brand()),
            Span::raw(" / "),
            Span::styled(text, Self::title()),
            Span::raw(suffix),
        ])
    }

    // Wrap static or dynamic command help in the standard keyboard affordance style.
    pub(super) fn key_line(text: impl Into<String>) -> Line<'static> {
        Line::styled(text.into(), Self::shortcut())
    }

    // Manual list prefixes must stay equal width so selected rows do not shift text columns.
    pub(super) fn selected_marker() -> &'static str {
        "> "
    }

    pub(super) fn idle_marker() -> &'static str {
        "  "
    }

    // Widget-level list highlights use the same glyph as manual projection rows.
    pub(super) fn list_highlight_symbol() -> &'static str {
        "> "
    }
}
