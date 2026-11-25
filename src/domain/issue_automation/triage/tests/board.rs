//! 보드 열 조건 (#2565).
use super::*;
use crate::domain::issue_automation::policy::IssueAutomationGitHubBoard;
use crate::domain::issue_automation::source::{IssueSourceBoardColumn, IssueSourceBoardRead};

fn board_policy() -> IssueAutomationPolicy {
    let mut policy = policy();
    policy.binding.github.board = Some(IssueAutomationGitHubBoard {
        project_id: "PVT_1".into(),
        field_id: "PVTSSF_status".into(),
        select_option_ids: vec!["ready".into()],
        progress: None,
    });
    policy
}

fn with_board(field_id: &str, read: IssueSourceBoardRead) -> IssueSourceRead {
    let mut current = source();
    observed(&mut current).board = Some(IssueSourceBoardColumn {
        project_id: "PVT_1".into(),
        field_id: field_id.into(),
        read,
    });
    current
}

fn item(option: Option<&str>) -> IssueSourceBoardRead {
    IssueSourceBoardRead::Item {
        item_id: "PVTI_1".into(),
        option_id: option.map(Into::into),
    }
}

#[test]
fn only_issues_in_the_selected_column_are_selected() {
    let policy = board_policy();
    let selected = run(
        Some(&with_board("PVTSSF_status", item(Some("ready")))),
        &policy,
        None,
    )
    .expect("selected column");
    assert_eq!(selected.eligibility(), IssueTriageEligibility::Eligible);

    for read in [
        item(Some("done")),
        item(None),
        IssueSourceBoardRead::NotOnBoard,
    ] {
        let result =
            run(Some(&with_board("PVTSSF_status", read)), &policy, None).expect("other column");
        assert_eq!(result.eligibility(), IssueTriageEligibility::Ineligible);
        assert!(
            result
                .reasons()
                .contains(&IssueTriageReasonCode::SelectionNotMatched)
        );
        assert!(
            !result
                .reasons()
                .contains(&IssueTriageReasonCode::BoardUnavailable)
        );
    }
}

#[test]
fn the_column_condition_adds_to_the_label_condition() {
    let policy = board_policy();
    let mut current = with_board("PVTSSF_status", item(Some("ready")));
    observed(&mut current).labels.items.clear();
    let result = run(Some(&current), &policy, None).expect("no selection label");
    assert_eq!(result.eligibility(), IssueTriageEligibility::Ineligible);
    let count = result
        .reasons()
        .iter()
        .filter(|reason| **reason == IssueTriageReasonCode::SelectionNotMatched)
        .count();
    assert_eq!(count, 1);
}

#[test]
fn an_unreadable_board_is_unknown_with_its_own_reason_and_not_a_mismatch() {
    let policy = board_policy();
    // read:project 가 없는 로그인, 보드 사실이 없는 과거 관찰, 다른 필드의 관찰.
    for current in [
        with_board("PVTSSF_status", IssueSourceBoardRead::Unavailable),
        source(),
        with_board("PVTSSF_other", item(Some("ready"))),
    ] {
        let result = run(Some(&current), &policy, None).expect("unreadable board");
        assert_eq!(result.eligibility(), IssueTriageEligibility::Unknown);
        assert!(
            result
                .reasons()
                .contains(&IssueTriageReasonCode::BoardUnavailable)
        );
        assert!(
            !result
                .reasons()
                .contains(&IssueTriageReasonCode::SelectionNotMatched)
        );
    }
}

#[test]
fn a_board_without_a_column_condition_does_not_filter() {
    let mut policy = board_policy();
    policy
        .binding
        .github
        .board
        .as_mut()
        .unwrap()
        .select_option_ids
        .clear();
    let result = run(Some(&source()), &policy, None).expect("no column condition");
    assert_eq!(result.eligibility(), IssueTriageEligibility::Eligible);
}
