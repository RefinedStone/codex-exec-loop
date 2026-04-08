use std::cmp::Ordering;

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
        self.events
            .sort_by(|left, right| left.cursor().cmp(&right.cursor()));
    }

    pub fn latest_cursor(&self) -> Option<GithubPullRequestActivityCursor> {
        self.events
            .last()
            .map(GithubPullRequestActivityEvent::cursor)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GithubPullRequestPollState {
    pub last_seen_event: Option<GithubPullRequestActivityCursor>,
}

impl GithubPullRequestPollState {
    pub fn from_snapshot(snapshot: &GithubPullRequestActivitySnapshot) -> Self {
        Self {
            last_seen_event: snapshot.latest_cursor(),
        }
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
    pub fn cursor(&self) -> GithubPullRequestActivityCursor {
        GithubPullRequestActivityCursor {
            submitted_at: self.submitted_at.clone(),
            kind: self.kind,
            event_id: self.id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GithubPullRequestActivityKind {
    Review,
    ReviewComment,
    IssueComment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubPullRequestActivityCursor {
    pub submitted_at: String,
    pub kind: GithubPullRequestActivityKind,
    pub event_id: u64,
}

impl Ord for GithubPullRequestActivityCursor {
    fn cmp(&self, other: &Self) -> Ordering {
        self.submitted_at
            .cmp(&other.submitted_at)
            .then_with(|| self.kind.cmp(&other.kind))
            .then_with(|| self.event_id.cmp(&other.event_id))
    }
}

impl PartialOrd for GithubPullRequestActivityCursor {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
