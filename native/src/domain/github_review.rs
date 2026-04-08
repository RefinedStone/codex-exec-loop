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
