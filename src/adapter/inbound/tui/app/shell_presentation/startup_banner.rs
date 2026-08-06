use super::{AkraTheme, Line, Span};

// The welcome surface is terminal data, not an image asset. Keeping this projection static means
// the first frame is available before diagnostics, file IO, or app-server attachment complete.
const STARTUP_LEDGER_RULE: &str = "  +----------------------------------------------------------";

// Build the C2 startup treatment: a compact identity line and an honest orientation ledger.
// These rows describe where work appears after the first task; they intentionally do not invent
// readiness, delivery, or worker state before the runtime has produced it.
pub(in super::super) fn startup_operator_ledger_lines(
    max_height: Option<u16>,
) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(vec![
            Span::styled("  [#] ", AkraTheme::brand()),
            Span::styled("AKRA", AkraTheme::title()),
            Span::styled(" / operator ledger", AkraTheme::subtle()),
        ]),
        Line::styled(STARTUP_LEDGER_RULE, AkraTheme::subtle()),
        Line::default(),
        Line::styled("  OPERATOR LEDGER", AkraTheme::accent()),
        ledger_row(
            "[>]",
            AkraTheme::brand(),
            " task intake",
            "  compose in the fixed editor below",
        ),
        ledger_row(
            "[ ]",
            AkraTheme::muted(),
            " activity trace",
            "  starts with the first turn",
        ),
        ledger_row(
            "[ ]",
            AkraTheme::muted(),
            " delivery lane",
            "  activates for parallel work",
        ),
    ];

    if let Some(max_height) = max_height {
        lines.truncate(usize::from(max_height));
    }
    lines
}

fn ledger_row(
    marker: &'static str,
    marker_style: ratatui::style::Style,
    label: &'static str,
    detail: &'static str,
) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(marker, marker_style),
        Span::styled(label, AkraTheme::muted()),
        Span::styled(detail, AkraTheme::subtle()),
    ])
}

#[cfg(test)]
mod tests {
    use super::{AkraTheme, startup_operator_ledger_lines};

    #[test]
    fn startup_operator_ledger_is_compact_and_terminal_safe() {
        let rendered = startup_operator_ledger_lines(None);

        assert_eq!(rendered.len(), 7);
        assert_eq!(rendered[0].to_string(), "  [#] AKRA / operator ledger");
        assert!(rendered[1].to_string().starts_with("  +---"));
        assert!(rendered.iter().all(|line| line.to_string().is_ascii()));
        assert_eq!(rendered[0].spans[0].style, AkraTheme::brand());
        assert_eq!(rendered[0].spans[1].style, AkraTheme::title());
        assert_eq!(rendered[4].spans[1].style, AkraTheme::brand());
    }

    #[test]
    fn startup_operator_ledger_truncates_from_the_bottom_on_short_viewports() {
        let rendered = startup_operator_ledger_lines(Some(3));

        assert_eq!(rendered.len(), 3);
        assert_eq!(rendered[0].to_string(), "  [#] AKRA / operator ledger");
    }
}
