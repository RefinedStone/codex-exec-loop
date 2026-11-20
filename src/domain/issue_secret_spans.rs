//! 이슈 공유본을 만들 때 사용하는 비밀값 구간 탐지 규칙이다.
//! 원문 값은 반환하지 않고 해당 필드의 바이트 범위만 반환한다.

use std::ops::Range;

use crate::domain::issue_masking::{IssueSensitiveSpan, IssueTextField};
use crate::domain::issue_policy::IssueSensitiveSpanClass;

/// 접두사 뒤에 이 수 이상의 토큰 글자가 있어야 토큰으로 본다.
const MIN_TOKEN_TAIL_CHARS: usize = 16;
/// 키 뒤의 값과 `Bearer` 뒤의 값은 이 수 이상의 글자여야 비밀값으로 본다.
const MIN_SECRET_VALUE_CHARS: usize = 8;
const TOKEN_PREFIXES: &[&str] = &[
    "ghp_",
    "gho_",
    "ghs_",
    "ghu_",
    "ghr_",
    "github_pat_",
    "glpat-",
    "xoxb-",
    "xoxp-",
    "sk-",
    "AKIA",
];
const SECRET_KEYS: &[&str] = &["password", "passwd", "secret", "token", "api_key", "apikey"];

pub fn secret_spans(field: IssueTextField, text: &str) -> Vec<IssueSensitiveSpan> {
    secret_ranges(text)
        .into_iter()
        .map(|range| IssueSensitiveSpan {
            field,
            range,
            class: Some(IssueSensitiveSpanClass::Secret),
            user_decision: None,
        })
        .collect()
}

/// 비밀값으로 보이는 구간이다. 겹칠 수 있다. 모든 경계는 ASCII 글자 옆이므로 문자 경계이다.
/// shortcut: 알려진 모양(토큰 접두사, Bearer, URL 의 계정 정보, `키=값`, PEM 개인 키)만 찾는다. 이름과 경로는
/// 찾지 않는다. 다른 모양이 필요하면 사용자 지정 검증기(#2431)에서 구간을 받는다.
fn secret_ranges(text: &str) -> Vec<Range<usize>> {
    // ASCII 만 바꾸므로 바이트 위치가 `text` 와 같다.
    let lower = text.to_ascii_lowercase();
    let mut ranges = Vec::new();

    for prefix in TOKEN_PREFIXES {
        for (start, _) in text.match_indices(prefix) {
            let inside_word = text[..start]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric);
            let tail_start = start + prefix.len();
            let tail = text[tail_start..]
                .bytes()
                .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                .count();
            if !inside_word && tail >= MIN_TOKEN_TAIL_CHARS {
                ranges.push(start..tail_start + tail);
            }
        }
    }

    for (start, matched) in lower.match_indices("bearer ") {
        let value = value_range(text, start + matched.len());
        if value.len() >= MIN_SECRET_VALUE_CHARS {
            ranges.push(value);
        }
    }

    for (start, matched) in text.match_indices("://") {
        let authority_start = start + matched.len();
        let authority = &text[authority_start..];
        let end = authority
            .find(|character: char| {
                character.is_whitespace()
                    || matches!(character, '/' | '?' | '#' | '"' | '\'' | '`' | '<' | '>')
            })
            .unwrap_or(authority.len());
        let authority = &authority[..end];
        if let Some(at) = authority.rfind('@')
            && authority[..at].contains(':')
        {
            ranges.push(authority_start..authority_start + at);
        }
    }

    for key in SECRET_KEYS {
        for (start, _) in lower.match_indices(key) {
            let after_key = text[start + key.len()..].trim_start_matches(['"', '\'', ' ']);
            let Some(after_separator) = after_key.strip_prefix(['=', ':']) else {
                continue;
            };
            let value_text = after_separator.trim_start();
            let value = quoted_value_range(text, text.len() - value_text.len());
            if text[value.clone()].chars().count() >= MIN_SECRET_VALUE_CHARS {
                ranges.push(value);
            }
        }
    }

    for (start, matched) in text.match_indices("-----BEGIN ") {
        let rest = &text[start..];
        let header = rest.lines().next().unwrap_or_default();
        if !header.contains("PRIVATE KEY") {
            continue;
        }
        // 끝 줄이 없으면 글의 끝까지 버린다.
        let end = rest[matched.len()..]
            .find("-----END ")
            .map(|offset| matched.len() + offset)
            .and_then(|end_marker| {
                let after_marker = end_marker + "-----END ".len();
                rest[after_marker..]
                    .find("-----")
                    .map(|offset| after_marker + offset + "-----".len())
            })
            .unwrap_or(rest.len());
        ranges.push(start..start + end);
    }

    ranges
}

/// 인용 값은 공백이나 쉼표를 포함할 수 있다. 닫히지 않은 인용 값은 나머지 전체를 가린다.
fn quoted_value_range(text: &str, start: usize) -> Range<usize> {
    let Some(quote @ ('"' | '\'')) = text[start..].chars().next() else {
        return value_range(text, start);
    };
    let value_start = start + quote.len_utf8();
    let mut escaped = false;
    for (offset, character) in text[value_start..].char_indices() {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == quote {
            return value_start..value_start + offset;
        }
    }
    value_start..text.len()
}

/// `start` 에서 시작하는 값의 범위이다. 공백, 따옴표, 쉼표, 쌍반점 앞에서 끝난다.
fn value_range(text: &str, start: usize) -> Range<usize> {
    let end = text[start..]
        .find(|character: char| {
            character.is_whitespace() || matches!(character, '"' | '\'' | '`' | ',' | ';')
        })
        .map_or(text.len(), |offset| start + offset);
    start..end
}
