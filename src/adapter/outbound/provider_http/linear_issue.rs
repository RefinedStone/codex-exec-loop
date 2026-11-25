//! Fixed Linear GraphQL documents for issue automation. Field names and filter syntax follow
//! Linear's public schema (`packages/sdk/src/schema.graphql`) and filtering/pagination docs.
use super::*;
use crate::application::port::outbound::linear_issue_api_port::*;
use crate::application::port::outbound::provider_credentials_port::ProviderCredentialLease;

const MAX_BODY_BYTES: usize = 64 * 1024;
const MAX_CURSOR_BYTES: usize = 2048;

const VIEWER: &str = "query AkraLinearIssueViewer { viewer { id active } }";
const TEAM: &str = "query AkraLinearIssueTeam($id: String!) { team(id: $id) { id } }";
const ISSUES: &str = concat!(
    "query AkraLinearIssues($filter: IssueFilter!, $first: Int!, $after: String) { ",
    "issues(first: $first, after: $after, orderBy: updatedAt, includeArchived: false, ",
    "filter: $filter) { nodes { id identifier title description url branchName updatedAt ",
    "state { id type } team { id } creator { id app } assignee { id app } ",
    "labels(first: 100) { nodes { id name } pageInfo { hasNextPage } } } ",
    "pageInfo { hasNextPage endCursor } } }"
);
const ISSUE: &str = concat!(
    "query AkraLinearIssue($id: String!) { issue(id: $id) { ",
    "id identifier title description url branchName updatedAt ",
    "state { id type } team { id } creator { id app } assignee { id app } ",
    "labels(first: 100) { nodes { id name } pageInfo { hasNextPage } } } }"
);
const COMMENTS: &str = concat!(
    "query AkraLinearComments($id: String!, $after: String) { issue(id: $id) { id ",
    "comments(first: 100, after: $after, orderBy: createdAt) { ",
    "nodes { id body user { id } issue { id } } pageInfo { hasNextPage endCursor } } } }"
);
const COMMENT: &str = "query AkraLinearComment($id: String!) { comment(id: $id) { id body user { id } issue { id } } }";
const HISTORY: &str = concat!(
    "query AkraLinearHistory($id: String!, $after: String) { issue(id: $id) { id ",
    "history(first: 100, after: $after, orderBy: createdAt) { ",
    "nodes { id createdAt fromStateId toStateId actorId } ",
    "pageInfo { hasNextPage endCursor } } } }"
);
const WORKFLOW_STATE: &str = "query AkraLinearWorkflowState($id: String!) { workflowState(id: $id) { id type team { id } } }";
const COMMENT_CREATE: &str = concat!(
    "mutation AkraLinearCommentCreate($issueId: String!, $body: String!) { ",
    "commentCreate(input: { issueId: $issueId, body: $body }) { ",
    "success comment { id body user { id } issue { id } } } }"
);
const COMMENT_UPDATE: &str = concat!(
    "mutation AkraLinearCommentUpdate($id: String!, $body: String!) { ",
    "commentUpdate(id: $id, input: { body: $body }) { ",
    "success comment { id body user { id } issue { id } } } }"
);
const STATE_UPDATE: &str = concat!(
    "mutation AkraLinearIssueState($id: String!, $stateId: String!) { ",
    "issueUpdate(id: $id, input: { stateId: $stateId }) { ",
    "success issue { id state { id type } } } }"
);

fn id(value: &str) -> CredentialResult<()> {
    if value.len() > 64 || !valid_linear_id(value) {
        return Err(ProviderCredentialError::InvalidRequest);
    }
    Ok(())
}

fn cursor(value: &Option<String>) -> CredentialResult<Value> {
    match value {
        None => Ok(Value::Null),
        Some(value)
            if !value.is_empty()
                && value.len() <= MAX_CURSOR_BYTES
                && !value.chars().any(char::is_control) =>
        {
            Ok(json!(value))
        }
        Some(_) => Err(ProviderCredentialError::InvalidRequest),
    }
}

fn timestamp(value: &str) -> CredentialResult<()> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|_| ())
        .map_err(|_| ProviderCredentialError::InvalidRequest)
}

fn body(value: &str) -> CredentialResult<()> {
    if value.is_empty() || value.len() > MAX_BODY_BYTES || value.contains('\0') {
        return Err(ProviderCredentialError::InvalidRequest);
    }
    Ok(())
}

/// Linear documents `IssueFilter.team.id`, `updatedAt` date comparators and
/// `state.type` string comparators. Completed, canceled and duplicate issues are not new work.
pub(super) fn issues_filter(team_id: &str, from: &str, before: &str) -> Value {
    json!({
        "team": {"id": {"eq": team_id}},
        "updatedAt": {"gte": from, "lt": before},
        "state": {"type": {"nin": ["completed", "canceled", "duplicate"]}},
    })
}

impl LinearIssueApiPort for ProviderHttpAdapter {
    fn read(
        &self,
        lease: &ProviderCredentialLease,
        request: &LinearIssueReadRequest,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<LinearIssueDocument> {
        if lease.reference.provider != Provider::Linear {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        let (query, variables) = match request {
            LinearIssueReadRequest::Viewer => (VIEWER, json!({})),
            LinearIssueReadRequest::Team { team_id } => {
                id(team_id)?;
                (TEAM, json!({"id": team_id}))
            }
            LinearIssueReadRequest::Issues {
                team_id,
                updated_from,
                updated_before,
                after,
                page_size,
            } => {
                id(team_id)?;
                timestamp(updated_from)?;
                timestamp(updated_before)?;
                if !(1..=100).contains(page_size) {
                    return Err(ProviderCredentialError::InvalidRequest);
                }
                (
                    ISSUES,
                    json!({
                        "filter": issues_filter(team_id, updated_from, updated_before),
                        "first": page_size,
                        "after": cursor(after)?,
                    }),
                )
            }
            LinearIssueReadRequest::Issue { issue_id } => {
                id(issue_id)?;
                (ISSUE, json!({"id": issue_id}))
            }
            LinearIssueReadRequest::Comments { issue_id, after } => {
                id(issue_id)?;
                (COMMENTS, json!({"id": issue_id, "after": cursor(after)?}))
            }
            LinearIssueReadRequest::Comment { comment_id } => {
                id(comment_id)?;
                (COMMENT, json!({"id": comment_id}))
            }
            LinearIssueReadRequest::History { issue_id, after } => {
                id(issue_id)?;
                (HISTORY, json!({"id": issue_id, "after": cursor(after)?}))
            }
            LinearIssueReadRequest::WorkflowState { state_id } => {
                id(state_id)?;
                (WORKFLOW_STATE, json!({"id": state_id}))
            }
        };
        self.linear_read(&lease.token, query, variables, cancelled, deadline)
            .map(LinearIssueDocument::new)
    }

    fn write(
        &self,
        lease: &ProviderCredentialLease,
        request: &LinearIssueWriteRequest,
        send_authorization: Arc<dyn ProviderMutationAuthorizationPort>,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> CredentialResult<LinearIssueDocument> {
        if lease.reference.provider != Provider::Linear {
            return Err(ProviderCredentialError::InvalidRequest);
        }
        let (query, variables) = match request {
            LinearIssueWriteRequest::CreateComment {
                issue_id,
                body: text,
            } => {
                id(issue_id)?;
                body(text)?;
                (COMMENT_CREATE, json!({"issueId": issue_id, "body": text}))
            }
            LinearIssueWriteRequest::UpdateComment {
                comment_id,
                body: text,
            } => {
                id(comment_id)?;
                body(text)?;
                (COMMENT_UPDATE, json!({"id": comment_id, "body": text}))
            }
            LinearIssueWriteRequest::UpdateState { issue_id, state_id } => {
                id(issue_id)?;
                id(state_id)?;
                (STATE_UPDATE, json!({"id": issue_id, "stateId": state_id}))
            }
        };
        self.linear_graphql(
            &lease.token,
            query,
            variables,
            Some(send_authorization),
            cancelled,
            deadline,
        )
        .map(LinearIssueDocument::new)
    }
}

#[cfg(test)]
mod tests;
