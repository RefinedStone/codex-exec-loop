//! 원격 JSON은 이 private 모듈 안에서만 역직렬화한다. Debug/Serialize를 구현하지 않는다.

use serde::{Deserialize, Deserializer, de::IgnoredAny};

pub(super) const ISSUE_FRAGMENT: &str = r#"
fragment AkraSourceIssue on Issue {
  __typename id number url title body state updatedAt repository { id nameWithOwner }
  author { login ... on Node { id } }
  labels(first: 100) {
    nodes { id name }
    pageInfo { hasNextPage hasPreviousPage }
  }
  assignees: assignedActors(first: 100) {
    nodes { ... on Node { id } ... on Actor { login } }
    pageInfo { hasNextPage hasPreviousPage }
  }
  timelineItems(last: 100, itemTypes: [LABELED_EVENT, UNLABELED_EVENT]) {
    nodes {
      __typename
      ... on LabeledEvent { id createdAt actor { login ... on Node { id } } label { id name } }
      ... on UnlabeledEvent { id createdAt actor { login ... on Node { id } } label { id name } }
    }
    pageInfo { hasNextPage hasPreviousPage }
  }
  closedByPullRequestsReferences(first: 1, includeClosedPrs: true) { totalCount }
}"#;

const DISCOVER_BY_NAME: &str = r#"
query($owner: String!, $name: String!, $first: Int!, $after: String) {
  repository(owner: $owner, name: $name) {
    __typename id
    issues(first: $first, after: $after, states: [OPEN], orderBy: {field: UPDATED_AT, direction: ASC}) {
      nodes { ...AkraSourceIssue }
      pageInfo { endCursor hasNextPage }
    }
  }
}"#;

const DISCOVER_BY_ID: &str = r#"
query($id: ID!, $first: Int!, $after: String) {
  node(id: $id) {
    __typename
    ... on Repository {
      id
      issues(first: $first, after: $after, states: [OPEN], orderBy: {field: UPDATED_AT, direction: ASC}) {
        nodes { ...AkraSourceIssue }
        pageInfo { endCursor hasNextPage }
      }
    }
  }
}"#;

const READ_BY_ID: &str = r#"
query($id: ID!) { node(id: $id) { __typename ...AkraSourceIssue } }
"#;

pub(super) fn discovery_query(stable: bool) -> String {
    format!(
        "{}\n{ISSUE_FRAGMENT}",
        if stable {
            DISCOVER_BY_ID
        } else {
            DISCOVER_BY_NAME
        }
    )
}

pub(super) fn resync_query() -> String {
    format!("{READ_BY_ID}\n{ISSUE_FRAGMENT}")
}

#[derive(Deserialize)]
pub(super) struct Envelope<T> {
    pub data: Option<T>,
    pub errors: Option<Vec<IgnoredAny>>,
}

#[derive(Deserialize)]
pub(super) struct DiscoveryData {
    pub repository: Option<RepositoryNode>,
    pub node: Option<RepositoryNode>,
}

#[derive(Deserialize)]
pub(super) struct ReadData {
    pub node: Option<IssueNode>,
}

#[derive(Deserialize)]
pub(super) struct RepositoryNode {
    #[serde(rename = "__typename")]
    pub typename: String,
    pub id: Option<String>,
    pub issues: Option<IssueConnection>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IssueConnection {
    pub nodes: Vec<Option<IssueNode>>,
    pub page_info: PageInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PageInfo {
    pub end_cursor: Option<String>,
    pub has_next_page: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MetadataConnection<T> {
    pub nodes: Vec<Option<T>>,
    pub page_info: MetadataPageInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MetadataPageInfo {
    pub has_next_page: bool,
    pub has_previous_page: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IssueNode {
    #[serde(rename = "__typename")]
    pub typename: String,
    pub id: Option<String>,
    pub number: Option<u64>,
    pub url: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub state: Option<String>,
    pub updated_at: Option<String>,
    pub repository: Option<RepositoryId>,
    #[serde(default)]
    pub author: AuthorField,
    pub labels: Option<MetadataConnection<Label>>,
    pub assignees: Option<MetadataConnection<Actor>>,
    pub timeline_items: Option<MetadataConnection<LabelEvent>>,
    pub closed_by_pull_requests_references: Option<PrCount>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RepositoryId {
    pub id: String,
    pub name_with_owner: String,
}

#[derive(Default)]
pub(super) enum AuthorField {
    #[default]
    Missing,
    Present(Option<Actor>),
}

impl<'de> Deserialize<'de> for AuthorField {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Option::<Actor>::deserialize(deserializer).map(Self::Present)
    }
}

#[derive(Deserialize)]
pub(super) struct Actor {
    pub id: Option<String>,
    pub login: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct Label {
    pub id: String,
    pub name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LabelEvent {
    #[serde(rename = "__typename")]
    pub typename: String,
    pub id: Option<String>,
    pub created_at: Option<String>,
    #[serde(default)]
    pub actor: AuthorField,
    pub label: Option<Label>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PrCount {
    pub total_count: Option<u64>,
}
