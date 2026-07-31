use super::*;
fn rendered_text(page: &BoundedDiffPage) -> String {
    page.lines
        .iter()
        .map(ToString::to_string)
        .map(|line| line.trim_end().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn unified_diff_renders_file_summary_and_source_line_gutter() {
    let page = build_bounded_diff_page(
        "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -98,3 +98,3 @@\n line 98\n-line 99\n+line 99 changed\n line 100\n",
        ProgressiveActivityPageCursor::at(0),
        80,
        12,
    );

    assert_eq!(
        rendered_text(&page),
        "• Edited src/lib.rs\n 98  line 98\n 99 -line 99\n 99 +line 99 changed\n100  line 100"
    );
    assert_eq!(page.lines[2].spans[1].style, AkraTheme::diff_deletion());
    assert_eq!(page.lines[3].spans[1].style, AkraTheme::diff_addition());
    assert_eq!(page.lines[2].width(), 80);
    assert_eq!(page.lines[3].width(), 80);
    assert_eq!(
        page.lines[2].spans.last().unwrap().style,
        AkraTheme::diff_deletion()
    );
    assert_eq!(
        page.lines[3].spans.last().unwrap().style,
        AkraTheme::diff_addition()
    );
    assert!(
        page.lines[1].width() < 80,
        "context rows should stay neutral"
    );
}

#[test]
fn multiple_hunks_use_vertical_ellipsis_and_reset_ranges() {
    let page = build_bounded_diff_page(
        "--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-old\n+new\n@@ -20,2 +30,2 @@ fn next\n context\n-old 21\n+new 31\n",
        ProgressiveActivityPageCursor::at(0),
        80,
        20,
    );
    let rendered = rendered_text(&page);

    assert!(rendered.contains(" 1 -old"));
    assert!(rendered.contains(" 1 +new"));
    assert!(rendered.contains(" ⋮"));
    assert!(rendered.contains("30  context"));
    assert!(rendered.contains("21 -old 21"));
    assert!(rendered.contains("31 +new 31"));
}

#[test]
fn file_like_source_lines_remain_hunk_content_and_paths_keep_spaces() {
    let page = build_bounded_diff_page(
        "--- a/file name.txt\n+++ b/file name.txt\n@@ -1 +1 @@\n--- old heading\n+++ new heading\n",
        ProgressiveActivityPageCursor::at(0),
        80,
        10,
    );

    assert_eq!(
        rendered_text(&page),
        "• Edited file name.txt\n1 --- old heading\n1 +++ new heading"
    );
}

#[test]
fn page_cursor_keeps_numbers_and_wrapped_continuations_exact() {
    let text = "--- a/a.txt\n+++ b/a.txt\n@@ -7 +7 @@\n+abcdefghijkl\n context\n";
    let first = build_bounded_diff_page(text, ProgressiveActivityPageCursor::at(0), 10, 2);
    let next = first
        .next_cursor
        .expect("wrapped diff should have a next page");
    let second = build_bounded_diff_page(text, next, 10, 3);
    let rendered = rendered_text(&second);

    assert!(
        rendered
            .lines()
            .next()
            .is_some_and(|line| line.starts_with("   "))
    );
    assert!(!rendered.lines().next().unwrap_or_default().contains("7 +"));
    assert!(rendered.contains("8  context"));
    assert!(first.scanned_bytes <= 10 * 2 * PAGE_SCAN_BYTES_PER_CELL);
    assert!(second.rendered_bytes <= 10 * 3 * PAGE_OUTPUT_BYTES_PER_CELL);
}

#[test]
fn long_hunk_header_keeps_ranges_across_bounded_pages() {
    let text = format!("@@ -1 +1 @@ {}\n-old\n+new\n", "scope".repeat(128));
    let mut cursor = ProgressiveActivityPageCursor::at(0);
    let mut rendered = String::new();

    for _ in 0..16 {
        let page = build_bounded_diff_page(&text, cursor, 16, 1);
        assert!(page.lines.iter().all(|line| line.width() <= 16));
        rendered.push_str(&rendered_text(&page));
        let Some(next) = page.next_cursor else {
            break;
        };
        cursor = next;
    }

    assert!(rendered.contains("1 -old"), "{rendered}");
    assert!(rendered.contains("1 +new"), "{rendered}");
}

#[test]
fn long_single_line_page_stays_within_scan_and_width_budgets() {
    let text = format!("@@ -1 +1 @@\n+{}", "x".repeat(2 * 1024 * 1024));
    let page = build_bounded_diff_page(&text, ProgressiveActivityPageCursor::at(0), 40, 3);

    assert_eq!(page.lines.len(), 3);
    assert!(page.lines.iter().all(|line| line.width() <= 40));
    assert!(page.next_cursor.is_some());
    assert!(page.scanned_bytes <= 40 * 3 * PAGE_SCAN_BYTES_PER_CELL);
    assert!(page.rendered_bytes <= 40 * 3 * PAGE_OUTPUT_BYTES_PER_CELL);
}

#[test]
fn zero_height_and_narrow_width_never_scan_or_overrun() {
    let text = "@@ -123 +456 @@\n-old\n+new\n";
    let header_only = build_bounded_diff_page(text, ProgressiveActivityPageCursor::at(0), 80, 0);
    assert!(header_only.current_cursor.diff.is_none());
    assert_eq!(header_only.scanned_bytes, 0);

    for width in [2, 4] {
        let narrow = build_bounded_diff_page(text, ProgressiveActivityPageCursor::at(0), width, 4);
        assert!(
            narrow
                .lines
                .iter()
                .all(|line| line.width() <= usize::from(width))
        );
    }
}

#[test]
fn controls_and_malformed_metadata_render_as_safe_text() {
    let page = build_bounded_diff_page(
        "Binary files a/x and b/x differ\u{1b}[31m\n@@@ combined header\n",
        ProgressiveActivityPageCursor::at(0),
        80,
        4,
    );
    let rendered = rendered_text(&page);

    assert!(rendered.contains("\\x1b[31m"));
    assert!(!rendered.contains('\u{1b}'));
    assert!(rendered.contains("@@@ combined header"));
}
