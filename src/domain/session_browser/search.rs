// session browser 검색은 adapter가 가져온 `SessionSummary`의 id/name/cwd/path/preview/branch를
// domain 안에서 점수화한다. TUI presentation은 점수 알고리즘을 모르고 결과 순서만 소비한다.
use crate::domain::session_summary::SessionSummary;

// `RankedSessionIndex`는 검색/필터 후에도 원본 `recent_sessions.items`를 복사하지 않기 위한
// 작은 ranking record다. page builder는 index로 원본 summary를 다시 참조하고, score/time/index로 정렬한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RankedSessionIndex {
    // index는 원본 session list 위치다. 같은 점수와 같은 updated_at이면 기존 catalog 순서를
    // 유지하는 tie-breaker다.
    pub(super) index: usize,
    // updated_at_epoch는 검색 결과에서 score가 같은 session을 최신순으로 정렬하기 위한 보조 키다.
    pub(super) updated_at_epoch: i64,
    // score는 모든 검색 token의 필드별 최고 점수와 current workspace bonus를 합친 값이다.
    pub(super) score: u32,
}

// 검색창의 raw query를 공백 기준 token으로 낮춘다. 이후 scoring은 모든 token이 session의
// 어느 필드든 매칭되어야 통과하는 AND 검색이므로, 여기서 empty token을 제거해 불필요한 항상-match를 피한다.
pub(super) fn tokenize_search_query(search_query: &str) -> Vec<String> {
    search_query
        .split_whitespace()
        // token은 lowercase로 저장하지만 실제 field 비교는 ASCII case-insensitive helper를 쓴다.
        // 이중 방어는 일반 ASCII 입력을 안정적으로 처리하고, 비ASCII는 원문 포함 비교의 보수적 동작을 유지한다.
        .map(|token| token.trim().to_lowercase())
        .filter(|token| !token.is_empty())
        .collect()
}

// 한 session이 현재 검색 query에 포함될지와 포함된다면 몇 점으로 정렬될지를 계산한다.
// token 중 하나라도 어떤 field에도 매칭되지 않으면 None을 반환해 browser page에서 필터링된다.
pub(super) fn search_query_score(
    // session은 catalog item 하나다. 여기서는 읽기 전용으로 여러 display/search field를 점수화한다.
    session: &SessionSummary,
    // search_tokens는 이미 `tokenize_search_query`를 거친 normalized query다. 빈 slice면
    // for loop가 실행되지 않아 current workspace bonus만 남고, page builder는 별도 sort를 하지 않는다.
    search_tokens: &[String],
    // current workspace directory는 검색어와 독립적인 affinity 신호다. 같은 workspace session을
    // 조금 위로 올리되 field match score보다 훨씬 작게 둔다.
    current_workspace_directory: Option<&str>,
) -> Option<u32> {
    let mut score = current_workspace_bonus(session, current_workspace_directory);
    // `?`는 token 하나가 매칭되지 않을 때 None으로 전체 session을 제외한다. 그래서 `"repo bug"`
    // 검색은 repo와 bug가 각각 어떤 필드든 존재하는 session만 남기는 AND semantics다.
    for search_token in search_tokens {
        score += search_token_score(session, search_token)?;
    }

    Some(score)
}

// 현재 workspace session에 작은 가산점을 준다. 검색 field 자체가 아니므로 낮은 값으로 두어
// id/name/cwd/path의 실제 검색 relevance를 뒤집지 않게 한다.
fn current_workspace_bonus(
    session: &SessionSummary,
    current_workspace_directory: Option<&str>,
) -> u32 {
    if current_workspace_directory
        // workspace가 없으면 bonus는 0이다. workspace 문자열은 adapter가 canonical하게 넘긴
        // 값을 그대로 비교해 path normalization 책임을 search layer에 끌어오지 않는다.
        .is_some_and(|workspace_directory| session.cwd == workspace_directory)
    {
        4
    } else {
        0
    }
}

// token 하나에 대해 session의 여러 searchable field를 점수화하고 가장 높은 field match만
// 사용한다. 같은 token이 id와 cwd에 동시에 걸려도 중복 가산하지 않아 한 token이 ranking을 과도하게 지배하지 않는다.
fn search_token_score(session: &SessionSummary, search_token: &str) -> Option<u32> {
    // 가중치는 사용자가 session을 식별하는 강한 신호일수록 높다. id/name은 직접 식별자라 높고,
    // preview는 느슨한 내용 힌트라 낮으며, cwd/path/branch는 프로젝트 맥락을 찾는 중간 신호다.
    [
        score_search_field(&session.id, search_token, 220, 200, 140),
        score_search_field(&session.preview, search_token, 90, 80, 60),
        score_search_field(&session.cwd, search_token, 150, 135, 100),
        score_search_field(&session.path, search_token, 130, 115, 90),
        session
            .name
            .as_deref()
            // name/git_branch는 optional metadata다. 없으면 해당 field는 점수 후보에서 빠지고,
            // 다른 field가 token을 만족해야 session이 남는다.
            .and_then(|name| score_search_field(name, search_token, 210, 190, 130)),
        session
            .git_branch
            .as_deref()
            .and_then(|branch| score_search_field(branch, search_token, 160, 145, 110)),
    ]
    .into_iter()
    // field별 Option score 중 Some만 남긴다. 전부 None이면 token이 session 어디에도 없다는 뜻이다.
    .flatten()
    .max()
}

// field 하나에서 exact/prefix/contains 계층으로 점수를 계산한다. exact가 가장 강하고,
// prefix는 사용자가 id/name 앞부분을 타이핑하는 흔한 흐름, contains는 느슨한 발견 용도다.
fn score_search_field(
    // haystack은 session field 값, needle은 검색 token이다.
    haystack: &str,
    needle: &str,
    // caller가 field별 가중치를 넘긴다. 같은 matching kind라도 id와 preview의 의미가 다르기 때문이다.
    exact_score: u32,
    prefix_score: u32,
    contains_score: u32,
) -> Option<u32> {
    // exact 비교는 표준 eq_ignore_ascii_case를 사용한다. 전체 field가 token과 같으면
    // prefix/contains보다 강한 의도라고 보고 즉시 반환한다.
    if haystack.eq_ignore_ascii_case(needle) {
        return Some(exact_score);
    }

    // prefix/contains는 byte window helper를 쓴다. Rust str slicing은 UTF-8 경계를 요구하므로
    // ASCII case-insensitive 검색을 명시적으로 byte 단위로 제한해 panic 없이 처리한다.
    if starts_with_ascii_case_insensitive(haystack, needle) {
        return Some(prefix_score);
    }

    contains_ascii_case_insensitive(haystack, needle).then_some(contains_score)
}

// ASCII case-insensitive prefix 검사다. field가 UTF-8 문자열이어도 byte prefix를 직접 가져오고
// ASCII 대소문자만 접기 때문에 multi-byte 문자를 중간에서 slicing하지 않는다.
fn starts_with_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }

    let haystack_bytes = haystack.as_bytes();
    let needle_bytes = needle.as_bytes();
    haystack_bytes
        // haystack이 needle보다 짧으면 get이 None을 반환한다. 별도 길이 분기를 두지 않아도
        // prefix 불일치로 안전하게 처리된다.
        .get(..needle_bytes.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(needle_bytes))
}

// ASCII case-insensitive contains 검사다. 표준 `str::contains`는 대소문자 무시를 제공하지
// 않으므로 needle 길이의 byte windows를 돌며 ASCII 비교한다.
fn contains_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }

    let haystack_bytes = haystack.as_bytes();
    let needle_bytes = needle.as_bytes();
    // windows(needle_len)는 needle이 더 길 때 아무 window도 만들 수 없다. 명시적으로 false를
    // 반환해 아래 iterator가 빈 needle edge case와 섞이지 않게 한다.
    if needle_bytes.len() > haystack_bytes.len() {
        return false;
    }

    haystack_bytes
        // byte window가 UTF-8 문자 경계를 보장하지는 않지만, 비교는 ASCII folding만 하므로
        // non-ASCII 바이트는 원래 바이트와 같을 때만 매칭된다. 그래서 panic 없이 보수적인 substring 검색이 된다.
        .windows(needle_bytes.len())
        .any(|window| window.eq_ignore_ascii_case(needle_bytes))
}
