use crate::domain::session_summary::SessionSummary;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RankedSessionIndex {
    pub(super) index: usize,
    pub(super) updated_at_epoch: i64,
    pub(super) score: u32,
}

pub(super) fn tokenize_search_query(search_query: &str) -> Vec<String> {
    search_query
        .split_whitespace()
        .map(|token| token.trim().to_lowercase())
        .filter(|token| !token.is_empty())
        .collect()
}

pub(super) fn search_query_score(
    session: &SessionSummary,
    search_tokens: &[String],
    current_workspace_directory: Option<&str>,
) -> Option<u32> {
    let mut score = current_workspace_bonus(session, current_workspace_directory);
    for search_token in search_tokens {
        score += search_token_score(session, search_token)?;
    }

    Some(score)
}

fn current_workspace_bonus(
    session: &SessionSummary,
    current_workspace_directory: Option<&str>,
) -> u32 {
    if current_workspace_directory
        .is_some_and(|workspace_directory| session.cwd == workspace_directory)
    {
        4
    } else {
        0
    }
}

fn search_token_score(session: &SessionSummary, search_token: &str) -> Option<u32> {
    [
        score_search_field(&session.id, search_token, 220, 200, 140),
        score_search_field(&session.preview, search_token, 90, 80, 60),
        score_search_field(&session.cwd, search_token, 150, 135, 100),
        score_search_field(&session.path, search_token, 130, 115, 90),
        session
            .name
            .as_deref()
            .and_then(|name| score_search_field(name, search_token, 210, 190, 130)),
        session
            .git_branch
            .as_deref()
            .and_then(|branch| score_search_field(branch, search_token, 160, 145, 110)),
    ]
    .into_iter()
    .flatten()
    .max()
}

fn score_search_field(
    haystack: &str,
    needle: &str,
    exact_score: u32,
    prefix_score: u32,
    contains_score: u32,
) -> Option<u32> {
    if haystack.eq_ignore_ascii_case(needle) {
        return Some(exact_score);
    }

    if starts_with_ascii_case_insensitive(haystack, needle) {
        return Some(prefix_score);
    }

    contains_ascii_case_insensitive(haystack, needle).then_some(contains_score)
}

fn starts_with_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }

    let haystack_bytes = haystack.as_bytes();
    let needle_bytes = needle.as_bytes();
    haystack_bytes
        .get(..needle_bytes.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(needle_bytes))
}

fn contains_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }

    let haystack_bytes = haystack.as_bytes();
    let needle_bytes = needle.as_bytes();
    if needle_bytes.len() > haystack_bytes.len() {
        return false;
    }

    haystack_bytes
        .windows(needle_bytes.len())
        .any(|window| window.eq_ignore_ascii_case(needle_bytes))
}
