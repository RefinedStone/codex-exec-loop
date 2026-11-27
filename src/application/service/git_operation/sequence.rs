use super::*;
use anyhow::Context;

impl GitOperationService {
    pub(super) fn start_sequence(
        &self,
        cwd: &str,
        plan: GitOperationPlan,
    ) -> Result<GitOperationOutcome> {
        let before = plan.before.context("cherry-pick needs HEAD")?;
        let branch = before
            .branch
            .as_deref()
            .context("cherry-pick needs a branch")?;
        let commits = plan
            .picks
            .into_iter()
            .filter(|p| p.prediction != PickPrediction::AlreadyApplied)
            .map(|p| p.commit)
            .collect();
        let (started_at, nonce) = (self.stamp)();
        let op_id = git_operation_id(started_at, nonce);
        let backup_ref = git_backup_ref_name(branch, &op_id);
        let record = GitOperationRecord {
            pick_index: Some(0),
            op_id,
            operation: GitOperation::CherryPick { commits },
            worktree: cwd.to_owned(),
            before: before.clone(),
            after: before,
            backup_ref: Some(backup_ref.clone()),
            status: GitOperationStatus::Started,
            started_at,
            finished_at: None,
            error: None,
            undo_of: None,
            undone_by: None,
        };
        self.journal.append(cwd, &record)?;
        self.workspace
            .create_ref(cwd, &backup_ref, &record.before.commit)?;
        self.advance(cwd, record)
    }

    fn position(&self, cwd: &str) -> Result<GitHeadPosition> {
        self.history
            .read_refs(cwd)?
            .as_ref()
            .and_then(head_position)
            .context("worktree HEAD is missing")
    }

    fn advance(&self, cwd: &str, mut r: GitOperationRecord) -> Result<GitOperationOutcome> {
        loop {
            let index = r.pick_index.context("missing pick checkpoint")?;
            let GitOperation::CherryPick { commits } = &r.operation else {
                bail!("not a pick record");
            };
            let Some(commit) = commits.get(index) else {
                r.status = GitOperationStatus::Done;
                r.finished_at = Some((self.stamp)().0);
                r.error = None;
                self.journal.append(cwd, &r)?;
                return Ok(GitOperationOutcome::Done { record: r });
            };
            let observed = self.position(cwd)?;
            if observed != r.after {
                return Ok(refused(
                    "HEAD differs from the last verified cherry-pick checkpoint",
                ));
            }
            let condition = self.workspace.read_condition(cwd)?;
            if condition.refusal.is_some()
                || condition.tracked_changes
                || condition.operation_in_progress.is_some()
            {
                return Ok(refused("worktree changed since the last checkpoint"));
            }
            let receipt = format!("akra-pick-{}-{index}", r.op_id);
            match self.workspace.pick_one(cwd, commit, &receipt) {
                Ok(()) => {
                    let next = self.position(cwd)?;
                    if next.branch != r.before.branch
                        || !self
                            .workspace
                            .pick_completed(cwd, &r.after.commit, &receipt)?
                    {
                        return Ok(refused("cannot verify the new cherry-pick tip"));
                    }
                    r.after = next;
                    r.pick_index = Some(index + 1);
                    self.journal.append(cwd, &r)?;
                }
                Err(e) => {
                    r.status = GitOperationStatus::Paused;
                    r.error = Some(format!("{e:#}"));
                    self.journal.append(cwd, &r)?;
                    return Ok(GitOperationOutcome::Paused { record: r });
                }
            }
        }
    }

    pub(super) fn resume_sequence(
        &self,
        cwd: &str,
        id: &str,
        action: PickAction,
    ) -> Result<GitOperationOutcome> {
        let Some(mut r) = self
            .journal
            .load(cwd)?
            .into_iter()
            .find(|r| r.op_id == id && same_git_worktree_path(&r.worktree, cwd))
        else {
            return Ok(refused("no sequence in this worktree"));
        };
        if !matches!(
            r.status,
            GitOperationStatus::Started | GitOperationStatus::Paused
        ) {
            return Ok(refused("sequence has finished"));
        }
        let GitOperation::CherryPick { commits } = &r.operation else {
            return Ok(refused("not a cherry-pick sequence"));
        };
        let index = r.pick_index.context("missing pick checkpoint")?;
        let refs = self.history.read_refs(cwd)?.context("not a repository")?;
        let backup = r
            .backup_ref
            .as_deref()
            .and_then(|b| b.strip_prefix(GIT_BACKUP_REF_PREFIX));
        if !refs.refs.iter().any(|b| {
            b.kind == GitGraphRefKind::Backup
                && Some(b.name.as_str()) == backup
                && b.target == r.before.commit
        }) {
            return Ok(refused("the journal backup is missing or changed"));
        }
        let head = head_position(&refs).context("missing HEAD")?;
        if head.branch != r.before.branch {
            return Ok(refused("the checked-out branch changed"));
        }
        let condition = self.workspace.read_condition(cwd)?;
        if condition.refusal.is_some() {
            return Ok(refused("repository configuration prevents recovery"));
        }
        let Some(target) = git_worktree_containing(&refs.worktrees, cwd) else {
            return Ok(refused("cannot identify the worktree"));
        };
        if self.turns.running_folders().iter().any(|dir| {
            git_worktree_containing(&refs.worktrees, dir)
                .is_some_and(|w| same_git_worktree_path(&w.path, &target.path))
        }) {
            return Ok(refused("a turn is running in this worktree"));
        }
        let receipt = format!("akra-pick-{}-{index}", r.op_id);
        let marker = self.workspace.pick_marker(cwd)?;
        if let Some(mark) = marker {
            if commits.get(index) != Some(&mark)
                || head != r.after
                || condition.operation_in_progress.as_deref() != Some("cherry-pick")
            {
                return Ok(refused("another operation owns the worktree"));
            }
            self.workspace.finish_pick(cwd, action, &receipt)?;
            if matches!(action, PickAction::Continue)
                && !self
                    .workspace
                    .pick_completed(cwd, &r.after.commit, &receipt)?
            {
                return Ok(refused("cannot verify the resolved commit"));
            }
            r.after = self.position(cwd)?;
            r.pick_index = Some(index + 1);
        } else {
            if condition.operation_in_progress.is_some() || condition.tracked_changes {
                return Ok(refused("unrelated changes or operation in progress"));
            }
            if head != r.after {
                if !self
                    .workspace
                    .pick_completed(cwd, &r.after.commit, &receipt)?
                {
                    return Ok(refused("unrelated commit appeared after the checkpoint"));
                }
                r.after = head;
                r.pick_index = Some(index + 1);
            } else if matches!(action, PickAction::Skip) {
                r.pick_index = Some(index + 1);
            }
        }
        r.error = None;
        if matches!(action, PickAction::Abort) {
            if self.position(cwd)? != r.after {
                return Ok(refused("HEAD changed during abort"));
            }
            self.workspace.reset_to(cwd, &r.before.commit)?;
            r.after = r.before.clone();
            r.status = GitOperationStatus::Aborted;
            r.finished_at = Some((self.stamp)().0);
            self.journal.append(cwd, &r)?;
            if let Some(backup) = &r.backup_ref {
                self.workspace
                    .release_backup(cwd, backup, &r.before.commit)?;
            }
            return Ok(GitOperationOutcome::Done { record: r });
        }
        r.status = GitOperationStatus::Started;
        self.journal.append(cwd, &r)?;
        self.advance(cwd, r)
    }
}
