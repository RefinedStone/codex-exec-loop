//! 보드 열 조건의 관찰 (#2565). 가짜 보드 port 만 쓴다.
use super::*;
use std::sync::Mutex;

use crate::application::port::outbound::issue_board_field_port::{
    IssueBoardField, IssueBoardFieldWriteRequest, IssueBoardFieldWriteResponse, IssueBoardItem,
    IssueBoardItemsReadRequest, IssueBoardItemsResponse,
};
use crate::application::service::issue_feedback::tests::support::{
    FakePolicy, FakeStore, pending_record,
};
use crate::domain::issue_automation::feedback::{
    IssueFeedbackEvidence, IssueFeedbackIntentState, IssueFeedbackReceipt,
};
use crate::domain::issue_automation::source::{IssueSourceBoardColumn, IssueSourceBoardRead};

const BOARD_POLICY: &str = "[binding.github.board]\nproject_id = \"PVT_1\"\nfield_id = \"PVTSSF_1\"\nselect_option_ids = [\"ready\"]\n";

struct FakeBoard {
    selected: Option<String>,
    fails: bool,
    reads: AtomicUsize,
}

impl IssueBoardFieldPort for FakeBoard {
    fn read_items(
        &self,
        request: IssueBoardItemsReadRequest,
    ) -> anyhow::Result<IssueBoardItemsResponse> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.fails {
            anyhow::bail!("the login has no read:project scope");
        }
        Ok(IssueBoardItemsResponse {
            issue: request.issue,
            items: vec![IssueBoardItem {
                project_id: "PVT_1".into(),
                project_title: "Board".into(),
                item_id: "PVTI_1".into(),
                fields: vec![IssueBoardField {
                    field_id: "PVTSSF_1".into(),
                    name: "Status".into(),
                    options: Vec::new(),
                    selected_option_id: self.selected.clone(),
                }],
            }],
        })
    }
    fn write_field(
        &self,
        _: IssueBoardFieldWriteRequest,
    ) -> anyhow::Result<IssueBoardFieldWriteResponse> {
        panic!("observation never writes the board")
    }
}

fn board(selected: Option<&str>, fails: bool) -> Arc<FakeBoard> {
    Arc::new(FakeBoard {
        selected: selected.map(Into::into),
        fails,
        reads: AtomicUsize::new(0),
    })
}

fn policy(project: &str) -> IssueAutomationPolicyService {
    IssueAutomationPolicyService::new(Arc::new(FakePolicy {
        user: Mutex::new(String::new()),
        project: Mutex::new(Some(project.into())),
        fail: AtomicBool::new(false),
    }))
}

fn service(
    board: Arc<FakeBoard>,
    project: &str,
    feedback: Arc<FakeStore>,
) -> IssueSourceProvenanceService {
    let (service, _) = setup(
        Some(snapshot(false)),
        Ok(page()),
        IssueSourceRead::Observed(Box::new(observation("I_created"))),
    );
    service
        .with_feedback(feedback, policy(""))
        .with_board_fields(board, policy(project))
}

fn column(read: IssueSourceBoardRead) -> Option<IssueSourceBoardColumn> {
    Some(IssueSourceBoardColumn {
        project_id: "PVT_1".into(),
        field_id: "PVTSSF_1".into(),
        read,
    })
}

fn item(option: Option<&str>) -> IssueSourceBoardRead {
    IssueSourceBoardRead::Item {
        item_id: "PVTI_1".into(),
        option_id: option.map(Into::into),
    }
}

fn board_resync() -> IssueResyncRequest {
    IssueResyncRequest {
        issue: issue("I_created"),
        disclosure: disclosure(),
    }
}

fn feedback() -> Arc<FakeStore> {
    Arc::new(FakeStore::new(issue("I_created")))
}

#[test]
fn the_board_is_read_only_when_the_policy_has_a_column_condition() {
    let unused = board(Some("ready"), false);
    let page = service(unused.clone(), "", feedback())
        .discover(request())
        .unwrap();
    assert!(
        page.observations
            .iter()
            .all(|observed| observed.board.is_none())
    );
    assert_eq!(unused.reads.load(Ordering::SeqCst), 0);

    let used = board(Some("ready"), false);
    let page = service(used.clone(), BOARD_POLICY, feedback())
        .discover(request())
        .unwrap();
    assert!(
        page.observations
            .iter()
            .all(|observed| observed.board == column(item(Some("ready"))))
    );
    assert_eq!(used.reads.load(Ordering::SeqCst), 2);
}

#[test]
fn a_board_read_failure_is_unavailable_and_not_an_empty_board() {
    let service = service(board(Some("ready"), true), BOARD_POLICY, feedback());
    let IssueSourceRead::Observed(observation) = service.resync(board_resync()) else {
        panic!("the issue itself was read");
    };
    assert_eq!(observation.board, column(IssueSourceBoardRead::Unavailable));
}

#[test]
fn a_confirmed_own_board_write_is_hidden_but_a_human_change_is_kept() {
    let store = feedback();
    store.data.lock().unwrap().snapshot.state.board_proof = Some(IssueFeedbackReceipt {
        issue: issue("I_created"),
        intent_id: "board-intent".into(),
        actor_id: "U_feedback".into(),
        evidence: IssueFeedbackEvidence::BoardField {
            project_id: "PVT_1".into(),
            item_id: "PVTI_1".into(),
            field_id: "PVTSSF_1".into(),
            option_id: "running".into(),
            origin_option_id: Some("ready".into()),
        },
    });
    let own = service(board(Some("running"), false), BOARD_POLICY, store.clone());
    let IssueSourceRead::Observed(observation) = own.resync(board_resync()) else {
        panic!("observed");
    };
    assert_eq!(observation.board, column(item(Some("ready"))));

    let human = service(board(Some("blocked"), false), BOARD_POLICY, store);
    let IssueSourceRead::Observed(observation) = human.resync(board_resync()) else {
        panic!("observed");
    };
    assert_eq!(observation.board, column(item(Some("blocked"))));
}

#[test]
fn a_pending_board_write_defers_the_issue_observation() {
    let store = feedback();
    let mut record = pending_record(issue("I_created"));
    record.state = IssueFeedbackIntentState::NeedsConfirmation;
    record.intent.operation = IssueFeedbackOperation::SetBoardField {
        project_id: "PVT_1".into(),
        item_id: "PVTI_1".into(),
        field_id: "PVTSSF_1".into(),
        option_id: "running".into(),
        from_option_id: Some("ready".into()),
        origin_option_id: Some("ready".into()),
    };
    record.validate().unwrap();
    store.data.lock().unwrap().records = vec![record];
    let read = service(board(Some("running"), false), BOARD_POLICY, store).resync(board_resync());
    assert!(matches!(
        read,
        IssueSourceRead::Unknown {
            reason: IssueSourceFailure::FeedbackUnconfirmed,
            ..
        }
    ));
}
