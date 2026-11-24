use chrono::{DateTime, FixedOffset};
use serde_json::Value;

use crate::domain::issue_automation::IssueAutomationSnapshot;
use crate::domain::issue_automation::policy::IssueAutomationJiraBinding;
use crate::domain::issue_automation::source::*;
use crate::domain::issue_masking::{IssueMaskingBlocked, IssueTextField, plan_issue_masking};
use crate::domain::issue_secret_spans::secret_spans;
use crate::domain::issue_validation::IssueFieldFormat;
use crate::domain::issue_work::{IssueComparisonBasis, IssueFieldHash, IssueReference};
use crate::domain::provider_credentials::ConnectionRef;
use crate::domain::provider_work::valid_jira_key;

use super::super::adf::adf_to_markdown;
use super::super::issue_snapshot::{EditMeta, field_format};

pub(super) fn observation(
    value: Value,
    editmeta: Value,
    repository: &IssueSourceRepository,
    binding: &IssueAutomationJiraBinding,
    connection: &ConnectionRef,
    disclosure: &IssueSourceDisclosure,
) -> Result<IssueSourceObservation, IssueSourceFailure> {
    let id = numeric(&value["id"])?;
    let key = text(&value["key"], 60)?;
    if !valid_jira_key(&key) {
        return Err(IssueSourceFailure::MalformedResponse);
    }
    let fields = value["fields"]
        .as_object()
        .ok_or(IssueSourceFailure::MalformedResponse)?;
    if numeric(&value["fields"]["project"]["id"])? != repository.repository_id {
        return Err(IssueSourceFailure::IdentityMismatch);
    }
    numeric(&value["fields"]["issuetype"]["id"])?;
    let project_key = text(&value["fields"]["project"]["key"], 50)?;
    if key.split_once('-').map(|pair| pair.0) != Some(project_key.as_str()) {
        return Err(IssueSourceFailure::IdentityMismatch);
    }
    let updated = text(&value["fields"]["updated"], 64)?;
    timestamp(&updated)?;
    let title = value["fields"]["summary"]
        .as_str()
        .ok_or(IssueSourceFailure::MalformedResponse)?
        .to_owned();
    if title.len() > 64 * 1024 {
        return Err(IssueSourceFailure::MalformedResponse);
    }
    let description = fields
        .get("description")
        .ok_or(IssueSourceFailure::MalformedResponse)?;
    let (mut body, unsupported) = if description.is_null() {
        (String::new(), false)
    } else {
        let unsupported_shape = bounded_adf(description)?;
        let converted = adf_to_markdown(description);
        (
            converted.markdown,
            unsupported_shape || !converted.unsupported.is_empty(),
        )
    };
    let oversized = body.len() > 1024 * 1024;
    let remote_field_hash = IssueFieldHash::of_fields(&title, &body);
    let mut problems = if oversized {
        vec![IssueSourceContentProblem::OversizedContent]
    } else {
        required_problems(editmeta, fields, &body)?
    };
    if oversized {
        // Keep the remote comparison hash but never persist or disclose the oversized body.
        body.clear();
    }
    if unsupported {
        problems.push(IssueSourceContentProblem::UnsupportedAdf);
    }
    problems.sort();
    problems.dedup();
    let status_id = numeric(&value["fields"]["status"]["id"])?;
    let status_category = match value["fields"]["status"]["statusCategory"]["key"].as_str() {
        Some("new") => JiraIssueStatusCategory::New,
        Some("indeterminate") => JiraIssueStatusCategory::Indeterminate,
        Some("done") => JiraIssueStatusCategory::Done,
        _ => return Err(IssueSourceFailure::MalformedResponse),
    };
    let issue = IssueReference {
        provider: repository.provider,
        server_id: repository.server_id.clone(),
        repository_id: repository.repository_id.clone(),
        issue_id: id,
        project_scope: repository.project_scope.clone(),
    };
    let basis = IssueComparisonBasis {
        field_hash: remote_field_hash,
        provider_updated_at: updated,
    };
    let mut spans = secret_spans(IssueTextField::Title, &title);
    spans.extend(secret_spans(IssueTextField::Body, &body));
    let content = if oversized {
        IssueSourceContent::Withheld {
            basis,
            policy_version: disclosure.policy_version.clone(),
            reason: IssueContentWithheldReason::UserConfirmationRequired,
        }
    } else {
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
    let raw_labels = fields
        .get("labels")
        .and_then(Value::as_array)
        .ok_or(IssueSourceFailure::MalformedResponse)?;
    if raw_labels.len() > 100 {
        return Err(IssueSourceFailure::MetadataIncomplete);
    }
    let mut labels = raw_labels
        .iter()
        .map(|value| {
            let name = text(value, 256)?;
            // Jira labels have string identities, not invented numeric remote IDs.
            Ok(IssueSourceLabel {
                id: name.clone(),
                name,
            })
        })
        .collect::<Result<Vec<_>, IssueSourceFailure>>()?;
    labels.sort_by(|left, right| left.name.cmp(&right.name));
    labels.dedup();
    let assignee = actor(
        fields
            .get("assignee")
            .ok_or(IssueSourceFailure::MalformedResponse)?,
    )?;
    let author = actor(
        fields
            .get("creator")
            .ok_or(IssueSourceFailure::MalformedResponse)?,
    )?;
    let observation = IssueSourceObservation {
        issue,
        content_problems: problems,
        jira: Some(IssueSourceJiraMetadata {
            url: format!("{}/browse/{key}", binding.site_url),
            key,
            status_id,
            status_category,
            pull_request_lookup: JiraPullRequestLookup::Unsupported,
            connection: connection.clone(),
        }),
        repository_visibility: IssueRepositoryVisibility::Unknown,
        repository_name_with_owner: None,
        web: None,
        remote_state: if status_category == JiraIssueStatusCategory::Done {
            IssueRemoteState::Closed
        } else {
            IssueRemoteState::Open
        },
        creation_provenance: IssueCreationProvenance::Unattributed,
        labels: complete(labels),
        assignees: complete(assignee.into_iter().collect()),
        author,
        // A creator/current assignee is not proof of who applied a selection label.
        label_actors: IssueMetadataList {
            items: vec![],
            completeness: IssueMetadataCompleteness::Unavailable,
        },
        // Basic Cloud REST cannot establish complete development-panel PR absence.
        linked_pull_request: IssuePrPresence::Unknown,
        content,
    };
    observation.validate_repository_metadata()?;
    Ok(observation)
}

fn required_problems(
    value: Value,
    fields: &serde_json::Map<String, Value>,
    body: &str,
) -> Result<Vec<IssueSourceContentProblem>, IssueSourceFailure> {
    let meta: EditMeta =
        serde_json::from_value(value).map_err(|_| IssueSourceFailure::MalformedResponse)?;
    if meta.fields.len() > 256 {
        return Err(IssueSourceFailure::MetadataIncomplete);
    }
    let mut result = Vec::new();
    for (key, field) in meta.fields {
        if !field.required || matches!(key.as_str(), "project" | "issuetype") {
            continue;
        }
        if key == "description" {
            if body.trim().is_empty() {
                result.push(IssueSourceContentProblem::MissingRequiredField);
            }
            continue;
        }
        let format = field_format(&key, &field.schema, &field.allowed_values);
        if field.allowed_values.len() > 100 || format.is_err() {
            result.push(IssueSourceContentProblem::UnsupportedRequiredField);
            continue;
        }
        // Additional Jira fields are not silently assumed present from edit metadata alone.
        let Some(value) = fields.get(&key) else {
            result.push(IssueSourceContentProblem::UnsupportedRequiredField);
            continue;
        };
        if value.is_null()
            || value.as_str().is_some_and(|value| value.trim().is_empty())
            || value.as_array().is_some_and(Vec::is_empty)
        {
            result.push(IssueSourceContentProblem::MissingRequiredField);
            continue;
        }
        let supported = match format.expect("checked field format") {
            None => value.is_string(),
            Some(IssueFieldFormat::SingleLine) => value
                .as_str()
                .is_some_and(|value| !value.contains(['\n', '\r'])),
            Some(IssueFieldFormat::OneOf(options)) => value
                .get("value")
                .or_else(|| value.get("name"))
                .and_then(Value::as_str)
                .is_some_and(|value| options.iter().any(|option| option == value)),
        };
        if !supported {
            result.push(IssueSourceContentProblem::UnsupportedRequiredField);
        }
    }
    Ok(result)
}

fn bounded_adf(value: &Value) -> Result<bool, IssueSourceFailure> {
    let mut pending = vec![(value, 0usize)];
    let mut nodes = 0usize;
    let mut unsupported =
        value["type"].as_str() != Some("doc") || value["version"].as_u64() != Some(1);
    while let Some((node, depth)) = pending.pop() {
        nodes += 1;
        if depth > 64 || nodes > 20_000 {
            return Err(IssueSourceFailure::MalformedResponse);
        }
        match node {
            Value::Array(items) => pending.extend(items.iter().map(|item| (item, depth + 1))),
            Value::Object(fields) => {
                // The Markdown converter deliberately ignores some containers (e.g. code marks).
                // A nested unsupported node must still prevent execution readiness.
                if let Some(kind) = fields.get("type").and_then(Value::as_str) {
                    unsupported |= !matches!(
                        kind,
                        "doc"
                            | "paragraph"
                            | "heading"
                            | "bulletList"
                            | "orderedList"
                            | "listItem"
                            | "codeBlock"
                            | "text"
                            | "hardBreak"
                            | "strong"
                            | "em"
                            | "code"
                            | "strike"
                            | "link"
                    );
                }
                pending.extend(fields.values().map(|item| (item, depth + 1)));
            }
            _ => {}
        }
    }
    Ok(unsupported)
}

fn complete<T>(items: Vec<T>) -> IssueMetadataList<T> {
    IssueMetadataList {
        items,
        completeness: IssueMetadataCompleteness::Complete,
    }
}

fn actor(value: &Value) -> Result<Option<IssueSourceActor>, IssueSourceFailure> {
    if value.is_null() {
        return Ok(None);
    }
    let id = text(&value["accountId"], 256)?;
    let kind = match value["accountType"].as_str() {
        Some("atlassian") => IssueSourceActorKind::UserAccount,
        Some("app") => IssueSourceActorKind::Bot,
        _ => IssueSourceActorKind::Unknown,
    };
    Ok(Some(IssueSourceActor {
        login: id.clone(),
        id,
        kind,
    }))
}

pub(super) fn numeric(value: &Value) -> Result<String, IssueSourceFailure> {
    let value = text(value, 64)?;
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
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

fn timestamp(value: &str) -> Result<DateTime<FixedOffset>, IssueSourceFailure> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .or_else(|_| DateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f%z"))
        .map_err(|_| IssueSourceFailure::MalformedResponse)?;
    if parsed.timestamp_millis() < 0 {
        return Err(IssueSourceFailure::MalformedResponse);
    }
    Ok(parsed)
}
