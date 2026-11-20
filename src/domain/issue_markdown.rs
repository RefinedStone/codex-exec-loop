//! 이슈 템플릿과 게시 검증이 공유하는 ATX 제목 및 fenced code 경계이다.
#[derive(Default)]
pub(crate) struct IssueMarkdownHeadings {
    fence: Option<(u8, usize)>,
}

impl IssueMarkdownHeadings {
    pub(crate) fn heading<'a>(&mut self, line: &'a str) -> Option<(usize, &'a str)> {
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
            return None;
        }
        if indent > 3 || content.starts_with('\t') {
            return None;
        }
        if let Some((marker, length, rest)) = fence
            && length >= 3
            && (marker != b'\x60' || !rest.contains(char::from(0x60)))
        {
            self.fence = Some((marker, length));
            return None;
        }
        let level = content.bytes().take_while(|byte| *byte == b'#').count();
        let rest = &content[level..];
        if !(1..=6).contains(&level) || (!rest.is_empty() && !rest.starts_with([' ', '\t'])) {
            return None;
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
        Some((level, title))
    }
}

#[cfg(test)]
mod tests {
    use super::IssueMarkdownHeadings;

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
