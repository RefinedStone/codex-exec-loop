//! 사용자 준비 요청마다 별도 worktree를 하나 만든다. worker pool이나 thread lifecycle에 편입하지 않는다.
use std::sync::Arc;

use anyhow::{Context, Result, ensure};

use crate::application::port::inbound::conversation_workspace_port::ConversationWorkspacePort;
use crate::application::port::outbound::conversation_workspace_store_port::{
    ConversationWorkspaceStorePort, ConversationWorkspaceTransaction,
};
use crate::application::port::outbound::git_history_port::GitHistoryPort;
use crate::application::port::outbound::git_workspace_port::GitWorkspacePort;
use crate::domain::conversation_workspace::{
    ConversationWorkspaceInventoryEntry, ConversationWorkspaceInventoryPage,
    ConversationWorkspaceInventoryQuery, ConversationWorkspaceQuery, ConversationWorkspaceRecord,
    ConversationWorkspaceState, PrepareConversationWorkspaceRequest, WORKSPACE_INVENTORY_LIMIT,
    request_key, validate_request_id,
};
use crate::domain::git_graph::GitGraphRefKind;
use crate::domain::git_operation::same_git_worktree_path;

pub struct ConversationWorkspaceService {
    history: Arc<dyn GitHistoryPort>,
    git: Arc<dyn GitWorkspacePort>,
    store: Arc<dyn ConversationWorkspaceStorePort>,
}

impl ConversationWorkspaceService {
    pub fn new(
        history: Arc<dyn GitHistoryPort>,
        git: Arc<dyn GitWorkspacePort>,
        store: Arc<dyn ConversationWorkspaceStorePort>,
    ) -> Self {
        Self {
            history,
            git,
            store,
        }
    }

    fn preserve(
        transaction: &mut dyn ConversationWorkspaceTransaction,
        mut record: ConversationWorkspaceRecord,
        reason: &str,
    ) -> Result<ConversationWorkspaceRecord> {
        record.state = ConversationWorkspaceState::Preserved;
        record.reason = Some(reason.to_string());
        transaction.save(&record)?;
        Ok(record)
    }

    fn reuse(
        &self,
        transaction: &mut dyn ConversationWorkspaceTransaction,
        mut record: ConversationWorkspaceRecord,
    ) -> Result<ConversationWorkspaceRecord> {
        if record.state == ConversationWorkspaceState::Preserved {
            return Ok(record);
        }
        let inspected = self
            .git
            .inspect_conversation_worktree(&record.repository_dir, &record.workspace_dir);
        match inspected {
            Ok(Some(actual))
                if actual.branch.as_deref() == Some(record.branch.as_str())
                    && (record.state == ConversationWorkspaceState::Prepared
                        || actual.head == record.base_commit) =>
            {
                // Prepared를 되살릴 때 사용자 파일·새 커밋을 초기화하지 않는다.
                if record.state == ConversationWorkspaceState::Preparing {
                    record.state = ConversationWorkspaceState::Prepared;
                    transaction.save(&record)?;
                }
                Ok(record)
            }
            _ => Self::preserve(transaction, record, "workspace_identity_unconfirmed"),
        }
    }
}

impl ConversationWorkspacePort for ConversationWorkspaceService {
    fn prepare(
        &self,
        request: PrepareConversationWorkspaceRequest,
    ) -> Result<ConversationWorkspaceRecord> {
        request.validate()?;
        ensure!(
            self.history.read_refs(&request.cwd)?.is_some(),
            "workspace preparation requires a Git repository"
        );
        let initial = self.git.read_condition(&request.cwd)?;
        ensure!(
            initial.refusal.is_none(),
            "repository Git execution is unsafe: {:?}",
            initial.refusal
        );
        let mut transaction = self.store.begin(&request.cwd)?;
        if let Some(record) = transaction.find(&request.request_id)? {
            ensure!(
                record.base_commit == request.base_commit,
                "requestId already names another baseCommit"
            );
            return self.reuse(transaction.as_mut(), record);
        }
        let repository = transaction.repository_dir().to_string();
        let refs = self
            .history
            .read_refs(&repository)?
            .context("workspace preparation requires a Git repository")?;
        ensure!(
            self.git
                .resolve_commit(&repository, &request.base_commit)?
                .as_deref()
                == Some(request.base_commit.as_str()),
            "baseCommit does not resolve to that exact commit"
        );
        let condition = self.git.read_condition(&repository)?;
        ensure!(
            condition.refusal.is_none(),
            "repository Git execution is unsafe: {:?}",
            condition.refusal
        );
        ensure!(
            condition.operation_in_progress.is_none(),
            "repository has an unfinished Git operation"
        );
        let key = request_key(&request.request_id);
        let workspace_dir = transaction
            .workspaces_dir()
            .join(&key)
            .to_str()
            .context("workspace path is not Unicode")?
            .to_string();
        let branch = format!("akra-conversation/{key}");
        ensure!(
            !refs
                .worktrees
                .iter()
                .any(|tree| same_git_worktree_path(&tree.path, &workspace_dir))
                && !refs
                    .refs
                    .iter()
                    .any(|reference| reference.kind == GitGraphRefKind::LocalBranch
                        && reference.name == branch),
            "the destination or branch exists without this request's ownership record"
        );
        let mut record = ConversationWorkspaceRecord {
            request_id: request.request_id,
            repository_dir: repository,
            workspace_dir,
            branch,
            base_commit: request.base_commit,
            state: ConversationWorkspaceState::Preparing,
            reason: None,
        };
        // git worktree add가 실행되기 전에 소유권과 목적지를 durable하게 남긴다.
        transaction.save(&record)?;
        if self
            .git
            .create_conversation_worktree(
                &record.repository_dir,
                &record.workspace_dir,
                &record.branch,
                &record.base_commit,
            )
            .is_err()
        {
            return Self::preserve(transaction.as_mut(), record, "worktree_creation_incomplete");
        }
        match self
            .git
            .inspect_conversation_worktree(&record.repository_dir, &record.workspace_dir)
        {
            Ok(Some(actual))
                if actual.branch.as_deref() == Some(record.branch.as_str())
                    && actual.head == record.base_commit =>
            {
                record.state = ConversationWorkspaceState::Prepared;
                transaction.save(&record)?;
                Ok(record)
            }
            _ => Self::preserve(
                transaction.as_mut(),
                record,
                "created_workspace_identity_unconfirmed",
            ),
        }
    }

    fn inventory(
        &self,
        query: ConversationWorkspaceInventoryQuery,
    ) -> Result<ConversationWorkspaceInventoryPage> {
        ensure!(
            std::path::Path::new(&query.cwd).is_absolute(),
            "cwd must be absolute"
        );
        if let Some(after) = &query.after {
            validate_request_id(after)?;
        }
        let mut records = self.store.inventory(&query.cwd, query.after.as_deref())?;
        let has_more = records.len() > WORKSPACE_INVENTORY_LIMIT;
        records.truncate(WORKSPACE_INVENTORY_LIMIT);
        let next_after = if has_more {
            records.last().map(|r| r.request_id.clone())
        } else {
            None
        };
        let mut items = Vec::with_capacity(records.len());
        for record in records {
            // A durable allocation is not durable thread ownership. Do not infer orphanhood
            // or safety from a clean tracked-file status or absent local subscription.
            let mut reasons = vec![
                "thread_ownership_unconfirmed".to_string(),
                "remote_retention_unconfirmed".to_string(),
            ];
            if record.state != ConversationWorkspaceState::Prepared {
                reasons.push("preparation_not_confirmed".into());
            }
            let current_head = match self
                .git
                .inspect_conversation_worktree(&record.repository_dir, &record.workspace_dir)
            {
                Ok(None) => {
                    reasons.push("workspace_missing".into());
                    None
                }
                Err(_) => {
                    reasons.push("workspace_identity_unconfirmed".into());
                    None
                }
                Ok(Some(actual)) => {
                    if actual.branch.as_deref() != Some(record.branch.as_str()) {
                        reasons.push("branch_changed".into());
                    }
                    if actual.head != record.base_commit {
                        reasons.push("head_changed".into());
                    }
                    match self.git.read_condition(&record.workspace_dir) {
                        Ok(condition) if condition.refusal.is_none() => {
                            if condition.tracked_changes {
                                reasons.push("tracked_changes".into());
                            }
                            if condition.operation_in_progress.is_some()
                                || condition.busy_branches.contains(&record.branch)
                            {
                                reasons.push("git_operation_in_progress".into());
                            }
                            // The condition port deliberately does not examine untracked files.
                            reasons.push("untracked_files_unconfirmed".into());
                        }
                        _ => reasons.push("workspace_status_unconfirmed".into()),
                    }
                    Some(actual.head)
                }
            };
            items.push(ConversationWorkspaceInventoryEntry {
                record,
                current_head,
                reasons,
                cleanup_allowed: false,
            });
        }
        Ok(ConversationWorkspaceInventoryPage { items, next_after })
    }

    fn get(
        &self,
        query: ConversationWorkspaceQuery,
    ) -> Result<Option<ConversationWorkspaceRecord>> {
        validate_request_id(&query.request_id)?;
        ensure!(
            std::path::Path::new(&query.cwd).is_absolute(),
            "cwd must be absolute"
        );
        self.store.get(&query.cwd, &query.request_id)
    }
}
