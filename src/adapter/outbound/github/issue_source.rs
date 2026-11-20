//! GitHub 이슈를 발견하고 안정 ID로 다시 읽는 무상태 adapter다 (#2537).
//! 원문은 이 경계에서 해시와 공유본으로 바뀐다. 댓글과 원격 변경은 요청하지 않는다.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use chrono::{DateTime, FixedOffset};
use serde::de::DeserializeOwned;

use super::issue_snapshot::{GhApiRunner, ensure_node_id, graphql_arguments, run_gh_api};
use super::resource_version::GITHUB_HOSTNAME;
use crate::application::port::outbound::issue_source_port::{
    IssueDiscoveryPage, IssueDiscoveryRequest, IssueDiscoveryTarget, IssueResyncRequest,
    IssueSourceCapabilities, IssueSourcePort,
};
use crate::domain::issue_automation::IssueAutomationSnapshot;
use crate::domain::issue_automation::source::{
    IssueContentWithheldReason, IssueCreationProvenance, IssueDiscoveryCursor,
    IssueLabelAttribution, IssueMetadataCompleteness, IssueMetadataList, IssuePrPresence,
    IssueRemoteState, IssueSourceActor, IssueSourceActorKind, IssueSourceContent,
    IssueSourceDisclosure, IssueSourceFailure, IssueSourceLabel, IssueSourceObservation,
    IssueSourceRead, IssueSourceRepository, IssueSourceWeb,
};
use crate::domain::issue_masking::{IssueMaskingBlocked, IssueTextField, plan_issue_masking};
use crate::domain::issue_secret_spans::secret_spans;
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueFieldHash, IssueProvider, IssueReference,
};

mod graphql;
use graphql::{
    Actor, AuthorField, DiscoveryData, Envelope, IssueNode, Label, LabelEvent, MetadataConnection,
    MetadataPageInfo, ReadData, RepositoryNode,
};

const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_METADATA_ITEMS: usize = 100;

pub struct GithubIssueSourceAdapter {
    run_gh_api: GhApiRunner,
}

impl GithubIssueSourceAdapter {
    /// 실행 파일과 자격 증명 환경, 중립 작업 폴더, 호출당 15초 제한은 snapshot 경계를 재사용한다.
    pub fn new(working_directory: PathBuf) -> Self {
        Self::with_runner(Box::new(move |arguments| {
            run_gh_api(&working_directory, arguments)
        }))
    }

    fn with_runner(run_gh_api: GhApiRunner) -> Self {
        Self { run_gh_api }
    }

    fn query<T: DeserializeOwned>(&self, arguments: &[String]) -> Result<T, IssueSourceFailure> {
        let stdout =
            (self.run_gh_api)(arguments).map_err(|_| IssueSourceFailure::ProviderUnavailable)?;
        if stdout.len() > MAX_RESPONSE_BYTES {
            return Err(IssueSourceFailure::MalformedResponse);
        }
        // GraphQL은 부분 data와 errors를 함께 줄 수 있다. 오류 원문을 저장하거나 반환하지 않는다.
        let envelope: Envelope<T> =
            serde_json::from_slice(&stdout).map_err(|_| IssueSourceFailure::MalformedResponse)?;
        if envelope.errors.is_some_and(|errors| !errors.is_empty()) {
            return Err(IssueSourceFailure::ProviderUnavailable);
        }
        envelope.data.ok_or(IssueSourceFailure::ProviderUnavailable)
    }

    fn read_repository(
        &self,
        request: &IssueDiscoveryRequest,
    ) -> Result<RepositoryNode, IssueSourceFailure> {
        let (mut arguments, stable) = match &request.target {
            IssueDiscoveryTarget::GitHubLocator {
                owner,
                name,
                expected_repository_id,
                ..
            } => {
                if let Some(expected) = expected_repository_id {
                    ensure_node_id(expected).map_err(|_| IssueSourceFailure::InvalidRequest)?;
                }
                (
                    graphql_arguments(
                        &graphql::discovery_query(false),
                        &[("owner", owner), ("name", name)],
                        None,
                    ),
                    false,
                )
            }
            IssueDiscoveryTarget::Repository(repository) => {
                validate_github_repository(repository)?;
                let mut arguments = graphql_arguments(
                    &graphql::discovery_query(true),
                    &[("id", &repository.repository_id)],
                    None,
                );
                if let Some(cursor) = &request.cursor {
                    let opaque = cursor.validate_for(
                        repository,
                        &request.selection,
                        request.page_size,
                        &request.disclosure,
                    )?;
                    arguments.extend(["-f".to_string(), format!("after={opaque}")]);
                }
                (arguments, true)
            }
        };
        arguments.extend(["-F".to_string(), format!("first={}", request.page_size)]);
        let data: DiscoveryData = self.query(&arguments)?;
        if stable { data.node } else { data.repository }
            .ok_or(IssueSourceFailure::ProviderUnavailable)
    }

    fn read_issue(
        &self,
        request: &IssueResyncRequest,
    ) -> Result<IssueSourceObservation, IssueSourceFailure> {
        request.validate()?;
        let repository = IssueSourceRepository::from_reference(&request.issue);
        validate_github_repository(&repository)?;
        ensure_node_id(&request.issue.issue_id).map_err(|_| IssueSourceFailure::InvalidRequest)?;
        let data: ReadData = self.query(&graphql_arguments(
            &graphql::resync_query(),
            &[("id", &request.issue.issue_id)],
            None,
        ))?;
        let node = data.node.ok_or(IssueSourceFailure::ProviderUnavailable)?;
        let observation = observation(node, &repository, &request.disclosure)?;
        if observation.issue != request.issue {
            return Err(IssueSourceFailure::IdentityMismatch);
        }
        Ok(observation)
    }
}

impl IssueSourcePort for GithubIssueSourceAdapter {
    fn capabilities(&self) -> IssueSourceCapabilities {
        IssueSourceCapabilities::read_only()
    }

    fn discover(
        &self,
        request: IssueDiscoveryRequest,
    ) -> Result<IssueDiscoveryPage, IssueSourceFailure> {
        request.validate()?;
        let node = self.read_repository(&request)?;
        if node.typename != "Repository" {
            return Err(IssueSourceFailure::IdentityMismatch);
        }
        let id = node.id.ok_or(IssueSourceFailure::MalformedResponse)?;
        ensure_node_id(&id).map_err(|_| IssueSourceFailure::MalformedResponse)?;
        let (scope, expected) = match &request.target {
            IssueDiscoveryTarget::GitHubLocator {
                project_scope,
                expected_repository_id,
                ..
            } => (project_scope.clone(), expected_repository_id.as_deref()),
            IssueDiscoveryTarget::Repository(repository) => (
                repository.project_scope.clone(),
                Some(repository.repository_id.as_str()),
            ),
        };
        if expected.is_some_and(|expected| expected != id) {
            return Err(IssueSourceFailure::IdentityMismatch);
        }
        let repository = IssueSourceRepository {
            provider: IssueProvider::GitHub,
            server_id: GITHUB_HOSTNAME.to_string(),
            repository_id: id,
            project_scope: scope,
        };
        let connection = node.issues.ok_or(IssueSourceFailure::MalformedResponse)?;
        if connection.nodes.len() > usize::from(request.page_size)
            || (connection.nodes.is_empty() && connection.page_info.has_next_page)
        {
            return Err(IssueSourceFailure::MalformedResponse);
        }
        let scanned_count = connection.nodes.len() as u16;
        let mut observations = Vec::new();
        let mut seen = HashSet::new();
        let mut previous_updated_at = None;
        for node in connection.nodes {
            let node = node.ok_or(IssueSourceFailure::MalformedResponse)?;
            let updated_at = parse_time(
                node.updated_at
                    .as_deref()
                    .ok_or(IssueSourceFailure::MalformedResponse)?,
            )?;
            if previous_updated_at.is_some_and(|previous| previous > updated_at) {
                return Err(IssueSourceFailure::MalformedResponse);
            }
            previous_updated_at = Some(updated_at);
            let item = observation(node, &repository, &request.disclosure)?;
            if !seen.insert(item.issue.issue_id.clone()) {
                return Err(IssueSourceFailure::MalformedResponse);
            }
            // 조회 중 닫힌 이슈는 새 발견에서 제외한다. 이미 추적한 이슈의 닫힘은 resync가 읽는다.
            if item.remote_state == IssueRemoteState::Open
                && request.selection.matches_labels(&item.labels)?
            {
                observations.push(item);
            }
        }
        let next_cursor = if connection.page_info.has_next_page {
            let opaque = connection
                .page_info
                .end_cursor
                .ok_or(IssueSourceFailure::MalformedResponse)?;
            let current = request
                .cursor
                .as_ref()
                .map(|cursor| {
                    cursor.validate_for(
                        &repository,
                        &request.selection,
                        request.page_size,
                        &request.disclosure,
                    )
                })
                .transpose()?;
            if current == Some(opaque.as_str()) {
                return Err(IssueSourceFailure::MalformedResponse);
            }
            Some(
                IssueDiscoveryCursor::new(
                    repository.clone(),
                    request.selection,
                    request.page_size,
                    &request.disclosure,
                    opaque,
                )
                .map_err(|_| IssueSourceFailure::MalformedResponse)?,
            )
        } else {
            None
        };
        Ok(IssueDiscoveryPage {
            repository,
            observations,
            next_cursor,
            scanned_count,
        })
    }

    fn resync(&self, request: IssueResyncRequest) -> IssueSourceRead {
        match self.read_issue(&request) {
            Ok(observation) => IssueSourceRead::Observed(Box::new(observation)),
            Err(reason) => IssueSourceRead::Unknown {
                issue: request.issue,
                reason,
            },
        }
    }
}

fn validate_github_repository(
    repository: &IssueSourceRepository,
) -> Result<(), IssueSourceFailure> {
    repository.validate()?;
    if repository.provider != IssueProvider::GitHub || repository.server_id != GITHUB_HOSTNAME {
        return Err(IssueSourceFailure::UnsupportedProvider);
    }
    ensure_node_id(&repository.repository_id).map_err(|_| IssueSourceFailure::InvalidRequest)
}

fn parse_time(value: &str) -> Result<DateTime<FixedOffset>, IssueSourceFailure> {
    if value.len() > 40 {
        return Err(IssueSourceFailure::MalformedResponse);
    }
    DateTime::parse_from_rfc3339(value).map_err(|_| IssueSourceFailure::MalformedResponse)
}

fn observation(
    node: IssueNode,
    repository: &IssueSourceRepository,
    disclosure: &IssueSourceDisclosure,
) -> Result<IssueSourceObservation, IssueSourceFailure> {
    if node.typename != "Issue" {
        return Err(IssueSourceFailure::IdentityMismatch);
    }
    let id = node.id.ok_or(IssueSourceFailure::MalformedResponse)?;
    ensure_node_id(&id).map_err(|_| IssueSourceFailure::MalformedResponse)?;
    let remote_repository = node
        .repository
        .ok_or(IssueSourceFailure::MalformedResponse)?;
    if remote_repository.id != repository.repository_id {
        return Err(IssueSourceFailure::IdentityMismatch);
    }
    let issue = IssueReference {
        provider: repository.provider,
        server_id: repository.server_id.clone(),
        repository_id: repository.repository_id.clone(),
        issue_id: id,
        project_scope: repository.project_scope.clone(),
    };
    let web = IssueSourceWeb {
        number: node.number.ok_or(IssueSourceFailure::MalformedResponse)?,
        url: node.url.ok_or(IssueSourceFailure::MalformedResponse)?,
    };
    web.validate_for(&issue)
        .map_err(|_| IssueSourceFailure::MalformedResponse)?;
    if web.url
        != format!(
            "https://github.com/{}/issues/{}",
            remote_repository.name_with_owner, web.number
        )
    {
        return Err(IssueSourceFailure::IdentityMismatch);
    }
    let remote_state = match node.state.as_deref() {
        Some("OPEN") => IssueRemoteState::Open,
        Some("CLOSED") => IssueRemoteState::Closed,
        _ => return Err(IssueSourceFailure::MalformedResponse),
    };
    let updated_at = node
        .updated_at
        .ok_or(IssueSourceFailure::MalformedResponse)?;
    parse_time(&updated_at)?;
    let title = node.title.ok_or(IssueSourceFailure::MalformedResponse)?;
    let body = node.body.ok_or(IssueSourceFailure::MalformedResponse)?;
    let basis = IssueComparisonBasis {
        field_hash: IssueFieldHash::of_fields(&title, &body),
        provider_updated_at: updated_at,
    };
    let mut spans = secret_spans(IssueTextField::Title, &title);
    spans.extend(secret_spans(IssueTextField::Body, &body));
    let content = match plan_issue_masking(&title, &body, &spans, &disclosure.policy) {
        Ok(plan) => IssueSourceContent::Shared(IssueAutomationSnapshot {
            basis,
            shared_copy: plan.shared_copy,
            policy_version: disclosure.policy_version.clone(),
        }),
        Err(IssueMaskingBlocked::NeedsUserConfirmation { .. }) => IssueSourceContent::Withheld {
            basis,
            policy_version: disclosure.policy_version.clone(),
            reason: IssueContentWithheldReason::UserConfirmationRequired,
        },
        Err(IssueMaskingBlocked::InvalidSpan { .. }) => {
            return Err(IssueSourceFailure::MalformedResponse);
        }
    };
    let labels = metadata(node.labels, label)?;
    let assignees = metadata(node.assignees, actor)?;
    let author = observed_actor(node.author)?;
    let label_actors = label_attributions(node.timeline_items, &labels)?;
    let linked_pull_request = match node
        .closed_by_pull_requests_references
        .and_then(|value| value.total_count)
    {
        Some(0) => IssuePrPresence::Absent,
        Some(_) => IssuePrPresence::Present,
        None => IssuePrPresence::Unknown,
    };
    Ok(IssueSourceObservation {
        issue,
        web: Some(web),
        remote_state,
        creation_provenance: IssueCreationProvenance::Unattributed,
        labels,
        assignees,
        author,
        label_actors,
        linked_pull_request,
        content,
    })
}

fn observed_actor(value: AuthorField) -> Result<Option<IssueSourceActor>, IssueSourceFailure> {
    match value {
        AuthorField::Missing => Err(IssueSourceFailure::MalformedResponse),
        AuthorField::Present(Some(value)) => actor(value)
            .map(Some)
            .ok_or(IssueSourceFailure::MalformedResponse),
        AuthorField::Present(None) => Ok(None),
    }
}

fn actor(value: Actor) -> Option<IssueSourceActor> {
    let id = value.id?;
    let login = value.login?;
    if ensure_node_id(&id).is_err()
        || login.is_empty()
        || login.len() > 100
        || !login
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.[]".contains(&byte))
    {
        return None;
    }
    let kind = match value.typename.as_deref() {
        Some("User") => IssueSourceActorKind::UserAccount,
        Some("Bot") => IssueSourceActorKind::Bot,
        _ => IssueSourceActorKind::Unknown,
    };
    Some(IssueSourceActor { id, login, kind })
}

fn label(value: Label) -> Option<IssueSourceLabel> {
    if ensure_node_id(&value.id).is_err()
        || value.name.trim().is_empty()
        || value.name.len() > 1024
        || value.name.chars().any(char::is_control)
    {
        return None;
    }
    Some(IssueSourceLabel {
        id: value.id,
        name: value.name,
    })
}

fn completeness(info: &MetadataPageInfo) -> IssueMetadataCompleteness {
    if info.has_next_page || info.has_previous_page {
        IssueMetadataCompleteness::Truncated
    } else {
        IssueMetadataCompleteness::Complete
    }
}

fn metadata<T, U>(
    connection: Option<MetadataConnection<T>>,
    mut convert: impl FnMut(T) -> Option<U>,
) -> Result<IssueMetadataList<U>, IssueSourceFailure> {
    let Some(connection) = connection else {
        return Ok(IssueMetadataList {
            items: Vec::new(),
            completeness: IssueMetadataCompleteness::Unavailable,
        });
    };
    if connection.nodes.len() > MAX_METADATA_ITEMS {
        return Err(IssueSourceFailure::MalformedResponse);
    }
    let mut status = completeness(&connection.page_info);
    let mut items = Vec::new();
    for item in connection.nodes {
        match item.and_then(&mut convert) {
            Some(item) => items.push(item),
            None => status = IssueMetadataCompleteness::Unavailable,
        }
    }
    Ok(IssueMetadataList {
        items,
        completeness: status,
    })
}

fn label_attributions(
    connection: Option<MetadataConnection<LabelEvent>>,
    labels: &IssueMetadataList<IssueSourceLabel>,
) -> Result<IssueMetadataList<IssueLabelAttribution>, IssueSourceFailure> {
    let Some(connection) = connection else {
        return Ok(IssueMetadataList {
            items: Vec::new(),
            completeness: IssueMetadataCompleteness::Unavailable,
        });
    };
    if connection.nodes.len() > MAX_METADATA_ITEMS {
        return Err(IssueSourceFailure::MalformedResponse);
    }
    // last 페이지의 뒤쪽이 잘렸거나 null event가 있으면 최신 행위자를 확정할 수 없다.
    if connection.page_info.has_next_page
        || connection.nodes.iter().any(Option::is_none)
        || labels.completeness == IssueMetadataCompleteness::Unavailable
    {
        return Ok(IssueMetadataList {
            items: Vec::new(),
            completeness: IssueMetadataCompleteness::Unavailable,
        });
    }
    let current: HashMap<_, _> = labels
        .items
        .iter()
        .map(|label| (label.id.as_str(), label))
        .collect();
    let mut status = completeness(&connection.page_info);
    if labels.completeness == IssueMetadataCompleteness::Truncated {
        status = IssueMetadataCompleteness::Truncated;
    }
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    for event in connection.nodes.into_iter().rev().flatten() {
        let Some(event_label) = event.label.and_then(label) else {
            return Ok(IssueMetadataList {
                items: Vec::new(),
                completeness: IssueMetadataCompleteness::Unavailable,
            });
        };
        let Some(current_label) = current.get(event_label.id.as_str()) else {
            continue;
        };
        if !seen.insert(event_label.id.clone()) {
            continue;
        }
        let event_id = event.id.filter(|id| ensure_node_id(id).is_ok());
        let created_at = event.created_at.filter(|time| parse_time(time).is_ok());
        if event.typename != "LabeledEvent"
            || event_label != **current_label
            || event_id.is_none()
            || created_at.is_none()
        {
            status = IssueMetadataCompleteness::Unavailable;
            continue;
        }
        let actor = match observed_actor(event.actor) {
            Ok(actor) => actor,
            Err(_) => {
                status = IssueMetadataCompleteness::Unavailable;
                continue;
            }
        };
        items.push(IssueLabelAttribution {
            label: event_label,
            event_id: event_id.expect("checked event ID"),
            actor,
            created_at: created_at.expect("checked event time"),
        });
    }
    if seen.len() < current.len() && status == IssueMetadataCompleteness::Complete {
        status = IssueMetadataCompleteness::Unavailable;
    }
    items.sort_by(|left, right| left.label.id.cmp(&right.label.id));
    Ok(IssueMetadataList {
        items,
        completeness: status,
    })
}

#[cfg(test)]
mod tests;
