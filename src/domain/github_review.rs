use std::path::Path;

/*
 * GitHub review polling은 adapter가 GitHub REST 응답을 그대로 흘려보내지 않고, application/TUI가
 * 필요한 PR 단위 activity model로 줄여서 다룬다. 이 파일의 타입들은 "어떤 PR을 보고 있는가",
 * "이번 poll에서 어떤 event가 새로 보였는가", "footer notice에는 어떤 짧은 문구를 보여 줄 것인가"를
 * adapter와 service 사이에서 공유하는 domain vocabulary다.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubPullRequestTarget {
    // `owner/name` 형태의 repository full name이다. adapter query와 TUI status label이 같은 값을 쓴다.
    pub repository: String,
    // GitHub PR number다. issue number와 같은 namespace라 display label에서는 `repo#number`로 노출한다.
    pub number: u64,
}

impl GithubPullRequestTarget {
    // 환경 변수나 auto-discovery 결과가 만든 target을 domain 값으로 고정한다.
    pub fn new(repository: impl Into<String>, number: u64) -> Self {
        Self {
            repository: repository.into(),
            number,
        }
    }

    // polling 상태와 footer copy에서 사람이 식별하기 쉬운 짧은 PR label이다.
    pub fn display_label(&self) -> String {
        format!("{}#{}", self.repository, self.number)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubPullRequestActivitySnapshot {
    // 이 snapshot이 어느 PR에서 왔는지 나타낸다. poll state는 target별로만 비교할 수 있다.
    pub target: GithubPullRequestTarget,
    // PR title/url/branches는 notice copy와 debug surface에서 GitHub를 다시 열지 않고 보여 주기 위한 metadata다.
    pub title: String,
    pub url: String,
    pub head_branch: String,
    pub base_branch: String,
    // review, review comment, issue comment를 하나의 시간순 stream으로 합친 activity list다.
    pub events: Vec<GithubPullRequestActivityEvent>,
}

impl GithubPullRequestActivitySnapshot {
    /*
     * GitHub API는 review, review comment, issue comment를 서로 다른 endpoint로 준다. service가 diff를
     * 계산하기 전에 이 method로 하나의 deterministic order를 만든다. timestamp가 같은 event는 id와 kind로
     * 다시 정렬해 매 poll마다 같은 ordering을 얻는다.
     */
    pub fn sort_events(&mut self) {
        self.events.sort_by(|left, right| {
            left.submitted_at
                .cmp(&right.submitted_at)
                .then_with(|| left.id.cmp(&right.id))
                .then_with(|| left.kind.cmp(&right.kind))
        });
    }

    /*
     * poll state는 "마지막 timestamp까지 봤고, 그 timestamp에 어떤 event ids를 이미 봤는가"만 저장한다.
     * 같은 timestamp에 여러 GitHub event가 뒤늦게 추가될 수 있어 timestamp 하나만 cursor로 쓰지 않고,
     * latest timestamp 안의 identity set을 함께 저장한다.
     */
    pub fn poll_state(&self) -> GithubPullRequestPollState {
        let Some(latest_submitted_at) = self.events.last().map(|event| event.submitted_at.clone())
        else {
            return GithubPullRequestPollState::default();
        };

        let mut seen_events_at_latest_timestamp = self
            .events
            .iter()
            .filter(|event| event.submitted_at == latest_submitted_at)
            .map(GithubPullRequestActivityEvent::identity)
            .collect::<Vec<_>>();
        seen_events_at_latest_timestamp.sort();
        seen_events_at_latest_timestamp.dedup();

        GithubPullRequestPollState {
            latest_submitted_at: Some(latest_submitted_at),
            seen_events_at_latest_timestamp,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GithubPullRequestPollState {
    // 이전 poll에서 관찰한 가장 최신 submitted_at 값이다. 비어 있으면 첫 poll로 간주한다.
    pub latest_submitted_at: Option<String>,
    // 최신 timestamp에 이미 처리한 event identity들이다. 같은 초/동일 timestamp event 중복 알림을 막는다.
    pub seen_events_at_latest_timestamp: Vec<GithubPullRequestActivityIdentity>,
}

impl GithubPullRequestPollState {
    // caller가 snapshot에서 다음 cursor를 만들 때 사용하는 얇은 domain helper다.
    pub fn from_snapshot(snapshot: &GithubPullRequestActivitySnapshot) -> Self {
        snapshot.poll_state()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubPullRequestPollResult {
    // 이번 poll에서 adapter가 가져온 전체 activity snapshot이다.
    pub snapshot: GithubPullRequestActivitySnapshot,
    // 이전 poll state와 비교해 TUI에 새로 알려야 하는 event들이다.
    pub changes: Vec<GithubPullRequestActivityEvent>,
    // 다음 background poll에 넘길 cursor다.
    pub next_state: GithubPullRequestPollState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubPullRequestActivityEvent {
    // GitHub event id다. review/review-comment/issue-comment endpoint마다 kind와 함께 identity를 이룬다.
    pub id: u64,
    // 서로 다른 GitHub activity source를 하나의 enum으로 접은 값이다.
    pub kind: GithubPullRequestActivityKind,
    // GitHub submitted/created timestamp 문자열이다. adapter가 정렬 가능한 ISO timestamp로 채워 넣는다.
    pub submitted_at: String,
    // footer notice에 바로 쓸 수 있는 GitHub login이다.
    pub author_login: String,
    // TUI footer summary에 붙일 원문 body다. 너무 길면 `notice_summary`에서 잘린다.
    pub body: String,
    // review event의 state(APPROVED, CHANGES_REQUESTED 등)다. comment 계열 event에서는 없을 수 있다.
    pub state: Option<String>,
    // 사용자가 상세 내용을 열 때 사용할 GitHub URL이다.
    pub url: String,
    // review comment가 달린 파일 경로다. review/issue comment에는 없을 수 있다.
    pub path: Option<String>,
}

impl GithubPullRequestActivityEvent {
    // poll cursor에서 쓰는 최소 identity만 추출한다. body 변경 같은 표시 정보는 중복 판정에 쓰지 않는다.
    pub fn identity(&self) -> GithubPullRequestActivityIdentity {
        GithubPullRequestActivityIdentity {
            kind: self.kind,
            event_id: self.id,
        }
    }

    /*
     * notice label은 footer나 status panel에서 event를 한 줄로 구분하기 위한 제목이다. review comment는
     * 파일명이 있으면 파일 단위 action처럼 보이게 하고, review event는 state를 사람이 읽는 문구로 바꾼다.
     */
    pub fn notice_label(&self) -> String {
        match self.kind {
            GithubPullRequestActivityKind::Review => self.review_notice_label(),
            GithubPullRequestActivityKind::ReviewComment => {
                match self.path.as_deref().and_then(review_comment_file_label) {
                    Some(file_label) => format!("comment on {file_label} by {}", self.author_login),
                    None => format!("review comment by {}", self.author_login),
                }
            }
            GithubPullRequestActivityKind::IssueComment => {
                format!("comment by {}", self.author_login)
            }
        }
    }

    /*
     * notice summary는 label과 body를 합친 footer용 문구다. rendering layer가 같은 truncation 규칙을
     * 반복하지 않도록 domain 쪽에서 whitespace collapse와 length budget을 함께 처리한다.
     */
    pub fn notice_summary(&self, max_total_len: usize) -> String {
        let review_state = self
            .state
            .as_deref()
            .map(str::trim)
            .filter(|state| !state.is_empty())
            .map(|state| state.to_ascii_lowercase())
            .unwrap_or_else(|| "updated".to_string());
        let label = match self.kind {
            GithubPullRequestActivityKind::Review => {
                format!("review {review_state} by {}", self.author_login)
            }
            GithubPullRequestActivityKind::ReviewComment => {
                match self.path.as_deref().and_then(review_comment_file_label) {
                    Some(file_label) => {
                        format!("review comment by {} on {file_label}", self.author_login)
                    }
                    None => format!("review comment by {}", self.author_login),
                }
            }
            GithubPullRequestActivityKind::IssueComment => {
                format!("comment by {}", self.author_login)
            }
        };

        let mut summary = label;
        if !self.body.trim().is_empty() {
            summary.push_str(": ");
            summary.push_str(&self.body);
        }

        truncate_notice_text(&summary, max_total_len)
    }

    // GitHub review state를 제품 copy로 변환한다. 알 수 없는 state는 숨기지 않고 normalized form으로 드러낸다.
    fn review_notice_label(&self) -> String {
        match self
            .state
            .as_deref()
            .map(str::trim)
            .filter(|state| !state.is_empty())
        {
            Some("APPROVED") => format!("approved review by {}", self.author_login),
            Some("CHANGES_REQUESTED") => {
                format!("changes requested by {}", self.author_login)
            }
            Some("COMMENTED") => format!("review by {}", self.author_login),
            Some("DISMISSED") => format!("dismissed review by {}", self.author_login),
            Some("PENDING") => format!("pending review by {}", self.author_login),
            Some(state) => format!(
                "review ({}) by {}",
                normalize_review_state(state),
                self.author_login
            ),
            None => format!("review by {}", self.author_login),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GithubPullRequestActivityKind {
    Review,
    ReviewComment,
    IssueComment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GithubPullRequestActivityIdentity {
    // endpoint가 다른 event id 충돌을 막기 위해 kind를 identity에 포함한다.
    pub kind: GithubPullRequestActivityKind,
    pub event_id: u64,
}

// review comment path는 footer에서 전체 경로보다 파일명이 더 유용하다. 파일명 추출에 실패하면 원래 path를 쓴다.
fn review_comment_file_label(path: &str) -> Option<&str> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }

    Path::new(trimmed)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .or(Some(trimmed))
}

// GitHub가 새 review state를 추가해도 notice가 screaming snake case를 그대로 노출하지 않게 한다.
fn normalize_review_state(state: &str) -> String {
    state.trim().to_ascii_lowercase().replace('_', " ")
}

/*
 * footer notice는 폭이 제한된 TUI 영역에 들어간다. 이 helper는 unicode scalar 기준으로 길이를 세고,
 * 여러 whitespace를 하나의 space로 접으며, 예산을 초과하면 ellipsis까지 포함해 정확히 max_len에 맞춘다.
 */
pub fn truncate_notice_text(text: &str, max_len: usize) -> String {
    if max_len == 0 {
        return String::new();
    }

    let mut compact = String::new();
    let mut compact_len = 0usize;
    let mut pending_space = false;
    let mut truncated = false;

    // whitespace를 먼저 접어야 긴 body의 줄바꿈/탭이 footer layout을 흔들지 않는다.
    for ch in text.chars() {
        if ch.is_whitespace() {
            if compact_len > 0 {
                pending_space = true;
            }
            continue;
        }

        if pending_space {
            if compact_len == max_len {
                truncated = true;
                break;
            }
            compact.push(' ');
            compact_len += 1;
            pending_space = false;
        }

        if compact_len == max_len {
            truncated = true;
            break;
        }
        compact.push(ch);
        compact_len += 1;
    }

    if !truncated {
        return compact;
    }

    if max_len <= 3 {
        return ".".repeat(max_len);
    }

    // ellipsis 공간을 먼저 확보하고 trailing space를 제거해 `word ...` 같은 어색한 copy를 피한다.
    let mut result: String = compact.chars().take(max_len - 3).collect();
    while result.ends_with(' ') {
        result.pop();
    }
    result.push_str("...");
    result
}

#[cfg(test)]
mod tests {
    use super::{GithubPullRequestActivityEvent, GithubPullRequestActivityKind};

    fn event(
        kind: GithubPullRequestActivityKind,
        author_login: &str,
        state: Option<&str>,
        path: Option<&str>,
    ) -> GithubPullRequestActivityEvent {
        GithubPullRequestActivityEvent {
            id: 42,
            kind,
            submitted_at: "2026-04-08T09:00:00Z".to_string(),
            author_login: author_login.to_string(),
            body: String::new(),
            state: state.map(|value| value.to_string()),
            url: "https://example.invalid/pr/42".to_string(),
            path: path.map(|value| value.to_string()),
        }
    }

    #[test]
    fn review_notice_uses_review_state_and_author() {
        let event = event(
            GithubPullRequestActivityKind::Review,
            "reviewer",
            Some("APPROVED"),
            None,
        );

        assert_eq!(event.notice_label(), "approved review by reviewer");
    }

    #[test]
    fn review_comment_notice_uses_file_name_when_available() {
        let event = event(
            GithubPullRequestActivityKind::ReviewComment,
            "reviewer",
            None,
            Some("src/adapter/inbound/tui/app/shell_presentation.rs"),
        );

        assert_eq!(
            event.notice_label(),
            "comment on shell_presentation.rs by reviewer"
        );
    }

    #[test]
    fn review_notice_normalizes_unknown_states() {
        let event = event(
            GithubPullRequestActivityKind::Review,
            "reviewer",
            Some("READY_FOR_REVIEW"),
            None,
        );

        assert_eq!(
            event.notice_label(),
            "review (ready for review) by reviewer"
        );
    }

    #[test]
    fn notice_summary_formats_review_state_and_author() {
        let event = GithubPullRequestActivityEvent {
            id: 100,
            kind: GithubPullRequestActivityKind::Review,
            submitted_at: "2026-04-08T09:00:00Z".to_string(),
            author_login: "reviewer".to_string(),
            body: "Looks good to me".to_string(),
            state: Some("COMMENTED".to_string()),
            url: "https://example.invalid/review/100".to_string(),
            path: None,
        };

        assert_eq!(
            event.notice_summary(64),
            "review commented by reviewer: Looks good to me"
        );
    }

    #[test]
    fn notice_summary_includes_review_comment_path_and_respects_total_budget() {
        let event = GithubPullRequestActivityEvent {
            id: 101,
            kind: GithubPullRequestActivityKind::ReviewComment,
            submitted_at: "2026-04-08T09:00:00Z".to_string(),
            author_login: "reviewer".to_string(),
            body: "Please rename this method because the current name is misleading.".to_string(),
            state: None,
            url: "https://example.invalid/review-comment/101".to_string(),
            path: Some("src/app.rs".to_string()),
        };

        let summary = event.notice_summary(24);

        assert_eq!(summary, "review comment by rev...");
        assert_eq!(summary.chars().count(), 24);
    }

    #[test]
    fn notice_summary_uses_exact_budget_for_small_limits() {
        let event = GithubPullRequestActivityEvent {
            id: 102,
            kind: GithubPullRequestActivityKind::IssueComment,
            submitted_at: "2026-04-08T09:00:00Z".to_string(),
            author_login: "reviewer".to_string(),
            body: "Looks good".to_string(),
            state: None,
            url: "https://example.invalid/comment/102".to_string(),
            path: None,
        };

        assert_eq!(event.notice_summary(3), "...");
        assert_eq!(event.notice_summary(2), "..");
    }
}
