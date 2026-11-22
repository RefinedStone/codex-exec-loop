use super::*;

pub(crate) struct FlowWorkerResult {
    pub task_id: String,
    pub lease: ParallelModeSlotLeaseSnapshot,
    pub source_sha: String,
}
impl FlowFixture {
    /// Only the worker's edit/commit and completion message are synthetic. Lease
    /// admission, running state, official completion and queue writes are real.
    pub fn finish_worker(&self, services: &FlowServices) -> Result<FlowWorkerResult> {
        let task_ids = services.task_ids()?;
        ensure!(
            task_ids.len() == 1,
            "worker requires exactly one actually registered task"
        );
        let task_id = task_ids[0].clone();
        let lease = services
            .parallel
            .acquire_slot_lease(
                &services.workspace,
                sample_lease_request(&task_id, "수집한 이슈의 결과", "flow-worker", "flow-result"),
            )
            .map_err(|error| anyhow!(error))?;
        services
            .parallel
            .record_workspace_slot_thread_prepared(&lease.worktree_path, "flow-worker-thread")
            .map_err(|error| anyhow!(error))?;
        services
            .parallel
            .mark_workspace_slot_running(&lease.worktree_path)
            .map_err(|error| anyhow!(error))?;
        self.repo.commit_file_in_slot(
            Path::new(&lease.worktree_path),
            "flow-result.txt",
            "approved result\n",
            "synthetic worker result for the collected issue",
        );
        services
            .parallel
            .begin_workspace_official_completion(
                &lease.worktree_path,
                "flow-worker-turn",
                None,
                Some("The synthetic worker wrote the requested result."),
                Some("Fixture verification: the expected result file was written."),
                None,
            )
            .map_err(|error| anyhow!(error))?;
        services
            .parallel
            .mark_workspace_official_completion_refreshing(&lease.worktree_path)
            .map_err(|error| anyhow!(error))?;
        services
            .parallel
            .mark_workspace_commit_ready(
                &lease.worktree_path,
                "synthetic worker completion passed the fixture's official refresh",
            )
            .map_err(|error| anyhow!(error))?;
        ensure!(
            services
                .parallel
                .enqueue_workspace_commit_ready_result(&lease.worktree_path)
                .map_err(|error| anyhow!(error))?
                .is_some(),
            "actual queue item was not created"
        );
        let source_sha = run_command(
            "git",
            ["-C", lease.worktree_path.as_str(), "rev-parse", "HEAD"],
            None,
        )
        .ok_or_else(|| anyhow!("the frozen worker commit is unavailable"))?;
        Ok(FlowWorkerResult {
            task_id,
            lease,
            source_sha,
        })
    }

    pub fn approve_pull_request(&self, source_sha: &str) {
        *self.github.inspect_review_decision.lock().unwrap() = Some("APPROVED".into());
        *self.github.inspect_head_commit_sha.lock().unwrap() = Some(source_sha.into());
        *self
            .github
            .inspect_approved_review_commit_shas
            .lock()
            .unwrap() = Some(vec![source_sha.into()]);
    }

    pub fn remote_integration_head(&self) -> String {
        run_command(
            "git",
            [
                "-C",
                self.workspace().as_str(),
                "ls-remote",
                DEFAULT_PUSH_REMOTE_NAME,
                &format!("refs/heads/{POOL_BASELINE_BRANCH}"),
            ],
            None,
        )
        .expect("the local bare integration ref should be readable")
        .split_whitespace()
        .next()
        .expect("integration ref should exist")
        .to_owned()
    }

    pub fn operations(&self) -> Vec<String> {
        self.github.operations.lock().unwrap().clone()
    }

    pub fn integration_push_count(&self) -> usize {
        self.operations()
            .iter()
            .filter(|operation| operation.starts_with("push-integration:"))
            .count()
    }
}
impl FlowServices {
    pub fn process_delivery(&self) -> Result<Vec<String>> {
        self.parallel
            .process_distributor_queue(&self.workspace)
            .map_err(|error| anyhow!(error))
    }
}
