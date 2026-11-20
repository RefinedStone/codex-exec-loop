//! Gateway 회귀가 실제 조립과 같은 장부·현재 정책으로 사람 관문 질문을 준비한다.
use super::*;
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::issue_automation_policy::FilesystemIssueAutomationPolicySourceAdapter;
use crate::adapter::outbound::filesystem::issue_policy::FilesystemIssuePolicySourceAdapter;
use crate::domain::issue_automation::policy::IssueAutomationGateAction;
use crate::domain::issue_automation::{
    IssueAutomationBinding, IssueAutomationBindingId, TrackedIssue,
};

pub(crate) fn seed_human_decision_for_gateway(workspace: &str) -> (String, String) {
    let ledger = Arc::new(SqliteIssueAutomationStoreAdapter::new(|workspace| {
        let location =
            SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)?;
        Ok(PathBuf::from(location.runtime_dir))
    }));
    let reference = IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".into(),
        repository_id: "R_gateway_confirmation".into(),
        issue_id: "I_gateway_confirmation".into(),
        project_scope: IssueProjectScope {
            workspace_dir: workspace.to_owned(),
        },
    };
    let binding = IssueAutomationBinding::new(
        IssueAutomationBindingId::new("gateway-confirmation").unwrap(),
        reference.provider,
        &reference.server_id,
        &reference.repository_id,
        reference.project_scope.clone(),
    )
    .unwrap();
    ledger.insert_binding(workspace, &binding).unwrap();
    let tracked = TrackedIssue::discovered(
        reference.clone(),
        &binding,
        IssueAutomationSnapshot {
            basis: IssueComparisonBasis {
                field_hash: IssueFieldHash::of_fields(
                    "historical title",
                    "PRIVATE_GATEWAY_ORIGINAL_CANARY",
                ),
                provider_updated_at: "2026-11-20T00:00:00Z".into(),
            },
            shared_copy: IssueSharedCopy {
                title: "historical title".into(),
                body: "PRIVATE_GATEWAY_ORIGINAL_CANARY".into(),
            },
            policy_version: "historical-policy".into(),
        },
    )
    .unwrap();
    ledger.insert_issue(workspace, &tracked).unwrap();
    ledger.ensure_collection(workspace).unwrap();
    let validation = Arc::new(FilesystemIssuePolicySourceAdapter::new());
    let triage = Arc::new(IssueTriageService::new(
        workspace.to_owned(),
        ledger.clone(),
        IssueAutomationPolicyService::new(Arc::new(
            FilesystemIssueAutomationPolicySourceAdapter::new(),
        )),
        IssuePolicyService::new(validation.clone()),
    ));
    triage
        .triage_all(&IssueCollectionCancellation::new(), &|_| {})
        .unwrap();
    let assessment = triage
        .issue(&reference)
        .unwrap()
        .item
        .unwrap()
        .triage
        .unwrap()
        .assessment;
    assert_eq!(
        assessment
            .gates()
            .iter()
            .find(|gate| gate.gate == IssueAutomationGate::Start)
            .unwrap()
            .action,
        IssueAutomationGateAction::Human,
    );
    let service = IssueDecisionService::new(
        workspace.to_owned(),
        ledger,
        triage,
        IssuePolicyService::new(validation),
    );
    let id = "gateway-human-decision";
    let option = "accept";
    let definition = IssueDecisionDefinition {
        subject: IssueDecisionSubject::Issue(reference),
        gate: IssueAutomationGate::Start,
        kind: IssueDecisionKind::AgentQuestion,
        question: shared("이 공유된 질문에 답을 기록할까요?"),
        options: vec![IssueDecisionOption {
            id: option.into(),
            label: shared("확인"),
            effect: shared("사람이 확인한 답을 기록합니다."),
        }],
        recommended: Some(option.into()),
        basis: IssueDecisionBasis::from_assessment(&assessment, IssueAutomationGate::Start, None)
            .unwrap(),
        on_timeout: IssueDecisionTimeout::Wait,
        due_at: None,
        asked_by: "gateway-confirmation-test".into(),
        evidence: Vec::new(),
    };
    assert!(
        service
            .ensure_request(IssueDecisionId::new(id).unwrap(), definition)
            .unwrap()
            .changed
    );
    (id.into(), option.into())
}
