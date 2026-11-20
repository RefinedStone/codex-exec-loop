use crate::domain::issue_markdown::{IssueMarkdownHeadings, IssueMarkdownLine};

use super::{MAX_ISSUE_CRITERIA, MAX_ISSUE_CRITERION_BYTES};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CriteriaProblem {
    Missing,
    NotList,
    TooLarge,
    Unsupported,
}

/// 완료 기준의 존재와 목록 문법만 읽는다. 체크 상태는 승인·통과·실행 channel이 아니다.
/// 한 항목은 한 줄의 비어 있지 않은 bullet/number/check 텍스트여야 한다.
pub(super) fn count_criteria(body: &str) -> Result<usize, CriteriaProblem> {
    let mut markdown = IssueMarkdownHeadings::default();
    let mut section_level = None;
    let mut found = false;
    let mut count = 0;
    let mut saw_text = false;
    for line in body.lines() {
        match markdown.line(line) {
            IssueMarkdownLine::Heading(level, title) => {
                if title == "완료 기준" {
                    if found {
                        return Err(CriteriaProblem::Unsupported);
                    }
                    found = true;
                    section_level = Some(level);
                } else if section_level.is_some_and(|start| level <= start) {
                    section_level = None;
                }
            }
            IssueMarkdownLine::Code => {}
            IssueMarkdownLine::Text(line) if section_level.is_some() => {
                let text = line.trim();
                if text.is_empty() || text.starts_with('>') || horizontal_rule(text) {
                    continue;
                }
                saw_text = true;
                let Some(item) = list_item(text) else {
                    continue;
                };
                let item = checkbox_text(item)?;
                if item.is_empty() || item == "_No response_" {
                    return Err(CriteriaProblem::Missing);
                }
                if item.len() > MAX_ISSUE_CRITERION_BYTES {
                    return Err(CriteriaProblem::TooLarge);
                }
                if item.starts_with('<')
                    || item.contains("![")
                    || item.contains("[[akra-mask:")
                    || item.contains("[removed]")
                {
                    return Err(CriteriaProblem::Unsupported);
                }
                count += 1;
                if count > MAX_ISSUE_CRITERIA {
                    return Err(CriteriaProblem::TooLarge);
                }
            }
            IssueMarkdownLine::Text(_) => {}
        }
    }
    if count > 0 {
        Ok(count)
    } else if found && saw_text {
        Err(CriteriaProblem::NotList)
    } else {
        Err(CriteriaProblem::Missing)
    }
}

fn list_item(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    if matches!(bytes.first(), Some(b'-' | b'+' | b'*')) {
        return after_marker(text, 1);
    }
    let digits = bytes
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if !(1..=9).contains(&digits) || !matches!(bytes.get(digits), Some(b'.' | b')')) {
        return None;
    }
    after_marker(text, digits + 1)
}

fn after_marker(text: &str, width: usize) -> Option<&str> {
    let rest = &text[width..];
    if rest.is_empty() {
        return Some("");
    }
    rest.starts_with([' ', '\t']).then(|| rest.trim())
}

fn checkbox_text(item: &str) -> Result<&str, CriteriaProblem> {
    let bytes = item.as_bytes();
    if bytes.first() != Some(&b'[') || bytes.get(2) != Some(&b']') {
        return Ok(item);
    }
    if !matches!(bytes.get(1), Some(b' ' | b'x' | b'X')) {
        return Err(CriteriaProblem::NotList);
    }
    let rest = &item[3..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return Err(CriteriaProblem::NotList);
    }
    Ok(rest.trim())
}

fn horizontal_rule(text: &str) -> bool {
    let characters: Vec<_> = text
        .chars()
        .filter(|value| !value.is_whitespace())
        .collect();
    characters.len() >= 3
        && matches!(characters[0], '-' | '*' | '_')
        && characters.iter().all(|value| *value == characters[0])
}
