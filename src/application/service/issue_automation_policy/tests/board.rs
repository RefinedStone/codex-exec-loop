//! Projects 보드 정책 (#2565).
use super::*;

const BOARD: &str = "[binding.github.board]\nproject_id = \"PVT_1\"\nfield_id = \"PVTSSF_1\"\nselect_option_ids = [\"ready\"]\n";
const PROGRESS: &str = "[binding.github.board.progress]\nrunning = \"running\"\n";

#[test]
fn a_policy_without_a_board_keeps_its_canonical_version() {
    let before = resolve_issue_automation_policy(None, None).unwrap();
    let explicit_off =
        resolve_issue_automation_policy(Some("[feedback]\nboard_field = false\n"), None).unwrap();
    assert_eq!(explicit_off.version, before.version);
    assert!(before.policy.binding.github.board.is_none());
}

#[test]
fn only_the_user_file_grants_board_writes_and_their_options() {
    let user = format!("[feedback]\nboard_field = true\n\n{BOARD}\n{PROGRESS}");
    let granted = resolve_issue_automation_policy(Some(&user), None).unwrap();
    assert!(granted.policy.feedback.board_field);
    let board = granted.policy.binding.github.board.unwrap();
    assert_eq!(board.progress.unwrap().running.as_deref(), Some("running"));

    // 저장소 파일은 쓰기를 켜지 못하고 진행 선택지를 정하지 못한다.
    let project =
        resolve_issue_automation_policy(None, Some("[feedback]\nboard_field = true\n")).unwrap();
    assert!(!project.policy.feedback.board_field);
    let blocked =
        resolve_issue_automation_policy(Some(&user), Some(&format!("{BOARD}\n{PROGRESS}")))
            .unwrap_err();
    assert_eq!(blocked.field, "binding.github.board.progress");
    assert_eq!(blocked.reason, "user_scope_only");

    // 저장소 파일이 같은 보드의 열 조건만 바꾸면 사용자 선택지를 잇는다.
    let narrowed = resolve_issue_automation_policy(
        Some(&user),
        Some(&BOARD.replace("[\"ready\"]", "[\"triaged\"]")),
    )
    .unwrap()
    .policy
    .binding
    .github
    .board
    .unwrap();
    assert_eq!(narrowed.select_option_ids, vec!["triaged".to_string()]);
    assert!(narrowed.progress.is_some());

    // 다른 보드를 고르면 사용자가 허가한 선택지는 따라가지 않는다.
    let other =
        resolve_issue_automation_policy(Some(&user), Some(&BOARD.replace("PVT_1", "PVT_2")))
            .unwrap()
            .policy
            .binding
            .github
            .board
            .unwrap();
    assert_eq!(other.project_id, "PVT_2");
    assert!(other.progress.is_none());
}

#[test]
fn invalid_board_identifiers_block_the_file() {
    let blocked =
        resolve_issue_automation_policy(None, Some(&BOARD.replace("PVT_1", "Board name")))
            .unwrap_err();
    assert_eq!(blocked.field, "binding.github.board.project_id");
}
