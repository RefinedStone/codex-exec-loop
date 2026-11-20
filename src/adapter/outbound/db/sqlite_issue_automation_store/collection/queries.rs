use anyhow::{Context, Result, bail};
use rusqlite::{Connection, named_params};

use crate::domain::issue_automation::collection::{
    IssueCollectionItem, IssueCollectionPage, IssueCollectionQuery, IssueResyncBatch,
    IssueResyncCursor, IssueTrackedSummary, MAX_ISSUE_COLLECTION_PAGE_SIZE, StoredIssueCollection,
};
use crate::domain::issue_automation::{IssueAutomationBinding, IssueAutomationBindingId};
use crate::domain::issue_work::{IssueProjectScope, IssueReference};

use super::super::{decode_enum, enum_name, read_binding, read_issue, validate_reference_scope};
use super::observations;

type IssueKey = (String, String, String, String, String);

pub(super) fn key(reference: &IssueReference) -> Result<IssueKey> {
    Ok((
        enum_name(&reference.provider)?,
        reference.server_id.clone(),
        reference.repository_id.clone(),
        reference.issue_id.clone(),
        reference.project_scope.workspace_dir.clone(),
    ))
}

pub(super) fn validate_limit(limit: u16) -> Result<()> {
    if limit == 0 || limit > MAX_ISSUE_COLLECTION_PAGE_SIZE {
        bail!("the issue collection page limit is invalid")
    }
    Ok(())
}

pub(in super::super) fn page(
    connection: &Connection,
    workspace: &str,
    query: &IssueCollectionQuery,
) -> Result<IssueCollectionPage> {
    let revision = super::super::triage::catalog_revision(connection, workspace)?;
    let mut references = select_references(
        connection,
        workspace,
        &CatalogQuery {
            binding_id: query.binding_id.as_ref(),
            after: query.after.as_ref(),
            through: None,
            unfinished: false,
            descending: false,
            limit: query.limit + 1,
        },
    )?;
    let has_more = references.len() > usize::from(query.limit);
    references.truncate(usize::from(query.limit));
    let mut items = Vec::with_capacity(references.len());
    for reference in references {
        items.push(
            item(connection, workspace, &reference)?
                .context("the selected issue disappeared from the collection")?,
        );
    }
    let next_after = has_more
        .then(|| items.last().map(|item| item.issue.clone()))
        .flatten();
    Ok(IssueCollectionPage {
        revision,
        items,
        next_after,
        report: None,
    })
}

pub(in super::super) fn item(
    connection: &Connection,
    workspace: &str,
    reference: &IssueReference,
) -> Result<Option<IssueCollectionItem>> {
    validate_reference_scope(workspace, reference)?;
    let tracked = read_issue(connection, workspace, reference)?;
    let observation = observations::read(connection, workspace, reference)?;
    if tracked.is_none() && observation.is_none() {
        return Ok(None);
    }
    if let (Some(tracked), Some(observation)) = (&tracked, &observation)
        && *tracked.binding_id() != observation.binding_id
    {
        bail!("the current issue observation and workflow row have different bindings")
    }
    let binding_id = tracked
        .as_ref()
        .map(|issue| issue.binding_id().clone())
        .or_else(|| observation.as_ref().map(|item| item.binding_id.clone()))
        .context("the selected issue disappeared from the collection")?;
    let observation_revision = observation.as_ref().map(|item| item.revision);
    let tracked_revision = tracked.as_ref().map(|issue| issue.revision());
    let triage = super::super::triage::read(connection, workspace, reference, &binding_id)?.map(
        |mut triage| {
            triage.stale = triage.observation_revision != observation_revision
                || triage.tracked_revision != tracked_revision;
            triage
        },
    );
    Ok(Some(IssueCollectionItem {
        issue: reference.clone(),
        binding_id,
        tracked: tracked.map(|issue| IssueTrackedSummary {
            state: issue.state(),
            flags: issue.flags(),
        }),
        current: observation.map(|item| item.current),
        observation_revision,
        tracked_revision,
        triage,
    }))
}

pub(super) fn resync_batch(
    connection: &Connection,
    workspace: &str,
    state: &StoredIssueCollection,
    limit: u16,
) -> Result<IssueResyncBatch> {
    validate_limit(limit)?;
    let (after, through) = if let Some(cursor) = &state.resync_cursor {
        (Some(cursor.after.clone()), Some(cursor.through.clone()))
    } else {
        let last = select_references(
            connection,
            workspace,
            &CatalogQuery {
                binding_id: None,
                after: None,
                through: None,
                unfinished: true,
                descending: true,
                limit: 1,
            },
        )?
        .pop();
        (None, last)
    };
    let Some(through) = through else {
        return Ok(IssueResyncBatch {
            expected: state.fence(),
            targets: Vec::new(),
            next_cursor: None,
        });
    };
    let mut targets = select_references(
        connection,
        workspace,
        &CatalogQuery {
            binding_id: None,
            after: after.as_ref(),
            through: Some(&through),
            unfinished: true,
            descending: false,
            limit: limit + 1,
        },
    )?;
    let has_more = targets.len() > usize::from(limit);
    targets.truncate(usize::from(limit));
    let next_cursor = if has_more {
        targets.last().map(|after| IssueResyncCursor {
            after: after.clone(),
            through,
        })
    } else {
        None
    };
    Ok(IssueResyncBatch {
        expected: state.fence(),
        targets,
        next_cursor,
    })
}

pub(super) fn binding_for_reference(
    connection: &Connection,
    workspace: &str,
    reference: &IssueReference,
) -> Result<IssueAutomationBinding> {
    let tracked = read_issue(connection, workspace, reference)?;
    let observation = observations::read(connection, workspace, reference)?;
    if let (Some(tracked), Some(observation)) = (&tracked, &observation)
        && *tracked.binding_id() != observation.binding_id
    {
        bail!("the current issue observation and workflow row have different bindings")
    }
    let binding_id = tracked
        .as_ref()
        .map(|issue| issue.binding_id().clone())
        .or_else(|| observation.as_ref().map(|item| item.binding_id.clone()))
        .context("the issue resync target has not been tracked")?;
    Ok(read_binding(connection, workspace, &binding_id)?
        .context("the issue resync target binding is missing")?
        .binding)
}

struct CatalogQuery<'a> {
    binding_id: Option<&'a IssueAutomationBindingId>,
    after: Option<&'a IssueReference>,
    through: Option<&'a IssueReference>,
    unfinished: bool,
    descending: bool,
    limit: u16,
}

fn select_references(
    connection: &Connection,
    workspace: &str,
    query: &CatalogQuery<'_>,
) -> Result<Vec<IssueReference>> {
    for reference in [query.after, query.through].into_iter().flatten() {
        validate_reference_scope(workspace, reference)?;
    }
    let after = query.after.map(key).transpose()?;
    let through = query.through.map(key).transpose()?;
    let order = if query.descending { "DESC" } else { "ASC" };
    let sql = format!(
        "WITH issue_keys AS (
             SELECT provider, server_id, repository_id, issue_id, workspace_dir
             FROM tracked_issues WHERE workspace_dir = :workspace
             UNION
             SELECT provider, server_id, repository_id, issue_id, workspace_dir
             FROM issue_source_observations WHERE workspace_dir = :workspace
         )
         SELECT k.provider, k.server_id, k.repository_id, k.issue_id, k.workspace_dir
         FROM issue_keys k
         LEFT JOIN tracked_issues t USING (provider, server_id, repository_id, issue_id, workspace_dir)
         LEFT JOIN issue_source_observations o USING (provider, server_id, repository_id, issue_id, workspace_dir)
         WHERE (:binding IS NULL OR COALESCE(t.binding_id, o.binding_id) = :binding)
           AND (:after_provider IS NULL OR
                (k.provider, k.server_id, k.repository_id, k.issue_id, k.workspace_dir) >
                (:after_provider, :after_server, :after_repository, :after_issue, :after_workspace))
           AND (:through_provider IS NULL OR
                (k.provider, k.server_id, k.repository_id, k.issue_id, k.workspace_dir) <=
                (:through_provider, :through_server, :through_repository, :through_issue, :through_workspace))
           AND (:unfinished = 0 OR t.state IS NULL OR t.state NOT IN ('done', 'cancelled'))
         ORDER BY k.provider COLLATE BINARY {order}, k.server_id COLLATE BINARY {order},
                  k.repository_id COLLATE BINARY {order}, k.issue_id COLLATE BINARY {order},
                  k.workspace_dir COLLATE BINARY {order}
         LIMIT :limit",
    );
    let mut statement = connection.prepare(&sql)?;
    let mut rows = statement.query(named_params! {
        ":workspace": workspace,
        ":binding": query.binding_id.map(IssueAutomationBindingId::as_str),
        ":after_provider": after.as_ref().map(|value| &value.0),
        ":after_server": after.as_ref().map(|value| &value.1),
        ":after_repository": after.as_ref().map(|value| &value.2),
        ":after_issue": after.as_ref().map(|value| &value.3),
        ":after_workspace": after.as_ref().map(|value| &value.4),
        ":through_provider": through.as_ref().map(|value| &value.0),
        ":through_server": through.as_ref().map(|value| &value.1),
        ":through_repository": through.as_ref().map(|value| &value.2),
        ":through_issue": through.as_ref().map(|value| &value.3),
        ":through_workspace": through.as_ref().map(|value| &value.4),
        ":unfinished": i64::from(query.unfinished),
        ":limit": i64::from(query.limit),
    })?;
    let mut references = Vec::new();
    while let Some(row) = rows.next()? {
        let reference = IssueReference {
            provider: decode_enum(row.get(0)?)?,
            server_id: row.get(1)?,
            repository_id: row.get(2)?,
            issue_id: row.get(3)?,
            project_scope: IssueProjectScope {
                workspace_dir: row.get(4)?,
            },
        };
        validate_reference_scope(workspace, &reference)?;
        references.push(reference);
    }
    Ok(references)
}
