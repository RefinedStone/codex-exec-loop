//! 처리한 이슈 댓글 명령의 기록이다 (IA8-2, #2567). 댓글의 글은 저장하지 않는다.
use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, params};

use super::{SqliteIssueAutomationStoreAdapter, validate_reference_scope};
use crate::application::port::outbound::issue_comment_command_port::IssueCommentCommandStorePort;
use crate::domain::issue_automation::comment_command::{
    IssueCommentCommandKind, IssueCommentCommandRecord,
};
use crate::domain::issue_work::IssueReference;

pub(super) const TABLE: &str = "issue_comment_commands";
pub(super) const SCHEMA: &str = r#"
CREATE TABLE issue_comment_commands (
    workspace_dir TEXT COLLATE BINARY NOT NULL,
    issue_key TEXT COLLATE BINARY NOT NULL,
    comment_id TEXT COLLATE BINARY NOT NULL,
    author_id TEXT COLLATE BINARY NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('hold', 'release', 'retriage', 'approve')),
    intervention_command_id TEXT COLLATE BINARY,
    PRIMARY KEY (workspace_dir, issue_key, comment_id),
    CHECK ((kind = 'hold') = (intervention_command_id IS NOT NULL))
) STRICT, WITHOUT ROWID
"#;

pub(super) fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(SCHEMA)?;
    Ok(())
}

fn issue_key(issue: &IssueReference) -> Result<String> {
    Ok(serde_json::to_string(issue)?)
}

fn kind_label(kind: IssueCommentCommandKind) -> &'static str {
    match kind {
        IssueCommentCommandKind::Hold => "hold",
        IssueCommentCommandKind::Release => "release",
        IssueCommentCommandKind::Retriage => "retriage",
        IssueCommentCommandKind::Approve => "approve",
    }
}

fn parse_kind(label: &str) -> Result<IssueCommentCommandKind> {
    Ok(match label {
        "hold" => IssueCommentCommandKind::Hold,
        "release" => IssueCommentCommandKind::Release,
        "retriage" => IssueCommentCommandKind::Retriage,
        "approve" => IssueCommentCommandKind::Approve,
        _ => bail!("the issue comment command kind is unknown"),
    })
}

fn validate_id(value: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        bail!("an issue comment command requires bounded stable IDs")
    }
    Ok(())
}

impl IssueCommentCommandStorePort for SqliteIssueAutomationStoreAdapter {
    fn issue_comment_command(
        &self,
        workspace: &str,
        issue: &IssueReference,
        comment_id: &str,
    ) -> Result<Option<IssueCommentCommandRecord>> {
        validate_reference_scope(workspace, issue)?;
        self.with_connection(workspace, |connection, _guard| {
            let row: Option<(String, String, Option<String>)> = connection
                .query_row(
                    "SELECT author_id, kind, intervention_command_id FROM issue_comment_commands \
                     WHERE workspace_dir=?1 AND issue_key=?2 AND comment_id=?3",
                    params![workspace, issue_key(issue)?, comment_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            row.map(|(author_id, kind, intervention_command_id)| {
                Ok(IssueCommentCommandRecord {
                    issue: issue.clone(),
                    comment_id: comment_id.to_owned(),
                    author_id,
                    kind: parse_kind(&kind)?,
                    intervention_command_id,
                })
            })
            .transpose()
        })
    }

    fn record_issue_comment_command(
        &self,
        workspace: &str,
        record: &IssueCommentCommandRecord,
    ) -> Result<bool> {
        validate_reference_scope(workspace, &record.issue)?;
        validate_id(&record.comment_id)?;
        validate_id(&record.author_id)?;
        if let Some(command_id) = &record.intervention_command_id {
            validate_id(command_id)?;
        }
        self.with_connection(workspace, |connection, _guard| {
            let inserted = connection.execute(
                "INSERT INTO issue_comment_commands \
                 (workspace_dir, issue_key, comment_id, author_id, kind, intervention_command_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT DO NOTHING",
                params![
                    workspace,
                    issue_key(&record.issue)?,
                    record.comment_id,
                    record.author_id,
                    kind_label(record.kind),
                    record.intervention_command_id,
                ],
            )?;
            Ok(inserted == 1)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::Fixture;
    use super::*;
    use crate::domain::issue_work::{IssueProjectScope, IssueProvider};

    fn record(workspace: &str, kind: IssueCommentCommandKind) -> IssueCommentCommandRecord {
        IssueCommentCommandRecord {
            issue: IssueReference {
                provider: IssueProvider::GitHub,
                server_id: "github.com".into(),
                repository_id: "R_1".into(),
                issue_id: "I_1".into(),
                project_scope: IssueProjectScope {
                    workspace_dir: workspace.into(),
                },
            },
            comment_id: "IC_1".into(),
            author_id: "U_1".into(),
            kind,
            intervention_command_id: (kind == IssueCommentCommandKind::Hold)
                .then(|| "issue-comment:IC_1".into()),
        }
    }

    #[test]
    fn the_same_comment_is_recorded_once_and_survives_reopening() {
        let fixture = Fixture::new("comment-command-once");
        let workspace = fixture.cwd();
        let first = record(workspace, IssueCommentCommandKind::Hold);
        assert!(
            fixture
                .adapter()
                .record_issue_comment_command(workspace, &first)
                .unwrap()
        );
        let mut replay = first.clone();
        replay.kind = IssueCommentCommandKind::Approve;
        replay.intervention_command_id = None;
        let reopened = fixture.adapter();
        assert!(
            !reopened
                .record_issue_comment_command(workspace, &replay)
                .unwrap()
        );
        assert_eq!(
            reopened
                .issue_comment_command(workspace, &first.issue, "IC_1")
                .unwrap(),
            Some(first.clone())
        );
        assert_eq!(
            reopened
                .issue_comment_command(workspace, &first.issue, "IC_2")
                .unwrap(),
            None
        );
    }

    #[test]
    fn a_record_outside_the_workspace_or_without_a_hold_link_is_rejected() {
        let fixture = Fixture::new("comment-command-reject");
        let workspace = fixture.cwd();
        let store = fixture.adapter();
        let mut unlinked = record(workspace, IssueCommentCommandKind::Hold);
        unlinked.intervention_command_id = None;
        assert!(
            store
                .record_issue_comment_command(workspace, &unlinked)
                .is_err()
        );
        let other = fixture.root.join("other");
        assert!(
            store
                .record_issue_comment_command(
                    other.to_str().unwrap(),
                    &record(workspace, IssueCommentCommandKind::Release)
                )
                .is_err()
        );
    }

    #[test]
    fn a_version_14_ledger_gains_the_empty_comment_table() {
        let fixture = Fixture::new("comment-command-migration");
        let workspace = fixture.cwd();
        let command = record(workspace, IssueCommentCommandKind::Retriage);
        let store = fixture.adapter();
        assert_eq!(
            store
                .issue_comment_command(workspace, &command.issue, "IC_1")
                .unwrap(),
            None
        );
        let connection = Connection::open(fixture.database()).unwrap();
        super::super::tests::make_pre_comment_command_schema(&connection);
        connection.pragma_update(None, "user_version", 15).unwrap();
        drop(connection);
        assert!(
            fixture
                .adapter()
                .record_issue_comment_command(workspace, &command)
                .unwrap()
        );
        let connection = Connection::open(fixture.database()).unwrap();
        assert_eq!(
            super::super::schema_identity(&connection).unwrap(),
            (super::super::APPLICATION_ID, super::super::SCHEMA_VERSION)
        );
    }
}
