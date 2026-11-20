//! 이슈 템플릿과 게시 검증이 공유하는 ATX 제목 및 fenced code 경계이다.
#[derive(Default)]
pub(crate) struct IssueMarkdownHeadings {
    fence: Option<(u8, usize)>,
    comment: bool,
    mixed_comment_line: bool,
}

/// 제목이 아닌 줄도 코드 경계와 구별한다. 판정 목록/경로 파서는 코드 예제를 증거로 쓰지 않는다.
pub(crate) enum IssueMarkdownLine<'a> {
    Heading(usize, &'a str),
    Text(&'a str),
    Code,
}

impl IssueMarkdownHeadings {
    pub(crate) fn heading<'a>(&mut self, line: &'a str) -> Option<(usize, &'a str)> {
        match self.line(line) {
            IssueMarkdownLine::Heading(level, title) => Some((level, title)),
            IssueMarkdownLine::Text(_) | IssueMarkdownLine::Code => None,
        }
    }

    /// 주석 때문에 줄 전체를 제외했지만 주석 밖에 보이는 내용도 있었는지 알린다.
    /// 완료 기준은 그 줄을 세지 않으며, 경로 판정은 읽지 못한 내용을 안전한 빈 목록으로 취급하지 않는다.
    pub(crate) fn comment_excludes_visible_text(&self) -> bool {
        self.mixed_comment_line
    }

    pub(crate) fn line<'a>(&mut self, line: &'a str) -> IssueMarkdownLine<'a> {
        self.mixed_comment_line = false;
        if self.comment {
            self.scan_comments(line);
            return IssueMarkdownLine::Code;
        }
        let indent = line.bytes().take_while(|byte| *byte == b' ').count();
        let content = &line[indent..];
        let marker = content.as_bytes().first().copied();
        let fence = marker
            .filter(|marker| matches!(marker, b'\x60' | b'~'))
            .map(|marker| {
                let length = content.bytes().take_while(|byte| *byte == marker).count();
                (marker, length, &content[length..])
            });
        if let Some((opened, width)) = self.fence {
            if indent <= 3
                && let Some((marker, length, rest)) = fence
                && marker == opened
                && length >= width
                && rest.trim_matches([' ', '\t']).is_empty()
            {
                self.fence = None;
            }
            return IssueMarkdownLine::Code;
        }
        if indent > 3 || content.starts_with('\t') {
            return IssueMarkdownLine::Code;
        }
        if let Some((marker, length, rest)) = fence
            && length >= 3
            && (marker != b'\x60' || !rest.contains(char::from(0x60)))
        {
            self.fence = Some((marker, length));
            return IssueMarkdownLine::Code;
        }
        // 주석이 섞인 줄 전체를 보수적으로 제외한다. fence와 inline code의 문자는 주석을 열지 않는다.
        if self.scan_comments(line) {
            return IssueMarkdownLine::Code;
        }
        let level = content.bytes().take_while(|byte| *byte == b'#').count();
        let rest = &content[level..];
        if !(1..=6).contains(&level) || (!rest.is_empty() && !rest.starts_with([' ', '\t'])) {
            return IssueMarkdownLine::Text(line);
        }
        let title = rest.trim();
        let without_closing = title.trim_end_matches('#');
        let title = if without_closing.len() < title.len()
            && (without_closing.is_empty() || without_closing.ends_with([' ', '\t']))
        {
            without_closing.trim_end()
        } else {
            title
        };
        IssueMarkdownLine::Heading(level, title)
    }

    /// 같은 줄에서 주석이 닫히고 다시 열리는 모든 경계를 읽는다.
    fn scan_comments(&mut self, line: &str) -> bool {
        let bytes = line.as_bytes();
        let mut index = 0;
        let mut saw_comment = self.comment;
        let mut saw_visible_text = false;
        while index < bytes.len() {
            if self.comment {
                if bytes[index..].starts_with(b"-->") {
                    self.comment = false;
                    index += 3;
                } else {
                    index += 1;
                }
            } else if bytes[index..].starts_with(b"<!--") && !escaped(bytes, index) {
                self.comment = true;
                saw_comment = true;
                index += 4;
            } else {
                saw_visible_text |= !bytes[index].is_ascii_whitespace();
                if bytes[index] == b'\x60' && !escaped(bytes, index) {
                    let width = bytes[index..]
                        .iter()
                        .take_while(|byte| **byte == b'\x60')
                        .count();
                    let start = index + width;
                    // 닫히지 않은 span은 보이는 문자다. 뒤의 실제 주석 상태는 계속 읽는다.
                    index = inline_code_end(bytes, start, width).unwrap_or(start);
                } else {
                    index += 1;
                }
            }
        }
        self.mixed_comment_line = saw_comment && saw_visible_text;
        saw_comment
    }
}

/// inline code의 내용에서는 backslash와 다른 길이의 backtick을 문자 그대로 읽는다.
fn inline_code_end(bytes: &[u8], mut index: usize, width: usize) -> Option<usize> {
    while index < bytes.len() {
        if bytes[index] == b'\x60' {
            let run = bytes[index..]
                .iter()
                .take_while(|byte| **byte == b'\x60')
                .count();
            if run == width {
                return Some(index + run);
            }
            index += run;
        } else {
            index += 1;
        }
    }
    None
}

fn escaped(bytes: &[u8], index: usize) -> bool {
    bytes[..index]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
        % 2
        == 1
}

#[cfg(test)]
mod tests {
    use super::IssueMarkdownHeadings;

    #[test]
    fn comments_never_supply_hidden_headings_or_list_text() {
        let mut parser = IssueMarkdownHeadings::default();
        for line in ["<!--", "## Hidden", "- hidden criterion", "-->"] {
            assert!(matches!(parser.line(line), super::IssueMarkdownLine::Code));
        }
        assert_eq!(parser.heading("## Visible"), Some((2, "Visible")));
        assert!(matches!(
            parser.line("- text <!-- note -->"),
            super::IssueMarkdownLine::Code
        ));
        let mut parser = IssueMarkdownHeadings::default();
        for line in ["\x60\x60\x60", "<!--", "\x60\x60\x60"] {
            assert!(matches!(parser.line(line), super::IssueMarkdownLine::Code));
        }
        assert_eq!(parser.heading("## Visible"), Some((2, "Visible")));
    }

    #[test]
    fn every_comment_close_and_reopen_on_a_line_preserves_hidden_state() {
        for body in [
            "<!-- note --> <!--\n## Hidden\n- hidden criterion\n-->\n## Visible",
            "<!--\n--> <!-- note --> <!--\n## Hidden\n- hidden criterion\n-->\n## Visible",
            "<!-- note --> <!-- other --> <!--\n## Hidden\n-->\n## Visible",
        ] {
            let mut parser = IssueMarkdownHeadings::default();
            let headings: Vec<_> = body
                .lines()
                .filter_map(|line| parser.heading(line))
                .collect();
            assert_eq!(headings, [(2, "Visible")], "{body}");
        }
    }

    #[test]
    fn comment_only_and_mixed_lines_expose_different_path_uncertainty() {
        let mut parser = IssueMarkdownHeadings::default();
        for (line, mixed) in [
            ("<!-- note --> <!-- other -->", false),
            ("visible <!-- note -->", true),
            ("<!-- note --> visible", true),
            ("<!--", false),
            ("hidden \x60src/hidden.rs\x60", false),
            ("--> visible <!-- note -->", true),
            ("<!--", false),
            ("--> <!-- next -->", false),
        ] {
            assert!(matches!(parser.line(line), super::IssueMarkdownLine::Code));
            assert_eq!(parser.comment_excludes_visible_text(), mixed, "{line}");
        }
        assert_eq!(parser.heading("## Visible"), Some((2, "Visible")));
        assert!(!parser.comment_excludes_visible_text());
        for line in ["\x60\x60\x60", "visible <!-- note -->", "\x60\x60\x60"] {
            assert!(matches!(parser.line(line), super::IssueMarkdownLine::Code));
            assert!(!parser.comment_excludes_visible_text());
        }
    }

    #[test]
    fn inline_code_comment_markers_are_literal_and_do_not_hide_following_headings() {
        for line in [
            "literal \x60<!--\x60 text",
            "literal \x60<!-- note -->\x60 text",
            "literal \x60\x60<!-- \x60 note \x60 -->\x60\x60 text",
            "literal \x60-->\x60 text",
        ] {
            let mut parser = IssueMarkdownHeadings::default();
            assert!(matches!(
                parser.line(line),
                super::IssueMarkdownLine::Text(_)
            ));
            assert!(!parser.comment_excludes_visible_text(), "{line}");
            assert_eq!(parser.heading("## Visible"), Some((2, "Visible")));
        }
    }

    #[test]
    fn only_matching_long_enough_fences_end_code() {
        for body in [
            "\x60\x60\x60\x60markdown\n\x60\x60\x60\n## Hidden\n\x60\x60\x60\n\x60\x60\x60\x60\n## Visible",
            "\x60\x60\x60\n~~~\n## Hidden\n~~~\n\x60\x60\x60\n## Visible",
            "~~~\n\x60\x60\x60\n## Hidden\n~~~~\n## Visible",
        ] {
            let mut parser = IssueMarkdownHeadings::default();
            let headings: Vec<_> = body
                .lines()
                .filter_map(|line| parser.heading(line))
                .collect();
            assert_eq!(headings, [(2, "Visible")]);
        }
    }

    #[test]
    fn headings_preserve_literal_hashes_and_ignore_indented_code() {
        let mut parser = IssueMarkdownHeadings::default();
        assert_eq!(parser.heading("### C#"), Some((3, "C#")));
        assert_eq!(parser.heading("### C# ###"), Some((3, "C#")));
        assert_eq!(parser.heading("   ## Heading\t##"), Some((2, "Heading")));
        assert_eq!(parser.heading("    ## Code"), None);
        assert_eq!(parser.heading("\t## Code"), None);
        assert_eq!(parser.heading("#no-space"), None);
    }

    #[test]
    fn unclosed_fences_and_nonempty_closing_lines_keep_headings_hidden() {
        let mut parser = IssueMarkdownHeadings::default();
        for line in ["\x60\x60\x60", "\x60\x60\x60 trailing", "## Still code"] {
            assert_eq!(parser.heading(line), None);
        }
    }
}
