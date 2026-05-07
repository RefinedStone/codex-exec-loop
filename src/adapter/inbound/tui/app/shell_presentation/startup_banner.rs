use super::Line;

// Startup banner art is intentionally compile-time data: the first shell frame can be built before diagnostics,
// file IO, or app-server attachment complete, and every frontend receives the same terminal-safe glyph grid.
const STARTUP_ASCII_ART_DEFAULT: &str = r#"
 █████╗ ██╗  ██╗██████╗  █████╗
██╔══██╗██║ ██╔╝██╔══██╗██╔══██╗
███████║█████╔╝ ██████╔╝███████║
██╔══██║██╔═██╗ ██╔══██╗██╔══██║
██║  ██║██║  ██╗██║  ██║██║  ██║
╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝
"#;

// Convert the raw AKRA logo into ratatui lines and apply the only geometry policy owned by this module.
// Callers decide whether a startup banner is active; this function only normalizes and crops the art.
pub(in super::super) fn startup_ascii_art_lines(max_height: Option<u16>) -> Vec<Line<'static>> {
    let art_lines_vec = STARTUP_ASCII_ART_DEFAULT.lines().collect::<Vec<_>>();
    // The raw string is formatted for source readability; trim only the outer decorative blank rows.
    let start = art_lines_vec
        .iter()
        .position(|line| !line.trim().is_empty())
        .unwrap_or(0);
    let end = art_lines_vec
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map(|index| index + 1)
        .unwrap_or(art_lines_vec.len());
    let mut art_lines = &art_lines_vec[start..end];

    if let Some(max_height) = max_height {
        let max_height = max_height as usize;
        if max_height > 0 && art_lines.len() > max_height {
            // Center cropping preserves the logo's visual weight when the inline terminal has fewer rows than the full mark.
            let start = art_lines.len().saturating_sub(max_height) / 2;
            art_lines = &art_lines[start..start + max_height];
        }
    }

    // Borrowing from a static string keeps startup projection allocation-light apart from the Vec itself.
    art_lines.iter().map(|line| Line::from(*line)).collect()
}

#[cfg(test)]
mod tests {
    use super::startup_ascii_art_lines;

    // The startup frame appears before richer diagnostics, so the mark must stay plain text with predictable width.
    #[test]
    fn startup_ascii_art_uses_plain_terminal_safe_glyphs() {
        let rendered = startup_ascii_art_lines(None)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();

        assert_eq!(rendered.len(), 6);
        assert!(rendered.iter().any(|line| line.contains("██████")));
        // Keep ANSI escapes, emoji, and ambiguous-width glyphs out of the first paint path.
        assert!(rendered.iter().all(|line| line.chars().all(|ch| {
            matches!(ch, ' ' | '█' | '╗' | '╔' | '╝' | '╚' | '═' | '║')
        })));
    }
}
