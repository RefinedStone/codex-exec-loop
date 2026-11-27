/*
 * git 작업 엔진이다 (plan 27 G3). 동작은 현재 브랜치의 reset, 체크아웃하지 않은 브랜치의 이동, 체크아웃이다.
 *
 * 순서는 항상 같다: 계획을 만든다 → 실행할 때 계획을 다시 만들어 기대 상태를 견준다 → 일지에 시작을 적는다 →
 * 브랜치를 옮기는 작업이면 백업 ref 를 쓴다 → git 명령을 실행한다 → 일지에 끝을 적는다. 되돌리기는 같은 길로 거꾸로
 * 가는 작업 하나이다.
 *
 * Gateway 가 넘기는 턴은 `TurnActivityRegistry` 가 기억한다. 계획은 이 작업 트리에 폴더가 있는 스레드의 턴이 실행
 * 중이면 막는다 (#2850). 그래서 Desktop 밖의 클라이언트(CLI 등)의 요청도 거절한다. Gateway 를 거치지 않는 턴과
 * 폴더를 모르는 턴은 막지 못한다.
 *
 * 큐는 저장소마다 하나이다 (plan 27 §3.4). 같은 저장소의 모든 작업 트리가 같은 큐를 쓴다.
 */
mod sequence;
use crate::domain::git_operation::cherry_pick::{PickAction, PickPrediction};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};

use crate::application::port::inbound::git_operation_port::{
    GitOperationOutcome, GitOperationPort,
};
use crate::application::port::outbound::git_history_port::{GitHistoryPort, GitHistoryRefs};
use crate::application::port::outbound::git_operation_journal_port::GitOperationJournalPort;
use crate::application::port::outbound::git_workspace_port::{
    GitWorkspaceCondition, GitWorkspacePort,
};
use crate::application::service::turn_activity::TurnActivityRegistry;
use crate::domain::git_graph::{
    GIT_BACKUP_REF_PREFIX, GitGraphRefKind, GitGraphWorktree, is_git_commit_id,
};
use crate::domain::git_operation::{
    GIT_AGENT_BRANCH_PREFIX, GIT_OPERATION_LIST_LIMIT, GitHeadPosition, GitOperation,
    GitOperationBlocker, GitOperationBlockerCode, GitOperationExpectation, GitOperationPlan,
    GitOperationRecord, GitOperationStatus, git_backup_ref_name, git_backups_to_prune,
    git_operation_id, git_worktree_containing, same_git_worktree_path,
};

/// 지금 시각(Unix 초)과 작업 id 를 가르는 임의의 수이다.
pub type GitOperationStamp = Arc<dyn Fn() -> (i64, u32) + Send + Sync>;

pub struct GitOperationService {
    history: Arc<dyn GitHistoryPort>,
    workspace: Arc<dyn GitWorkspacePort>,
    journal: Arc<dyn GitOperationJournalPort>,
    stamp: GitOperationStamp,
    turns: Arc<TurnActivityRegistry>,
    /// 저장소(대표 작업 트리)마다 큐 하나이다.
    queues: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl GitOperationService {
    pub fn new(
        history: Arc<dyn GitHistoryPort>,
        workspace: Arc<dyn GitWorkspacePort>,
        journal: Arc<dyn GitOperationJournalPort>,
        stamp: GitOperationStamp,
        turns: Arc<TurnActivityRegistry>,
    ) -> Self {
        Self {
            history,
            workspace,
            journal,
            stamp,
            turns,
            queues: Mutex::new(HashMap::new()),
        }
    }

    /// 저장소의 큐이다. 저장소의 신원은 대표 작업 트리(`git worktree list` 의 첫째)이다. 읽지 못하면 폴더이다.
    fn repository_queue(&self, repository_dir: &str) -> Arc<Mutex<()>> {
        let identity = self
            .history
            .read_refs(repository_dir)
            .ok()
            .flatten()
            .and_then(|refs| refs.worktrees.first().map(|main| main.path.clone()))
            .unwrap_or_else(|| repository_dir.to_string());
        // shortcut: lower-case keys merge case-different paths on case-sensitive file systems; that only serializes more.
        let key = identity.replace('\\', "/").to_ascii_lowercase();
        self.queues
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(key)
            .or_default()
            .clone()
    }
}

impl GitOperationPort for GitOperationService {
    fn plan(&self, repository_dir: &str, operation: GitOperation) -> Result<GitOperationPlan> {
        Ok(self.build_plan(repository_dir, operation)?.plan)
    }

    fn execute(
        &self,
        repository_dir: &str,
        operation: GitOperation,
        expectation: GitOperationExpectation,
    ) -> Result<GitOperationOutcome> {
        let queue = self.repository_queue(repository_dir);
        let _queue = queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let plan = self.build_plan(repository_dir, operation)?.plan;
        if plan.expectation != expectation {
            return Ok(GitOperationOutcome::Stale { plan });
        }
        self.execute_plan(repository_dir, plan, None)
    }

    fn operations(&self, repository_dir: &str) -> Result<Vec<GitOperationRecord>> {
        let mut records = self.journal.load(repository_dir)?;
        records.reverse();
        records.truncate(GIT_OPERATION_LIST_LIMIT);
        Ok(records)
    }

    fn resolve_pick(
        &self,
        cwd: &str,
        op_id: &str,
        action: PickAction,
    ) -> Result<GitOperationOutcome> {
        let queue = self.repository_queue(cwd);
        let _lock = queue.lock().unwrap_or_else(|p| p.into_inner());
        self.resume_sequence(cwd, op_id, action)
    }

    fn undo(&self, repository_dir: &str, op_id: &str) -> Result<GitOperationOutcome> {
        let queue = self.repository_queue(repository_dir);
        let _queue = queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let records = self.journal.load(repository_dir)?;
        let Some(record) = records.into_iter().find(|record| record.op_id == op_id) else {
            return Ok(refused("the journal has no operation with this id"));
        };
        if !record.can_undo() {
            return Ok(refused(
                "this operation did not finish, was undone before, or is an undo itself",
            ));
        }
        // 일지는 모든 작업 트리가 함께 쓴다. reset 과 체크아웃은 작업 트리 하나의 HEAD 를 바꿨다. 다른 작업 트리에서
        // 되돌리면 그 작업 트리를 바꾼다.
        if record.is_bound_to_worktree()
            && !same_git_worktree_path(&record.worktree, repository_dir)
        {
            return Ok(refused(
                "this operation ran in another worktree; undo it from that worktree",
            ));
        }
        let mut observed = self.build_plan(repository_dir, record.inverse())?;
        let branch_commit = match &record.operation {
            GitOperation::MoveBranch { branch, .. } => observed
                .refs
                .as_ref()
                .and_then(|refs| local_branch_commit(refs, branch)),
            _ => None,
        };
        if !record.still_applies(observed.plan.expectation.head.as_ref(), branch_commit) {
            return Ok(refused(
                "the repository changed after this operation, so the undo would discard that change",
            ));
        }
        let is_sequence = matches!(record.operation, GitOperation::CherryPick { .. });
        if is_sequence {
            observed.plan.branch = None;
        } // An undo must restore the original set of refs.
        let outcome = self.execute_plan(repository_dir, observed.plan, Some(op_id.to_string()))?;
        if let GitOperationOutcome::Done { record: undo } = &outcome {
            if is_sequence && let Some(backup) = &record.backup_ref {
                self.workspace
                    .release_backup(repository_dir, backup, &record.before.commit)?;
            }
            let undone = GitOperationRecord {
                undone_by: Some(undo.op_id.clone()),
                ..record
            };
            if let Err(error) = self.journal.append(repository_dir, &undone) {
                tracing::warn!(error = %format!("{error:#}"), "git operation journal: undo mark was not written");
            }
        }
        Ok(outcome)
    }
}

fn refused(message: &str) -> GitOperationOutcome {
    GitOperationOutcome::Refused {
        message: message.to_string(),
    }
}

/// 계획과, 계획을 만들 때 읽은 ref 들이다.
struct ObservedPlan {
    plan: GitOperationPlan,
    refs: Option<GitHistoryRefs>,
}

impl GitOperationService {
    fn build_plan(&self, repository_dir: &str, operation: GitOperation) -> Result<ObservedPlan> {
        validate_operation(&operation)?;
        let mut plan = GitOperationPlan {
            picks: Vec::new(),
            operation: operation.clone(),
            expectation: GitOperationExpectation::default(),
            branch: None,
            before: None,
            after: None,
            impact: None,
            blockers: Vec::new(),
        };
        let Some(refs) = self.history.read_refs(repository_dir)? else {
            block(
                &mut plan,
                GitOperationBlockerCode::NotRepository,
                "this folder is not in a git repository",
            );
            return Ok(ObservedPlan { plan, refs: None });
        };
        plan.expectation.head = head_position(&refs);
        let Some(head) = plan.expectation.head.clone() else {
            block(
                &mut plan,
                GitOperationBlockerCode::NoCommits,
                "this repository has no commit",
            );
            return Ok(ObservedPlan {
                plan,
                refs: Some(refs),
            });
        };
        let condition = self.workspace.read_condition(repository_dir)?;
        if let Some(reason) = &condition.refusal {
            block(&mut plan, GitOperationBlockerCode::ExecutionConfig, reason);
            return Ok(ObservedPlan {
                plan,
                refs: Some(refs),
            });
        }
        if let Some(name) = &condition.operation_in_progress {
            block(
                &mut plan,
                GitOperationBlockerCode::OperationInProgress,
                format!("a {name} is in progress in this worktree"),
            );
        }
        if self.journal.load(repository_dir)?.iter().any(|r| {
            r.pick_index.is_some()
                && matches!(
                    r.status,
                    GitOperationStatus::Paused | GitOperationStatus::Started
                )
                && same_git_worktree_path(&r.worktree, repository_dir)
        }) {
            block(
                &mut plan,
                GitOperationBlockerCode::OperationInProgress,
                "a journalled cherry-pick needs to finish first",
            );
        }
        self.block_running_turn(&refs, repository_dir, &mut plan);
        match &operation {
            GitOperation::CherryPick { commits } => {
                plan.before = Some(head.clone());
                plan.branch = head.branch.clone();
                if let Some(branch) = &head.branch {
                    block_agent_branch(&mut plan, branch);
                } else {
                    block(
                        &mut plan,
                        GitOperationBlockerCode::DetachedHead,
                        "select a local branch before cherry-picking",
                    );
                }
                if condition.tracked_changes {
                    block(
                        &mut plan,
                        GitOperationBlockerCode::UncommittedChanges,
                        "commit or stash tracked changes first",
                    );
                }
                if plan.blockers.is_empty() {
                    plan.picks =
                        self.workspace
                            .preview_picks(repository_dir, &head.commit, commits)?;
                    if plan
                        .picks
                        .iter()
                        .all(|p| p.prediction == PickPrediction::AlreadyApplied)
                    {
                        block(
                            &mut plan,
                            GitOperationBlockerCode::NothingToDo,
                            "all selected patches are already applied",
                        );
                    }
                }
            }
            GitOperation::Reset { target } => {
                self.plan_reset(repository_dir, &head, &condition, target, &mut plan)?;
            }
            GitOperation::MoveBranch { branch, target } => {
                self.plan_move(repository_dir, &refs, &condition, branch, target, &mut plan)?;
            }
            GitOperation::CheckoutBranch { branch } => {
                plan_checkout_branch(&refs, &condition, &head, branch, &mut plan);
            }
            GitOperation::CheckoutCommit { target } => {
                self.plan_checkout_commit(repository_dir, &head, target, &mut plan)?;
            }
        }
        Ok(ObservedPlan {
            plan,
            refs: Some(refs),
        })
    }

    /// 이 작업 트리에 폴더가 있는 스레드의 턴이 실행 중이면 막는다.
    fn block_running_turn(
        &self,
        refs: &GitHistoryRefs,
        repository_dir: &str,
        plan: &mut GitOperationPlan,
    ) {
        let Some(target) = git_worktree_containing(&refs.worktrees, repository_dir) else {
            return;
        };
        let busy = self.turns.running_folders().iter().any(|folder| {
            git_worktree_containing(&refs.worktrees, folder)
                .is_some_and(|worktree| same_git_worktree_path(&worktree.path, &target.path))
        });
        if busy {
            block(
                plan,
                GitOperationBlockerCode::TurnRunning,
                "a thread whose folder is in this worktree has a turn in progress; wait until it ends",
            );
        }
    }

    fn plan_reset(
        &self,
        repository_dir: &str,
        head: &GitHeadPosition,
        condition: &GitWorkspaceCondition,
        target: &str,
        plan: &mut GitOperationPlan,
    ) -> Result<()> {
        plan.before = Some(head.clone());
        let Some(branch) = head.branch.clone() else {
            block(
                plan,
                GitOperationBlockerCode::DetachedHead,
                "HEAD is not on a branch, so there is no current branch to move",
            );
            return Ok(());
        };
        plan.branch = Some(branch.clone());
        block_agent_branch(plan, &branch);
        if condition.tracked_changes {
            block(
                plan,
                GitOperationBlockerCode::UncommittedChanges,
                "tracked files have uncommitted changes; commit or stash them first",
            );
        }
        self.plan_branch_move(repository_dir, &branch, &head.commit, target, plan)
    }

    fn plan_move(
        &self,
        repository_dir: &str,
        refs: &GitHistoryRefs,
        condition: &GitWorkspaceCondition,
        branch: &str,
        target: &str,
        plan: &mut GitOperationPlan,
    ) -> Result<()> {
        let Some(current) = local_branch_commit(refs, branch) else {
            block(
                plan,
                GitOperationBlockerCode::UnknownBranch,
                format!("there is no local branch `{branch}`"),
            );
            return Ok(());
        };
        let current = current.to_string();
        plan.expectation.branch_commit = Some(current.clone());
        plan.branch = Some(branch.to_string());
        plan.before = Some(GitHeadPosition {
            commit: current.clone(),
            branch: Some(branch.to_string()),
        });
        block_agent_branch(plan, branch);
        block_branch_in_use(refs, condition, branch, plan);
        self.plan_branch_move(repository_dir, branch, &current, target, plan)
    }

    /// 브랜치를 `from` 에서 `target` 으로 옮기는 계획의 공통 부분이다: 대상 커밋, 작업 뒤의 자리, 영향.
    fn plan_branch_move(
        &self,
        repository_dir: &str,
        branch: &str,
        from: &str,
        target: &str,
        plan: &mut GitOperationPlan,
    ) -> Result<()> {
        let Some(to) = self.resolve_target(repository_dir, target, plan)? else {
            return Ok(());
        };
        plan.after = Some(GitHeadPosition {
            commit: to.clone(),
            branch: Some(branch.to_string()),
        });
        if to == from {
            block(
                plan,
                GitOperationBlockerCode::NothingToDo,
                format!("`{branch}` is already at this commit"),
            );
            return Ok(());
        }
        plan.impact = Some(
            self.workspace
                .read_move_impact(repository_dir, branch, from, &to)?,
        );
        Ok(())
    }

    fn plan_checkout_commit(
        &self,
        repository_dir: &str,
        head: &GitHeadPosition,
        target: &str,
        plan: &mut GitOperationPlan,
    ) -> Result<()> {
        plan.before = Some(head.clone());
        if let Some(branch) = &head.branch {
            block_agent_branch(plan, branch);
        }
        let Some(to) = self.resolve_target(repository_dir, target, plan)? else {
            return Ok(());
        };
        if head.branch.is_none() && head.commit == to {
            block(
                plan,
                GitOperationBlockerCode::NothingToDo,
                "HEAD is already at this commit without a branch",
            );
        }
        plan.after = Some(GitHeadPosition {
            commit: to,
            branch: None,
        });
        Ok(())
    }

    fn resolve_target(
        &self,
        repository_dir: &str,
        target: &str,
        plan: &mut GitOperationPlan,
    ) -> Result<Option<String>> {
        let resolved = self.workspace.resolve_commit(repository_dir, target)?;
        if resolved.is_none() {
            block(
                plan,
                GitOperationBlockerCode::UnknownCommit,
                "the repository has no such commit",
            );
        }
        Ok(resolved)
    }

    /// 막는 이유가 없는 계획을 실행하고 일지에 적는다. 큐 안에서 부른다.
    fn execute_plan(
        &self,
        repository_dir: &str,
        plan: GitOperationPlan,
        undo_of: Option<String>,
    ) -> Result<GitOperationOutcome> {
        if plan.blockers.is_empty() && matches!(plan.operation, GitOperation::CherryPick { .. }) {
            return self.start_sequence(repository_dir, plan);
        }
        let (Some(before), Some(after), true) = (
            plan.before.clone(),
            plan.after.clone(),
            plan.blockers.is_empty(),
        ) else {
            return Ok(GitOperationOutcome::Stale { plan });
        };
        let (started_at, nonce) = (self.stamp)();
        let op_id = git_operation_id(started_at, nonce);
        let backup_ref = plan
            .branch
            .as_deref()
            .map(|branch| git_backup_ref_name(branch, &op_id));
        let mut record = GitOperationRecord {
            pick_index: None,
            op_id,
            operation: plan.operation,
            worktree: repository_dir.to_string(),
            before,
            after,
            backup_ref,
            status: GitOperationStatus::Started,
            started_at,
            finished_at: None,
            error: None,
            undo_of,
            undone_by: None,
        };
        // 일지에 적지 못하면 실행하지 않는다. 적히지 않은 작업은 되돌릴 수 없다.
        self.journal.append(repository_dir, &record)?;
        let result = self.run_steps(repository_dir, &record);
        record.finished_at = Some((self.stamp)().0);
        match result {
            Ok(()) => record.status = GitOperationStatus::Done,
            Err(error) => {
                record.status = GitOperationStatus::Failed;
                record.error = Some(format!("{error:#}"));
            }
        }
        if let Err(error) = self.journal.append(repository_dir, &record) {
            tracing::warn!(error = %format!("{error:#}"), "git operation journal: the end of an operation was not written");
        }
        if record.status == GitOperationStatus::Failed {
            return Ok(GitOperationOutcome::Failed { record });
        }
        if record.backup_ref.is_some() {
            self.prune_backups(repository_dir, started_at);
        }
        Ok(GitOperationOutcome::Done { record })
    }

    fn run_steps(&self, repository_dir: &str, record: &GitOperationRecord) -> Result<()> {
        if let Some(backup_ref) = &record.backup_ref {
            self.workspace
                .create_ref(repository_dir, backup_ref, &record.before.commit)?;
        }
        match &record.operation {
            GitOperation::CherryPick { .. } => bail!("cherry-pick requires sequence execution"),
            GitOperation::Reset { .. } => self
                .workspace
                .reset_to(repository_dir, &record.after.commit),
            GitOperation::MoveBranch { branch, .. } => self.workspace.move_branch(
                repository_dir,
                branch,
                &record.after.commit,
                &record.before.commit,
            ),
            GitOperation::CheckoutBranch { branch } => {
                self.workspace.switch_branch(repository_dir, branch)
            }
            GitOperation::CheckoutCommit { .. } => self
                .workspace
                .switch_detached(repository_dir, &record.after.commit),
        }
    }

    /// 오래되고 수를 넘은 백업 ref 를 지운다. 실패해도 작업의 결과는 바뀌지 않는다.
    fn prune_backups(&self, repository_dir: &str, now: i64) {
        let Ok(Some(refs)) = self.history.read_refs(repository_dir) else {
            return;
        };
        let backups: Vec<String> = refs
            .refs
            .into_iter()
            .filter(|reference| reference.kind == GitGraphRefKind::Backup)
            .map(|reference| reference.name)
            .collect();
        for name in git_backups_to_prune(&backups, now) {
            let full_ref = format!("{GIT_BACKUP_REF_PREFIX}{name}");
            if let Err(error) = self.workspace.delete_ref(repository_dir, &full_ref) {
                tracing::warn!(error = %format!("{error:#}"), "git backup ref was not pruned");
            }
        }
    }
}

fn plan_checkout_branch(
    refs: &GitHistoryRefs,
    condition: &GitWorkspaceCondition,
    head: &GitHeadPosition,
    branch: &str,
    plan: &mut GitOperationPlan,
) {
    plan.before = Some(head.clone());
    if let Some(current) = &head.branch {
        block_agent_branch(plan, current);
    }
    let Some(commit) = local_branch_commit(refs, branch) else {
        block(
            plan,
            GitOperationBlockerCode::UnknownBranch,
            format!("there is no local branch `{branch}`"),
        );
        return;
    };
    // 계획 뒤에 그 브랜치가 옮겨지면 사용자가 본 커밋과 다른 커밋을 체크아웃한다. 기대 상태에 넣어 막는다.
    plan.expectation.branch_commit = Some(commit.to_string());
    plan.after = Some(GitHeadPosition {
        commit: commit.to_string(),
        branch: Some(branch.to_string()),
    });
    if head.branch.as_deref() == Some(branch) {
        block(
            plan,
            GitOperationBlockerCode::NothingToDo,
            format!("`{branch}` is already checked out here"),
        );
        return;
    }
    block_agent_branch(plan, branch);
    block_branch_in_use(refs, condition, branch, plan);
}

/// 브랜치를 어느 작업 트리가 쓰고 있으면 막는다: 체크아웃했거나, 그 브랜치의 rebase 나 bisect 가 끝나지 않았다.
fn block_branch_in_use(
    refs: &GitHistoryRefs,
    condition: &GitWorkspaceCondition,
    branch: &str,
    plan: &mut GitOperationPlan,
) {
    if let Some(worktree) = worktree_holding(refs, branch) {
        block(
            plan,
            GitOperationBlockerCode::BranchCheckedOut,
            format!(
                "the worktree `{}` has `{branch}` checked out",
                worktree.path
            ),
        );
    } else if condition.busy_branches.iter().any(|busy| busy == branch) {
        block(
            plan,
            GitOperationBlockerCode::BranchCheckedOut,
            format!("an unfinished rebase or bisect in a worktree uses `{branch}`"),
        );
    }
}

fn validate_operation(operation: &GitOperation) -> Result<()> {
    let (branch, target) = match operation {
        GitOperation::CherryPick { commits } => {
            if commits.is_empty()
                || commits.len() > 32
                || commits.iter().any(|c| !is_git_commit_id(c))
                || commits
                    .iter()
                    .collect::<std::collections::HashSet<_>>()
                    .len()
                    != commits.len()
            {
                bail!("select 1 to 32 distinct full commit SHAs");
            }
            return Ok(());
        }
        GitOperation::Reset { target } | GitOperation::CheckoutCommit { target } => {
            (None, Some(target))
        }
        GitOperation::MoveBranch { branch, target } => (Some(branch), Some(target)),
        GitOperation::CheckoutBranch { branch } => (Some(branch), None),
    };
    if target.is_some_and(|target| !is_git_commit_id(target)) {
        bail!("the commit id must be a full sha of 40 or 64 hexadecimal characters");
    }
    if branch.is_some_and(|branch| branch.is_empty()) {
        bail!("the branch name is empty");
    }
    Ok(())
}

fn block(plan: &mut GitOperationPlan, code: GitOperationBlockerCode, message: impl Into<String>) {
    // 같은 이유를 두 번 적지 않는다 (지금 브랜치와 대상 브랜치가 둘 다 병렬 슬롯일 때).
    if !plan.blockers.iter().any(|blocker| blocker.code == code) {
        plan.blockers.push(GitOperationBlocker::new(code, message));
    }
}

fn block_agent_branch(plan: &mut GitOperationPlan, branch: &str) {
    if branch.starts_with(GIT_AGENT_BRANCH_PREFIX) {
        block(
            plan,
            GitOperationBlockerCode::AgentWorktree,
            format!("`{branch}` belongs to an Akra parallel slot; the parallel runtime owns it"),
        );
    }
}

fn head_position(refs: &GitHistoryRefs) -> Option<GitHeadPosition> {
    let commit = refs.head.clone()?;
    let branch = refs
        .refs
        .iter()
        .find(|reference| reference.kind == GitGraphRefKind::LocalBranch && reference.head)
        .map(|reference| reference.name.clone());
    Some(GitHeadPosition { commit, branch })
}

fn local_branch_commit<'refs>(refs: &'refs GitHistoryRefs, branch: &str) -> Option<&'refs str> {
    refs.refs
        .iter()
        .find(|reference| {
            reference.kind == GitGraphRefKind::LocalBranch && reference.name == branch
        })
        .map(|reference| reference.target.as_str())
}

fn worktree_holding<'refs>(
    refs: &'refs GitHistoryRefs,
    branch: &str,
) -> Option<&'refs GitGraphWorktree> {
    refs.worktrees
        .iter()
        .find(|worktree| worktree.branch.as_deref() == Some(branch))
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashSet};
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;
    use crate::domain::git_graph::{GitCommitDetail, GitGraphCommit, GitGraphRef};
    use crate::domain::git_operation::{GitOperationImpact, fold_git_operation_records};

    fn sha(letter: char) -> String {
        letter.to_string().repeat(40)
    }

    /// 브랜치, HEAD, 백업 ref 만 가진 가짜 저장소이다. 고치는 명령은 `calls` 에 남는다.
    #[derive(Default)]
    struct FakeState {
        branches: BTreeMap<String, String>,
        backups: BTreeMap<String, String>,
        head_commit: String,
        head_branch: Option<String>,
        other_worktree_branch: Option<String>,
        tracked_changes: bool,
        operation_in_progress: Option<String>,
        busy_branches: Vec<String>,
        refusal: Option<String>,
        fail_reset: bool,
        calls: Vec<String>,
    }

    struct FakeRepository {
        state: Mutex<FakeState>,
    }

    impl FakeRepository {
        /// `main` 은 `c` 에 있고 체크아웃되어 있다. `topic` 은 `b` 에 있다.
        fn new() -> Arc<Self> {
            Arc::new(Self {
                state: Mutex::new(FakeState {
                    branches: BTreeMap::from([
                        ("main".to_string(), sha('c')),
                        ("topic".to_string(), sha('b')),
                    ]),
                    head_commit: sha('c'),
                    head_branch: Some("main".to_string()),
                    ..FakeState::default()
                }),
            })
        }

        fn edit(&self, change: impl FnOnce(&mut FakeState)) {
            change(&mut self.state.lock().unwrap());
        }

        fn calls(&self) -> Vec<String> {
            self.state.lock().unwrap().calls.clone()
        }

        fn refs_snapshot(&self) -> (BTreeMap<String, String>, String, Option<String>) {
            let state = self.state.lock().unwrap();
            (
                state.branches.clone(),
                state.head_commit.clone(),
                state.head_branch.clone(),
            )
        }
    }

    impl GitHistoryPort for FakeRepository {
        fn read_worktree_progress(
            &self,
            _repository_dir: &str,
        ) -> Result<crate::application::port::outbound::git_history_port::GitWorktreeProgress>
        {
            Ok(Default::default())
        }

        fn read_refs(&self, repository_dir: &str) -> Result<Option<GitHistoryRefs>> {
            let state = self.state.lock().unwrap();
            let mut refs: Vec<GitGraphRef> = state
                .branches
                .iter()
                .map(|(name, target)| GitGraphRef {
                    name: name.clone(),
                    kind: GitGraphRefKind::LocalBranch,
                    target: target.clone(),
                    head: state.head_branch.as_deref() == Some(name),
                })
                .collect();
            refs.extend(state.backups.iter().map(|(name, target)| {
                GitGraphRef {
                    name: name
                        .strip_prefix(GIT_BACKUP_REF_PREFIX)
                        .unwrap_or(name)
                        .to_string(),
                    kind: GitGraphRefKind::Backup,
                    target: target.clone(),
                    head: false,
                }
            }));
            // `/second` 은 다른 저장소이다. 나머지 폴더는 모두 `/repo` 저장소이다.
            let main = if repository_dir.starts_with("/second") {
                "/second"
            } else {
                "/repo"
            };
            let mut worktrees = vec![GitGraphWorktree {
                path: main.to_string(),
                head: Some(state.head_commit.clone()),
                branch: state.head_branch.clone(),
            }];
            if let Some(branch) = &state.other_worktree_branch {
                worktrees.push(GitGraphWorktree {
                    path: "/repo-other".to_string(),
                    head: state.branches.get(branch).cloned(),
                    branch: Some(branch.clone()),
                });
            }
            Ok(Some(GitHistoryRefs {
                head: Some(state.head_commit.clone()),
                refs,
                worktrees,
            }))
        }

        fn read_commits(
            &self,
            _repository_dir: &str,
            _include_head: bool,
            _skip: usize,
            _limit: usize,
        ) -> Result<Vec<GitGraphCommit>> {
            Ok(Vec::new())
        }

        fn read_backup_only_commits(
            &self,
            _repository_dir: &str,
            _include_head: bool,
        ) -> Result<HashSet<String>> {
            Ok(HashSet::new())
        }

        fn read_commit_detail(
            &self,
            _repository_dir: &str,
            _commit: &str,
        ) -> Result<Option<GitCommitDetail>> {
            Ok(None)
        }
    }

    impl GitWorkspacePort for FakeRepository {
        fn read_condition(&self, _repository_dir: &str) -> Result<GitWorkspaceCondition> {
            let state = self.state.lock().unwrap();
            Ok(GitWorkspaceCondition {
                refusal: state.refusal.clone(),
                tracked_changes: state.tracked_changes,
                operation_in_progress: state.operation_in_progress.clone(),
                busy_branches: state.busy_branches.clone(),
            })
        }

        fn resolve_commit(&self, _repository_dir: &str, commit: &str) -> Result<Option<String>> {
            // `f` 로 된 sha 는 없는 커밋이다.
            Ok((commit != sha('f')).then(|| commit.to_string()))
        }

        fn read_move_impact(
            &self,
            _repository_dir: &str,
            _branch: &str,
            _from: &str,
            _to: &str,
        ) -> Result<GitOperationImpact> {
            Ok(GitOperationImpact {
                dropped: 2,
                unreachable: 1,
                gained: 0,
            })
        }

        fn create_ref(&self, _repository_dir: &str, full_ref: &str, commit: &str) -> Result<()> {
            let mut state = self.state.lock().unwrap();
            state.calls.push(format!("create_ref {full_ref}"));
            if state.backups.contains_key(full_ref) {
                bail!("the ref exists");
            }
            state
                .backups
                .insert(full_ref.to_string(), commit.to_string());
            Ok(())
        }

        fn delete_ref(&self, _repository_dir: &str, full_ref: &str) -> Result<()> {
            let mut state = self.state.lock().unwrap();
            state.calls.push(format!("delete_ref {full_ref}"));
            state.backups.remove(full_ref);
            Ok(())
        }

        fn reset_to(&self, _repository_dir: &str, commit: &str) -> Result<()> {
            let mut state = self.state.lock().unwrap();
            state.calls.push(format!("reset_to {}", &commit[..1]));
            if state.fail_reset {
                bail!("an untracked file would be overwritten");
            }
            let branch = state.head_branch.clone().expect("reset needs a branch");
            state.branches.insert(branch, commit.to_string());
            state.head_commit = commit.to_string();
            Ok(())
        }

        fn move_branch(
            &self,
            _repository_dir: &str,
            branch: &str,
            to: &str,
            from: &str,
        ) -> Result<()> {
            let mut state = self.state.lock().unwrap();
            state
                .calls
                .push(format!("move_branch {branch} {}", &to[..1]));
            if state.branches.get(branch).map(String::as_str) != Some(from) {
                bail!("the branch is not at the expected commit");
            }
            state.branches.insert(branch.to_string(), to.to_string());
            Ok(())
        }

        fn switch_branch(&self, _repository_dir: &str, branch: &str) -> Result<()> {
            let mut state = self.state.lock().unwrap();
            state.calls.push(format!("switch_branch {branch}"));
            state.head_commit = state.branches[branch].clone();
            state.head_branch = Some(branch.to_string());
            Ok(())
        }

        fn switch_detached(&self, _repository_dir: &str, commit: &str) -> Result<()> {
            let mut state = self.state.lock().unwrap();
            state
                .calls
                .push(format!("switch_detached {}", &commit[..1]));
            state.head_commit = commit.to_string();
            state.head_branch = None;
            Ok(())
        }
    }

    #[derive(Default)]
    struct MemoryJournal {
        lines: Mutex<Vec<GitOperationRecord>>,
        refuse_append: Mutex<bool>,
    }

    impl GitOperationJournalPort for MemoryJournal {
        fn append(&self, _repository_dir: &str, record: &GitOperationRecord) -> Result<()> {
            if *self.refuse_append.lock().unwrap() {
                bail!("the journal is not writable");
            }
            self.lines.lock().unwrap().push(record.clone());
            Ok(())
        }

        fn load(&self, _repository_dir: &str) -> Result<Vec<GitOperationRecord>> {
            Ok(fold_git_operation_records(
                self.lines.lock().unwrap().clone(),
            ))
        }
    }

    struct Fixture {
        repository: Arc<FakeRepository>,
        journal: Arc<MemoryJournal>,
        turns: Arc<TurnActivityRegistry>,
        service: GitOperationService,
    }

    const NOW: i64 = 1_795_000_000;

    fn fixture() -> Fixture {
        let repository = FakeRepository::new();
        let journal = Arc::new(MemoryJournal::default());
        let nonce = AtomicU32::new(0);
        let turns = Arc::new(TurnActivityRegistry::default());
        let service = GitOperationService::new(
            repository.clone(),
            repository.clone(),
            journal.clone(),
            Arc::new(move || (NOW, nonce.fetch_add(1, Ordering::SeqCst))),
            turns.clone(),
        );
        Fixture {
            repository,
            journal,
            turns,
            service,
        }
    }

    fn reset_to(letter: char) -> GitOperation {
        GitOperation::Reset {
            target: sha(letter),
        }
    }

    fn codes(plan: &GitOperationPlan) -> Vec<GitOperationBlockerCode> {
        plan.blockers.iter().map(|blocker| blocker.code).collect()
    }

    fn done(outcome: GitOperationOutcome) -> GitOperationRecord {
        match outcome {
            GitOperationOutcome::Done { record } => record,
            other => panic!("the operation did not finish: {other:?}"),
        }
    }

    #[test]
    fn a_running_turn_in_this_worktree_blocks_the_plan_and_the_execution() {
        let fixture = fixture();
        fixture
            .repository
            .edit(|state| state.other_worktree_branch = Some("topic".to_string()));
        let observer = fixture.turns.observer();
        observer.thread_folder("here", "/repo/src");
        observer.turn_running("here", true);

        let plan = fixture.service.plan("/repo", reset_to('a')).unwrap();
        assert_eq!(codes(&plan), [GitOperationBlockerCode::TurnRunning]);
        let outcome = fixture
            .service
            .execute("/repo", reset_to('a'), plan.expectation.clone())
            .unwrap();
        assert!(matches!(outcome, GitOperationOutcome::Stale { .. }));
        assert!(fixture.repository.calls().is_empty());

        // A turn in another worktree of the same repository leaves this worktree alone.
        observer.turn_running("here", false);
        let other = fixture.turns.observer();
        other.thread_folder("there", "/repo-other");
        other.turn_running("there", true);
        let plan = fixture.service.plan("/repo", reset_to('a')).unwrap();
        assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
        done(
            fixture
                .service
                .execute("/repo", reset_to('a'), plan.expectation)
                .unwrap(),
        );
    }

    #[test]
    fn each_repository_has_its_own_queue() {
        let fixture = fixture();
        let service = Arc::new(fixture.service);
        assert!(Arc::ptr_eq(
            &service.repository_queue("/repo"),
            &service.repository_queue("/repo-other"),
        ));
        let busy = service.repository_queue("/repo");
        let _held = busy.lock().unwrap();
        let (sent, received) = std::sync::mpsc::channel();
        let other = service.clone();
        std::thread::spawn(move || {
            let plan = other.plan("/second", reset_to('a')).unwrap();
            sent.send(
                other
                    .execute("/second", reset_to('a'), plan.expectation)
                    .map(|_| ()),
            )
            .unwrap();
        });
        received
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("another repository must not wait for this queue")
            .unwrap();
    }

    #[test]
    fn a_reset_plan_shows_the_move_and_its_impact_and_changes_nothing() {
        let fixture = fixture();

        let plan = fixture.service.plan("/repo", reset_to('a')).unwrap();

        assert!(plan.blockers.is_empty());
        assert_eq!(plan.branch.as_deref(), Some("main"));
        assert_eq!(plan.before.as_ref().unwrap().commit, sha('c'));
        assert_eq!(plan.after.as_ref().unwrap().commit, sha('a'));
        assert_eq!(plan.impact.unwrap().dropped, 2);
        assert_eq!(
            plan.expectation.head,
            Some(GitHeadPosition {
                commit: sha('c'),
                branch: Some("main".to_string())
            })
        );
        assert!(fixture.repository.calls().is_empty());
        assert!(fixture.journal.lines.lock().unwrap().is_empty());
    }

    #[test]
    fn a_reset_plan_is_blocked_by_the_state_of_the_worktree() {
        let fixture = fixture();
        fixture.repository.edit(|state| {
            state.tracked_changes = true;
            state.operation_in_progress = Some("rebase".to_string());
        });
        let plan = fixture.service.plan("/repo", reset_to('a')).unwrap();
        assert_eq!(
            codes(&plan),
            [
                GitOperationBlockerCode::OperationInProgress,
                GitOperationBlockerCode::UncommittedChanges
            ]
        );

        let same = fixture.service.plan("/repo", reset_to('c')).unwrap();
        assert!(codes(&same).contains(&GitOperationBlockerCode::NothingToDo));
        let unknown = fixture.service.plan("/repo", reset_to('f')).unwrap();
        assert!(codes(&unknown).contains(&GitOperationBlockerCode::UnknownCommit));

        fixture.repository.edit(|state| {
            state.tracked_changes = false;
            state.operation_in_progress = None;
            state.head_branch = None;
        });
        let detached = fixture.service.plan("/repo", reset_to('a')).unwrap();
        assert_eq!(codes(&detached), [GitOperationBlockerCode::DetachedHead]);

        fixture.repository.edit(|state| {
            state
                .branches
                .insert("akra-agent/slot-1/task".to_string(), sha('c'));
            state.head_branch = Some("akra-agent/slot-1/task".to_string());
        });
        let slot = fixture.service.plan("/repo", reset_to('a')).unwrap();
        assert_eq!(codes(&slot), [GitOperationBlockerCode::AgentWorktree]);
    }

    #[test]
    fn a_repository_that_the_workspace_refuses_is_not_planned_or_changed() {
        let fixture = fixture();
        fixture
            .repository
            .edit(|state| state.refusal = Some("filter.lfs.smudge is set".to_string()));

        let plan = fixture.service.plan("/repo", reset_to('a')).unwrap();

        assert_eq!(codes(&plan), [GitOperationBlockerCode::ExecutionConfig]);
        assert_eq!(plan.blockers[0].message, "filter.lfs.smudge is set");
        let outcome = fixture
            .service
            .execute("/repo", reset_to('a'), plan.expectation.clone())
            .unwrap();
        assert!(matches!(outcome, GitOperationOutcome::Stale { .. }));
        assert!(fixture.repository.calls().is_empty());
    }

    #[test]
    fn a_text_that_is_not_a_full_sha_is_an_error() {
        let fixture = fixture();
        let short = GitOperation::Reset {
            target: "abc1234".to_string(),
        };
        assert!(fixture.service.plan("/repo", short).is_err());
        let option = GitOperation::CheckoutCommit {
            target: "--force".to_string(),
        };
        assert!(fixture.service.plan("/repo", option).is_err());
    }

    #[test]
    fn execute_refuses_a_plan_made_before_the_repository_changed() {
        let fixture = fixture();
        let plan = fixture.service.plan("/repo", reset_to('a')).unwrap();
        // 계획 뒤에 누가 커밋을 더했다.
        fixture.repository.edit(|state| {
            state.branches.insert("main".to_string(), sha('d'));
            state.head_commit = sha('d');
        });

        let outcome = fixture
            .service
            .execute("/repo", reset_to('a'), plan.expectation)
            .unwrap();

        let GitOperationOutcome::Stale { plan: fresh } = outcome else {
            panic!("a stale plan must not run");
        };
        assert_eq!(fresh.before.unwrap().commit, sha('d'));
        assert!(fixture.repository.calls().is_empty());
        assert!(fixture.journal.lines.lock().unwrap().is_empty());
    }

    #[test]
    fn a_reset_writes_the_backup_first_and_the_journal_around_the_command() {
        let fixture = fixture();
        let plan = fixture.service.plan("/repo", reset_to('a')).unwrap();

        let record = done(
            fixture
                .service
                .execute("/repo", reset_to('a'), plan.expectation)
                .unwrap(),
        );

        let backup = "refs/akra/backup/main/20261118T110640Z-00000000";
        assert_eq!(record.backup_ref.as_deref(), Some(backup));
        assert_eq!(
            fixture.repository.calls(),
            [format!("create_ref {backup}"), "reset_to a".to_string()]
        );
        let state = fixture.repository.state.lock().unwrap();
        assert_eq!(state.backups[backup], sha('c'));
        assert_eq!(state.branches["main"], sha('a'));
        drop(state);
        let statuses: Vec<GitOperationStatus> = fixture
            .journal
            .lines
            .lock()
            .unwrap()
            .iter()
            .map(|line| line.status)
            .collect();
        assert_eq!(
            statuses,
            [GitOperationStatus::Started, GitOperationStatus::Done]
        );
        assert_eq!(record.finished_at, Some(NOW));
    }

    #[test]
    fn nothing_runs_when_the_journal_cannot_record_the_start() {
        let fixture = fixture();
        let plan = fixture.service.plan("/repo", reset_to('a')).unwrap();
        *fixture.journal.refuse_append.lock().unwrap() = true;

        assert!(
            fixture
                .service
                .execute("/repo", reset_to('a'), plan.expectation)
                .is_err()
        );
        assert!(fixture.repository.calls().is_empty());
    }

    #[test]
    fn a_failed_command_is_recorded_and_cannot_be_undone() {
        let fixture = fixture();
        fixture.repository.edit(|state| state.fail_reset = true);
        let plan = fixture.service.plan("/repo", reset_to('a')).unwrap();

        let outcome = fixture
            .service
            .execute("/repo", reset_to('a'), plan.expectation)
            .unwrap();

        let GitOperationOutcome::Failed { record } = outcome else {
            panic!("the reset must fail");
        };
        assert_eq!(
            record.error.as_deref(),
            Some("an untracked file would be overwritten")
        );
        assert_eq!(fixture.repository.refs_snapshot().0["main"], sha('c'));
        let undo = fixture.service.undo("/repo", &record.op_id).unwrap();
        assert!(matches!(undo, GitOperationOutcome::Refused { .. }));
    }

    #[test]
    fn every_operation_and_its_undo_leave_the_refs_as_they_were() {
        let operations = [
            reset_to('a'),
            GitOperation::MoveBranch {
                branch: "topic".to_string(),
                target: sha('a'),
            },
            GitOperation::CheckoutBranch {
                branch: "topic".to_string(),
            },
            GitOperation::CheckoutCommit { target: sha('a') },
        ];
        for operation in operations {
            let fixture = fixture();
            let before = fixture.repository.refs_snapshot();
            let plan = fixture.service.plan("/repo", operation.clone()).unwrap();
            assert!(
                plan.blockers.is_empty(),
                "{operation:?}: {:?}",
                plan.blockers
            );
            let record = done(
                fixture
                    .service
                    .execute("/repo", operation.clone(), plan.expectation)
                    .unwrap(),
            );
            assert_ne!(fixture.repository.refs_snapshot(), before, "{operation:?}");

            let undo = done(fixture.service.undo("/repo", &record.op_id).unwrap());

            assert_eq!(fixture.repository.refs_snapshot(), before, "{operation:?}");
            assert_eq!(undo.undo_of.as_deref(), Some(record.op_id.as_str()));
            let listed = fixture.service.operations("/repo").unwrap();
            // 새 것이 앞이다. 되돌린 작업에는 되돌린 작업의 id 가 남는다.
            assert_eq!(listed[0].op_id, undo.op_id);
            assert_eq!(listed[1].undone_by.as_deref(), Some(undo.op_id.as_str()));
            // 한 번 되돌린 작업과 되돌리기 자신은 다시 되돌리지 않는다.
            for op_id in [&record.op_id, &undo.op_id] {
                let again = fixture.service.undo("/repo", op_id).unwrap();
                assert!(matches!(again, GitOperationOutcome::Refused { .. }));
            }
        }
    }

    #[test]
    fn undo_is_refused_after_the_repository_moved_on() {
        let fixture = fixture();
        let plan = fixture.service.plan("/repo", reset_to('a')).unwrap();
        let record = done(
            fixture
                .service
                .execute("/repo", reset_to('a'), plan.expectation)
                .unwrap(),
        );
        // reset 뒤에 새 커밋을 만들었다. 되돌리면 그 커밋이 브랜치에서 빠진다.
        fixture.repository.edit(|state| {
            state.branches.insert("main".to_string(), sha('e'));
            state.head_commit = sha('e');
        });
        let calls_before = fixture.repository.calls().len();

        let outcome = fixture.service.undo("/repo", &record.op_id).unwrap();

        assert!(matches!(outcome, GitOperationOutcome::Refused { .. }));
        assert_eq!(fixture.repository.calls().len(), calls_before);
        assert!(matches!(
            fixture.service.undo("/repo", "missing").unwrap(),
            GitOperationOutcome::Refused { .. }
        ));
    }

    #[test]
    fn a_branch_that_a_worktree_has_checked_out_is_not_moved() {
        let fixture = fixture();
        fixture
            .repository
            .edit(|state| state.other_worktree_branch = Some("topic".to_string()));
        let move_topic = GitOperation::MoveBranch {
            branch: "topic".to_string(),
            target: sha('a'),
        };
        let plan = fixture.service.plan("/repo", move_topic).unwrap();
        assert_eq!(codes(&plan), [GitOperationBlockerCode::BranchCheckedOut]);
        assert_eq!(plan.expectation.branch_commit, Some(sha('b')));

        // 현재 브랜치도 작업 트리가 체크아웃한 브랜치이다. reset 을 써야 한다.
        let move_main = GitOperation::MoveBranch {
            branch: "main".to_string(),
            target: sha('a'),
        };
        let current = fixture.service.plan("/repo", move_main).unwrap();
        assert_eq!(codes(&current), [GitOperationBlockerCode::BranchCheckedOut]);

        let checkout = GitOperation::CheckoutBranch {
            branch: "topic".to_string(),
        };
        let held = fixture.service.plan("/repo", checkout).unwrap();
        assert_eq!(codes(&held), [GitOperationBlockerCode::BranchCheckedOut]);

        let missing = GitOperation::MoveBranch {
            branch: "nowhere".to_string(),
            target: sha('a'),
        };
        let unknown = fixture.service.plan("/repo", missing).unwrap();
        assert_eq!(codes(&unknown), [GitOperationBlockerCode::UnknownBranch]);
    }

    #[test]
    fn a_branch_that_an_unfinished_rebase_uses_is_not_moved_or_checked_out() {
        let fixture = fixture();
        // 다른 작업 트리가 `topic` 을 rebase 하는 중이다. 그 작업 트리의 HEAD 는 브랜치 없이 있다.
        fixture
            .repository
            .edit(|state| state.busy_branches = vec!["topic".to_string()]);
        for operation in [
            GitOperation::MoveBranch {
                branch: "topic".to_string(),
                target: sha('a'),
            },
            GitOperation::CheckoutBranch {
                branch: "topic".to_string(),
            },
        ] {
            let plan = fixture.service.plan("/repo", operation).unwrap();
            assert_eq!(codes(&plan), [GitOperationBlockerCode::BranchCheckedOut]);
            assert!(plan.blockers[0].message.contains("rebase"));
        }
    }

    #[test]
    fn a_checkout_is_stale_when_its_branch_moved_after_the_plan() {
        let fixture = fixture();
        let checkout = GitOperation::CheckoutBranch {
            branch: "topic".to_string(),
        };
        let plan = fixture.service.plan("/repo", checkout.clone()).unwrap();
        assert_eq!(plan.expectation.branch_commit, Some(sha('b')));
        fixture.repository.edit(|state| {
            state.branches.insert("topic".to_string(), sha('d'));
        });

        let outcome = fixture
            .service
            .execute("/repo", checkout, plan.expectation)
            .unwrap();

        let GitOperationOutcome::Stale { plan: fresh } = outcome else {
            panic!("the checkout must not run on another commit");
        };
        assert_eq!(fresh.after.unwrap().commit, sha('d'));
        assert!(fixture.repository.calls().is_empty());
    }

    #[test]
    fn a_reset_or_a_checkout_is_undone_only_from_its_own_worktree() {
        let fixture = fixture();
        let plan = fixture.service.plan("/repo", reset_to('a')).unwrap();
        let reset = done(
            fixture
                .service
                .execute("/repo", reset_to('a'), plan.expectation)
                .unwrap(),
        );
        let calls_before = fixture.repository.calls().len();

        // 일지는 저장소의 모든 작업 트리가 함께 쓴다. 다른 작업 트리의 요청은 이 기록을 본다.
        let elsewhere = fixture.service.undo("/repo-other", &reset.op_id).unwrap();
        assert!(matches!(elsewhere, GitOperationOutcome::Refused { .. }));
        assert_eq!(fixture.repository.calls().len(), calls_before);
        // 같은 폴더의 다른 표기는 같은 작업 트리이다.
        done(fixture.service.undo("/repo/", &reset.op_id).unwrap());

        // 브랜치 이동은 ref 만 바꿨다. 어느 작업 트리에서나 되돌린다.
        let move_topic = GitOperation::MoveBranch {
            branch: "topic".to_string(),
            target: sha('a'),
        };
        let plan = fixture.service.plan("/repo", move_topic.clone()).unwrap();
        let moved = done(
            fixture
                .service
                .execute("/repo", move_topic, plan.expectation)
                .unwrap(),
        );
        done(fixture.service.undo("/repo-other", &moved.op_id).unwrap());
    }

    #[test]
    fn old_backups_beyond_the_kept_count_are_pruned_after_a_new_backup() {
        let fixture = fixture();
        let day = 24 * 60 * 60;
        fixture.repository.edit(|state| {
            for index in 0..21 {
                let name =
                    git_backup_ref_name("main", &git_operation_id(NOW - 60 * day - index, 0));
                state.backups.insert(name, sha('b'));
            }
        });
        let plan = fixture.service.plan("/repo", reset_to('a')).unwrap();

        done(
            fixture
                .service
                .execute("/repo", reset_to('a'), plan.expectation)
                .unwrap(),
        );

        // 새 백업 1 개와 오래된 백업 21 개가 있었다. 최근 20 개에 들지 못한 오래된 2 개를 지운다.
        let deleted = fixture
            .repository
            .calls()
            .iter()
            .filter(|call| call.starts_with("delete_ref "))
            .count();
        assert_eq!(deleted, 2);
        assert_eq!(fixture.repository.state.lock().unwrap().backups.len(), 20);
    }
}
