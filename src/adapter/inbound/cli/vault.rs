use std::ffi::OsString;
use std::io::{self, BufRead, IsTerminal, Read, Write};

use anyhow::{Context, Result, bail};

use crate::application::port::outbound::evidence_vault_port::{
    EvidenceVaultStorePort, VaultLifecycleAction, VaultRecordLoadRequest, VaultRecordScope,
};
use crate::application::port::outbound::issue_publish_intent_journal_port::IssuePublishIntentJournalPort;
use crate::application::service::evidence_vault_lifecycle;
use crate::domain::issue_work::{IssueProjectScope, IssueProvider, IssueReference};

pub(super) const USAGE: &str = "Usage: akra vault <retain|delete> <workspace> <github|jira|linear> <server-id> <repository-id> <issue-id>";

pub(super) fn run(args: &[OsString], output: &mut impl Write) -> Result<i32> {
    let [action, workspace, provider, server, repository, issue] = args else {
        bail!("{USAGE}");
    };
    let action = match action.to_str() {
        Some("retain") => VaultLifecycleAction::RetainRecent,
        Some("delete") => VaultLifecycleAction::DeleteIssue,
        _ => bail!("{USAGE}"),
    };
    let provider = match provider.to_str() {
        Some("github") => IssueProvider::GitHub,
        Some("jira") => IssueProvider::Jira,
        Some("linear") => IssueProvider::Linear,
        _ => bail!("{USAGE}"),
    };
    let issue_text = coordinate(issue)?;
    let server_id = coordinate(server)?.to_owned();
    let repository_id = coordinate(repository)?.to_owned();
    let input = io::stdin();
    if action == VaultLifecycleAction::DeleteIssue && !input.is_terminal() {
        bail!("Vault deletion requires interactive confirmation");
    }
    let local = crate::composition::desktop_restore::local_maintenance(coordinate(workspace)?)?;
    let request_scope = VaultRecordScope::current_version(IssueReference {
        provider,
        server_id,
        repository_id,
        issue_id: issue_text.to_owned(),
        project_scope: IssueProjectScope {
            workspace_dir: local.workspace,
        },
    });
    execute(
        (local.vault.as_ref(), local.journal.as_ref()),
        Execution {
            request: VaultRecordLoadRequest {
                scope: request_scope,
                key: local.key,
            },
            action,
        },
        (&mut input.lock(), output),
    )
}

fn coordinate(value: &OsString) -> Result<&str> {
    value
        .to_str()
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 4096
                && value.trim() == *value
                && !value.chars().any(char::is_control)
        })
        .context("invalid Vault issue coordinate")
}

struct Execution {
    request: VaultRecordLoadRequest,
    action: VaultLifecycleAction,
}

fn execute(
    stores: (
        &dyn EvidenceVaultStorePort,
        &dyn IssuePublishIntentJournalPort,
    ),
    operation: Execution,
    io: (&mut impl BufRead, &mut impl Write),
) -> Result<i32> {
    let Execution { request, action } = operation;
    let (input, output) = io;
    let scope = request.scope.issue.clone();
    let result = evidence_vault_lifecycle::maintain(stores, request, || {
        if action == VaultLifecycleAction::DeleteIssue {
            writeln!(
                output,
                "Delete local Vault records for {:?} {}/{}/{} in {}?",
                scope.provider,
                scope.server_id,
                scope.repository_id,
                scope.issue_id,
                scope.project_scope.workspace_dir
            )?;
            writeln!(
                output,
                "This blocks restore and import for this issue. Backups and unverified legacy records are not erased."
            )?;
            writeln!(output, "Type DELETE {} to continue.", scope.issue_id)?;
            output.flush()?;
            let mut answer = String::new();
            input.take(8192).read_line(&mut answer)?;
            if answer.trim_end_matches(['\r', '\n']) != format!("DELETE {}", scope.issue_id) {
                return Ok(None);
            }
        }
        Ok(Some(action))
    })?;
    let Some(report) = result else {
        writeln!(output, "Cancelled; no Vault records changed.")?;
        return Ok(1);
    };
    writeln!(
        output,
        "Tombstone: {}; removed: {}; retained: {}; cleanup pending: {}.",
        report.tombstoned, report.removed, report.retained, report.cleanup_pending
    )?;
    if report.legacy_unverified {
        writeln!(
            output,
            "Unindexed legacy records were not verified or erased."
        )?;
    }
    Ok(
        if report.cleanup_pending > 0
            || (report.tombstoned && (report.retained > 0 || report.legacy_unverified))
        {
            2
        } else {
            0
        },
    )
}

#[cfg(all(test, any(unix, windows)))]
mod tests;
