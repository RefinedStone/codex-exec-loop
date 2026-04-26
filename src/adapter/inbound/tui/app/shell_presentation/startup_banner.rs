use super::Line;
#[cfg(test)]
use super::ShellCorePresentationContext;

const STARTUP_ASCII_ART_DEFAULT: &str = r#"
 █████╗ ██╗  ██╗██████╗  █████╗
██╔══██╗██║ ██╔╝██╔══██╗██╔══██╗
███████║█████╔╝ ██████╔╝███████║
██╔══██║██╔═██╗ ██╔══██╗██╔══██║
██║  ██║██║  ██╗██║  ██║██║  ██║
╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝
"#;

#[cfg(test)]
pub(super) fn build_startup_banner_lines_from_context(
    context: &ShellCorePresentationContext<'_>,
    max_height: Option<u16>,
) -> Option<Vec<Line<'static>>> {
    if !context.startup_banner_is_active() || max_height == Some(0) {
        return None;
    }

    Some(startup_ascii_art_lines(max_height))
}

pub(in super::super) fn startup_ascii_art_lines(max_height: Option<u16>) -> Vec<Line<'static>> {
    let art_lines_vec = STARTUP_ASCII_ART_DEFAULT.lines().collect::<Vec<_>>();
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
            let start = art_lines.len().saturating_sub(max_height) / 2;
            art_lines = &art_lines[start..start + max_height];
        }
    }

    art_lines.iter().map(|line| Line::from(*line)).collect()
}

#[cfg(test)]
mod tests {
    use super::startup_ascii_art_lines;

    #[test]
    fn startup_ascii_art_uses_plain_terminal_safe_glyphs() {
        let rendered = startup_ascii_art_lines(None)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();

        assert_eq!(rendered.len(), 6);
        assert!(rendered.iter().any(|line| line.contains("██████")));
        assert!(rendered.iter().all(|line| line.chars().all(|ch| {
            matches!(ch, ' ' | '█' | '╗' | '╔' | '╝' | '╚' | '═' | '║')
        })));
    }
}
