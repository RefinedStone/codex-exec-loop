pub mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use anyhow::Result;
    #[cfg(windows)]
    use std::fs::OpenOptions;
    #[cfg(windows)]
    use std::os::windows::fs::OpenOptionsExt;

    use super::super::{FakeGithubAutomationPort, TempGitRepo, sample_lease_request};
    use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
    use crate::application::port::inbound::pr_validation_query_port::{
        PrValidationQueryPort, PrValidationStatusRequest,
    };
    use crate::application::port::outbound::github_pr_validation_port::{
        GithubPrMergeState, GithubPrValidationObservationRequest, GithubPrValidationPort,
        GithubPrValidationSnapshot, GithubValidationActivity, GithubValidationActivityKind,
        GithubValidationCheckRun, GithubValidationRunStatus, GithubValidationSource,
        GithubValidationSourceObservation, GithubValidationSourceStatus,
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
    };
    use crate::application::service::planning::{
        PlanningRuntimeProjection, PlanningServices, PlanningTaskCreateInput,
    };
    use crate::application::service::pr_validation_query::PrValidationQueryService;
    use crate::domain::github_review::{GithubCommitSha, GithubOpaqueId, GithubPullRequestTarget};
    use crate::domain::parallel_mode::{
        PrValidationCommitSha, PrValidationPhase, PrValidationRecord, PrValidationRecordKey,
        PrValidationTarget, PrValidationTargetShaSnapshot,
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

    impl GithubPrValidationPort for DeterministicGithub {
        fn load_validation_snapshot(
            &self,
            request: &GithubPrValidationObservationRequest,
        ) -> Result<GithubPrValidationSnapshot> {
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
        }
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
            .with_commit_sha(GithubCommitSha::new(HEAD_B)),
            GithubValidationActivity::new(
                GithubOpaqueId::new("comment:late-a"),
                GithubValidationActivityKind::ReviewComment,
                "2026-08-07T22:00:01Z",
            )
            .with_commit_sha(GithubCommitSha::new(HEAD_B)),
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

        let mut late = with_late_review(merged_snapshot(HEAD_B));
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
        let mut reordered_late = late.clone();
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
    fn runtime_tick_polls_durable_validation_with_monotonic_revisions_and_normal_remediation() {
        let repo = temp_repo("pr-validation-runtime-tick");
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

        assert_eq!(
            service
                .poll_pr_validations_for_runtime_tick(&repo.workspace_dir(), &planning.queue)
                .unwrap(),
            vec![PrValidationPollResult::Waiting]
        );
        assert!(
            !repo.pool_root().join(".leases").exists(),
            "waiting validation must not reserve a parallel slot"
        );
        assert!(matches!(
            service
                .poll_pr_validations_for_runtime_tick(&repo.workspace_dir(), &planning.queue)
                .unwrap()
                .as_slice(),
            [PrValidationPollResult::RemediationRequested { .. }]
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
