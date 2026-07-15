use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const ELLIPSIS: &str = "…";

pub(super) fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

pub(super) fn pad_right_to_cells(text: &str, target_width: usize) -> String {
    let padding = target_width.saturating_sub(display_width(text));
    format!("{text}{}", " ".repeat(padding))
}

pub(super) fn truncate_end_to_cells(text: &str, max_width: usize) -> String {
    if display_width(text) <= max_width {
        return text.to_string();
    }
    if max_width == 0 {
        return String::new();
    }

    let ellipsis_width = display_width(ELLIPSIS);
    if ellipsis_width > max_width {
        return String::new();
    }

    let content_budget = max_width - ellipsis_width;
    let mut output = String::new();
    let mut output_width = 0usize;
    for grapheme in text.graphemes(true) {
        let grapheme_width = display_width(grapheme);
        if output_width.saturating_add(grapheme_width) > content_budget {
            break;
        }
        output.push_str(grapheme);
        output_width = output_width.saturating_add(grapheme_width);
    }
    output.push_str(ELLIPSIS);
    output
}

#[cfg(test)]
mod tests {
    use super::{display_width, pad_right_to_cells, truncate_end_to_cells};

    #[test]
    fn display_width_counts_cjk_terminal_cells() {
        assert_eq!(display_width("Akra"), 4);
        assert_eq!(display_width("한글"), 4);
    }

    #[test]
    fn truncation_preserves_graphemes_and_cell_budget() {
        assert_eq!(truncate_end_to_cells("abcdef", 5), "abcd…");
        assert_eq!(truncate_end_to_cells("한글제목", 5), "한글…");
        assert_eq!(truncate_end_to_cells("e\u{301}clair", 4), "e\u{301}cl…");
        assert_eq!(truncate_end_to_cells("👩‍💻 coding", 3), "👩‍💻…");
        assert!(display_width(&truncate_end_to_cells("긴한글제목", 6)) <= 6);
    }

    #[test]
    fn truncation_handles_empty_tiny_and_exact_budgets() {
        assert_eq!(truncate_end_to_cells("abc", 0), "");
        assert_eq!(truncate_end_to_cells("abc", 1), "…");
        assert_eq!(truncate_end_to_cells("abc", 3), "abc");
        assert_eq!(truncate_end_to_cells("한", 2), "한");
    }

    #[test]
    fn padding_uses_display_cells_instead_of_bytes() {
        assert_eq!(display_width(&pad_right_to_cells("한글", 8)), 8);
        assert_eq!(display_width(&pad_right_to_cells(":help", 8)), 8);
    }
}
