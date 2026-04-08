use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubPullRequestTarget {
    pub repository: String,
    pub number: u64,
}

impl GithubPullRequestTarget {
    pub fn new(repository: impl Into<String>, number: u64) -> Self {
        Self {
            repository: repository.into(),
            number,
        }
    }

    pub fn display_label(&self) -> String {
        format!("{}#{}", self.repository, self.number)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubPullRequestActivitySnapshot {
    pub target: GithubPullRequestTarget,
    pub title: String,
    pub url: String,
    pub head_branch: String,
    pub base_branch: String,
    pub events: Vec<GithubPullRequestActivityEvent>,
}

impl GithubPullRequestActivitySnapshot {
    pub fn sort_events(&mut self) {
        self.events.sort_by(|left, right| {
            left.submitted_at
                .cmp(&right.submitted_at)
                .then_with(|| left.id.cmp(&right.id))
                .then_with(|| left.kind.cmp(&right.kind))
        });
    }

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
    pub latest_submitted_at: Option<String>,
    pub seen_events_at_latest_timestamp: Vec<GithubPullRequestActivityIdentity>,
}

impl GithubPullRequestPollState {
    pub fn from_snapshot(snapshot: &GithubPullRequestActivitySnapshot) -> Self {
        snapshot.poll_state()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubPullRequestPollResult {
    pub snapshot: GithubPullRequestActivitySnapshot,
    pub changes: Vec<GithubPullRequestActivityEvent>,
    pub next_state: GithubPullRequestPollState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubPullRequestActivityEvent {
    pub id: u64,
    pub kind: GithubPullRequestActivityKind,
    pub submitted_at: String,
    pub author_login: String,
    pub body: String,
    pub state: Option<String>,
    pub url: String,
    pub path: Option<String>,
}

impl GithubPullRequestActivityEvent {
    pub fn identity(&self) -> GithubPullRequestActivityIdentity {
        GithubPullRequestActivityIdentity {
            kind: self.kind,
            event_id: self.id,
        }
    }

    pub fn notice_label(&self) -> String {
        match self.kind {
            GithubPullRequestActivityKind::Review => self.review_notice_label(),
            GithubPullRequestActivityKind::ReviewComment => {
                match self.path.as_deref().and_then(review_comment_file_label) {
                    Some(file_label) => {
                        format!("comment on {file_label} by {}", self.author_login)
                    }
                    None => format!("review comment by {}", self.author_login),
                }
            }
            GithubPullRequestActivityKind::IssueComment => {
                format!("comment by {}", self.author_login)
            }
        }
    }

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
    pub kind: GithubPullRequestActivityKind,
    pub event_id: u64,
}

fn review_comment_file_label(path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }

    Path::new(trimmed)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string())
        .or_else(|| Some(trimmed.to_string()))
}

fn normalize_review_state(state: &str) -> String {
    state.trim().to_ascii_lowercase().replace('_', " ")
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
            Some("native/src/adapter/inbound/tui/app/shell_presentation.rs"),
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
}
