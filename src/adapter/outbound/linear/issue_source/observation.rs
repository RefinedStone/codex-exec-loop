use serde_json::Value;

use crate::domain::issue_automation::IssueAutomationSnapshot;
use crate::domain::issue_automation::source::*;
use crate::domain::issue_masking::{IssueMaskingBlocked, IssueTextField, plan_issue_masking};
use crate::domain::issue_secret_spans::secret_spans;
use crate::domain::issue_work::{IssueComparisonBasis, IssueFieldHash, IssueReference};
use crate::domain::provider_credentials::ConnectionRef;
use crate::domain::provider_work::{valid_linear_id, valid_linear_identifier};

const MAX_TITLE_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;

pub(super) fn observation(
    value: &Value,
    repository: &IssueSourceRepository,
    connection: &ConnectionRef,
    disclosure: &IssueSourceDisclosure,
) -> Result<IssueSourceObservation, IssueSourceFailure> {
    let id = stable_id(&value["id"])?;
    // Linear issues have no repository. Only the explicitly bound team is accepted.
    if stable_id(&value["team"]["id"])? != repository.repository_id {
        return Err(IssueSourceFailure::IdentityMismatch);
    }
    let identifier = text(&value["identifier"], 128)?;
    if !valid_linear_identifier(&identifier) {
        return Err(IssueSourceFailure::MalformedResponse);
    }
    let url = canonical_linear_issue_url(&text(&value["url"], 2048)?, &identifier)
        .ok_or(IssueSourceFailure::MalformedResponse)?;
    // Only a hint: a workspace branch format outside the Git-safe subset is dropped.
    let branch_name = value["branchName"]
        .as_str()
        .filter(|name| name.len() <= MAX_LINEAR_BRANCH_NAME_BYTES && valid_linear_branch_name(name))
        .map(str::to_owned);
    let updated = text(&value["updatedAt"], 64)?;
    chrono::DateTime::parse_from_rfc3339(&updated)
        .map_err(|_| IssueSourceFailure::MalformedResponse)?;
    let title = value["title"]
        .as_str()
        .filter(|title| title.len() <= MAX_TITLE_BYTES)
        .ok_or(IssueSourceFailure::MalformedResponse)?
        .to_owned();
    let mut body = match &value["description"] {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        _ => return Err(IssueSourceFailure::MalformedResponse),
    };
    let oversized = body.len() > MAX_BODY_BYTES;
    let basis = IssueComparisonBasis {
        field_hash: IssueFieldHash::of_fields(&title, &body),
        provider_updated_at: updated,
    };
    if oversized {
        // Keep the remote comparison hash but never persist or disclose the oversized body.
        body.clear();
    }
    let state_id = stable_id(&value["state"]["id"])?;
    let state_type = value["state"]["type"]
        .as_str()
        .and_then(LinearIssueStateType::parse)
        .ok_or(IssueSourceFailure::MalformedResponse)?;
    let issue = IssueReference {
        provider: repository.provider,
        server_id: repository.server_id.clone(),
        repository_id: repository.repository_id.clone(),
        issue_id: id,
        project_scope: repository.project_scope.clone(),
    };
    let content = if oversized {
        IssueSourceContent::Withheld {
            basis,
            policy_version: disclosure.policy_version.clone(),
            reason: IssueContentWithheldReason::UserConfirmationRequired,
        }
    } else {
        let mut spans = secret_spans(IssueTextField::Title, &title);
        spans.extend(secret_spans(IssueTextField::Body, &body));
        match plan_issue_masking(&title, &body, &spans, &disclosure.policy) {
            Ok(plan) => IssueSourceContent::Shared(IssueAutomationSnapshot {
                basis,
                shared_copy: plan.shared_copy,
                policy_version: disclosure.policy_version.clone(),
            }),
            Err(IssueMaskingBlocked::NeedsUserConfirmation { .. }) => {
                IssueSourceContent::Withheld {
                    basis,
                    policy_version: disclosure.policy_version.clone(),
                    reason: IssueContentWithheldReason::UserConfirmationRequired,
                }
            }
            Err(IssueMaskingBlocked::InvalidSpan { .. }) => {
                return Err(IssueSourceFailure::MalformedResponse);
            }
        }
    };
    let observation = IssueSourceObservation {
        issue,
        content_problems: if oversized {
            vec![IssueSourceContentProblem::OversizedContent]
        } else {
            vec![]
        },
        jira: None,
        linear: Some(IssueSourceLinearMetadata {
            identifier,
            url,
            state_id,
            state_type,
            branch_name,
            connection: connection.clone(),
        }),
        repository_visibility: IssueRepositoryVisibility::Unknown,
        repository_name_with_owner: None,
        web: None,
        remote_state: if state_type.closed() {
            IssueRemoteState::Closed
        } else {
            IssueRemoteState::Open
        },
        creation_provenance: IssueCreationProvenance::Unattributed,
        labels: labels(&value["labels"])?,
        assignees: IssueMetadataList {
            items: actor(&value["assignee"])?.into_iter().collect(),
            completeness: IssueMetadataCompleteness::Complete,
        },
        author: actor(&value["creator"])?,
        // A creator or assignee is not proof of who applied a selection label.
        label_actors: IssueMetadataList {
            items: vec![],
            completeness: IssueMetadataCompleteness::Unavailable,
        },
        // Linear attachments depend on an optional integration and cannot prove PR absence.
        linked_pull_request: IssuePrPresence::Unknown,
        content,
    };
    observation.validate_repository_metadata()?;
    Ok(observation)
}

fn labels(value: &Value) -> Result<IssueMetadataList<IssueSourceLabel>, IssueSourceFailure> {
    let nodes = value["nodes"]
        .as_array()
        .filter(|nodes| nodes.len() <= 100)
        .ok_or(IssueSourceFailure::MalformedResponse)?;
    let mut items = nodes
        .iter()
        .map(|node| {
            Ok(IssueSourceLabel {
                id: stable_id(&node["id"])?,
                name: text(&node["name"], 256)?,
            })
        })
        .collect::<Result<Vec<_>, IssueSourceFailure>>()?;
    items.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
    items.dedup();
    let truncated = value["pageInfo"]["hasNextPage"]
        .as_bool()
        .ok_or(IssueSourceFailure::MalformedResponse)?;
    Ok(IssueMetadataList {
        items,
        completeness: if truncated {
            IssueMetadataCompleteness::Truncated
        } else {
            IssueMetadataCompleteness::Complete
        },
    })
}

fn actor(value: &Value) -> Result<Option<IssueSourceActor>, IssueSourceFailure> {
    if value.is_null() {
        return Ok(None);
    }
    let id = stable_id(&value["id"])?;
    let kind = match value["app"].as_bool() {
        Some(false) => IssueSourceActorKind::UserAccount,
        Some(true) => IssueSourceActorKind::Bot,
        None => return Err(IssueSourceFailure::MalformedResponse),
    };
    Ok(Some(IssueSourceActor {
        login: id.clone(),
        id,
        kind,
    }))
}

pub(super) fn stable_id(value: &Value) -> Result<String, IssueSourceFailure> {
    let value = text(value, 64)?;
    if !valid_linear_id(&value) {
        return Err(IssueSourceFailure::MalformedResponse);
    }
    Ok(value)
}

pub(super) fn text(value: &Value, limit: usize) -> Result<String, IssueSourceFailure> {
    let value = value
        .as_str()
        .ok_or(IssueSourceFailure::MalformedResponse)?;
    if value.trim().is_empty() || value.len() > limit || value.chars().any(char::is_control) {
        return Err(IssueSourceFailure::MalformedResponse);
    }
    Ok(value.to_owned())
}
