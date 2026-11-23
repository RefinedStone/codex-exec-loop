use std::collections::BTreeSet;

use crate::domain::issue_markdown::{IssueMarkdownHeadings, IssueMarkdownLine};

const MAX_MENTIONED_PATHS: usize = 128;
const MAX_MENTIONED_PATH_BYTES: usize = 1024;
const MAX_MATCHED_GLOB_BYTES: usize = 4096;

pub(super) struct MentionedPaths {
    pub paths: Vec<String>,
    pub incomplete: bool,
}

/// Markdown 링크, URL, 절대 경로, 코드 울타리의 예제에서 저장소 경로를 추론하지 않는다.
/// inline code 안의 명시한 상대 경로만 읽는다. 읽기 한도를 넘으면 위험을 높일 근거를 남긴다.
pub(super) fn mentioned_paths(body: &str) -> MentionedPaths {
    let mut markdown = IssueMarkdownHeadings::default();
    let mut paths = BTreeSet::new();
    let mut incomplete = false;
    for line in body.lines() {
        let text = match markdown.line(line) {
            IssueMarkdownLine::Text(text) | IssueMarkdownLine::Heading(_, text) => text,
            IssueMarkdownLine::Code => {
                // 완료 기준과 달리, 주석 밖 내용이 함께 있던 줄을 안전한 빈 경로로 확정할 수 없다.
                incomplete |= markdown.comment_excludes_visible_text();
                continue;
            }
        };
        let bytes = text.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] != b'\x60' || escaped(bytes, index) {
                index += 1;
                continue;
            }
            let width = bytes[index..]
                .iter()
                .take_while(|byte| **byte == b'\x60')
                .count();
            let start = index + width;
            let mut closing = start;
            let mut end = None;
            while closing < bytes.len() {
                if bytes[closing] == b'\x60' {
                    let run = bytes[closing..]
                        .iter()
                        .take_while(|byte| **byte == b'\x60')
                        .count();
                    if run == width {
                        end = Some(closing);
                        break;
                    }
                    closing += run;
                } else {
                    closing += 1;
                }
            }
            let Some(end) = end else {
                // 미완성 inline code가 경로를 숨길 수 있으므로 normal/low로 확정하지 않는다.
                incomplete = true;
                break;
            };
            match repository_path(text[start..end].trim_matches(' ')) {
                PathCandidate::Relative(path) => {
                    if paths.len() >= MAX_MENTIONED_PATHS && !paths.contains(path) {
                        incomplete = true;
                    } else {
                        paths.insert(path.to_string());
                    }
                }
                PathCandidate::Unreadable => incomplete = true,
                PathCandidate::NotRepository => {}
            }
            index = end + width;
        }
    }
    MentionedPaths {
        paths: paths.into_iter().collect(),
        incomplete,
    }
}

enum PathCandidate<'a> {
    Relative(&'a str),
    NotRepository,
    Unreadable,
}

fn repository_path(value: &str) -> PathCandidate<'_> {
    if value.is_empty()
        || value.contains("://")
        || value.starts_with(['/', '\\'])
        || value.as_bytes().get(1) == Some(&b':')
    {
        return PathCandidate::NotRepository;
    }
    // 확장자가 없는 루트 파일도 보호/위험 패턴의 입력이다. 명령이나 markup 문자열은
    // 경로로 추정하지 않되, 한 단어인 파일명 후보는 일반 상대 경로와 같은 검사를 거친다.
    if !value.contains('/')
        && !value.contains('.')
        && !value.contains('\\')
        && !root_file_name_candidate(value)
    {
        return PathCandidate::NotRepository;
    }
    if value.len() > MAX_MENTIONED_PATH_BYTES
        || value
            .chars()
            .any(|value| value.is_whitespace() || value.is_control())
        || value
            .chars()
            .any(|value| matches!(value, '\\' | ':' | '*' | '?' | '[' | ']' | '{' | '}' | '!'))
        || value
            .split('/')
            .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
    {
        return PathCandidate::Unreadable;
    }
    PathCandidate::Relative(value)
}

fn root_file_name_candidate(value: &str) -> bool {
    // 명확한 markup과 복합 표현만 제외한다. 파일명 후보의 지원 불가 문자는
    // repository_path의 공통 검증에서 Unreadable로 처리해야 한다.
    !value.starts_with('<') && !value.chars().any(char::is_whitespace)
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

/// None은 지원 범위를 넘겨 읽지 못했다는 뜻이다. 불일치(false)로 대신하면 안 된다.
pub(in crate::domain::issue_automation) fn matches_glob(pattern: &str, path: &str) -> Option<bool> {
    if pattern.len() > MAX_MATCHED_GLOB_BYTES || path.len() > MAX_MENTIONED_PATH_BYTES {
        return None;
    }
    let pattern: Vec<_> = pattern.split('/').collect();
    let path: Vec<_> = path.split('/').collect();
    let mut reachable = vec![false; path.len() + 1];
    reachable[0] = true;
    for segment in pattern {
        let mut next = vec![false; path.len() + 1];
        if segment == "**" {
            next[0] = reachable[0];
            for index in 1..=path.len() {
                next[index] = reachable[index] || next[index - 1];
            }
        } else {
            for index in 1..=path.len() {
                next[index] = reachable[index - 1] && matches_segment(segment, path[index - 1]);
            }
        }
        reachable = next;
    }
    Some(reachable[path.len()])
}

/// *와 ?는 slash를 넘지 않는다. Unicode 문자를 byte 하나로 잘라 비교하지 않는다.
fn matches_segment(pattern: &str, text: &str) -> bool {
    let pattern: Vec<_> = pattern.chars().collect();
    let text: Vec<_> = text.chars().collect();
    let (mut pattern_index, mut text_index) = (0, 0);
    let mut star = None;
    let mut retry_text = 0;
    while text_index < text.len() {
        if pattern.get(pattern_index) == Some(&'?')
            || pattern.get(pattern_index) == text.get(text_index)
        {
            pattern_index += 1;
            text_index += 1;
        } else if pattern.get(pattern_index) == Some(&'*') {
            star = Some(pattern_index);
            pattern_index += 1;
            retry_text = text_index;
        } else if let Some(star_index) = star {
            retry_text += 1;
            text_index = retry_text;
            pattern_index = star_index + 1;
        } else {
            return false;
        }
    }
    while pattern.get(pattern_index) == Some(&'*') {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}
