use super::*;
use crate::application::port::outbound::issue_comment_command_port::IssueCommentCommandSourcePort;
use crate::domain::issue_automation::comment_command::{
    IssueCommentCommand, IssueCommentCommandKind,
};

fn command_comment(id: &str, author: Value, body: &str, edited: bool) -> Value {
    json!({
        "__typename":"IssueComment","id":id,"body":body,
        "author":author,"editor":if edited { actor("U_editor") } else { Value::Null },
        "lastEditedAt":if edited { json!("2026-11-25T00:01:00Z") } else { Value::Null },
        "issue":issue_node(),"repository":{"id":"R_1"},"pullRequest":null
    })
}

#[test]
fn only_unedited_whole_commands_of_trusted_user_accounts_are_returned() {
    let (adapter, calls) = adapter_with(vec![
        Ok(comment_page(
            vec![
                command_comment("IC_hold", actor("U_trusted"), "/akra hold", false),
                command_comment("IC_text", actor("U_trusted"), "looks fine", false),
                command_comment("IC_other", actor("U_other"), "/akra release", false),
            ],
            5,
            Some("cursor-1"),
        )),
        Ok(comment_page(
            vec![
                command_comment("IC_edited", actor("U_trusted"), "/akra hold", true),
                command_comment(
                    "IC_bot",
                    json!({"__typename":"Bot","id":"U_trusted"}),
                    "/akra approve",
                    false,
                ),
            ],
            5,
            None,
        )),
    ]);
    let commands = adapter
        .read_comment_commands(&issue(), &["U_trusted".to_string()])
        .unwrap();
    assert_eq!(
        commands,
        vec![IssueCommentCommand {
            comment_id: "IC_hold".into(),
            author_id: "U_trusted".into(),
            kind: IssueCommentCommandKind::Hold,
        }]
    );
    assert_eq!(calls.lock().unwrap().len(), 2);
}

#[test]
fn an_incomplete_comment_read_is_an_error_and_not_an_empty_list() {
    let (adapter, _) = adapter_with(vec![Ok(comment_page(
        vec![command_comment(
            "IC_hold",
            actor("U_trusted"),
            "/akra hold",
            false,
        )],
        2,
        None,
    ))]);
    assert!(
        adapter
            .read_comment_commands(&issue(), &["U_trusted".to_string()])
            .is_err()
    );
}
