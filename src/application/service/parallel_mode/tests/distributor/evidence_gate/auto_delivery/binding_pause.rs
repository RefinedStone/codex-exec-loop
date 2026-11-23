use super::*;
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::domain::issue_automation::binding_pause::{
    IssueBindingPauseIntent, IssueBindingPauseState,
};

fn start_pause(
    workspace: &str,
    store: &crate::adapter::outbound::db::SqliteIssueAutomationStoreAdapter,
    admission: &crate::application::service::issue_binding_admission::IssueBindingAdmission,
) -> IssueBindingPauseState {
    let binding = store
        .resolve_issue_work_binding(workspace, &work_id(workspace))
        .unwrap()
        .unwrap();
    let before = store
        .issue_binding_pause_state(workspace, &binding)
        .unwrap();
    admission.register_if_absent(&before).unwrap();
    let intent = IssueBindingPauseIntent {
        command_id: "pause-delivery-at-effect".into(),
        binding_id: binding.clone(),
        paused: true,
        expected_revision: before.revision,
        resume_basis: None,
    };
    admission
        .begin_transition(&binding, || {
            store.begin_issue_binding_pause(workspace, &intent)?;
            Ok(((), store.issue_binding_pause_state(workspace, &binding)?))
        })
        .unwrap();
    let pending = store
        .issue_binding_pause_state(workspace, &binding)
        .unwrap();
    assert!(pending.pending.is_some());
    pending
}

fn assert_cannot_finish_while_action_is_inflight(
    admission: &crate::application::service::issue_binding_admission::IssueBindingAdmission,
    state: &IssueBindingPauseState,
) {
    assert!(admission.has_inflight());
    assert!(
        admission
            .finish_if_drained(
                &state.binding_id,
                state.revision,
                || -> anyhow::Result<((), IssueBindingPauseState)> {
                    panic!("an admitted action still owns the response/unknown boundary")
                }
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn pause_at_each_final_delivery_gate_blocks_that_effect_and_preserves_prior_refs() {
    for stage in [
        TaskEvidenceDeliveryStage::Push,
        TaskEvidenceDeliveryStage::PullRequest,
        TaskEvidenceDeliveryStage::Integrate,
    ] {
        let fixture = AutoFixture::new(
            "pause-final-effect",
            TaskEvidenceDeliveryMode::Enforce,
            true,
            FakeGithubAutomationPort::ready(),
        );
        fixture.prepare();
        let workspace = fixture.inner.repo.workspace_dir();
        let store = fixture.inner.issue_store.clone();
        let admission = fixture.inner.admission.clone();
        *fixture.inner.gate.after.lock().unwrap() = Some((
            stage,
            Box::new(move |_| {
                let state = start_pause(&workspace, store.as_ref(), admission.as_ref());
                assert_cannot_finish_while_action_is_inflight(admission.as_ref(), &state);
            }),
        ));

        let _ = fixture
            .inner
            .service
            .process_distributor_queue(&fixture.inner.repo.workspace_dir());

        assert!(!fixture.inner.admission.has_inflight());
        let operations = fixture.inner.operations();
        let count = |prefix: &str| {
            operations
                .iter()
                .filter(|operation| operation.starts_with(prefix))
                .count()
        };
        assert_eq!(
            count("push:"),
            usize::from(stage != TaskEvidenceDeliveryStage::Push),
            "{operations:?}"
        );
        assert_eq!(
            count("ensure-pr:"),
            usize::from(stage == TaskEvidenceDeliveryStage::Integrate),
            "{operations:?}"
        );
        fixture.assert_no_integration();
        if stage == TaskEvidenceDeliveryStage::Push {
            assert!(
                fixture
                    .inner
                    .remote_ref(&fixture.inner.lease.branch_name)
                    .is_none()
            );
        } else {
            assert_eq!(
                fixture.inner.remote_ref(&fixture.inner.lease.branch_name),
                Some(fixture.inner.queue().source_commit_sha)
            );
        }
        let before_retry = operations;
        fixture
            .inner
            .service
            .process_distributor_queue(&fixture.inner.repo.workspace_dir())
            .unwrap();
        assert_eq!(
            fixture.inner.operations(),
            before_retry,
            "paused queue head must not retry an external effect"
        );
    }
}

#[test]
fn source_push_response_is_drained_and_pause_prevents_the_following_pr() {
    let fixture = AutoFixture::new(
        "pause-published-source",
        TaskEvidenceDeliveryMode::Enforce,
        true,
        FakeGithubAutomationPort::ready(),
    );
    fixture.prepare();
    let workspace = fixture.inner.repo.workspace_dir();
    let store = fixture.inner.issue_store.clone();
    let admission = fixture.inner.admission.clone();
    fixture
        .inner
        .github
        .set_boundary_hook(FakeGithubAutomationBoundary::SourcePushed, move || {
            let pending = start_pause(&workspace, store.as_ref(), admission.as_ref());
            assert_cannot_finish_while_action_is_inflight(admission.as_ref(), &pending);
        });
    let _ = fixture
        .inner
        .service
        .process_distributor_queue(&fixture.inner.repo.workspace_dir());
    assert!(!fixture.inner.admission.has_inflight());
    assert_eq!(
        fixture.inner.remote_ref(&fixture.inner.lease.branch_name),
        Some(fixture.inner.queue().source_commit_sha)
    );
    assert!(
        fixture
            .inner
            .operations()
            .iter()
            .all(|operation| !operation.starts_with("ensure-pr:"))
    );
    fixture.assert_no_integration();
}

#[test]
fn pause_after_integration_preserves_the_pr_or_branch_before_a_new_cleanup_write() {
    for boundary in [
        FakeGithubAutomationBoundary::PullRequestCloseReadiness,
        FakeGithubAutomationBoundary::PullRequestClosed,
    ] {
        let fixture = AutoFixture::new(
            "pause-cleanup-write",
            TaskEvidenceDeliveryMode::Enforce,
            true,
            FakeGithubAutomationPort::ready(),
        );
        fixture.prepare();
        let workspace = fixture.inner.repo.workspace_dir();
        let store = fixture.inner.issue_store.clone();
        let admission = fixture.inner.admission.clone();
        fixture.inner.github.set_boundary_hook(boundary, move || {
            let pending = start_pause(&workspace, store.as_ref(), admission.as_ref());
            assert_cannot_finish_while_action_is_inflight(admission.as_ref(), &pending);
        });
        let _ = fixture
            .inner
            .service
            .process_distributor_queue(&fixture.inner.repo.workspace_dir());
        assert!(!fixture.inner.admission.has_inflight());
        fixture.assert_integrated();
        let operations = fixture.inner.operations();
        assert_eq!(
            operations
                .iter()
                .filter(|operation| operation.starts_with("close-pr:"))
                .count(),
            usize::from(boundary == FakeGithubAutomationBoundary::PullRequestClosed),
            "{operations:?}"
        );
        assert_eq!(
            *fixture
                .inner
                .github
                .source_branch_cleanup_calls
                .lock()
                .unwrap(),
            0
        );
        assert!(
            fixture
                .inner
                .remote_ref(&fixture.inner.lease.branch_name)
                .is_some()
        );
    }
}

#[test]
fn native_completion_uses_its_launch_capture_and_never_adopts_the_paused_generation() {
    use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
    let fixture = AutoFixture::new(
        "pause-native-completion",
        TaskEvidenceDeliveryMode::Enforce,
        true,
        FakeGithubAutomationPort::ready(),
    );
    let turn = ParallelModeTurnService::new(fixture.inner.service.clone());
    let capture = turn
        .capture_issue_completion_action(&fixture.inner.lease)
        .unwrap()
        .unwrap();
    assert_ne!(
        fixture.inner.lease.worktree_path,
        fixture.inner.repo.workspace_dir()
    );
    assert_eq!(
        capture.work_order_id,
        work_id(&fixture.inner.repo.workspace_dir())
    );
    assert_eq!(
        fixture.inner.issue_store.resolve_issue_work_binding(
            &fixture.inner.repo.workspace_dir(), &capture.work_order_id,
        ).unwrap().as_ref(),
        Some(&capture.binding_id),
        "the launch capture must retain the primary workspace's full issue identity",
    );
    let captured = turn
        .for_completion_capture(
            &fixture.inner.repo.workspace_dir(),
            &fixture.inner.lease,
            Some(&capture),
        )
        .unwrap();
    let guard = captured
        .acquire_completion_action(&fixture.inner.repo.workspace_dir(), &fixture.inner.lease)
        .unwrap();
    assert!(guard.validate().is_ok());
    assert!(
        turn.acquire_completion_action(&fixture.inner.repo.workspace_dir(), &fixture.inner.lease)
            .is_err(),
        "a callback missing the original identity must fail closed"
    );
    let pending = start_pause(
        &fixture.inner.repo.workspace_dir(),
        fixture.inner.issue_store.as_ref(),
        fixture.inner.admission.as_ref(),
    );
    assert_cannot_finish_while_action_is_inflight(fixture.inner.admission.as_ref(), &pending);
    assert!(guard.validate().is_err());
    drop(guard);
    assert!(!fixture.inner.admission.has_inflight());
    assert!(
        turn.for_completion_capture(
            &fixture.inner.repo.workspace_dir(),
            &fixture.inner.lease,
            Some(&capture)
        )
        .is_err()
    );
    assert!(fixture.inner.operations().is_empty());
}

#[cfg(windows)]
#[test]
fn windows_git_path_aliases_preserve_binding_lineage_and_pause_the_same_delivery() {
    use crate::application::service::parallel_mode::binding_action;

    let fixture = AutoFixture::new(
        "pause-windows-path-alias",
        TaskEvidenceDeliveryMode::Enforce,
        true,
        FakeGithubAutomationPort::ready(),
    );
    fixture.prepare();
    let workspace = fixture.inner.repo.workspace_dir();
    let git_workspace =
        crate::trusted_executable::windows_process_argument_path(Path::new(&workspace))
            .unwrap()
            .to_str()
            .unwrap()
            .replace('\\', "/");
    assert_ne!(git_workspace, workspace);
    assert_eq!(
        fs::canonicalize(&git_workspace).unwrap(),
        Path::new(&workspace)
    );
    let configuration = fixture.inner.service.issue_binding.as_ref();
    let action = binding_action::capture(
        fixture.inner.authority.as_ref(),
        configuration,
        &git_workspace,
        TASK,
    )
    .unwrap();
    let capture = action.issue_capture().unwrap();
    assert_eq!(capture.work_order_id, work_id(&workspace));
    assert_eq!(
        fixture
            .inner
            .issue_store
            .resolve_issue_work_binding(&workspace, &capture.work_order_id,)
            .unwrap()
            .as_ref(),
        Some(&capture.binding_id),
    );
    // Both an ordinary Git spelling and a linked worktree must resolve the same
    // primary scope without weakening the captured work epoch or binding generation.
    binding_action::validate(
        fixture.inner.authority.as_ref(),
        configuration,
        &git_workspace,
        TASK,
        &action,
    )
    .unwrap();
    binding_action::restore_issue_capture(
        fixture.inner.authority.as_ref(),
        configuration,
        &fixture.inner.lease.worktree_path,
        TASK,
        &capture,
    )
    .unwrap();
    assert!(
        !binding_action::paused(
            fixture.inner.authority.as_ref(),
            configuration,
            &git_workspace,
            TASK,
        )
        .unwrap()
    );

    let reached_gate = Arc::new(AtomicBool::new(false));
    let reached = reached_gate.clone();
    let store = fixture.inner.issue_store.clone();
    let admission = fixture.inner.admission.clone();
    let canonical_workspace = workspace.clone();
    *fixture.inner.gate.after.lock().unwrap() = Some((
        TaskEvidenceDeliveryStage::Push,
        Box::new(move |_| {
            reached.store(true, Ordering::SeqCst);
            let pending = start_pause(&canonical_workspace, store.as_ref(), admission.as_ref());
            assert_cannot_finish_while_action_is_inflight(admission.as_ref(), &pending);
        }),
    ));
    assert!(
        fixture
            .inner
            .service
            .process_distributor_queue(&git_workspace)
            .is_err()
    );
    assert!(
        reached_gate.load(Ordering::SeqCst),
        "the exact delivery gate must be reached"
    );
    assert!(!fixture.inner.admission.has_inflight());
    fixture.inner.assert_no_source_or_pr();
    assert!(
        fixture
            .inner
            .remote_ref(&fixture.inner.lease.branch_name)
            .is_none()
    );
    assert!(
        binding_action::paused(
            fixture.inner.authority.as_ref(),
            configuration,
            &git_workspace,
            TASK,
        )
        .unwrap()
    );
    assert!(
        binding_action::restore_issue_capture(
            fixture.inner.authority.as_ref(),
            configuration,
            &git_workspace,
            TASK,
            &capture,
        )
        .is_err()
    );
    let operations = fixture.inner.operations();
    fixture
        .inner
        .service
        .process_distributor_queue(&git_workspace)
        .unwrap();
    assert_eq!(fixture.inner.operations(), operations);
}

#[test]
fn a_paused_issue_queue_head_does_not_stop_an_unrelated_ordinary_task() {
    let fixture = AutoFixture::new(
        "pause-queue-isolation",
        TaskEvidenceDeliveryMode::Observe,
        true,
        FakeGithubAutomationPort::ready(),
    );
    fixture.inner.enqueue();
    let workspace = fixture.inner.repo.workspace_dir();
    let paused_row = fixture.inner.queue();
    start_pause(
        &workspace,
        fixture.inner.issue_store.as_ref(),
        fixture.inner.admission.as_ref(),
    );
    let mut snapshot = fixture
        .inner
        .authority
        .load_task_authority_snapshot(&workspace)
        .unwrap()
        .unwrap();
    let mut task = snapshot.task_authority.tasks[0].clone();
    task.id = "ordinary-task".into();
    task.title = "Unrelated ordinary work".into();
    task.acceptance.clear();
    task.acceptance_revision = 0;
    task.created_by = TaskActor::System;
    task.last_updated_by = TaskActor::System;
    task.provenance = TaskMutationProvenance::new(OriginSessionKind::System);
    snapshot.task_authority.tasks.push(task);
    let projection = PriorityQueueProjection {
        next_task: None,
        active_tasks: vec![],
        proposed_tasks: vec![],
        skipped_tasks: vec![],
    };
    fixture
        .inner
        .authority
        .commit_task_authority_snapshot(
            &workspace,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: Some(snapshot.planning_revision),
                task_authority: &snapshot.task_authority,
                queue_projection: &projection,
            },
        )
        .unwrap();
    let service = fixture.inner.service.clone().with_delivery_evidence(Arc::new(
        crate::application::port::inbound::task_delivery_evidence_port::ObserveTaskDeliveryEvidenceGate,
    ));
    let ordinary = service
        .acquire_slot_lease(
            &workspace,
            sample_lease_request("ordinary-task", "Ordinary", "agent-ordinary", "ordinary"),
        )
        .unwrap();
    service
        .mark_workspace_slot_running(&ordinary.worktree_path)
        .unwrap();
    fixture.inner.repo.commit_file_in_slot(
        Path::new(&ordinary.worktree_path),
        "ordinary.txt",
        "ordinary result\n",
        "unrelated ordinary task",
    );
    service
        .begin_workspace_official_completion(
            &ordinary.worktree_path,
            "ordinary-completion",
            None,
            Some("Ordinary result"),
            Some("Explicit ordinary fixture"),
            None,
        )
        .unwrap();
    service
        .mark_workspace_official_completion_refreshing(&ordinary.worktree_path)
        .unwrap();
    service
        .mark_workspace_commit_ready(&ordinary.worktree_path, "ordinary ready")
        .unwrap();
    service
        .enqueue_workspace_commit_ready_result(&ordinary.worktree_path)
        .unwrap()
        .unwrap();

    service.process_distributor_queue(&workspace).unwrap();

    let operations = fixture.inner.operations();
    assert!(
        operations
            .iter()
            .any(|operation| operation == &format!("push:{}:false", ordinary.branch_name)),
        "{operations:?}"
    );
    assert!(
        operations.iter().all(
            |operation| operation != &format!("push:{}:false", fixture.inner.lease.branch_name)
        ),
        "{operations:?}"
    );
    assert_eq!(
        fixture.inner.queue(),
        paused_row,
        "skip must preserve the paused row instead of rearming or overwriting it"
    );
}
