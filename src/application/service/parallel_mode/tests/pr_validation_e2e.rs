pub mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use anyhow::Result;
    use chrono::{TimeDelta, Utc};
    #[cfg(windows)]
    use std::fs::OpenOptions;
    #[cfg(windows)]
    use std::os::windows::fs::OpenOptionsExt;

    use super::super::{FakeGithubAutomationPort, TempGitRepo, run_git, sample_lease_request};
    use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
    use crate::application::port::inbound::pr_validation_query_port::{
        PrValidationQueryPort, PrValidationStatusRequest,
    };
    use crate::application::port::outbound::github_pr_validation_port::{
        GithubPrMergeState, GithubPrValidationError, GithubPrValidationObservationRequest,
        GithubPrValidationPort, GithubPrValidationSnapshot, GithubValidationActivity,
        GithubValidationActivityKind, GithubValidationActor, GithubValidationActorKind,
        GithubValidationCheckRun, GithubValidationProviderMetadata, GithubValidationReviewState,
        GithubValidationRunStatus, GithubValidationSource, GithubValidationSourceLifecycle,
        GithubValidationSourceObservation, GithubValidationSourceStatus,
    };
    use crate::application::port::outbound::planning_authority_port::{
        PrValidationPollLeaseClaimRequest, PrValidationPollLeaseRenewalRequest,
        PrValidationPollSettlement,
    };
    use crate::application::port::outbound::planning_task_repository_port::{
        PlanningDirectionAuthorityCommit, PlanningTaskAuthorityCommit, PlanningTaskRepositoryPort,
    };
    use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
    use crate::application::port::outbound::pr_validation_remediation_port::{
        PrValidationRemediationPort, PrValidationRemediationRequest,
    };
    use crate::application::service::parallel_mode::{
        ParallelModeService, PrValidationPollRequest, PrValidationPollResult,
        PrValidationSchedulerConfig, PrValidationSchedulerRunOutcome, PrValidationSchedulerService,
    };
    use crate::application::service::planning::{
        PlanningRuntimeProjection, PlanningServices, PlanningTaskCreateInput,
    };
    use crate::application::service::pr_validation_query::PrValidationQueryService;
    use crate::domain::github_review::{GithubCommitSha, GithubOpaqueId, GithubPullRequestTarget};
    use crate::domain::parallel_mode::{
        PrValidationCommitSha, PrValidationPhase, PrValidationPollErrorClass, PrValidationRecord,
        PrValidationRecordKey, PrValidationSchedulerMode, PrValidationTarget,
        PrValidationTargetShaSnapshot,
    };
    use crate::domain::planning::{
        DirectionCatalogDocument, DirectionDefinition, DirectionState, PLANNING_FORMAT_VERSION,
        PriorityQueueProjection, QueueIdleConfig, TaskAuthorityDocument, TaskStatus,
    };

    const HEAD_A: &str = "1111111111111111111111111111111111111111";
    const HEAD_B: &str = "2222222222222222222222222222222222222222";
    const BASE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const MERGE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    struct DeterministicGithub {
        snapshots: Mutex<VecDeque<GithubPrValidationSnapshot>>,
        requests: Mutex<Vec<GithubPrValidationObservationRequest>>,
    }

    struct ScriptedGithub {
        responses: Mutex<
            VecDeque<std::result::Result<GithubPrValidationSnapshot, GithubPrValidationError>>,
        >,
        calls: AtomicUsize,
    }

    impl GithubPrValidationPort for ScriptedGithub {
        fn load_validation_snapshot(
            &self,
            _request: &GithubPrValidationObservationRequest,
        ) -> std::result::Result<GithubPrValidationSnapshot, GithubPrValidationError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("the scheduler scenario must provide every response")
        }
    }

    struct EchoGithub {
        calls: AtomicUsize,
        delay: Duration,
    }

    impl GithubPrValidationPort for EchoGithub {
        fn load_validation_snapshot(
            &self,
            request: &GithubPrValidationObservationRequest,
        ) -> std::result::Result<GithubPrValidationSnapshot, GithubPrValidationError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if !self.delay.is_zero() {
                thread::sleep(self.delay);
            }
            Ok(snapshot_for_request(
                request,
                GithubValidationRunStatus::InProgress,
            ))
        }
    }

    impl GithubPrValidationPort for DeterministicGithub {
        fn load_validation_snapshot(
            &self,
            request: &GithubPrValidationObservationRequest,
        ) -> std::result::Result<
            GithubPrValidationSnapshot,
            crate::application::port::outbound::github_pr_validation_port::GithubPrValidationError,
        > {
            self.requests.lock().unwrap().push(request.clone());
            let mut snapshot = self
                .snapshots
                .lock()
                .unwrap()
                .pop_front()
                .expect("the scenario must provide every observation");
            snapshot.normalize();
            Ok(snapshot)
        }
    }

    fn complete_sources() -> Vec<GithubValidationSourceObservation> {
        GithubValidationSource::ALL
            .into_iter()
            .map(|source| {
                GithubValidationSourceObservation::new(
                    source,
                    format!("rest:{source:?}"),
                    GithubValidationSourceStatus::Complete,
                    None,
                )
            })
            .collect()
    }

    fn snapshot(sha: &str, status: GithubValidationRunStatus) -> GithubPrValidationSnapshot {
        GithubPrValidationSnapshot {
            target: GithubPullRequestTarget::new("acme/widgets", 42),
            target_sha: GithubCommitSha::new(sha),
            evidence_sha: GithubCommitSha::new(sha),
            merge_state: GithubPrMergeState::Open,
            merge_sha: None,
            activities: Vec::new(),
            check_runs: vec![
                GithubValidationCheckRun::new(
                    GithubOpaqueId::new("check:ci"),
                    "Post-Merge Gate",
                    GithubCommitSha::new(sha),
                    status,
                )
                .with_attempt_metadata(
                    Some("github-actions".to_string()),
                    Some(GithubOpaqueId::new("check-suite:ci")),
                    Some("2026-08-08T00:00:00Z".to_string()),
                    Some("2026-08-08T00:01:00Z".to_string()),
                ),
            ],
            workflow_runs: Vec::new(),
            sources: complete_sources(),
            next_cursor: None,
            provider_metadata: Default::default(),
        }
    }

    fn snapshot_for_request(
        request: &GithubPrValidationObservationRequest,
        status: GithubValidationRunStatus,
    ) -> GithubPrValidationSnapshot {
        let mut snapshot = snapshot(request.target_sha.as_str(), status);
        snapshot.target = request.target.clone();
        snapshot
    }

    fn merged_snapshot(sha: &str) -> GithubPrValidationSnapshot {
        let mut snapshot = snapshot(sha, GithubValidationRunStatus::Succeeded);
        snapshot.evidence_sha = GithubCommitSha::new(MERGE);
        snapshot.check_runs[0].target_sha = GithubCommitSha::new(MERGE);
        snapshot.merge_state = GithubPrMergeState::Merged;
        snapshot.merge_sha = Some(GithubCommitSha::new(MERGE));
        snapshot
    }

    fn with_late_review(mut snapshot: GithubPrValidationSnapshot) -> GithubPrValidationSnapshot {
        snapshot.activities = vec![
            GithubValidationActivity::new(
                GithubOpaqueId::new("review:late-z"),
                GithubValidationActivityKind::Review,
                "2026-08-07T22:00:02Z",
            )
            .with_commit_sha(GithubCommitSha::new(HEAD_B))
            .with_actor(Some(GithubValidationActor::new(
                "reviewer",
                GithubValidationActorKind::User,
            )))
            .with_review_state(GithubValidationReviewState::ChangesRequested),
            GithubValidationActivity::new(
                GithubOpaqueId::new("comment:late-a"),
                GithubValidationActivityKind::ReviewComment,
                "2026-08-07T22:00:01Z",
            )
            .with_commit_sha(GithubCommitSha::new(HEAD_B))
            .with_actor(Some(GithubValidationActor::new(
                "reviewer",
                GithubValidationActorKind::User,
            ))),
        ];
        snapshot
    }

    fn target_shas(sha: &str) -> PrValidationTargetShaSnapshot {
        PrValidationTargetShaSnapshot::new(
            PrValidationCommitSha::new(sha).unwrap(),
            PrValidationCommitSha::new(BASE).unwrap(),
        )
    }

    fn record() -> PrValidationRecord {
        PrValidationRecord::register(
            PrValidationRecordKey::new("akra-unit-42").unwrap(),
            PrValidationTarget::new("acme/widgets", 42).unwrap(),
            target_shas(HEAD_A),
        )
    }

    fn record_for(key: &str, repository: &str, pull_request_number: u64) -> PrValidationRecord {
        PrValidationRecord::register(
            PrValidationRecordKey::new(key).unwrap(),
            PrValidationTarget::new(repository, pull_request_number).unwrap(),
            target_shas(HEAD_A),
        )
    }

    fn request(repo: &TempGitRepo, revision: u64, sha: &str) -> PrValidationPollRequest {
        PrValidationPollRequest {
            workspace_dir: repo.workspace_dir(),
            pool_root: repo.pool_root(),
            record_key: PrValidationRecordKey::new("akra-unit-42").unwrap(),
            target_shas: target_shas(sha),
            delivery_revision: revision,
        }
    }

    fn temp_repo(prefix: &str) -> TempGitRepo {
        let repo = TempGitRepo::new(prefix);
        #[cfg(windows)]
        {
            use crate::private_fs::{
                WINDOWS_FILE_FLAG_BACKUP_SEMANTICS, WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
                WINDOWS_FILE_SHARE_ALL, WINDOWS_GENERIC_READ, WINDOWS_READ_CONTROL,
                WINDOWS_WRITE_DAC, set_windows_private_acl, validate_windows_private_owner_and_acl,
            };

            let root = OpenOptions::new()
                .read(true)
                .access_mode(WINDOWS_GENERIC_READ | WINDOWS_READ_CONTROL | WINDOWS_WRITE_DAC)
                .share_mode(WINDOWS_FILE_SHARE_ALL)
                .custom_flags(
                    WINDOWS_FILE_FLAG_BACKUP_SEMANTICS | WINDOWS_FILE_FLAG_OPEN_REPARSE_POINT,
                )
                .open(&repo.root)
                .expect("Windows E2E temp root should open for ACL hardening");
            set_windows_private_acl(&root, true)
                .expect("Windows E2E temp root should become owner-private");
            validate_windows_private_owner_and_acl(&repo.root, &root)
                .expect("Windows E2E temp root should retain its private ACL");
        }
        repo
    }

    fn planning(authority: Arc<SqlitePlanningAuthorityAdapter>) -> PlanningServices {
        PlanningServices::from_ports(
            Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
            authority.clone(),
            authority,
            Arc::new(NoopPlanningWorkerPort),
        )
    }

    fn bootstrap(authority: &SqlitePlanningAuthorityAdapter, workspace: &str) {
        authority
            .commit_direction_authority_snapshot(
                workspace,
                PlanningDirectionAuthorityCommit {
                    observed_planning_revision: None,
                    directions: &DirectionCatalogDocument {
                        version: PLANNING_FORMAT_VERSION,
                        queue_idle: QueueIdleConfig::default(),
                        directions: vec![DirectionDefinition {
                            id: "general-workstream".to_string(),
                            title: "General".to_string(),
                            summary: "General work".to_string(),
                            success_criteria: vec!["done".to_string()],
                            scope_hints: Vec::new(),
                            detail_doc_path: String::new(),
                            state: DirectionState::Active,
                        }],
                    },
                    authority_mutation_owner_token: None,
                },
            )
            .unwrap();
        authority
            .commit_task_authority_snapshot(
                workspace,
                PlanningTaskAuthorityCommit {
                    observed_planning_revision: None,
                    task_authority: &TaskAuthorityDocument {
                        version: PLANNING_FORMAT_VERSION,
                        tasks: Vec::new(),
                    },
                    queue_projection: &PriorityQueueProjection {
                        next_task: None,
                        active_tasks: Vec::new(),
                        proposed_tasks: Vec::new(),
                        skipped_tasks: Vec::new(),
                    },
                },
            )
            .unwrap();
    }

    struct RejectUnexpectedRemediation;

    impl PrValidationRemediationPort for RejectUnexpectedRemediation {
        fn request_remediation(
            &self,
            _request: &PrValidationRemediationRequest,
        ) -> Result<PrValidationRecordKey> {
            panic!("this scenario must not admit remediation")
        }
    }

    fn build_service(authority: Arc<SqlitePlanningAuthorityAdapter>) -> ParallelModeService {
        ParallelModeService::new(
            authority,
            Arc::new(FakeGithubAutomationPort::ready()),
            Arc::new(GitParallelModeRuntimeAdapter::new()),
        )
        .with_test_delivery_safety_policy(false, false)
    }

    #[test]
    fn delivery_remediation_merge_and_post_merge_settle() {
        let repo = temp_repo("pr-validation-e2e-happy");
        let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let planning = planning(authority.clone());
        bootstrap(authority.as_ref(), &repo.workspace_dir());
        let service = build_service(authority.clone());
        service
            .persist_pr_validation_record(&repo.workspace_dir(), &repo.pool_root(), None, &record())
            .unwrap();

        let mut late = merged_snapshot(HEAD_B);
        late.check_runs[0] = GithubValidationCheckRun::new(
            GithubOpaqueId::new("check:late"),
            "Post-Merge Gate",
            GithubCommitSha::new(MERGE),
            GithubValidationRunStatus::Failed,
        )
        .with_attempt_metadata(
            Some("github-actions".to_string()),
            Some(GithubOpaqueId::new("check-suite:late")),
            Some("2026-08-08T00:02:00Z".to_string()),
            Some("2026-08-08T00:03:00Z".to_string()),
        );
        let mut reordered_late = with_late_review(late.clone());
        reordered_late.activities.reverse();
        reordered_late.check_runs[0].status = GithubValidationRunStatus::Succeeded;
        let github = DeterministicGithub {
            snapshots: Mutex::new(
                vec![
                    snapshot(HEAD_A, GithubValidationRunStatus::InProgress),
                    snapshot(HEAD_A, GithubValidationRunStatus::Failed),
                    snapshot(HEAD_B, GithubValidationRunStatus::InProgress),
                    merged_snapshot(HEAD_B),
                    late,
                    reordered_late.clone(),
                    reordered_late,
                ]
                .into(),
            ),
            requests: Mutex::new(Vec::new()),
        };

        assert_eq!(
            service
                .poll_pr_validation_into_normal_queue(
                    &github,
                    &planning.queue,
                    request(&repo, 1, HEAD_A),
                )
                .unwrap(),
            PrValidationPollResult::Waiting
        );
        let requested = service
            .poll_pr_validation_into_normal_queue(
                &github,
                &planning.queue,
                request(&repo, 2, HEAD_A),
            )
            .unwrap();
        assert!(matches!(
            requested,
            PrValidationPollResult::RemediationRequested { .. }
        ));

        let queue = planning
            .queue
            .load_authority_snapshot(&repo.workspace_dir())
            .unwrap();
        assert_eq!(queue.tasks.len(), 1);
        let remediation_task = queue.tasks[0].clone();
        let queue_projection =
            SqlitePlanningAuthorityAdapter::load_task_authority_snapshot(&repo.workspace_dir())
                .unwrap()
                .unwrap()
                .queue_projection;
        let projection = PlanningRuntimeProjection::ready_with_queue_projection(
            "authority-backed remediation queue".to_string(),
            queue_projection.queue_summary(),
            None,
            queue_projection.next_task.clone(),
            queue_projection,
        );
        let dispatch = service
            .build_dispatch_plan(&repo.workspace_dir(), &projection, usize::MAX)
            .unwrap();
        assert_eq!(dispatch.candidates.len(), 1);
        assert_eq!(dispatch.candidates[0].task_id, remediation_task.id);

        let lease = service
            .acquire_slot_lease(
                &repo.workspace_dir(),
                sample_lease_request(
                    &remediation_task.id,
                    &remediation_task.title,
                    "agent-remediation",
                    "pr-validation-remediation",
                ),
            )
            .expect("remediation must acquire an ordinary pool lease");
        assert_eq!(lease.task_id, remediation_task.id);
        assert!(
            service
                .transition_pr_validation_remediation_started(
                    &repo.workspace_dir(),
                    &repo.pool_root(),
                    &remediation_task.id,
                )
                .unwrap()
        );
        assert!(
            service
                .transition_pr_validation_remediation_completed(
                    &repo.workspace_dir(),
                    &repo.pool_root(),
                    &remediation_task.id,
                )
                .unwrap()
        );

        assert_eq!(
            service
                .poll_pr_validation_into_normal_queue(
                    &github,
                    &planning.queue,
                    request(&repo, 3, HEAD_B),
                )
                .unwrap(),
            PrValidationPollResult::Waiting
        );
        assert!(github.requests.lock().unwrap()[2].cursor.is_none());

        assert_eq!(
            service
                .poll_pr_validation_into_normal_queue(
                    &github,
                    &planning.queue,
                    request(&repo, 4, HEAD_B),
                )
                .unwrap(),
            PrValidationPollResult::Waiting
        );
        assert!(matches!(
            service
                .poll_pr_validation_into_normal_queue(
                    &github,
                    &planning.queue,
                    request(&repo, 5, HEAD_B),
                )
                .unwrap(),
            PrValidationPollResult::RemediationRequested { .. }
        ));

        let queue = planning
            .queue
            .load_authority_snapshot(&repo.workspace_dir())
            .unwrap();
        assert_eq!(queue.tasks.len(), 2);
        let late_remediation_task = queue.tasks[1].clone();
        let queue_projection =
            SqlitePlanningAuthorityAdapter::load_task_authority_snapshot(&repo.workspace_dir())
                .unwrap()
                .unwrap()
                .queue_projection;
        let projection = PlanningRuntimeProjection::ready_with_queue_projection(
            "authority-backed late remediation queue".to_string(),
            queue_projection.queue_summary(),
            None,
            queue_projection.next_task.clone(),
            queue_projection,
        );
        let dispatch = service
            .build_dispatch_plan(&repo.workspace_dir(), &projection, usize::MAX)
            .unwrap();
        assert_eq!(dispatch.candidates.len(), 1);
        assert_eq!(dispatch.candidates[0].task_id, late_remediation_task.id);
        service
            .acquire_slot_lease(
                &repo.workspace_dir(),
                sample_lease_request(
                    &late_remediation_task.id,
                    &late_remediation_task.title,
                    "agent-late-remediation",
                    "late-pr-validation-remediation",
                ),
            )
            .expect("late remediation must acquire an ordinary pool lease");
        assert!(
            service
                .transition_pr_validation_remediation_started(
                    &repo.workspace_dir(),
                    &repo.pool_root(),
                    &late_remediation_task.id,
                )
                .unwrap()
        );
        assert!(
            service
                .transition_pr_validation_remediation_completed(
                    &repo.workspace_dir(),
                    &repo.pool_root(),
                    &late_remediation_task.id,
                )
                .unwrap()
        );

        let restarted = build_service(authority.clone());
        assert!(matches!(
            restarted
                .poll_pr_validation_into_normal_queue(
                    &github,
                    &planning.queue,
                    request(&repo, 6, HEAD_B),
                )
                .unwrap(),
            PrValidationPollResult::RemediationRequested { .. }
        ));
        let queue = planning
            .queue
            .load_authority_snapshot(&repo.workspace_dir())
            .unwrap();
        assert_eq!(queue.tasks.len(), 3);
        let review_remediation_task = queue.tasks[2].clone();
        let queue_projection =
            SqlitePlanningAuthorityAdapter::load_task_authority_snapshot(&repo.workspace_dir())
                .unwrap()
                .unwrap()
                .queue_projection;
        let projection = PlanningRuntimeProjection::ready_with_queue_projection(
            "authority-backed review remediation queue".to_string(),
            queue_projection.queue_summary(),
            None,
            queue_projection.next_task.clone(),
            queue_projection,
        );
        let dispatch = restarted
            .build_dispatch_plan(&repo.workspace_dir(), &projection, usize::MAX)
            .unwrap();
        assert_eq!(dispatch.candidates.len(), 1);
        assert_eq!(dispatch.candidates[0].task_id, review_remediation_task.id);
        restarted
            .acquire_slot_lease(
                &repo.workspace_dir(),
                sample_lease_request(
                    &review_remediation_task.id,
                    &review_remediation_task.title,
                    "agent-review-remediation",
                    "review-pr-validation-remediation",
                ),
            )
            .expect("review remediation must acquire an ordinary pool lease");
        assert!(
            restarted
                .transition_pr_validation_remediation_started(
                    &repo.workspace_dir(),
                    &repo.pool_root(),
                    &review_remediation_task.id,
                )
                .unwrap()
        );
        assert!(
            restarted
                .transition_pr_validation_remediation_completed(
                    &repo.workspace_dir(),
                    &repo.pool_root(),
                    &review_remediation_task.id,
                )
                .unwrap()
        );
        assert_eq!(
            restarted
                .poll_pr_validation_into_normal_queue(
                    &github,
                    &planning.queue,
                    request(&repo, 7, HEAD_B),
                )
                .unwrap(),
            PrValidationPollResult::Settled
        );
        assert_eq!(
            restarted
                .poll_pr_validation_into_normal_queue(
                    &github,
                    &planning.queue,
                    request(&repo, 7, HEAD_B),
                )
                .unwrap(),
            PrValidationPollResult::StaleDeliveryIgnored
        );

        let query = PrValidationQueryService::new(repo.workspace_dir(), authority);
        let summary = query
            .status_for_pr(PrValidationStatusRequest {
                pull_request_number: 42,
            })
            .unwrap()
            .unwrap();
        assert_eq!(summary.phase, PrValidationPhase::Settled);
        assert_eq!(
            summary.finding_count, 2,
            "the completed post-merge check and review remain visible"
        );
        assert_eq!(
            summary.remediation_count, 2,
            "the post-merge check and review each retain a correlation"
        );
        assert!(summary.post_merge_checkpoint_observed);
        assert_eq!(summary.next_action(), "none");
    }

    #[test]
    fn exhausted_pool_keeps_remediation_queued_without_duplicate_lease() {
        let repo = temp_repo("pr-validation-exhausted-pool");
        let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let planning = planning(authority.clone());
        bootstrap(authority.as_ref(), &repo.workspace_dir());
        let github = DeterministicGithub {
            snapshots: Mutex::new(
                vec![
                    snapshot(HEAD_A, GithubValidationRunStatus::Failed),
                    snapshot(HEAD_A, GithubValidationRunStatus::Failed),
                ]
                .into(),
            ),
            requests: Mutex::new(Vec::new()),
        };
        let service = build_service(authority.clone());
        for (idempotency_key, title) in [
            (
                "occupy-normal-pool-capacity-1",
                "Hold ordinary pool capacity one",
            ),
            (
                "occupy-normal-pool-capacity-2",
                "Hold ordinary pool capacity two",
            ),
            (
                "occupy-normal-pool-capacity-3",
                "Hold ordinary pool capacity three",
            ),
        ] {
            planning
                .queue
                .admit_system_task_once(
                    &repo.workspace_dir(),
                    idempotency_key,
                    PlanningTaskCreateInput {
                        direction_id: None,
                        direction_relation_note: Some(
                            "occupies an ordinary pool slot".to_string(),
                        ),
                        title: title.to_string(),
                        description: Some(
                            "Keeps normal pool capacity exhausted while PR validation admits remediation."
                                .to_string(),
                        ),
                        status: Some(TaskStatus::Ready),
                        base_priority: None,
                        dynamic_priority_delta: None,
                        priority_reason: None,
                        depends_on: Vec::new(),
                        blocked_by: Vec::new(),
                    },
                )
                .unwrap();
        }
        let queue = planning
            .queue
            .load_authority_snapshot(&repo.workspace_dir())
            .unwrap();
        assert_eq!(queue.tasks.len(), 3);
        let first_capacity_task = queue.tasks[0].clone();
        let second_capacity_task = queue.tasks[1].clone();
        let third_capacity_task = queue.tasks[2].clone();
        for (task, agent_id, session_label) in [
            (
                first_capacity_task.clone(),
                "agent-holding-capacity-one",
                "pr-validation-capacity-exhaustion-one",
            ),
            (
                second_capacity_task.clone(),
                "agent-holding-capacity-two",
                "pr-validation-capacity-exhaustion-two",
            ),
            (
                third_capacity_task.clone(),
                "agent-holding-capacity-three",
                "pr-validation-capacity-exhaustion-three",
            ),
        ] {
            let lease = service
                .acquire_slot_lease(
                    &repo.workspace_dir(),
                    sample_lease_request(&task.id, &task.title, agent_id, session_label),
                )
                .expect("the capacity holder must occupy an ordinary pool slot");
            assert_eq!(lease.task_id, task.id);
            service
                .mark_workspace_slot_running(&lease.worktree_path)
                .expect("capacity holder must transition to running");
        }
        let held_capacity_task_ids = [
            first_capacity_task.id,
            second_capacity_task.id,
            third_capacity_task.id,
        ];

        service
            .persist_pr_validation_record(&repo.workspace_dir(), &repo.pool_root(), None, &record())
            .unwrap();

        assert!(matches!(
            service
                .poll_pr_validation_into_normal_queue(
                    &github,
                    &planning.queue,
                    request(&repo, 1, HEAD_A),
                )
                .unwrap(),
            PrValidationPollResult::RemediationRequested { .. }
        ));
        let queue = planning
            .queue
            .load_authority_snapshot(&repo.workspace_dir())
            .unwrap();
        assert_eq!(queue.tasks.len(), 4);
        let queued_remediation_task = queue
            .tasks
            .iter()
            .find(|task| !held_capacity_task_ids.contains(&task.id))
            .cloned()
            .expect("PR remediation must remain queued");
        let queue_projection =
            SqlitePlanningAuthorityAdapter::load_task_authority_snapshot(&repo.workspace_dir())
                .unwrap()
                .unwrap()
                .queue_projection;
        let projection = PlanningRuntimeProjection::ready_with_queue_projection(
            "authority-backed exhausted remediation queue".to_string(),
            queue_projection.queue_summary(),
            None,
            queue_projection.next_task.clone(),
            queue_projection,
        );
        let dispatch = service
            .build_dispatch_plan(&repo.workspace_dir(), &projection, usize::MAX)
            .unwrap();
        assert_eq!(dispatch.idle_slot_count, 0);
        assert!(dispatch.candidates.is_empty());

        service
            .poll_pr_validation_into_normal_queue(
                &github,
                &planning.queue,
                request(&repo, 2, HEAD_A),
            )
            .unwrap();
        let queue = planning
            .queue
            .load_authority_snapshot(&repo.workspace_dir())
            .unwrap();
        assert_eq!(queue.tasks.len(), 4);
        assert!(
            queue
                .tasks
                .iter()
                .any(|task| task.id == queued_remediation_task.id)
        );
    }

    #[test]
    fn durable_scheduler_polls_with_fake_time_and_normal_remediation() {
        let repo = temp_repo("pr-validation-scheduler");
        let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let planning = planning(authority.clone());
        bootstrap(authority.as_ref(), &repo.workspace_dir());
        let github = Arc::new(DeterministicGithub {
            snapshots: Mutex::new(
                vec![
                    snapshot(HEAD_A, GithubValidationRunStatus::InProgress),
                    snapshot(HEAD_A, GithubValidationRunStatus::Failed),
                ]
                .into(),
            ),
            requests: Mutex::new(Vec::new()),
        });
        let service =
            build_service(authority.clone()).with_pr_validation_observation(github.clone());
        service
            .persist_pr_validation_record(&repo.workspace_dir(), &repo.pool_root(), None, &record())
            .unwrap();
        let mut config = PrValidationSchedulerConfig::default();
        config.mode = PrValidationSchedulerMode::Remediate;
        let scheduler = PrValidationSchedulerService::new(
            service.clone(),
            planning.queue.clone(),
            repo.workspace_dir(),
            config,
        )
        .unwrap();
        let first_poll_at = Utc::now() + TimeDelta::seconds(1);

        let first_next_poll_at = match scheduler.run_due_once_at(first_poll_at).unwrap() {
            PrValidationSchedulerRunOutcome::PollSettled {
                result: PrValidationPollResult::Waiting,
                next_poll_at,
                ..
            } => next_poll_at,
            outcome => panic!("unexpected first scheduler outcome: {outcome:?}"),
        };
        assert!(
            !repo.pool_root().join(".leases").exists(),
            "waiting validation must not reserve a parallel slot"
        );
        assert_eq!(
            scheduler
                .run_due_once_at(first_poll_at + TimeDelta::seconds(10))
                .unwrap(),
            PrValidationSchedulerRunOutcome::Idle,
            "UI-rate refreshes must not accelerate the durable scheduler clock"
        );
        assert!(matches!(
            scheduler
                .run_due_once_at(first_next_poll_at + TimeDelta::seconds(1))
                .unwrap(),
            PrValidationSchedulerRunOutcome::PollSettled {
                result: PrValidationPollResult::RemediationRequested { .. },
                ..
            }
        ));

        let persisted = service
            .recover_pr_validation_record(
                &repo.workspace_dir(),
                &repo.pool_root(),
                &PrValidationRecordKey::new("akra-unit-42").unwrap(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(persisted.observation_revision(), 2);
        assert_eq!(github.requests.lock().unwrap().len(), 2);
        let queue = planning
            .queue
            .load_authority_snapshot(&repo.workspace_dir())
            .unwrap();
        assert_eq!(queue.tasks.len(), 1);
        let queue_projection =
            SqlitePlanningAuthorityAdapter::load_task_authority_snapshot(&repo.workspace_dir())
                .unwrap()
                .unwrap()
                .queue_projection;
        let projection = PlanningRuntimeProjection::ready_with_queue_projection(
            "runtime validation remediation".to_string(),
            queue_projection.queue_summary(),
            None,
            queue_projection.next_task.clone(),
            queue_projection,
        );
        let dispatch = service
            .build_dispatch_plan(&repo.workspace_dir(), &projection, usize::MAX)
            .unwrap();
        assert_eq!(dispatch.candidates.len(), 1);
        assert_eq!(dispatch.candidates[0].task_id, queue.tasks[0].id);
        assert!(!repo.pool_root().join(".leases").exists());
    }

    #[test]
    fn scheduler_mode_uses_strict_repository_local_configuration() {
        let repo = temp_repo("pr-validation-mode-config");
        let service = build_service(Arc::new(SqlitePlanningAuthorityAdapter::new()));
        assert_eq!(
            PrValidationSchedulerConfig::from_repository(&service, &repo.workspace_dir())
                .unwrap()
                .mode,
            PrValidationSchedulerMode::Observe
        );
        for (configured, expected) in [
            ("off", PrValidationSchedulerMode::Off),
            ("observe", PrValidationSchedulerMode::Observe),
            ("remediate", PrValidationSchedulerMode::Remediate),
        ] {
            run_git(
                &repo.repo_root,
                &["config", "akra.prValidationMode", configured],
            );
            assert_eq!(
                PrValidationSchedulerConfig::from_repository(&service, &repo.workspace_dir())
                    .unwrap()
                    .mode,
                expected
            );
        }
        run_git(
            &repo.repo_root,
            &["config", "akra.prValidationMode", "enabled"],
        );
        assert!(
            PrValidationSchedulerConfig::from_repository(&service, &repo.workspace_dir()).is_err()
        );
    }

    #[test]
    fn observe_scheduler_records_findings_without_queue_admission() {
        let repo = temp_repo("pr-validation-observe-mode");
        let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let planning = planning(authority.clone());
        bootstrap(authority.as_ref(), &repo.workspace_dir());
        let github = Arc::new(DeterministicGithub {
            snapshots: Mutex::new(vec![snapshot(HEAD_A, GithubValidationRunStatus::Failed)].into()),
            requests: Mutex::new(Vec::new()),
        });
        let service = build_service(authority).with_pr_validation_observation(github.clone());
        service
            .persist_pr_validation_record(&repo.workspace_dir(), &repo.pool_root(), None, &record())
            .unwrap();
        let scheduler = PrValidationSchedulerService::new(
            service.clone(),
            planning.queue.clone(),
            repo.workspace_dir(),
            PrValidationSchedulerConfig::default(),
        )
        .unwrap();
        assert!(matches!(
            scheduler
                .run_due_once_at(Utc::now() + TimeDelta::seconds(1))
                .unwrap(),
            PrValidationSchedulerRunOutcome::PollSettled {
                result: PrValidationPollResult::Waiting,
                ..
            }
        ));
        let persisted = service
            .recover_pr_validation_record(&repo.workspace_dir(), &repo.pool_root(), record().key())
            .unwrap()
            .unwrap();
        assert_eq!(persisted.finding_keys().len(), 1);
        assert!(
            planning
                .queue
                .load_authority_snapshot(&repo.workspace_dir())
                .unwrap()
                .tasks
                .is_empty(),
            "observe mode must never admit a remediation task"
        );
        assert_eq!(github.requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn sqlite_due_projection_claims_only_settled_records_with_an_active_review_watch() {
        for (suffix, watchable) in [("watchable", true), ("finite", false)] {
            let repo = temp_repo(&format!("pr-validation-settled-{suffix}"));
            let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
            let mut observed = merged_snapshot(HEAD_A);
            if !watchable {
                for source in &mut observed.sources {
                    source.lifecycle = GithubValidationSourceLifecycle::Finite;
                }
            }
            let github = DeterministicGithub {
                snapshots: Mutex::new(vec![observed.clone(), observed].into()),
                requests: Mutex::new(Vec::new()),
            };
            let service = build_service(authority);
            service
                .persist_pr_validation_record(
                    &repo.workspace_dir(),
                    &repo.pool_root(),
                    None,
                    &record(),
                )
                .unwrap();
            assert_eq!(
                service
                    .poll_pr_validation(
                        &github,
                        &RejectUnexpectedRemediation,
                        request(&repo, 1, HEAD_A),
                    )
                    .unwrap(),
                PrValidationPollResult::Waiting
            );
            assert_eq!(
                service
                    .poll_pr_validation(
                        &github,
                        &RejectUnexpectedRemediation,
                        request(&repo, 2, HEAD_A),
                    )
                    .unwrap(),
                PrValidationPollResult::Settled
            );
            let settled = service
                .recover_pr_validation_record(
                    &repo.workspace_dir(),
                    &repo.pool_root(),
                    record().key(),
                )
                .unwrap()
                .unwrap();
            assert_eq!(settled.review_watch_active(), watchable);

            let due_at = Utc::now() + TimeDelta::seconds(1);
            let due = SqlitePlanningAuthorityAdapter::load_due_runtime_pr_validation_record_keys(
                &repo.workspace_dir(),
                due_at,
                due_at - TimeDelta::seconds(30),
                8,
            )
            .unwrap();
            if watchable {
                assert_eq!(due, vec![record().key().clone()]);
                assert!(
                    SqlitePlanningAuthorityAdapter::try_claim_runtime_pr_validation_poll(
                        &repo.workspace_dir(),
                        PrValidationPollLeaseClaimRequest {
                            record_key: record().key(),
                            owner: "watch-owner",
                            token: "watch-token",
                            claimed_at: due_at,
                            expires_at: due_at + TimeDelta::seconds(180),
                            repository_cooldown_since: due_at - TimeDelta::seconds(30),
                        },
                    )
                    .unwrap()
                    .is_some()
                );
            } else {
                assert!(due.is_empty());
                assert!(
                    SqlitePlanningAuthorityAdapter::try_claim_runtime_pr_validation_poll(
                        &repo.workspace_dir(),
                        PrValidationPollLeaseClaimRequest {
                            record_key: record().key(),
                            owner: "finite-owner",
                            token: "finite-token",
                            claimed_at: due_at,
                            expires_at: due_at + TimeDelta::seconds(180),
                            repository_cooldown_since: due_at - TimeDelta::seconds(30),
                        },
                    )
                    .unwrap()
                    .is_none()
                );
            }
        }
    }

    #[test]
    fn scheduler_moves_verified_records_from_active_cadence_to_bounded_review_watch_cadence() {
        let repo = temp_repo("pr-validation-review-watch-cadence");
        let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let planning = planning(authority.clone());
        bootstrap(authority.as_ref(), &repo.workspace_dir());
        let observed = merged_snapshot(HEAD_A);
        let github = Arc::new(DeterministicGithub {
            snapshots: Mutex::new(vec![observed.clone(), observed.clone(), observed].into()),
            requests: Mutex::new(Vec::new()),
        });
        let service = build_service(authority).with_pr_validation_observation(github.clone());
        service
            .persist_pr_validation_record(&repo.workspace_dir(), &repo.pool_root(), None, &record())
            .unwrap();
        let scheduler = PrValidationSchedulerService::new(
            service,
            planning.queue,
            repo.workspace_dir(),
            PrValidationSchedulerConfig::default(),
        )
        .unwrap();

        let first_due = Utc::now() + TimeDelta::seconds(1);
        let active_next = match scheduler.run_due_once_at(first_due).unwrap() {
            PrValidationSchedulerRunOutcome::PollSettled {
                result: PrValidationPollResult::Waiting,
                next_poll_at,
                ..
            } => next_poll_at,
            outcome => panic!("unexpected initial validation outcome: {outcome:?}"),
        };
        let settle_at = active_next + TimeDelta::seconds(1);
        let watch_next = match scheduler.run_due_once_at(settle_at).unwrap() {
            PrValidationSchedulerRunOutcome::PollSettled {
                result: PrValidationPollResult::Settled,
                next_poll_at,
                ..
            } => next_poll_at,
            outcome => panic!("unexpected verified validation outcome: {outcome:?}"),
        };
        assert!(watch_next >= settle_at + TimeDelta::seconds(299));
        assert_eq!(
            scheduler
                .run_due_once_at(watch_next - TimeDelta::milliseconds(1))
                .unwrap(),
            PrValidationSchedulerRunOutcome::Idle
        );
        assert!(matches!(
            scheduler.run_due_once_at(watch_next).unwrap(),
            PrValidationSchedulerRunOutcome::PollSettled {
                result: PrValidationPollResult::Settled,
                ..
            }
        ));
        assert_eq!(github.requests.lock().unwrap().len(), 3);
    }

    #[test]
    fn durable_poll_lease_uses_exact_cas_and_allows_takeover_only_after_expiry() {
        let repo = temp_repo("pr-validation-lease-cas");
        let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let service = build_service(authority);
        service
            .persist_pr_validation_record(&repo.workspace_dir(), &repo.pool_root(), None, &record())
            .unwrap();
        let claimed_at = Utc::now() + TimeDelta::seconds(1);
        let first_expiry = claimed_at + TimeDelta::seconds(180);
        let cooldown_since = claimed_at - TimeDelta::seconds(30);
        let first = SqlitePlanningAuthorityAdapter::try_claim_runtime_pr_validation_poll(
            &repo.workspace_dir(),
            PrValidationPollLeaseClaimRequest {
                record_key: record().key(),
                owner: "owner-a",
                token: "token-a",
                claimed_at,
                expires_at: first_expiry,
                repository_cooldown_since: cooldown_since,
            },
        )
        .unwrap()
        .expect("first owner should claim the due record");
        assert_eq!(first.poll_attempt, 1);
        assert!(
            SqlitePlanningAuthorityAdapter::try_claim_runtime_pr_validation_poll(
                &repo.workspace_dir(),
                PrValidationPollLeaseClaimRequest {
                    record_key: record().key(),
                    owner: "owner-b",
                    token: "token-b",
                    claimed_at: claimed_at + TimeDelta::seconds(1),
                    expires_at: first_expiry + TimeDelta::seconds(1),
                    repository_cooldown_since: cooldown_since,
                },
            )
            .unwrap()
            .is_none()
        );
        assert!(
            !SqlitePlanningAuthorityAdapter::renew_runtime_pr_validation_poll_lease(
                &repo.workspace_dir(),
                PrValidationPollLeaseRenewalRequest {
                    record_key: record().key(),
                    owner: "owner-a",
                    token: "wrong-token",
                    expected_expires_at: first_expiry,
                    renewed_at: claimed_at + TimeDelta::seconds(30),
                    renewed_expires_at: first_expiry + TimeDelta::seconds(30),
                },
            )
            .unwrap()
        );
        let renewed_expiry = first_expiry + TimeDelta::seconds(30);
        assert!(
            SqlitePlanningAuthorityAdapter::renew_runtime_pr_validation_poll_lease(
                &repo.workspace_dir(),
                PrValidationPollLeaseRenewalRequest {
                    record_key: record().key(),
                    owner: "owner-a",
                    token: "token-a",
                    expected_expires_at: first_expiry,
                    renewed_at: claimed_at + TimeDelta::seconds(30),
                    renewed_expires_at: renewed_expiry,
                },
            )
            .unwrap()
        );
        let stale_settlement = PrValidationPollSettlement {
            polled_at: claimed_at + TimeDelta::seconds(31),
            next_poll_at: claimed_at + TimeDelta::seconds(61),
            consecutive_error_count: 0,
            error_class: None,
            rate_limit_remaining: Some(4_999),
            rate_limit_reset_at: None,
        };
        assert!(
            !SqlitePlanningAuthorityAdapter::settle_runtime_pr_validation_poll(
                &repo.workspace_dir(),
                record().key(),
                "owner-a",
                "token-a",
                first_expiry,
                &stale_settlement,
            )
            .unwrap(),
            "a stale expiry must not settle a renewed generation"
        );
        assert!(
            SqlitePlanningAuthorityAdapter::try_claim_runtime_pr_validation_poll(
                &repo.workspace_dir(),
                PrValidationPollLeaseClaimRequest {
                    record_key: record().key(),
                    owner: "owner-b",
                    token: "token-b",
                    claimed_at: renewed_expiry - TimeDelta::milliseconds(1),
                    expires_at: renewed_expiry + TimeDelta::seconds(180),
                    repository_cooldown_since: renewed_expiry - TimeDelta::seconds(30),
                },
            )
            .unwrap()
            .is_none()
        );
        let takeover = SqlitePlanningAuthorityAdapter::try_claim_runtime_pr_validation_poll(
            &repo.workspace_dir(),
            PrValidationPollLeaseClaimRequest {
                record_key: record().key(),
                owner: "owner-b",
                token: "token-b",
                claimed_at: renewed_expiry,
                expires_at: renewed_expiry + TimeDelta::seconds(180),
                repository_cooldown_since: renewed_expiry - TimeDelta::seconds(30),
            },
        )
        .unwrap()
        .expect("an expired owner must not block takeover");
        assert_eq!(takeover.poll_attempt, 2);
        assert!(
            !SqlitePlanningAuthorityAdapter::settle_runtime_pr_validation_poll(
                &repo.workspace_dir(),
                record().key(),
                "owner-a",
                "token-a",
                renewed_expiry,
                &stale_settlement,
            )
            .unwrap(),
            "the replaced owner/token pair must not settle the new generation"
        );
    }

    #[test]
    fn two_scheduler_processes_allow_only_one_provider_caller() {
        let repo = temp_repo("pr-validation-two-process-race");
        let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let planning = planning(authority.clone());
        bootstrap(authority.as_ref(), &repo.workspace_dir());
        let github = Arc::new(EchoGithub {
            calls: AtomicUsize::new(0),
            delay: Duration::from_millis(250),
        });
        let service = build_service(authority).with_pr_validation_observation(github.clone());
        service
            .persist_pr_validation_record(&repo.workspace_dir(), &repo.pool_root(), None, &record())
            .unwrap();
        let config = PrValidationSchedulerConfig::default();
        let scheduler_a = PrValidationSchedulerService::new(
            service.clone(),
            planning.queue.clone(),
            repo.workspace_dir(),
            config.clone(),
        )
        .unwrap();
        let scheduler_b = PrValidationSchedulerService::new(
            service,
            planning.queue.clone(),
            repo.workspace_dir(),
            config,
        )
        .unwrap();
        let due_at = Utc::now() + TimeDelta::seconds(1);
        let first = thread::spawn(move || scheduler_a.run_due_once_at(due_at));
        for _ in 0..100 {
            if github.calls.load(Ordering::SeqCst) == 1 {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(github.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            scheduler_b.run_due_once_at(due_at).unwrap(),
            PrValidationSchedulerRunOutcome::Idle
        );
        assert!(matches!(
            first.join().unwrap().unwrap(),
            PrValidationSchedulerRunOutcome::PollSettled {
                result: PrValidationPollResult::Waiting,
                ..
            }
        ));
        assert_eq!(github.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn provider_failures_back_off_and_only_identity_mismatch_is_terminal_failed() {
        let repo = temp_repo("pr-validation-provider-backoff");
        let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let planning = planning(authority.clone());
        bootstrap(authority.as_ref(), &repo.workspace_dir());
        let due_at = Utc::now() + TimeDelta::seconds(1);
        let rate_reset_at = due_at + TimeDelta::seconds(180);
        let github = Arc::new(ScriptedGithub {
            responses: Mutex::new(
                vec![
                    Err(GithubPrValidationError::retryable("GitHub HTTP 429")
                        .with_retry_after_at(Some(due_at + TimeDelta::seconds(120)))
                        .with_provider_metadata(GithubValidationProviderMetadata {
                            rate_limit_remaining: Some(0),
                            rate_limit_reset_at: Some(rate_reset_at),
                        })),
                    Err(GithubPrValidationError::retryable("GitHub HTTP 503")),
                    Err(GithubPrValidationError::retryable(
                        "GitHub request timed out",
                    )),
                    Err(GithubPrValidationError::authentication_blocked(
                        "GitHub credential scope is unavailable",
                    )),
                    Err(GithubPrValidationError::integrity_failed(
                        "GitHub response envelope was malformed",
                    )),
                    Err(GithubPrValidationError::identity_failed(
                        "GitHub PR head identity changed",
                    )),
                ]
                .into(),
            ),
            calls: AtomicUsize::new(0),
        });
        let service = build_service(authority).with_pr_validation_observation(github.clone());
        service
            .persist_pr_validation_record(&repo.workspace_dir(), &repo.pool_root(), None, &record())
            .unwrap();
        let mut scheduler = PrValidationSchedulerService::new(
            service.clone(),
            planning.queue.clone(),
            repo.workspace_dir(),
            PrValidationSchedulerConfig::default(),
        )
        .unwrap();
        let expected_classes = [
            PrValidationPollErrorClass::RetryableProvider,
            PrValidationPollErrorClass::RetryableProvider,
            PrValidationPollErrorClass::RetryableProvider,
            PrValidationPollErrorClass::AuthenticationBlocked,
            PrValidationPollErrorClass::IntegrityFailed,
        ];
        let mut next_attempt_at = due_at;
        for (index, expected_class) in expected_classes.into_iter().enumerate() {
            let next_poll_at = match scheduler.run_due_once_at(next_attempt_at).unwrap() {
                PrValidationSchedulerRunOutcome::RetryScheduled {
                    error_class,
                    next_poll_at,
                    ..
                } => {
                    assert_eq!(error_class, expected_class);
                    next_poll_at
                }
                outcome => panic!("unexpected retry outcome: {outcome:?}"),
            };
            if index == 0 {
                assert!(next_poll_at >= rate_reset_at);
                scheduler = PrValidationSchedulerService::new(
                    service.clone(),
                    planning.queue.clone(),
                    repo.workspace_dir(),
                    PrValidationSchedulerConfig::default(),
                )
                .unwrap();
            } else {
                assert!(next_poll_at > next_attempt_at);
            }
            assert_ne!(
                service
                    .recover_pr_validation_record(
                        &repo.workspace_dir(),
                        &repo.pool_root(),
                        record().key(),
                    )
                    .unwrap()
                    .unwrap()
                    .phase(),
                PrValidationPhase::Failed
            );
            assert_eq!(
                scheduler
                    .run_due_once_at(next_poll_at - TimeDelta::milliseconds(1))
                    .unwrap(),
                PrValidationSchedulerRunOutcome::Idle
            );
            next_attempt_at = next_poll_at + TimeDelta::seconds(1);
        }
        assert!(matches!(
            scheduler.run_due_once_at(next_attempt_at).unwrap(),
            PrValidationSchedulerRunOutcome::Terminal {
                result: PrValidationPollResult::Failed,
                error_class: PrValidationPollErrorClass::IdentityFailed,
                ..
            }
        ));
        assert_eq!(
            service
                .recover_pr_validation_record(
                    &repo.workspace_dir(),
                    &repo.pool_root(),
                    record().key(),
                )
                .unwrap()
                .unwrap()
                .phase(),
            PrValidationPhase::Failed
        );
        assert_eq!(github.calls.load(Ordering::SeqCst), 6);
    }

    #[test]
    fn five_due_records_share_one_repository_api_budget_window() {
        let repo = temp_repo("pr-validation-repository-budget");
        let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let planning = planning(authority.clone());
        bootstrap(authority.as_ref(), &repo.workspace_dir());
        let github = Arc::new(EchoGithub {
            calls: AtomicUsize::new(0),
            delay: Duration::ZERO,
        });
        let service = build_service(authority).with_pr_validation_observation(github.clone());
        for index in 0..5 {
            let record = record_for(
                &format!("repository-budget-{index}"),
                "acme/widgets",
                40 + index,
            );
            service
                .persist_pr_validation_record(
                    &repo.workspace_dir(),
                    &repo.pool_root(),
                    None,
                    &record,
                )
                .unwrap();
        }
        let scheduler = PrValidationSchedulerService::new(
            service,
            planning.queue,
            repo.workspace_dir(),
            PrValidationSchedulerConfig::default(),
        )
        .unwrap();
        let due_at = Utc::now() + TimeDelta::seconds(1);
        let next_poll_at = match scheduler.run_due_once_at(due_at).unwrap() {
            PrValidationSchedulerRunOutcome::PollSettled { next_poll_at, .. } => next_poll_at,
            outcome => panic!("unexpected repository budget outcome: {outcome:?}"),
        };
        for offset in 1..30 {
            assert_eq!(
                scheduler
                    .run_due_once_at(due_at + TimeDelta::seconds(offset))
                    .unwrap(),
                PrValidationSchedulerRunOutcome::Idle
            );
        }
        assert_eq!(
            github.calls.load(Ordering::SeqCst),
            1,
            "five due records must not recreate a seven-request-per-second repository burst"
        );
        assert!(matches!(
            scheduler
                .run_due_once_at(next_poll_at + TimeDelta::seconds(1))
                .unwrap(),
            PrValidationSchedulerRunOutcome::PollSettled { .. }
        ));
        assert_eq!(github.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn one_rate_limited_record_fences_every_due_record_in_the_repository() {
        let repo = temp_repo("pr-validation-repository-rate-limit");
        let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let planning = planning(authority.clone());
        bootstrap(authority.as_ref(), &repo.workspace_dir());
        let due_at = Utc::now() + TimeDelta::seconds(1);
        let rate_reset_at = due_at + TimeDelta::seconds(180);
        let mut recovered_snapshot = snapshot(HEAD_A, GithubValidationRunStatus::InProgress);
        recovered_snapshot.target = GithubPullRequestTarget::new("acme/widgets", 41);
        let github = Arc::new(ScriptedGithub {
            responses: Mutex::new(
                vec![
                    Err(GithubPrValidationError::retryable("GitHub HTTP 429")
                        .with_retry_after_at(Some(due_at + TimeDelta::seconds(90)))
                        .with_provider_metadata(GithubValidationProviderMetadata {
                            rate_limit_remaining: Some(0),
                            rate_limit_reset_at: Some(rate_reset_at),
                        })),
                    Ok(recovered_snapshot),
                ]
                .into(),
            ),
            calls: AtomicUsize::new(0),
        });
        let service = build_service(authority).with_pr_validation_observation(github.clone());
        for (key, number) in [("rate-limit-0", 40), ("rate-limit-1", 41)] {
            service
                .persist_pr_validation_record(
                    &repo.workspace_dir(),
                    &repo.pool_root(),
                    None,
                    &record_for(key, "acme/widgets", number),
                )
                .unwrap();
        }
        let scheduler = PrValidationSchedulerService::new(
            service,
            planning.queue,
            repo.workspace_dir(),
            PrValidationSchedulerConfig::default(),
        )
        .unwrap();
        assert!(matches!(
            scheduler.run_due_once_at(due_at).unwrap(),
            PrValidationSchedulerRunOutcome::RetryScheduled {
                error_class: PrValidationPollErrorClass::RetryableProvider,
                next_poll_at,
                ..
            } if next_poll_at >= rate_reset_at
        ));
        for attempt_at in [
            due_at + TimeDelta::seconds(31),
            rate_reset_at - TimeDelta::milliseconds(1),
        ] {
            assert_eq!(
                scheduler.run_due_once_at(attempt_at).unwrap(),
                PrValidationSchedulerRunOutcome::Idle
            );
        }
        assert_eq!(github.calls.load(Ordering::SeqCst), 1);
        assert!(matches!(
            scheduler
                .run_due_once_at(rate_reset_at + TimeDelta::seconds(1))
                .unwrap(),
            PrValidationSchedulerRunOutcome::PollSettled { .. }
        ));
        assert_eq!(github.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn unknown_provider_and_late_event_cannot_false_settle() {
        let repo = temp_repo("pr-validation-e2e-blocked");
        let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let service = build_service(authority.clone());
        service
            .persist_pr_validation_record(&repo.workspace_dir(), &repo.pool_root(), None, &record())
            .unwrap();

        let merged = merged_snapshot(HEAD_A);
        let mut unknown = merged.clone();
        unknown.sources[1].status = GithubValidationSourceStatus::Unknown;
        let late = with_late_review(merged_snapshot(HEAD_A));
        let github = DeterministicGithub {
            snapshots: Mutex::new(vec![merged, unknown, late].into()),
            requests: Mutex::new(Vec::new()),
        };
        let no_remediation = RejectUnexpectedRemediation;

        assert_eq!(
            service
                .poll_pr_validation(&github, &no_remediation, request(&repo, 1, HEAD_A))
                .unwrap(),
            PrValidationPollResult::Waiting
        );
        assert_eq!(
            service
                .poll_pr_validation(&github, &no_remediation, request(&repo, 2, HEAD_A))
                .unwrap(),
            PrValidationPollResult::Waiting
        );
        assert_eq!(
            service
                .poll_pr_validation(&github, &no_remediation, request(&repo, 3, HEAD_A))
                .unwrap(),
            PrValidationPollResult::Waiting,
            "a delayed relevant event must invalidate the catch-up fingerprint"
        );
        assert_eq!(
            service
                .recover_pr_validation_record(
                    &repo.workspace_dir(),
                    &repo.pool_root(),
                    &PrValidationRecordKey::new("akra-unit-42").unwrap(),
                )
                .unwrap()
                .unwrap()
                .phase(),
            PrValidationPhase::PostMergeObservation
        );
        assert!(!repo.pool_root().join(".leases").exists());
    }
}
