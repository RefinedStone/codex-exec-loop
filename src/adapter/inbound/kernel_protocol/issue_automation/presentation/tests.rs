use super::super::{IssueListItem, IssuesState};
use super::*;
use crate::domain::issue_automation::collection::{IssueCollectionPage, IssueTrackedSummary};
use crate::domain::issue_automation::source::{
    IssueContentWithheldReason, IssueMetadataCompleteness, IssueMetadataList, IssuePrPresence,
    IssueSourceFailure, IssueSourceObservation, IssueSourceWeb,
};
use crate::domain::issue_automation::{
    IssueAutomationBindingId, IssueAutomationFlags, IssueAutomationSnapshot,
};
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueFieldHash, IssueProjectScope, IssueProvider, IssueReference,
    IssueSharedCopy,
};
use serde_json::json;

fn item(id: &str, state: Option<IssueAutomationState>) -> IssueCollectionItem {
    let issue = IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".to_owned(),
        repository_id: "R_stable".to_owned(),
        issue_id: id.to_owned(),
        project_scope: IssueProjectScope {
            workspace_dir: "/workspace".to_owned(),
        },
    };
    IssueCollectionItem {
        binding_run_revision: 0,
        binding_run_blocked: false,
        start_budget_wait: None,
        work: None,
        open_decisions: Vec::new(),
        observation_revision: None,
        tracked_revision: None,
        triage: None,
        issue: issue.clone(),
        binding_id: IssueAutomationBindingId::new("binding").unwrap(),
        tracked: state.map(|state| IssueTrackedSummary {
            state,
            flags: IssueAutomationFlags::default(),
        }),
        current: Some(IssueSourceRead::Observed(Box::new(
            IssueSourceObservation {
                content_problems: vec![],
                jira: None,
                linear: None,
                repository_visibility: Default::default(),
                repository_name_with_owner: None,
                creation_provenance:
                    crate::domain::issue_automation::source::IssueCreationProvenance::Unattributed,
                issue,
                web: Some(IssueSourceWeb {
                    number: 17,
                    url: "https://github.com/sample/widgets/issues/17".to_owned(),
                }),
                remote_state: IssueRemoteState::Open,
                labels: IssueMetadataList {
                    items: vec![],
                    completeness: IssueMetadataCompleteness::Complete,
                },
                assignees: IssueMetadataList {
                    items: vec![],
                    completeness: IssueMetadataCompleteness::Complete,
                },
                author: None,
                label_actors: IssueMetadataList {
                    items: vec![],
                    completeness: IssueMetadataCompleteness::Complete,
                },
                linked_pull_request: IssuePrPresence::Absent,
                board: None,
                content: IssueSourceContent::Shared(IssueAutomationSnapshot {
                    basis: basis(),
                    shared_copy: IssueSharedCopy {
                        title: "현재 공유 제목".to_owned(),
                        body: "현재 공유 본문".to_owned(),
                    },
                    policy_version: "policy-1".to_owned(),
                }),
            },
        ))),
    }
}

fn basis() -> IssueComparisonBasis {
    IssueComparisonBasis {
        field_hash: IssueFieldHash::of_fields("private-title-sentinel", "private-body-sentinel"),
        provider_updated_at: "2026-11-20T00:00:00Z".to_owned(),
    }
}

fn observed(item: &mut IssueCollectionItem) -> &mut IssueSourceObservation {
    let Some(IssueSourceRead::Observed(value)) = &mut item.current else {
        panic!("expected observation")
    };
    value
}

fn withheld() -> IssueSourceContent {
    IssueSourceContent::Withheld {
        basis: basis(),
        policy_version: "policy-2".to_owned(),
        reason: IssueContentWithheldReason::UserConfirmationRequired,
    }
}

#[test]
fn issue_list_presentation_never_fills_withheld_unknown_or_legacy_from_older_content() {
    let mut item = item("I_A", Some(IssueAutomationState::Running));
    let shared = IssueListItem::from(&item);
    assert_eq!(shared.presentation.number, Some(17));
    assert_eq!(
        shared.presentation.url.as_deref(),
        Some("https://github.com/sample/widgets/issues/17")
    );
    assert_eq!(shared.presentation.title.as_deref(), Some("현재 공유 제목"));

    observed(&mut item).content = withheld();
    let withheld = IssueListItem::from(&item);
    assert_eq!(withheld.presentation.number, Some(17));
    assert!(withheld.presentation.url.is_some());
    assert!(withheld.presentation.title.is_none());
    assert_eq!(
        withheld.presentation.status.code,
        IssueListStatusCode::Running
    );
    assert_eq!(
        withheld.presentation.reason.code,
        IssueListReasonCode::DisclosureConfirmationRequired
    );
    assert!(
        !serde_json::to_string(&withheld)
            .unwrap()
            .contains("현재 공유")
    );

    item.current = Some(IssueSourceRead::Unknown {
        issue: item.issue.clone(),
        reason: IssueSourceFailure::ProviderUnavailable,
    });
    let unknown = IssueListItem::from(&item);
    assert_eq!(
        (
            unknown.presentation.number,
            unknown.presentation.url.as_ref(),
            unknown.presentation.title.as_ref()
        ),
        (None, None, None)
    );
    assert_eq!(
        unknown.presentation.status.code,
        IssueListStatusCode::Running
    );
    assert_eq!(
        unknown.presentation.reason.code,
        IssueListReasonCode::SourceReadFailed
    );
    assert!(
        !serde_json::to_string(&unknown)
            .unwrap()
            .contains("https://")
    );
    assert!(
        !serde_json::to_string(&unknown)
            .unwrap()
            .contains("현재 공유")
    );

    item.current = None;
    let legacy = IssueListItem::from(&item);
    assert_eq!(
        (
            legacy.presentation.number,
            legacy.presentation.url.as_ref(),
            legacy.presentation.title.as_ref()
        ),
        (None, None, None)
    );
    assert_eq!(
        legacy.presentation.status.code,
        IssueListStatusCode::Running
    );
    assert_eq!(
        legacy.presentation.reason.code,
        IssueListReasonCode::NotYetObserved
    );
}

#[test]
fn issue_list_presentation_uses_an_old_shared_title_without_inventing_missing_web_metadata() {
    let mut item = item("I_A", None);
    observed(&mut item).web = None;
    let value = IssueListPresentation::from_domain(&item);
    assert_eq!(value.title.as_deref(), Some("현재 공유 제목"));
    assert!(value.number.is_none() && value.url.is_none());

    observed(&mut item).web = Some(IssueSourceWeb {
        number: 17,
        url: "javascript:alert(1)".to_owned(),
    });
    let wire = serde_json::to_value(IssueListItem::from(&item)).unwrap();
    assert!(wire["presentation"]["number"].is_null());
    assert!(wire["presentation"]["url"].is_null());
    assert!(!wire.to_string().contains("javascript:"));
}

#[test]
fn issue_list_presentation_keeps_all_existing_workflow_states_and_fixed_server_copy() {
    for (state, code) in [
        (IssueAutomationState::Discovered, "discovered"),
        (IssueAutomationState::Ineligible, "ineligible"),
        (IssueAutomationState::NeedsInfo, "needs_info"),
        (IssueAutomationState::AwaitingDecision, "awaiting_decision"),
        (IssueAutomationState::Ready, "ready"),
        (IssueAutomationState::Queued, "queued"),
        (IssueAutomationState::Running, "running"),
        (IssueAutomationState::InReview, "in_review"),
        (IssueAutomationState::Done, "done"),
        (IssueAutomationState::Failed, "failed"),
        (IssueAutomationState::Cancelled, "cancelled"),
    ] {
        let item = item("I_A", Some(state));
        let before = item.clone();
        let value = IssueListPresentation::from_domain(&item);
        assert_eq!(serde_json::to_value(value.status.code).unwrap(), code);
        assert_eq!(value.reason.code, IssueListReasonCode::Workflow);
        assert!(!value.status.label.is_empty() && !value.reason.summary.is_empty());
        assert!(!value.reason.summary.contains('\n'));
        assert_eq!(item, before);
    }
    let mut running = item("I_A", Some(IssueAutomationState::Running));
    observed(&mut running).remote_state = IssueRemoteState::Closed;
    let value = IssueListPresentation::from_domain(&running);
    assert_eq!(value.status.code, IssueListStatusCode::Running);
    assert_eq!(value.reason.code, IssueListReasonCode::RemoteClosed);
    let done = IssueListPresentation::from_domain(&item("I_B", Some(IssueAutomationState::Done)));
    assert_eq!(done.status.code, IssueListStatusCode::Done);
    assert_eq!(done.reason.code, IssueListReasonCode::Workflow);
}

#[test]
fn issue_list_presentation_separates_current_read_status_from_flags_and_workflow() {
    let mut item = item("I_A", None);
    assert_eq!(
        IssueListPresentation::from_domain(&item).status.code,
        IssueListStatusCode::Observed
    );
    observed(&mut item).content = withheld();
    assert_eq!(
        IssueListPresentation::from_domain(&item).status.code,
        IssueListStatusCode::Withheld
    );
    item.current = Some(IssueSourceRead::Unknown {
        issue: item.issue.clone(),
        reason: IssueSourceFailure::MalformedResponse,
    });
    assert_eq!(
        IssueListPresentation::from_domain(&item).status.code,
        IssueListStatusCode::Unknown
    );
    item.current = None;
    assert_eq!(
        IssueListPresentation::from_domain(&item).status.code,
        IssueListStatusCode::NotYetObserved
    );

    let mut flagged = self::item("I_B", Some(IssueAutomationState::Ready));
    flagged.tracked.as_mut().unwrap().flags.held = true;
    flagged.tracked.as_mut().unwrap().flags.stale = true;
    assert_eq!(
        IssueListPresentation::from_domain(&flagged).reason.code,
        IssueListReasonCode::Held
    );
    flagged.tracked.as_mut().unwrap().flags.held = false;
    assert_eq!(
        IssueListPresentation::from_domain(&flagged).reason.code,
        IssueListReasonCode::Stale
    );
    observed(&mut flagged).content = withheld();
    assert_eq!(
        IssueListPresentation::from_domain(&flagged).reason.code,
        IssueListReasonCode::DisclosureConfirmationRequired
    );
    flagged.current = Some(IssueSourceRead::Unknown {
        issue: flagged.issue.clone(),
        reason: IssueSourceFailure::ProviderUnavailable,
    });
    assert_eq!(
        IssueListPresentation::from_domain(&flagged).reason.code,
        IssueListReasonCode::SourceReadFailed
    );
}

#[test]
fn issue_list_groups_repeat_per_page_and_preserve_every_item_and_the_stable_cursor() {
    let first = item("I_A", Some(IssueAutomationState::Done));
    let second = item("I_B", Some(IssueAutomationState::Discovered));
    let after = second.issue.clone();
    let page = IssueCollectionPage {
        report: None,
        revision: 7,
        items: vec![first, second],
        next_after: Some(after.clone()),
    };
    let IssuesState::Ready(projected) = IssuesState::from_domain(&page) else {
        panic!("ready")
    };
    assert_eq!(
        serde_json::to_value(&projected.groups).unwrap(),
        json!([
            {"id":"done","label":"끝남","item_indices":[0]},
            {"id":"collection","label":"수집한 이슈","item_indices":[1]}
        ])
    );
    assert_eq!(
        projected
            .items
            .iter()
            .map(|item| item.issue.issue_id.as_str())
            .collect::<Vec<_>>(),
        ["I_A", "I_B"]
    );
    let query = super::super::super::KernelQuery::Issues {
        after: projected.next_after,
        limit: None,
    };
    assert_eq!(
        query.issues_range("/workspace").unwrap(),
        Some((Some(after), 50))
    );

    for items in [
        vec![item("I_C", Some(IssueAutomationState::Running))],
        vec![],
    ] {
        let count = items.len();
        let IssuesState::Ready(projected) = IssuesState::from_domain(&IssueCollectionPage {
            report: None,
            revision: 7,
            items,
            next_after: None,
        }) else {
            panic!("ready")
        };
        assert_eq!(projected.groups.len(), 1);
        assert_eq!(
            projected.groups[0].id,
            if count == 0 {
                IssueListGroupId::Collection
            } else {
                IssueListGroupId::Running
            }
        );
        assert_eq!(
            projected.groups[0].item_indices,
            (0..count).collect::<Vec<_>>()
        );
        assert!(projected.next_after.is_none());
    }
}

#[test]
fn issue_list_presentation_and_group_codes_are_closed_enums() {
    assert!(serde_json::from_value::<IssueListStatusCode>(json!("triage_pending")).is_err());
    assert!(serde_json::from_value::<IssueListReasonCode>(json!("provider_raw_error")).is_err());
    assert!(serde_json::from_value::<IssueListGroupId>(json!("my_turn")).is_ok());
    assert!(serde_json::from_value::<IssueListGroupId>(json!("provider_custom_group")).is_err());
}

fn delivery_item(reason: IssueDeliveryRiskReason) -> IssueCollectionItem {
    use crate::domain::issue_automation::coordinator::{IssueWorkBlockReason, IssueWorkSummary};
    use crate::domain::issue_automation::decision::{
        IssueDecisionBasis, IssueDecisionDefinition, IssueDecisionDeliveryContext, IssueDecisionId,
        IssueDecisionOption, IssueDecisionRequest, IssueDecisionSharedText, IssueDecisionSubject,
        IssueDecisionTimeout, StoredIssueDecision,
    };
    let mut item = item("I_delivery", Some(IssueAutomationState::Running));
    item.work = Some(IssueWorkSummary {
        work_order_id: "work-delivery".to_owned(),
        task_ids: vec!["task-delivery".to_owned()],
        blocked_reason: Some(IssueWorkBlockReason::AwaitingDeliveryApproval),
        delivery_granted: false,
    });
    let text = |value: &str| IssueDecisionSharedText::new_shared(value).unwrap();
    let request = IssueDecisionRequest::new(
        IssueDecisionId::new("delivery-1").unwrap(),
        IssueDecisionDefinition {
            subject: IssueDecisionSubject::Issue(item.issue.clone()),
            gate: IssueAutomationGate::Delivery,
            kind: IssueDecisionKind::RiskException,
            question: text("검토한 결과를 제출할까요?"),
            options: vec![IssueDecisionOption {
                id: "approve".to_owned(),
                label: text("제출 승인"),
                effect: text("검토한 결과를 제출합니다."),
            }],
            recommended: None,
            basis: IssueDecisionBasis {
                authorization_epoch: 0,
                policy_version: "a".repeat(64),
                field_hash: Some(basis().field_hash),
                work_order_id: Some("work-delivery".to_owned()),
                gate_revision: 2,
                delivery: Some(IssueDecisionDeliveryContext {
                    base_commit_sha: "a".repeat(40),
                    source_commit_sha: "b".repeat(40),
                }),
            },
            delivery_review: Some(IssueDeliveryDiffVerdict {
                changed_files: if reason == IssueDeliveryRiskReason::DiffTooLarge {
                    51
                } else {
                    2
                },
                changed_lines: (reason != IssueDeliveryRiskReason::BinaryDiff).then_some(10),
                reasons: vec![reason],
            }),
            on_timeout: IssueDecisionTimeout::Wait,
            due_at: None,
            asked_by: "coordinator".to_owned(),
            evidence: Vec::new(),
        },
    )
    .unwrap();
    item.open_decisions = vec![StoredIssueDecision {
        request,
        revision: 1,
    }];
    item
}

#[test]
fn open_delivery_risks_display_my_turn_without_changing_execution_or_start_state() {
    for (reason, code) in [
        (
            IssueDeliveryRiskReason::ProtectedPath,
            IssueListReasonCode::ProtectedPath,
        ),
        (
            IssueDeliveryRiskReason::DiffTooLarge,
            IssueListReasonCode::DiffTooLarge,
        ),
        (
            IssueDeliveryRiskReason::BinaryDiff,
            IssueListReasonCode::BinaryDiff,
        ),
    ] {
        let item = delivery_item(reason);
        let before = item.clone();
        let presentation = IssueListPresentation::from_domain(&item);
        assert_eq!(group_id(&item), IssueListGroupId::MyTurn);
        assert_eq!(
            presentation.status.code,
            IssueListStatusCode::AwaitingDecision
        );
        assert_eq!(presentation.reason.code, code);
        assert!(presentation.reason.summary.contains("결과 제출 승인"));
        assert_eq!(
            item.tracked.as_ref().unwrap().state,
            IssueAutomationState::Running
        );
        assert_eq!(item, before);
    }
}

#[test]
fn delivery_display_requires_a_current_open_work_bound_decision_and_preserves_terminal_states() {
    use crate::domain::issue_automation::decision::IssueDecisionRequest;
    let original = delivery_item(IssueDeliveryRiskReason::ProtectedPath);
    for state in [IssueAutomationState::Done, IssueAutomationState::Cancelled] {
        let mut item = original.clone();
        item.tracked.as_mut().unwrap().state = state;
        assert_eq!(group_id(&item), IssueListGroupId::Done);
        assert_eq!(
            IssueListPresentation::from_domain(&item).status.code,
            IssueListStatusCode::from(state)
        );
    }
    for change in 0..6 {
        let mut item = original.clone();
        match change {
            0 => item.work = None,
            1 => item.work.as_mut().unwrap().work_order_id = "other-work".to_owned(),
            2 => item.work.as_mut().unwrap().delivery_granted = true,
            3 => observed(&mut item).content = withheld(),
            4 => {
                let mut record = item.open_decisions[0].request.to_record();
                record.state = IssueDecisionState::Superseded;
                item.open_decisions[0].request = IssueDecisionRequest::hydrate(record).unwrap();
            }
            _ => {
                let mut record = item.open_decisions[0].request.to_record();
                record.definition.kind = IssueDecisionKind::StartApproval;
                record.definition.gate = IssueAutomationGate::Start;
                record.definition.basis.work_order_id = None;
                record.definition.basis.delivery = None;
                record.definition.delivery_review = None;
                item.open_decisions[0].request = IssueDecisionRequest::hydrate(record).unwrap();
            }
        }
        assert_eq!(group_id(&item), IssueListGroupId::Running, "{change}");
        assert_eq!(
            IssueListPresentation::from_domain(&item).status.code,
            IssueListStatusCode::Running
        );
    }
}
