//! 진행 단계의 보드 필드 쓰기 (#2565). 가짜 보드 port 만 쓴다. 네트워크와 `gh` 가 없다.
use super::*;
use crate::application::port::outbound::issue_board_field_port::{
    IssueBoardField, IssueBoardFieldOption, IssueBoardFieldWriteRequest,
    IssueBoardFieldWriteResponse, IssueBoardItem, IssueBoardItemsReadRequest,
    IssueBoardItemsResponse,
};

use std::sync::atomic::{AtomicBool, Ordering};

const PROJECT: &str = "PVT_1";
const ITEM: &str = "PVTI_1";
const FIELD: &str = "PVTSSF_status";

#[derive(Default)]
struct FakeBoard {
    selected: Mutex<Option<String>>,
    writes: Mutex<Vec<IssueBoardFieldWriteRequest>>,
    read_fails: AtomicBool,
    lose_write_response: AtomicBool,
}

impl FakeBoard {
    fn in_column(option: &str) -> Arc<Self> {
        let board = Self::default();
        *board.selected.lock().unwrap() = Some(option.into());
        Arc::new(board)
    }
    fn items(&self) -> Vec<IssueBoardItem> {
        vec![IssueBoardItem {
            project_id: PROJECT.into(),
            project_title: "Board".into(),
            item_id: ITEM.into(),
            fields: vec![IssueBoardField {
                field_id: FIELD.into(),
                name: "Status".into(),
                options: ["opt_ready", "opt_running", "opt_done", "opt_blocked"]
                    .map(|id| IssueBoardFieldOption {
                        option_id: id.into(),
                        name: id.into(),
                    })
                    .to_vec(),
                selected_option_id: self.selected.lock().unwrap().clone(),
            }],
        }]
    }
    fn write_count(&self) -> usize {
        self.writes.lock().unwrap().len()
    }
}

impl IssueBoardFieldPort for FakeBoard {
    fn read_items(&self, request: IssueBoardItemsReadRequest) -> Result<IssueBoardItemsResponse> {
        if self.read_fails.load(Ordering::SeqCst) {
            bail!("the login has no read:project scope");
        }
        Ok(IssueBoardItemsResponse {
            issue: request.issue,
            items: self.items(),
        })
    }
    fn write_field(
        &self,
        request: IssueBoardFieldWriteRequest,
    ) -> Result<IssueBoardFieldWriteResponse> {
        request.ensure_authorized()?;
        request.ensure_targets_one_of(&self.items())?;
        *self.selected.lock().unwrap() = Some(request.option_id.clone());
        let item_id = request.item_id.clone();
        self.writes.lock().unwrap().push(request);
        if self.lose_write_response.load(Ordering::SeqCst) {
            bail!("the response was lost");
        }
        Ok(IssueBoardFieldWriteResponse { item_id })
    }
}

fn user_policy(board_field: bool) -> String {
    format!(
        "[feedback]\ncomment = false\nlabels = false\nboard_field = {board_field}\n\n\
         [binding.github.board]\nproject_id = \"{PROJECT}\"\nfield_id = \"{FIELD}\"\n\
         select_option_ids = [\"opt_ready\"]\n\n\
         [binding.github.board.progress]\nrunning = \"opt_running\"\ndone = \"opt_done\"\n"
    )
}

fn board_fixture(board_field: bool) -> Fixture {
    let fixture = Fixture::new(false, false);
    *fixture.policy.user.lock().unwrap() = user_policy(board_field);
    fixture
}

fn sync(fixture: &Fixture, board: &Arc<FakeBoard>) -> Result<()> {
    fixture
        .service()
        .with_board_fields(board.clone())
        .sync_issue(&issue(), 123, &IssueCollectionCancellation::new())
}

fn set_execution(fixture: &Fixture, state: IssueExecutionState) {
    let mut data = fixture.store.data.lock().unwrap();
    let execution = data.snapshot.execution.as_mut().unwrap();
    execution.progress.state = state;
    execution.progress.revision += 1;
}

fn owned_board(fixture: &Fixture) -> Option<IssueFeedbackEvidence> {
    fixture
        .store
        .snapshot_value()
        .state
        .board_proof
        .map(|proof| proof.evidence)
}

#[test]
fn a_board_write_needs_its_own_permission_and_not_the_comment_or_label_permission() {
    let fixture = board_fixture(false);
    let mut policy = user_policy(false);
    policy = policy.replace(
        "comment = false\nlabels = false",
        "comment = true\nlabels = true",
    );
    *fixture.policy.user.lock().unwrap() = policy;
    let board = FakeBoard::in_column("opt_ready");
    sync(&fixture, &board).unwrap();
    assert_eq!(board.write_count(), 0);
    assert!(fixture.writes().iter().all(|intent| !matches!(
        intent.operation,
        IssueFeedbackOperation::SetBoardField { .. }
    )));

    // 저장소 파일은 사용자 허가를 낮출 수만 있다.
    let fixture = board_fixture(true);
    *fixture.policy.project.lock().unwrap() = Some("[feedback]\nboard_field = false\n".into());
    sync(&fixture, &board).unwrap();
    assert_eq!(board.write_count(), 0);
}

#[test]
fn a_board_write_is_journaled_once_with_a_board_field_approval() {
    let fixture = board_fixture(true);
    let board = FakeBoard::in_column("opt_ready");
    sync(&fixture, &board).unwrap();

    let writes = board.writes.lock().unwrap().clone();
    assert_eq!(writes.len(), 1);
    assert_eq!(
        writes[0].approval.operation,
        crate::domain::issue_work::IssueWriteOperation::BoardField
    );
    assert_eq!(writes[0].option_id, "opt_running");
    let record = fixture.store.record();
    assert_eq!(record.state, IssueFeedbackIntentState::Confirmed);
    assert!(matches!(
        &record.intent.operation,
        IssueFeedbackOperation::SetBoardField { from_option_id: Some(from), origin_option_id: Some(origin), .. }
            if from == "opt_ready" && origin == "opt_ready"
    ));
    assert_eq!(
        fixture
            .store
            .data
            .lock()
            .unwrap()
            .events
            .last()
            .unwrap()
            .action,
        IssueFeedbackAction::BoardFieldSet
    );

    // 같은 단계는 다시 쓰지 않는다.
    sync(&fixture, &board).unwrap();
    assert_eq!(board.write_count(), 1);

    // 다음 단계는 자기 쓰기 위에서 이어 쓰고 원래 값을 잇는다.
    set_execution(&fixture, IssueExecutionState::Done);
    sync(&fixture, &board).unwrap();
    assert_eq!(board.write_count(), 2);
    assert!(matches!(
        owned_board(&fixture),
        Some(IssueFeedbackEvidence::BoardField { option_id, origin_option_id: Some(origin), .. })
            if option_id == "opt_done" && origin == "opt_ready"
    ));
}

#[test]
fn a_column_changed_by_a_person_is_respected_on_the_next_reconciliation() {
    let fixture = board_fixture(true);
    let board = FakeBoard::in_column("opt_ready");
    sync(&fixture, &board).unwrap();
    assert_eq!(board.write_count(), 1);

    *board.selected.lock().unwrap() = Some("opt_blocked".into());
    set_execution(&fixture, IssueExecutionState::Done);
    sync(&fixture, &board).unwrap();
    assert_eq!(board.write_count(), 1);
    assert_eq!(
        board.selected.lock().unwrap().as_deref(),
        Some("opt_blocked")
    );
}

#[test]
fn the_first_write_starts_only_from_a_selected_column() {
    let fixture = board_fixture(true);
    let board = FakeBoard::in_column("opt_blocked");
    sync(&fixture, &board).unwrap();
    assert_eq!(board.write_count(), 0);
    assert!(fixture.store.data.lock().unwrap().records.is_empty());
}

#[test]
fn an_unreadable_board_is_not_written() {
    let fixture = board_fixture(true);
    let board = FakeBoard::in_column("opt_ready");
    board.read_fails.store(true, Ordering::SeqCst);
    sync(&fixture, &board).unwrap();
    assert_eq!(board.write_count(), 0);
    assert!(fixture.store.data.lock().unwrap().records.is_empty());
}

#[test]
fn a_lost_write_response_is_confirmed_by_reading_and_never_resent() {
    let fixture = board_fixture(true);
    let board = FakeBoard::in_column("opt_ready");
    board.lose_write_response.store(true, Ordering::SeqCst);
    // 응답도 잃고 확인 읽기도 실패한다.
    let failing_reads = board.clone();
    *fixture.store.after_record.lock().unwrap() = Some(Box::new(move || {
        failing_reads.read_fails.store(true, Ordering::SeqCst);
    }));
    sync(&fixture, &board).unwrap();
    assert_eq!(board.write_count(), 1);
    assert_eq!(
        fixture.store.record().state,
        IssueFeedbackIntentState::NeedsConfirmation
    );

    // 결과 모름이 남아 있으면 다시 보내지 않는다.
    sync(&fixture, &board).unwrap();
    assert_eq!(board.write_count(), 1);

    board.read_fails.store(false, Ordering::SeqCst);
    fixture
        .service()
        .with_board_fields(board.clone())
        .reconcile_pending(&IssueCollectionCancellation::new())
        .unwrap();
    assert_eq!(
        fixture.store.record().state,
        IssueFeedbackIntentState::Confirmed
    );
    assert_eq!(board.write_count(), 1);
}
