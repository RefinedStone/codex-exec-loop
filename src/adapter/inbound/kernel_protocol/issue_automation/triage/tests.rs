use super::super::presentation::{IssueListGroupId, IssueListStatusCode};
use super::super::{IssueListItem, IssuesState};
use super::*;
use crate::adapter::inbound::kernel_protocol::{KernelQuery, KernelQueryAnswer, describe};
use crate::domain::issue_automation::collection::{
    IssueCollectionItem, IssueCollectionPage, IssueTrackedSummary,
};
use crate::domain::issue_automation::policy::{IssueAutomationPolicy, IssueAutomationTrustedActor};
use crate::domain::issue_automation::source::{
    IssueContentWithheldReason, IssueMetadataCompleteness, IssueMetadataList, IssuePrPresence,
    IssueRemoteState, IssueSourceActor, IssueSourceContent, IssueSourceFailure, IssueSourceLabel,
    IssueSourceObservation, IssueSourceRead, IssueSourceWeb,
};
use crate::domain::issue_automation::triage::{
    IssueObservationDisposition, IssueTriageInput, triage_issue,
};
use crate::domain::issue_automation::{
    IssueAutomationBindingId, IssueAutomationFlags, IssueAutomationSnapshot, IssueAutomationState,
};
use crate::domain::issue_validation::IssueRequirements;
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueFieldHash, IssueProjectScope, IssueProvider, IssueReference,
    IssueSharedCopy,
};
use serde_json::{Value, json};

const SHARED_TITLE: &str = "현재 공유 제목";
const SHARED_BODY: &str = "## 완료 기준\n- 현재 공유 기준의 결과를 확인한다.\n";

fn reference(id: &str) -> IssueReference {
    IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".to_owned(),
        repository_id: "R_stable".to_owned(),
        issue_id: id.to_owned(),
        project_scope: IssueProjectScope {
            workspace_dir: "/workspace".to_owned(),
        },
    }
}

fn basis() -> IssueComparisonBasis {
    IssueComparisonBasis {
        field_hash: IssueFieldHash::of_fields("raw-title-sentinel", "raw-body-sentinel"),
        provider_updated_at: "2026-11-20T12:00:00Z".to_owned(),
    }
}

fn observed(issue: &IssueReference) -> IssueSourceRead {
    IssueSourceRead::Observed(Box::new(IssueSourceObservation {
        issue: issue.clone(),
        web: Some(IssueSourceWeb {
            number: 17,
            url: "https://github.com/sample/widgets/issues/17".to_owned(),
        }),
        remote_state: IssueRemoteState::Open,
        labels: IssueMetadataList {
            items: vec![IssueSourceLabel {
                id: "L_auto".to_owned(),
                name: "akra:auto".to_owned(),
            }],
            completeness: IssueMetadataCompleteness::Complete,
        },
        assignees: IssueMetadataList {
            items: vec![],
            completeness: IssueMetadataCompleteness::Complete,
        },
        author: Some(IssueSourceActor {
            id: "U_trusted".to_owned(),
            login: "display-only".to_owned(),
        }),
        label_actors: IssueMetadataList {
            items: vec![],
            completeness: IssueMetadataCompleteness::Complete,
        },
        linked_pull_request: IssuePrPresence::Absent,
        content: IssueSourceContent::Shared(IssueAutomationSnapshot {
            basis: basis(),
            shared_copy: IssueSharedCopy {
                title: SHARED_TITLE.to_owned(),
                body: SHARED_BODY.to_owned(),
            },
            policy_version: "b".repeat(64),
        }),
    }))
}

fn policy(mode: IssueAutomationAutonomy) -> IssueAutomationPolicy {
    let mut policy = IssueAutomationPolicy::builtin();
    policy.binding.github.mode = mode;
    policy.authority.autonomy_cap = IssueAutomationAutonomy::Auto;
    policy.select.trusted_actors = vec![IssueAutomationTrustedActor {
        provider: IssueProvider::GitHub,
        server_id: "github.com".to_owned(),
        actor_id: "U_trusted".to_owned(),
    }];
    policy.gates.auto.normal.start = IssueAutomationGateAction::Auto;
    policy.gates.gated.normal.start = IssueAutomationGateAction::TimeoutDefault;
    policy
}

fn projection(
    issue: &IssueReference,
    current: Option<&IssueSourceRead>,
    mode: IssueAutomationAutonomy,
) -> IssueTriageProjection {
    let policy = policy(mode);
    let assessment = triage_issue(IssueTriageInput {
        issue,
        current,
        policy: &policy,
        automation_policy_version: &"a".repeat(64),
        requirements: &IssueRequirements::default(),
        validation_policy_version: &"b".repeat(64),
        held: false,
        pinned_source_commit: None,
        previous: None,
    })
    .unwrap();
    IssueTriageProjection {
        assessment,
        revision: 11,
        observation_revision: current.map(|_| 7),
        tracked_revision: Some(3),
        stale: false,
    }
}

fn item(id: &str, mode: IssueAutomationAutonomy) -> IssueCollectionItem {
    let issue = reference(id);
    let current = observed(&issue);
    let triage = projection(&issue, Some(&current), mode);
    IssueCollectionItem {
        issue,
        binding_id: IssueAutomationBindingId::new("binding").unwrap(),
        tracked: Some(IssueTrackedSummary {
            state: IssueAutomationState::Discovered,
            flags: IssueAutomationFlags::default(),
        }),
        current: Some(current),
        observation_revision: Some(7),
        tracked_revision: Some(3),
        triage: Some(triage),
    }
}

fn global_report() -> IssueTriageObserveReport {
    IssueTriageObserveReport {
        revision: 40,
        binding_id: Some(IssueAutomationBindingId::new("binding").unwrap()),
        policy_version: Some("a".repeat(64)),
        assumed_mode: Some(IssueAutomationAutonomy::Auto),
        policy_blocked: false,
        total: 150,
        automatic_start: 10,
        human_approval: 20,
        deferred: 30,
        excluded: 40,
        needs_info: 5,
        unknown: 6,
        observation_only: 7,
        stale: 8,
        untriaged: 9,
        blocked: 15,
    }
}

fn assert_issues_schema(value: &Value) {
    let spec = describe();
    let mut schema = spec
        .queries
        .iter()
        .find(|entry| entry.name == "issues")
        .unwrap()
        .output
        .clone();
    schema
        .as_object_mut()
        .unwrap()
        .insert("$defs".to_owned(), Value::Object(spec.definitions));
    let validator = jsonschema::draft202012::options()
        .offline()
        .build(&schema)
        .unwrap();
    let errors: Vec<_> = validator
        .iter_errors(value)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
}

#[test]
fn every_triage_reason_has_a_stable_code_summary_and_non_mutating_next_step() {
    let spec = describe();
    for reason in IssueTriageReasonCode::ALL {
        let wire = IssueTriageReasonState::from(reason);
        assert_eq!(
            serde_json::to_value(wire.code).unwrap(),
            json!(reason.as_str())
        );
        assert!(!wire.summary.trim().is_empty());
        assert!(!wire.next_summary.trim().is_empty());
        assert!(!wire.next.is_empty());
        assert_eq!(wire.next, reason_next(wire.code));
        let declared = spec
            .reasons
            .iter()
            .find(|entry| entry.name == reason.as_str())
            .unwrap();
        assert!(!declared.summary.is_empty());
        assert_eq!(declared.next, wire.next);
        // 재조회·사람 확인·중단만 안내하며 실행이나 정책 확대 명령을 만들지 않는다.
        for next in &wire.next {
            let json = serde_json::to_value(next).unwrap();
            assert!(matches!(
                json["step"].as_str(),
                Some("query" | "ask_user" | "stop")
            ));
        }
    }
    assert!(
        serde_json::from_value::<IssueTriageReasonState>(json!({
            "code":"provider_message", "summary":"opaque", "next":[]
        }))
        .is_err()
    );
}

#[test]
fn triage_wire_keeps_original_hash_policy_versions_and_descriptive_gates_separate() {
    let issue = reference("I_A");
    let current = observed(&issue);
    let mut persisted = projection(&issue, Some(&current), IssueAutomationAutonomy::Auto);
    let wire = IssueTriageState::from(&persisted);
    assert_eq!(wire.policy_version, "a".repeat(64));
    assert_eq!(wire.basis.disclosure_policy_version, Some("b".repeat(64)));
    assert_eq!(wire.basis.validation_policy_version, "b".repeat(64));
    assert_eq!(
        wire.field_hash.as_ref().unwrap().sha256,
        basis().field_hash.sha256
    );
    assert_ne!(
        wire.field_hash.as_ref().unwrap().sha256,
        IssueFieldHash::of_fields(SHARED_TITLE, SHARED_BODY).sha256
    );
    assert_eq!(wire.observation_revision, Some(7));
    assert_eq!(wire.tracked_revision, Some(3));
    assert!(wire.observation.observation_only);
    assert_eq!(
        wire.observation.disposition,
        IssueObservationDispositionState::AutomaticStart
    );
    assert_eq!(wire.gates.len(), 6);
    assert!(wire.gates.iter().any(|gate| gate.enabled));
    let close = wire
        .gates
        .iter()
        .find(|gate| gate.gate == IssueTriageGateKind::Close)
        .unwrap();
    assert!(!close.enabled);
    assert_eq!(close.action, IssueTriageGateActionState::Human);
    assert!(wire.gates.iter().all(|gate| gate.gate_revision == 1));

    persisted.stale = true;
    let stale = IssueTriageState::from(&persisted);
    assert!(stale.stale);
    assert!(stale.gates.iter().all(|gate| !gate.enabled));
    assert!(persisted.assessment.gates().iter().any(|gate| gate.enabled));
    for mode in [
        IssueAutomationAutonomy::Off,
        IssueAutomationAutonomy::Observe,
    ] {
        let wire = IssueTriageState::from(&projection(&issue, Some(&current), mode));
        assert!(wire.gates.iter().all(|gate| !gate.enabled));
        assert!(wire.observation.observation_only);
    }
    let text = serde_json::to_string(&wire).unwrap();
    for hidden in [
        "raw-title-sentinel",
        "raw-body-sentinel",
        SHARED_TITLE,
        SHARED_BODY,
    ] {
        assert!(!text.contains(hidden), "{hidden}");
    }
}

#[test]
fn current_withheld_unknown_and_legacy_never_reuse_the_previous_shared_text() {
    let mut item = item("I_A", IssueAutomationAutonomy::Auto);
    item.triage.as_mut().unwrap().stale = true;
    let Some(IssueSourceRead::Observed(observation)) = &mut item.current else {
        unreachable!()
    };
    observation.content = IssueSourceContent::Withheld {
        basis: basis(),
        policy_version: "b".repeat(64),
        reason: IssueContentWithheldReason::UserConfirmationRequired,
    };
    for step in 0..3 {
        if step == 1 {
            item.current = Some(IssueSourceRead::Unknown {
                issue: item.issue.clone(),
                reason: IssueSourceFailure::ProviderUnavailable,
            });
        } else if step == 2 {
            item.current = None;
        }
        let wire = IssueListItem::from(&item);
        assert!(wire.presentation.title.is_none());
        if step > 0 {
            assert!(wire.presentation.url.is_none());
            assert!(wire.presentation.number.is_none());
        }
        assert!(
            wire.triage
                .as_ref()
                .unwrap()
                .gates
                .iter()
                .all(|gate| !gate.enabled)
        );
        let text = serde_json::to_string(&wire).unwrap();
        for hidden in [
            "raw-title-sentinel",
            "raw-body-sentinel",
            SHARED_TITLE,
            "현재 공유 기준의 결과",
        ] {
            assert!(!text.contains(hidden), "{hidden}");
        }
    }

    let issue = reference("I_A");
    let unknown = IssueSourceRead::Unknown {
        issue: issue.clone(),
        reason: IssueSourceFailure::ProviderUnavailable,
    };
    for current in [Some(&unknown), None] {
        let wire =
            IssueTriageState::from(&projection(&issue, current, IssueAutomationAutonomy::Auto));
        assert!(wire.field_hash.is_none());
        assert!(wire.basis.disclosure_policy_version.is_none());
        assert_eq!(wire.readiness, IssueTriageReadinessState::Unknown);
        assert!(wire.gates.iter().all(|gate| !gate.enabled));
    }
}

#[test]
fn server_groups_keep_stable_page_order_cursor_and_workflow_while_report_is_global() {
    let mut items: Vec<_> = (0..8)
        .map(|index| item(&format!("I_{index}"), IssueAutomationAutonomy::Auto))
        .collect();
    items[0] = item("I_0", IssueAutomationAutonomy::Propose);
    items[1].triage.as_mut().unwrap().stale = true;
    for (index, state) in [
        (3, IssueAutomationState::Running),
        (4, IssueAutomationState::InReview),
        (5, IssueAutomationState::Done),
    ] {
        items[index].tracked.as_mut().unwrap().state = state;
        items[index].triage.as_mut().unwrap().stale = true;
    }
    let Some(IssueSourceRead::Observed(observation)) = &mut items[6].current else {
        unreachable!()
    };
    observation.remote_state = IssueRemoteState::Closed;
    items[6].triage = Some(projection(
        &items[6].issue,
        items[6].current.as_ref(),
        IssueAutomationAutonomy::Auto,
    ));
    items[7].triage = None;
    let after = items.last().unwrap().issue.clone();
    let expected_order: Vec<_> = items
        .iter()
        .map(|item| item.issue.issue_id.clone())
        .collect();
    let page = IssueCollectionPage {
        revision: 40,
        items,
        next_after: Some(after.clone()),
        report: Some(global_report()),
    };
    let IssuesState::Ready(wire) = IssuesState::from_domain(&page) else {
        panic!("ready")
    };
    assert_eq!(
        wire.items
            .iter()
            .map(|item| item.issue.issue_id.clone())
            .collect::<Vec<_>>(),
        expected_order
    );
    assert_eq!(
        wire.groups
            .iter()
            .map(|group| (group.id, group.item_indices.clone()))
            .collect::<Vec<_>>(),
        vec![
            (IssueListGroupId::MyTurn, vec![0, 1]),
            (IssueListGroupId::Upcoming, vec![2]),
            (IssueListGroupId::Running, vec![3]),
            (IssueListGroupId::InReview, vec![4]),
            (IssueListGroupId::Done, vec![5]),
            (IssueListGroupId::Excluded, vec![6]),
            (IssueListGroupId::Collection, vec![7]),
        ]
    );
    assert_eq!(
        wire.items[2].presentation.status.code,
        IssueListStatusCode::Ready
    );
    assert_eq!(
        wire.items[3].presentation.status.code,
        IssueListStatusCode::Running
    );
    assert_eq!(
        wire.items[4].presentation.status.code,
        IssueListStatusCode::InReview
    );
    assert_eq!(
        wire.items[5].presentation.status.code,
        IssueListStatusCode::Done
    );
    assert_eq!(wire.report.as_ref().unwrap().total, 150);
    assert_eq!(wire.report.as_ref().unwrap().automatic_start, 10);
    assert_eq!(wire.report.as_ref().unwrap().revision, wire.revision);
    let query = KernelQuery::Issues {
        after: wire.next_after.clone(),
        limit: Some(8),
    };
    assert_eq!(
        query.issues_range("/workspace").unwrap(),
        Some((Some(after), 8))
    );

    let mut json =
        serde_json::to_value(KernelQueryAnswer::Issues(IssuesState::Ready(wire))).unwrap();
    assert_issues_schema(&json);
    // 추가 필드가 없던 장부 응답도 입력 타입에서 읽되 판정이나 보고를 만들어 채우지 않는다.
    json.as_object_mut().unwrap().remove("report");
    for item in json["items"].as_array_mut().unwrap() {
        item.as_object_mut().unwrap().remove("triage");
    }
    let KernelQueryAnswer::Issues(IssuesState::Ready(decoded)) =
        serde_json::from_value(json).unwrap()
    else {
        panic!("issues")
    };
    assert!(decoded.report.is_none());
    assert!(decoded.items.iter().all(|item| item.triage.is_none()));
}

#[test]
fn label_cap_in_observe_report_is_distinct_from_the_global_assumed_mode() {
    let mut item = item("I_A", IssueAutomationAutonomy::Auto);
    let Some(IssueSourceRead::Observed(observation)) = &mut item.current else {
        unreachable!()
    };
    observation.labels.items[0].name = "akra:gated".to_owned();
    item.triage = Some(projection(
        &item.issue,
        item.current.as_ref(),
        IssueAutomationAutonomy::Auto,
    ));
    let page = IssueCollectionPage {
        revision: 40,
        items: vec![item],
        next_after: None,
        report: Some(global_report()),
    };
    let IssuesState::Ready(wire) = IssuesState::from_domain(&page) else {
        panic!("ready")
    };
    assert_eq!(
        wire.report.as_ref().unwrap().assumed_mode,
        Some(IssueTriageModeState::Auto)
    );
    let triage = wire.items[0].triage.as_ref().unwrap();
    assert_eq!(triage.observation.assumed_mode, IssueTriageModeState::Gated);
    assert_eq!(
        triage.observation.disposition,
        IssueObservationDispositionState::Deferred
    );
    assert!(triage.observation.observation_only);
    assert_eq!(wire.groups[0].id, IssueListGroupId::Upcoming);

    let mut report = global_report();
    report.policy_blocked = true;
    report.policy_version = None;
    report.assumed_mode = None;
    report.blocked = report.total;
    report.automatic_start = 0;
    let wire = IssueTriageObserveReportState::from(&report);
    assert!(wire.policy_blocked);
    assert!(wire.policy_version.is_none());
    assert!(wire.assumed_mode.is_none());
    assert_eq!(wire.blocked, wire.total);
}

#[test]
fn server_groups_use_current_start_gate_without_promoting_hypothetical_observe_advice() {
    let cases = [
        (IssueAutomationAutonomy::Off, IssueListGroupId::Collection),
        (
            IssueAutomationAutonomy::Observe,
            IssueListGroupId::Collection,
        ),
        (IssueAutomationAutonomy::Propose, IssueListGroupId::MyTurn),
        (IssueAutomationAutonomy::Gated, IssueListGroupId::Upcoming),
        (IssueAutomationAutonomy::Auto, IssueListGroupId::Upcoming),
    ];
    for (mode, expected_group) in cases {
        let subject = item("I_mode", mode);
        assert_eq!(
            subject
                .triage
                .as_ref()
                .unwrap()
                .assessment
                .observation()
                .disposition,
            IssueObservationDisposition::AutomaticStart,
            "the report assumes the user cap Auto for every current mode"
        );
        let page = IssueCollectionPage {
            revision: 40,
            items: vec![subject],
            next_after: None,
            report: Some(global_report()),
        };
        let IssuesState::Ready(wire) = IssuesState::from_domain(&page) else {
            panic!("ready");
        };
        assert_eq!(wire.groups.len(), 1);
        assert_eq!(wire.groups[0].id, expected_group, "{mode:?}");
        assert_eq!(wire.groups[0].item_indices, vec![0]);
        assert_eq!(wire.report.as_ref().unwrap().automatic_start, 10);
    }
}
