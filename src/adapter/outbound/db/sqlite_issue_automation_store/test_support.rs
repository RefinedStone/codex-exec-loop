//! 실제 저장 API로 issue lineage를 준비한다. 임의 work ID 또는 lookup 예외는 만들지 않는다.
use super::*;
use crate::application::port::outbound::issue_automation_store_port::{
    IssueAutomationStorePort, IssueTriageFence, IssueTriageSaveResult, IssueWorkAdmissionResult,
    IssueWorkSaveResult,
};
#[cfg(test)]
use crate::application::service::issue_automation_policy::resolve_issue_automation_policy;
#[cfg(test)]
use crate::application::service::issue_policy::issue_policy_version;
use crate::domain::issue_automation::autonomy::{IssueAutonomyIntent, IssueAutonomyLevel};
use crate::domain::issue_automation::collection::IssueCollectionItem;
use crate::domain::issue_automation::coordinator::{
    IssueWorkCheckpoint, IssueWorkOrder, IssueWorkStartAuthority, issue_work_order_id,
};
use crate::domain::issue_automation::decision::IssueDecisionBasis;
use crate::domain::issue_automation::policy::IssueAutomationGate;
use crate::domain::issue_automation::source::*;
use crate::domain::issue_automation::triage::{
    IssueTriageInput, issue_acceptance_criteria, triage_issue,
};
use crate::domain::issue_automation::{IssueTransition, IssueTriageResult};
use crate::domain::issue_policy::IssuePolicy;
use crate::domain::issue_validation::IssueRequirements;
use sha2::{Digest, Sha256};

const POLICY: &str = r#"
[binding.github]
mode = "gated"
[authority]
autonomy_cap = "gated"
[select]
mode = "all_open"
trusted_actors = [{provider="github",server_id="github.com",actor_id="U_fixture"}]
[gates.gated.normal]
start = "auto"
[budget]
max_running = 64
max_started_per_day = 256
max_attempts = 3
"#;

/// 반환 work ID를 그대로 planning 등록에 사용한다. task_ids는 같은 fixture가 실제 등록할 ID다.
pub(crate) fn seed_issue_work(
    adapter: &SqliteIssueAutomationStoreAdapter,
    issue: &IssueReference,
    task_ids: &[String],
) -> Result<IssueWorkCheckpoint> {
    let workspace = &issue.project_scope.workspace_dir;
    let policy = resolve_issue_automation_policy(Some(POLICY), None)?;
    let control = adapter.ensure_collection(workspace)?;
    let selection = adapter.issue_autonomy_selection(workspace)?;
    if selection.requested_level.is_none() {
        let intent = IssueAutonomyIntent {
            command_id: "fixture-lineage-gated".into(),
            level: IssueAutonomyLevel::Gated,
            expected_revision: selection.revision,
            policy_version: policy.version.clone(),
        };
        adapter.begin_issue_autonomy(workspace, &intent)?;
        adapter.finish_issue_autonomy(workspace, &intent)?;
    }
    let selection = adapter.issue_autonomy_selection(workspace)?;
    let binding = binding_for_issue(issue)?;
    adapter.insert_binding(workspace, &binding)?;
    let observation = observation(issue);
    let IssueSourceContent::Shared(snapshot) = &observation.content else {
        unreachable!()
    };
    let tracked = TrackedIssue::discovered(issue.clone(), &binding, snapshot.clone())?;
    let tracked = adapter.insert_issue(workspace, &tracked)?.issue;
    adapter.apply_issue_resync(
        workspace,
        control.fence(),
        issue,
        &IssueSourceRead::Observed(Box::new(observation.clone())),
    )?;
    let mut tracked = adapter
        .get_issue(workspace, tracked.reference())?
        .ok_or_else(|| anyhow::anyhow!("fixture issue disappeared"))?;
    tracked.transition(IssueTransition::Assess(IssueTriageResult::Ready))?;
    if !matches!(
        adapter.save_issue(workspace, tracked.revision(), &tracked)?,
        IssueAutomationIssueSaveResult::Saved { .. }
    ) {
        bail!("fixture issue readiness was not saved");
    }
    let (fence, item) = subject(adapter, issue)?;
    let assessment = triage_issue(IssueTriageInput {
        runtime_selection_revision: selection.revision,
        issue,
        current: item.current.as_ref(),
        policy: &policy.policy,
        automation_policy_version: &policy.version,
        requirements: &IssueRequirements::default(),
        validation_policy_version: &issue_policy_version(&IssuePolicy::builtin()),
        held: false,
        pinned_source_commit: None,
        previous: item.triage.as_ref().map(|value| &value.assessment),
    })?;
    if !matches!(
        adapter.save_triage(workspace, fence, &assessment)?,
        IssueTriageSaveResult::Saved { .. }
    ) {
        bail!("fixture assessment was not saved");
    }
    let (fence, _) = subject(adapter, issue)?;
    let work = IssueWorkCheckpoint {
        authorization: None,
        permit_authorization_epoch: 0,
        order: IssueWorkOrder {
            work_order_id: issue_work_order_id(issue),
            issue: issue.clone(),
            field_hash: snapshot.basis.field_hash.clone(),
            title: snapshot.shared_copy.title.clone(),
            handoff: snapshot.shared_copy.clone(),
            acceptance: issue_acceptance_criteria(&snapshot.shared_copy.body)
                .map_err(|reason| anyhow::anyhow!(reason))?,
            scope: workspace.clone(),
            branch_hint: None,
            link_reference: None,
            provider_link: false,
            profile: None,
            priority: 0,
            attempt: 1,
        },
        start_authority: IssueWorkStartAuthority::Policy,
        decision_id: None,
        approved_basis: IssueDecisionBasis::from_assessment(
            &assessment,
            IssueAutomationGate::Start,
            None,
        )?,
        task_ids: vec![],
        permit_source: None,
        permit_revision: 0,
        delivery_granted: false,
        delivery_revision: 0,
        delivery_assessment: None,
        blocked_reason: None,
        revision: 0,
    };
    let IssueWorkAdmissionResult::Saved { record, .. } = adapter.admit_issue_work(
        workspace,
        fence,
        &assessment,
        &work,
        &policy.policy.budget,
        0,
    )?
    else {
        bail!("fixture work was not admitted");
    };
    if task_ids.is_empty() {
        return Ok(*record);
    }
    let mut candidate = *record;
    candidate.task_ids = task_ids.to_vec();
    match adapter.save_issue_work(workspace, fence, &assessment, &candidate)? {
        IssueWorkSaveResult::Saved { record, .. } => Ok(*record),
        IssueWorkSaveResult::Conflict => bail!("fixture work linkage was not saved"),
    }
}

fn binding_for_issue(issue: &IssueReference) -> Result<IssueAutomationBinding> {
    let repository = IssueSourceRepository::from_reference(issue);
    let id = format!(
        "fixture-binding-{:x}",
        Sha256::digest(serde_json::to_vec(&repository)?)
    );
    Ok(IssueAutomationBinding::new(
        IssueAutomationBindingId::new(id)?,
        issue.provider,
        &issue.server_id,
        &issue.repository_id,
        issue.project_scope.clone(),
    )?)
}

fn observation(issue: &IssueReference) -> IssueSourceObservation {
    let title = "Fixture issue";
    let body = "## 완료 기준\n- 변경 결과를 검증한다.\n";
    IssueSourceObservation {
        content_problems: vec![],
        jira: None,
        linear: None,
        issue: issue.clone(),
        repository_visibility: IssueRepositoryVisibility::Private,
        repository_name_with_owner: None,
        web: None,
        remote_state: IssueRemoteState::Open,
        creation_provenance: IssueCreationProvenance::Unattributed,
        labels: IssueMetadataList {
            items: vec![],
            completeness: IssueMetadataCompleteness::Complete,
        },
        assignees: IssueMetadataList {
            items: vec![],
            completeness: IssueMetadataCompleteness::Complete,
        },
        author: Some(IssueSourceActor {
            kind: IssueSourceActorKind::UserAccount,
            id: "U_fixture".into(),
            login: "fixture".into(),
        }),
        label_actors: IssueMetadataList {
            items: vec![],
            completeness: IssueMetadataCompleteness::Complete,
        },
        linked_pull_request: IssuePrPresence::Absent,
        board: None,
        content: IssueSourceContent::Shared(IssueAutomationSnapshot {
            basis: IssueComparisonBasis {
                field_hash: IssueFieldHash::of_fields(title, body),
                provider_updated_at: "2026-11-23T00:00:00Z".into(),
            },
            shared_copy: IssueSharedCopy {
                title: title.into(),
                body: body.into(),
            },
            policy_version: issue_policy_version(&IssuePolicy::builtin()),
        }),
    }
}

fn subject(
    adapter: &SqliteIssueAutomationStoreAdapter,
    issue: &IssueReference,
) -> Result<(IssueTriageFence, IssueCollectionItem)> {
    let mut after = None;
    loop {
        let page =
            adapter.read_triage_batch(&issue.project_scope.workspace_dir, after.as_ref(), 100)?;
        let control = page
            .collection
            .ok_or_else(|| anyhow::anyhow!("fixture collection is missing"))?;
        if let Some(item) = page.items.into_iter().find(|item| item.issue == *issue) {
            return Ok((
                IssueTriageFence {
                    collection_epoch: control.epoch,
                    binding_run_revision: item.binding_run_revision,
                    observation_revision: item.observation_revision,
                    tracked_revision: item.tracked_revision,
                    assessment_revision: item.triage.as_ref().map(|value| value.revision),
                },
                item,
            ));
        }
        after = page.next_after;
        if after.is_none() {
            bail!("fixture collection item is missing");
        }
    }
}
