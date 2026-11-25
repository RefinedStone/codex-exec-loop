//! 신뢰하는 사람의 이슈 댓글 명령을 수집 주기마다 처리한다 (IA8-2, #2567, plan 28 §10.2).
//! 댓글은 사람 확인의 증거가 아니다. 실행 중인 에이전트도 사용자 계정으로 댓글을 쓸 수 있다.
//! 그래서 `hold` 만 기존 개입 경로로 적용하고 다른 명령은 적용하지 않고 기록만 한다.
use std::sync::Arc;

use anyhow::Result;

use crate::application::port::inbound::issue_intervention_port::IssueInterventionPort;
use crate::application::port::inbound::stored_command_port::StoredCommandContext;
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::application::port::outbound::issue_comment_command_port::{
    IssueCommentCommandSourcePort, IssueCommentCommandStorePort,
};
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::application::service::issue_collection::IssueCollectionCancellation;
use crate::domain::issue_automation::collection::IssueCollectionItem;
use crate::domain::issue_automation::comment_command::{
    IssueCommentCommand, IssueCommentCommandKind, IssueCommentCommandRecord,
    issue_comment_intervention_command_id,
};
use crate::domain::issue_automation::intervention::{
    IssueInterventionAction, IssueInterventionReceipt, IssueInterventionRequest,
};
use crate::domain::issue_automation::source::{IssueRemoteState, IssueSourceRead};
use crate::domain::issue_work::{IssueProvider, IssueReference};

const GITHUB_SERVER_ID: &str = "github.com";
const COMMENT_CHANNEL: &str = "issue_comment";

pub struct IssueCommentCommandService {
    workspace: String,
    store: Arc<dyn IssueAutomationStorePort>,
    records: Arc<dyn IssueCommentCommandStorePort>,
    source: Arc<dyn IssueCommentCommandSourcePort>,
    policy: IssueAutomationPolicyService,
    interventions: Arc<dyn IssueInterventionPort>,
}

impl IssueCommentCommandService {
    pub fn new(
        workspace: String,
        store: Arc<dyn IssueAutomationStorePort>,
        records: Arc<dyn IssueCommentCommandStorePort>,
        source: Arc<dyn IssueCommentCommandSourcePort>,
        policy: IssueAutomationPolicyService,
        interventions: Arc<dyn IssueInterventionPort>,
    ) -> Self {
        Self {
            workspace,
            store,
            records,
            source,
            policy,
            interventions,
        }
    }

    /// 새로 바뀐 보류의 영수증을 돌려준다. 한 이슈의 읽기 실패는 다른 이슈를 막지 않는다.
    pub fn poll(
        &self,
        cancel: &IssueCollectionCancellation,
        at_unix_ms: u64,
    ) -> Result<Vec<IssueInterventionReceipt>> {
        let context = comment_context(at_unix_ms);
        let trusted = self.trusted_actor_ids();
        let mut receipts = Vec::new();
        if trusted.is_empty() {
            return Ok(receipts);
        }
        let mut after = None;
        loop {
            let batch = self
                .store
                .read_triage_batch(&self.workspace, after.as_ref(), 100)?;
            for item in batch.items {
                if cancel.is_cancelled() {
                    return Ok(receipts);
                }
                if !is_open_tracked_github_issue(&item) {
                    continue;
                }
                let commands = match self.source.read_comment_commands(&item.issue, &trusted) {
                    Ok(commands) => commands,
                    Err(_) => {
                        tracing::warn!(
                            "issue comment commands were not read; the next cycle retries"
                        );
                        continue;
                    }
                };
                for command in commands {
                    if !trusted.contains(&command.author_id) {
                        continue;
                    }
                    let receipt =
                        cancel.commit_if_active(|| self.process(&item.issue, command, &context))?;
                    match receipt {
                        None => return Ok(receipts),
                        Some(Some(receipt)) if receipt.changed => receipts.push(receipt),
                        Some(_) => {}
                    }
                }
            }
            after = batch.next_after;
            if after.is_none() {
                return Ok(receipts);
            }
        }
    }

    /// 정책은 사용자 파일이 켜야 한다. 저장소 파일은 목록을 줄이거나 끄기만 한다 (plan 28 §4 원칙 5).
    fn trusted_actor_ids(&self) -> Vec<String> {
        let Ok(resolved) = self.policy.resolve(&self.workspace) else {
            return Vec::new();
        };
        if !resolved.policy.select.comment_commands {
            return Vec::new();
        }
        resolved
            .policy
            .select
            .trusted_actors
            .into_iter()
            .filter(|actor| {
                actor.provider == IssueProvider::GitHub && actor.server_id == GITHUB_SERVER_ID
            })
            .map(|actor| actor.actor_id)
            .collect()
    }

    /// 보류를 먼저 적용하고 기록한다. 그 사이에 끝나도 같은 개입 command_id 가 영수증을 다시 준다.
    fn process(
        &self,
        issue: &IssueReference,
        command: IssueCommentCommand,
        context: &StoredCommandContext,
    ) -> Result<Option<IssueInterventionReceipt>> {
        if self
            .records
            .issue_comment_command(&self.workspace, issue, &command.comment_id)?
            .is_some()
        {
            return Ok(None);
        }
        let mut record = IssueCommentCommandRecord {
            issue: issue.clone(),
            comment_id: command.comment_id,
            author_id: command.author_id,
            kind: command.kind,
            intervention_command_id: None,
        };
        let mut receipt = None;
        if command.kind == IssueCommentCommandKind::Hold {
            let command_id = issue_comment_intervention_command_id(&record.comment_id);
            let request = IssueInterventionRequest {
                command_id: command_id.clone(),
                issue: issue.clone(),
                action: IssueInterventionAction::Hold,
            };
            match self.interventions.intervene(request, context) {
                Ok(applied) => receipt = Some(applied),
                Err(_) => {
                    tracing::warn!("an issue comment hold was not applied; the next cycle retries");
                    return Ok(None);
                }
            }
            record.intervention_command_id = Some(command_id);
        }
        self.records
            .record_issue_comment_command(&self.workspace, &record)?;
        Ok(receipt)
    }
}

/// 댓글 계정은 사람 확인이 아니다. 확인 값은 언제나 없다.
fn comment_context(at_unix_ms: u64) -> StoredCommandContext {
    let at = i64::try_from(at_unix_ms)
        .ok()
        .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
        .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .unwrap_or_default();
    StoredCommandContext {
        human_confirmation: None,
        caller: Some(COMMENT_CHANNEL.to_owned()),
        channel: COMMENT_CHANNEL.to_owned(),
        at,
    }
}

fn is_open_tracked_github_issue(item: &IssueCollectionItem) -> bool {
    item.tracked.is_some()
        && item.issue.provider == IssueProvider::GitHub
        && matches!(
            &item.current,
            Some(IssueSourceRead::Observed(observation))
                if observation.remote_state == IssueRemoteState::Open
        )
}

#[cfg(test)]
mod tests;
