//! GitHub 응답은 이 모듈에서만 해석한다. 원문과 공급자 오류는 Debug/Serialize하지 않는다.
//! 계약: https://docs.github.com/en/graphql/reference/issues
use serde::{Deserialize, de::IgnoredAny};

pub(super) const VIEWER: &str = r#"
query AkraFeedbackViewer($id: ID!) {
  viewer { __typename id }
  node(id: $id) { __typename ... on Issue { id repository { id } } }
}"#;

pub(super) const COMMENT_FRAGMENT: &str = r#"
fragment AkraFeedbackComment on IssueComment {
  __typename id body
  author { __typename ... on Node { id } }
  editor { __typename ... on Node { id } }
  lastEditedAt
  issue { __typename id repository { id } }
  repository { id }
  pullRequest { id }
}"#;

pub(super) const COMMENTS: &str = r#"
query AkraFeedbackComments($id: ID!, $after: String) {
  node(id: $id) {
    __typename
    ... on Issue {
      id repository { id }
      comments(first: 100, after: $after) {
        nodes { ...AkraFeedbackComment }
        totalCount
        pageInfo { endCursor hasNextPage }
      }
    }
  }
}"#;

pub(super) const COMMENT: &str = r#"
query AkraFeedbackCommentById($id: ID!) {
  node(id: $id) { __typename ...AkraFeedbackComment }
}"#;

pub(super) const CREATE_COMMENT: &str = r#"
mutation AkraFeedbackCreateComment($id: ID!, $body: String!, $intent: String!) {
  feedback: addComment(input: {subjectId: $id, body: $body, clientMutationId: $intent}) {
    clientMutationId
    commentEdge { node { ...AkraFeedbackComment } }
  }
}"#;

pub(super) const UPDATE_COMMENT: &str = r#"
mutation AkraFeedbackUpdateComment($id: ID!, $body: String!, $intent: String!) {
  feedback: updateIssueComment(input: {id: $id, body: $body, clientMutationId: $intent}) {
    clientMutationId
    issueComment { ...AkraFeedbackComment }
  }
}"#;

pub(super) const RESOLVE_LABEL: &str = r#"
query AkraFeedbackResolveLabel($id: ID!, $name: String!) {
  node(id: $id) {
    __typename
    ... on Issue {
      id
      repository {
        id
        label(name: $name) { id name repository { id } }
      }
    }
  }
}"#;

pub(super) const LABELS: &str = r#"
query AkraFeedbackLabels($id: ID!, $after: String) {
  node(id: $id) {
    __typename
    ... on Issue {
      id repository { id }
      labels(first: 100, after: $after) {
        nodes { id name repository { id } }
        totalCount
        pageInfo { endCursor hasNextPage }
      }
    }
  }
}"#;

pub(super) const LABEL_EVENTS: &str = r#"
query AkraFeedbackLabelEvents($id: ID!, $after: String) {
  node(id: $id) {
    __typename
    ... on Issue {
      id repository { id }
      timelineItems(first: 100, after: $after, itemTypes: [LABELED_EVENT, UNLABELED_EVENT]) {
        nodes {
          __typename
          ... on LabeledEvent {
            id
            actor { __typename ... on Node { id } }
            label { id name repository { id } }
          }
          ... on UnlabeledEvent {
            id
            actor { __typename ... on Node { id } }
            label { id name repository { id } }
          }
        }
        totalCount
        pageInfo { endCursor hasNextPage }
      }
    }
  }
}"#;

pub(super) const ADD_LABEL: &str = r#"
mutation AkraFeedbackAddLabel($id: ID!, $label: ID!, $intent: String!) {
  feedback: addLabelsToLabelable(input: {labelableId: $id, labelIds: [$label], clientMutationId: $intent}) {
    clientMutationId
    labelable { __typename ... on Issue { id repository { id } } }
  }
}"#;

pub(super) const REMOVE_LABEL: &str = r#"
mutation AkraFeedbackRemoveLabel($id: ID!, $label: ID!, $intent: String!) {
  feedback: removeLabelsFromLabelable(input: {labelableId: $id, labelIds: [$label], clientMutationId: $intent}) {
    clientMutationId
    labelable { __typename ... on Issue { id repository { id } } }
  }
}"#;

pub(super) fn with_comment_fragment(query: &str) -> String {
    format!("{query}\n{COMMENT_FRAGMENT}")
}

#[derive(Deserialize)]
pub(super) struct Envelope<T> {
    pub data: Option<T>,
    pub errors: Option<Vec<IgnoredAny>>,
}

#[derive(Deserialize)]
pub(super) struct NodeData<T> {
    pub node: Option<T>,
}

#[derive(Deserialize)]
pub(super) struct ViewerData {
    pub viewer: Option<Actor>,
    pub node: Option<IssueNode>,
}

#[derive(Deserialize)]
pub(super) struct Actor {
    #[serde(rename = "__typename")]
    pub typename: String,
    pub id: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct RepositoryId {
    pub id: String,
}

#[derive(Deserialize)]
pub(super) struct Repository {
    pub id: String,
    pub label: Option<Label>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IssueNode {
    #[serde(rename = "__typename")]
    pub typename: String,
    pub id: Option<String>,
    pub repository: Option<Repository>,
    pub comments: Option<Connection<Comment>>,
    pub labels: Option<Connection<Label>>,
    pub timeline_items: Option<Connection<LabelEvent>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Comment {
    #[serde(rename = "__typename")]
    pub typename: String,
    pub id: String,
    pub body: String,
    pub author: Option<Actor>,
    pub editor: Option<Actor>,
    pub last_edited_at: Option<String>,
    pub issue: IssueNode,
    pub repository: RepositoryId,
    pub pull_request: Option<RepositoryId>,
}

#[derive(Deserialize)]
pub(super) struct Label {
    pub id: String,
    pub name: String,
    pub repository: RepositoryId,
}

#[derive(Deserialize)]
pub(super) struct LabelEvent {
    #[serde(rename = "__typename")]
    pub typename: String,
    pub id: String,
    pub actor: Option<Actor>,
    pub label: Label,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Connection<T> {
    pub nodes: Option<Vec<Option<T>>>,
    pub total_count: usize,
    pub page_info: PageInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PageInfo {
    pub has_next_page: bool,
    pub end_cursor: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct MutationData {
    pub feedback: Option<MutationPayload>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MutationPayload {
    pub client_mutation_id: Option<String>,
    pub comment_edge: Option<CommentEdge>,
    pub issue_comment: Option<Comment>,
    pub labelable: Option<IssueNode>,
}

#[derive(Deserialize)]
pub(super) struct CommentEdge {
    pub node: Option<Comment>,
}
