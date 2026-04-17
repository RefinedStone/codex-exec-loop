use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;

use super::super::shell_runtime;
use super::{
    AutoFollowupSubmitContext, ConversationRuntimeEvent, ConversationState, PromptOrigin,
    StartupState, TempGitWorkspace, commit_active_planning_workspace_into_akra, current_git_branch,
    git_branch_exists, make_test_app, ready_conversation, sample_startup_diagnostics,
};
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::planning::{
    BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT, PlanningTaskHandoff,
};
use crate::domain::parallel_mode::{ParallelModeReadinessSnapshot, ParallelModeReadinessState};

fn current_branch(workspace_directory: &str) -> String {
    let output = Command::new("git")
        .current_dir(workspace_directory)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git rev-parse should spawn");
    assert!(
        output.status.success(),
        "git rev-parse should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("branch name should be utf-8")
        .trim()
        .to_string()
}

fn wait_for_stream_call(check: impl Fn() -> bool) {
    for _ in 0..50 {
        if check() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }

    panic!("timed out waiting for fake stream call");
}

fn official_completion_task_ledger() -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "version": 1,
        "tasks": [{
            "id": "task-follow-up",
            "direction_id": "general-workstream",
            "direction_relation_note": "official completion refresh accepted the completed slice and queued the next runtime task",
            "title": "Continue distributor queue wiring",
            "description": "Take the next queued follow-up after the official completion refresh.",
            "status": "ready",
            "base_priority": 89,
            "dynamic_priority_delta": 0,
            "priority_reason": "official completion refresh left one executor-visible follow-up",
            "depends_on": [],
            "blocked_by": [],
            "created_by": "llm",
            "last_updated_by": "llm",
            "source_turn_id": "turn-2",
            "updated_at": "2026-04-17T09:40:00Z"
        }]
    }))
    .expect("task ledger should serialize")
}

#[test]
fn parallel_mode_handoff_launch_uses_leased_slot_workspace_and_new_thread_stream() {
    let repo = TempGitWorkspace::new("parallel-mode-runtime-slot");
    let (mut app, codex_port) = make_test_app();
    app.parallel_mode_enabled = true;
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(repo.workspace_dir(), true));
    let mut conversation = ready_conversation();
    conversation.cwd = repo.workspace_dir().to_string();
    conversation.draft_workspace_directory = repo.workspace_dir().to_string();
    app.conversation_state = ConversationState::ready(conversation);

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::SubmitPrompt {
        prompt: "continue queued task".to_string(),
        transcript_text: BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT.to_string(),
        origin: PromptOrigin::AutoFollow(Box::new(AutoFollowupSubmitContext {
            queued_from_turn_id: "turn-1".to_string(),
            mode_label: "planning queue".to_string(),
            transcript_text: BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT.to_string(),
            debug_detail: None,
            handoff_task: Some(PlanningTaskHandoff {
                task_id: "task-supersession-runtime".to_string(),
                task_title: "Wire runtime into slot lease lifecycle".to_string(),
                direction_id: "supersession-git-worktree-pool".to_string(),
                combined_priority: 96,
                updated_at: "2026-04-17T05:20:00Z".to_string(),
                status_label: "ready".to_string(),
            }),
        })),
    });

    let leased_workspace = match &app.conversation_state {
        ConversationState::Ready(conversation) => conversation
            .active_turn_workspace_directory
            .clone()
            .expect("active turn workspace should be recorded"),
        ConversationState::Loading | ConversationState::Failed(_) => {
            panic!("conversation should stay ready during launch setup")
        }
    };
    assert_ne!(leased_workspace, repo.workspace_dir());
    assert!(Path::new(&leased_workspace).exists());
    assert!(current_branch(&leased_workspace).starts_with("akra-agent/slot-1/"));
    assert_eq!(
        app.active_turn_planning_capture
            .as_ref()
            .map(|capture| capture.workspace_directory.as_str()),
        Some(leased_workspace.as_str())
    );

    wait_for_stream_call(|| {
        !codex_port
            .new_thread_calls
            .lock()
            .expect("new-thread calls mutex poisoned")
            .is_empty()
    });

    assert!(
        codex_port
            .turn_calls
            .lock()
            .expect("turn calls mutex poisoned")
            .is_empty()
    );
    let new_thread_calls = codex_port
        .new_thread_calls
        .lock()
        .expect("new-thread calls mutex poisoned")
        .clone();
    assert_eq!(new_thread_calls.len(), 1);
    assert_eq!(new_thread_calls[0].0, leased_workspace);
    assert_eq!(new_thread_calls[0].1, "continue queued task");
}

#[test]
fn leased_slot_success_completion_waits_for_official_refresh_before_cleanup() {
    let repo = TempGitWorkspace::new("parallel-mode-runtime-cleanup");
    commit_active_planning_workspace_into_akra(repo.workspace_dir());
    let (mut app, codex_port) = make_test_app();
    {
        let mut behavior = codex_port
            .new_thread_stream_behavior
            .lock()
            .expect("new-thread stream behavior mutex poisoned");
        behavior.events = vec![
            ConversationStreamEvent::AgentMessageCompleted {
                item_id: "item-1".to_string(),
                phase: None,
                text: "Runtime slot wiring is complete.".to_string(),
            },
            ConversationStreamEvent::TurnStarted {
                turn_id: "turn-2".to_string(),
            },
            ConversationStreamEvent::TurnCompleted {
                turn_id: "turn-2".to_string(),
                changed_planning_file_paths: Vec::new(),
            },
        ];
        behavior.merge_active_branch_into_akra_repo = Some(repo.workspace_dir().to_string());
    }
    {
        let mut behavior = codex_port
            .hidden_planning_stream_behavior
            .lock()
            .expect("hidden planning stream behavior mutex poisoned");
        behavior.events = vec![
            ConversationStreamEvent::AgentMessageCompleted {
                item_id: "item-2".to_string(),
                phase: None,
                text: "official refresh accepted the completion".to_string(),
            },
            ConversationStreamEvent::TurnCompleted {
                turn_id: "planner-turn-1".to_string(),
                changed_planning_file_paths: vec![
                    ".codex-exec-loop/planning/task-ledger.json".to_string(),
                ],
            },
        ];
        behavior.planning_file_writes = vec![(
            ".codex-exec-loop/planning/task-ledger.json".to_string(),
            official_completion_task_ledger(),
        )];
    }
    app.parallel_mode_enabled = true;
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(repo.workspace_dir(), true));
    let mut conversation = ready_conversation();
    conversation.cwd = repo.workspace_dir().to_string();
    conversation.draft_workspace_directory = repo.workspace_dir().to_string();
    app.conversation_state = ConversationState::ready(conversation);
    app.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(repo.workspace_dir());
    let mut runtime = shell_runtime::ShellRuntime::new(app);

    runtime
        .app_mut()
        .dispatch_conversation_runtime(ConversationRuntimeEvent::SubmitPrompt {
            prompt: "continue queued task".to_string(),
            transcript_text: BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT.to_string(),
            origin: PromptOrigin::AutoFollow(Box::new(AutoFollowupSubmitContext {
                queued_from_turn_id: "turn-1".to_string(),
                mode_label: "planning queue".to_string(),
                transcript_text: BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT.to_string(),
                debug_detail: None,
                handoff_task: Some(PlanningTaskHandoff {
                    task_id: "task-supersession-runtime".to_string(),
                    task_title: "Wire runtime into slot lease lifecycle".to_string(),
                    direction_id: "supersession-git-worktree-pool".to_string(),
                    combined_priority: 96,
                    updated_at: "2026-04-17T05:20:00Z".to_string(),
                    status_label: "ready".to_string(),
                }),
            })),
        });

    let leased_workspace = match &runtime.app().conversation_state {
        ConversationState::Ready(conversation) => conversation
            .active_turn_workspace_directory
            .clone()
            .expect("active turn workspace should be recorded"),
        ConversationState::Loading | ConversationState::Failed(_) => {
            panic!("conversation should stay ready during launch setup")
        }
    };
    let lease_path = Path::new(&leased_workspace)
        .parent()
        .expect("slot workspace should have a pool root")
        .join(".leases")
        .join("slot-1.json");
    let leased_branch = current_git_branch(&leased_workspace);

    for _ in 0..50 {
        runtime.poll_background_messages();
        if !lease_path.exists() {
            assert_eq!(current_git_branch(&leased_workspace), "HEAD");
            assert!(!git_branch_exists(repo.workspace_dir(), &leased_branch));
            assert_eq!(
                codex_port
                    .hidden_planning_calls
                    .lock()
                    .expect("hidden planning calls mutex poisoned")
                    .len(),
                1
            );
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }

    panic!("timed out waiting for the slot to return after official refresh");
}

#[test]
fn parallel_mode_runtime_invalidates_cached_supervisor_roster_when_slot_starts_running() {
    let repo = TempGitWorkspace::new("parallel-mode-runtime-roster");
    let (mut app, codex_port) = make_test_app();
    {
        let mut behavior = codex_port
            .new_thread_stream_behavior
            .lock()
            .expect("new-thread stream behavior mutex poisoned");
        behavior.events = vec![ConversationStreamEvent::TurnStarted {
            turn_id: "turn-2".to_string(),
        }];
    }
    app.parallel_mode_enabled = true;
    app.parallel_mode_readiness_snapshot = Some(ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    ));
    let cached_snapshot = app.parallel_mode_service().build_supervisor_snapshot(
        repo.workspace_dir(),
        true,
        app.parallel_mode_readiness_snapshot(),
    );
    app.parallel_mode_supervisor_snapshot = Some(cached_snapshot);
    assert_eq!(
        app.parallel_mode_supervisor_snapshot
            .as_ref()
            .expect("cached supervisor snapshot should exist")
            .roster
            .active_count(),
        0
    );
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(repo.workspace_dir(), true));
    let mut conversation = ready_conversation();
    conversation.cwd = repo.workspace_dir().to_string();
    conversation.draft_workspace_directory = repo.workspace_dir().to_string();
    app.conversation_state = ConversationState::ready(conversation);
    app.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(repo.workspace_dir());
    let mut runtime = shell_runtime::ShellRuntime::new(app);

    runtime
        .app_mut()
        .dispatch_conversation_runtime(ConversationRuntimeEvent::SubmitPrompt {
            prompt: "continue queued task".to_string(),
            transcript_text: BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT.to_string(),
            origin: PromptOrigin::AutoFollow(Box::new(AutoFollowupSubmitContext {
                queued_from_turn_id: "turn-1".to_string(),
                mode_label: "planning queue".to_string(),
                transcript_text: BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT.to_string(),
                debug_detail: None,
                handoff_task: Some(PlanningTaskHandoff {
                    task_id: "task-supersession-runtime".to_string(),
                    task_title: "Wire runtime into slot lease lifecycle".to_string(),
                    direction_id: "supersession-git-worktree-pool".to_string(),
                    combined_priority: 96,
                    updated_at: "2026-04-17T05:20:00Z".to_string(),
                    status_label: "ready".to_string(),
                }),
            })),
        });

    wait_for_stream_call(|| {
        !codex_port
            .new_thread_calls
            .lock()
            .expect("new-thread calls mutex poisoned")
            .is_empty()
    });

    for _ in 0..50 {
        runtime.poll_background_messages();
        let snapshot = runtime.app().parallel_mode_supervisor_snapshot();
        if snapshot.roster.active_count() == 1
            && snapshot.roster.entries[0].state_label == "running"
        {
            assert_eq!(
                snapshot.roster.entries[0].agent_id,
                "agent-task-supersession-runtime"
            );
            assert_eq!(snapshot.roster.entries[0].slot_id, "slot-1");
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }

    panic!("timed out waiting for the supervisor roster to refresh");
}

#[test]
fn parallel_mode_runtime_keeps_cleaned_session_detail_after_slot_return() {
    let repo = TempGitWorkspace::new("parallel-mode-runtime-detail-history");
    commit_active_planning_workspace_into_akra(repo.workspace_dir());
    let (mut app, codex_port) = make_test_app();
    {
        let mut behavior = codex_port
            .new_thread_stream_behavior
            .lock()
            .expect("new-thread stream behavior mutex poisoned");
        behavior.events = vec![
            ConversationStreamEvent::ThreadPrepared {
                thread_id: "thread-9".to_string(),
                title: "Queued task".to_string(),
                cwd: repo.workspace_dir().to_string(),
            },
            ConversationStreamEvent::TurnStarted {
                turn_id: "turn-2".to_string(),
            },
            ConversationStreamEvent::AgentMessageCompleted {
                item_id: "item-1".to_string(),
                phase: None,
                text: "Work finished and ready for official refresh.".to_string(),
            },
            ConversationStreamEvent::TurnCompleted {
                turn_id: "turn-2".to_string(),
                changed_planning_file_paths: Vec::new(),
            },
        ];
        behavior.merge_active_branch_into_akra_repo = Some(repo.workspace_dir().to_string());
    }
    {
        let mut behavior = codex_port
            .hidden_planning_stream_behavior
            .lock()
            .expect("hidden planning stream behavior mutex poisoned");
        behavior.events = vec![
            ConversationStreamEvent::AgentMessageCompleted {
                item_id: "item-2".to_string(),
                phase: None,
                text: "official refresh accepted the completion".to_string(),
            },
            ConversationStreamEvent::TurnCompleted {
                turn_id: "planner-turn-9".to_string(),
                changed_planning_file_paths: vec![
                    ".codex-exec-loop/planning/task-ledger.json".to_string(),
                ],
            },
        ];
        behavior.planning_file_writes = vec![(
            ".codex-exec-loop/planning/task-ledger.json".to_string(),
            official_completion_task_ledger(),
        )];
    }
    app.parallel_mode_enabled = true;
    app.parallel_mode_readiness_snapshot = Some(ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    ));
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(repo.workspace_dir(), true));
    let mut conversation = ready_conversation();
    conversation.cwd = repo.workspace_dir().to_string();
    conversation.draft_workspace_directory = repo.workspace_dir().to_string();
    app.conversation_state = ConversationState::ready(conversation);
    app.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(repo.workspace_dir());
    let mut runtime = shell_runtime::ShellRuntime::new(app);

    runtime
        .app_mut()
        .dispatch_conversation_runtime(ConversationRuntimeEvent::SubmitPrompt {
            prompt: "continue queued task".to_string(),
            transcript_text: BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT.to_string(),
            origin: PromptOrigin::AutoFollow(Box::new(AutoFollowupSubmitContext {
                queued_from_turn_id: "turn-1".to_string(),
                mode_label: "planning queue".to_string(),
                transcript_text: BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT.to_string(),
                debug_detail: None,
                handoff_task: Some(PlanningTaskHandoff {
                    task_id: "task-supersession-runtime".to_string(),
                    task_title: "Wire runtime into slot lease lifecycle".to_string(),
                    direction_id: "supersession-git-worktree-pool".to_string(),
                    combined_priority: 96,
                    updated_at: "2026-04-17T05:20:00Z".to_string(),
                    status_label: "ready".to_string(),
                }),
            })),
        });

    wait_for_stream_call(|| {
        !codex_port
            .new_thread_calls
            .lock()
            .expect("new-thread calls mutex poisoned")
            .is_empty()
    });

    for _ in 0..50 {
        runtime.poll_background_messages();
        let snapshot = runtime.app().parallel_mode_supervisor_snapshot();
        if snapshot.roster.active_count() == 0
            && snapshot.detail.session.as_ref().is_some_and(|detail| {
                detail.state_label == "cleaned" && detail.thread_id.as_deref() == Some("thread-9")
            })
        {
            let detail = snapshot
                .detail
                .session
                .as_ref()
                .expect("detail should exist once the session is cleaned");
            assert_eq!(detail.completion_state_label, "cleaned");
            assert_eq!(snapshot.distributor.head_summary, "idle");
            assert_eq!(
                detail
                    .history
                    .iter()
                    .map(|entry| entry.state_label.as_str())
                    .collect::<Vec<_>>(),
                vec![
                    "assigned",
                    "starting",
                    "running",
                    "reported_complete",
                    "ledger_refreshing",
                    "commit_ready",
                    "merge_queued",
                    "integrating",
                    "merged",
                    "cleanup_pending",
                    "cleaned"
                ]
            );
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }

    panic!("timed out waiting for cleaned session detail to refresh");
}

#[test]
fn leased_slot_official_refresh_failure_keeps_slot_reserved_for_operator_recovery() {
    let repo = TempGitWorkspace::new("parallel-mode-runtime-official-failure");
    commit_active_planning_workspace_into_akra(repo.workspace_dir());
    let (mut app, codex_port) = make_test_app();
    {
        let mut behavior = codex_port
            .new_thread_stream_behavior
            .lock()
            .expect("new-thread stream behavior mutex poisoned");
        behavior.events = vec![
            ConversationStreamEvent::ThreadPrepared {
                thread_id: "thread-11".to_string(),
                title: "Queued task".to_string(),
                cwd: repo.workspace_dir().to_string(),
            },
            ConversationStreamEvent::TurnStarted {
                turn_id: "turn-2".to_string(),
            },
            ConversationStreamEvent::AgentMessageCompleted {
                item_id: "item-1".to_string(),
                phase: None,
                text: "Implementation is complete.".to_string(),
            },
            ConversationStreamEvent::TurnCompleted {
                turn_id: "turn-2".to_string(),
                changed_planning_file_paths: Vec::new(),
            },
        ];
        behavior.merge_active_branch_into_akra_repo = Some(repo.workspace_dir().to_string());
    }
    {
        let mut behavior = codex_port
            .hidden_planning_stream_behavior
            .lock()
            .expect("hidden planning stream behavior mutex poisoned");
        behavior.error = Some("planner refresh crashed".to_string());
    }
    app.parallel_mode_enabled = true;
    app.parallel_mode_readiness_snapshot = Some(ParallelModeReadinessSnapshot::new(
        repo.workspace_dir(),
        ParallelModeReadinessState::Ready,
        vec![],
        None,
    ));
    app.startup_state = StartupState::Ready(sample_startup_diagnostics(repo.workspace_dir(), true));
    let mut conversation = ready_conversation();
    conversation.cwd = repo.workspace_dir().to_string();
    conversation.draft_workspace_directory = repo.workspace_dir().to_string();
    app.conversation_state = ConversationState::ready(conversation);
    app.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(repo.workspace_dir());
    let mut runtime = shell_runtime::ShellRuntime::new(app);

    runtime
        .app_mut()
        .dispatch_conversation_runtime(ConversationRuntimeEvent::SubmitPrompt {
            prompt: "continue queued task".to_string(),
            transcript_text: BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT.to_string(),
            origin: PromptOrigin::AutoFollow(Box::new(AutoFollowupSubmitContext {
                queued_from_turn_id: "turn-1".to_string(),
                mode_label: "planning queue".to_string(),
                transcript_text: BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT.to_string(),
                debug_detail: None,
                handoff_task: Some(PlanningTaskHandoff {
                    task_id: "task-supersession-runtime".to_string(),
                    task_title: "Wire runtime into slot lease lifecycle".to_string(),
                    direction_id: "supersession-git-worktree-pool".to_string(),
                    combined_priority: 96,
                    updated_at: "2026-04-17T05:20:00Z".to_string(),
                    status_label: "ready".to_string(),
                }),
            })),
        });

    let leased_workspace = match &runtime.app().conversation_state {
        ConversationState::Ready(conversation) => conversation
            .active_turn_workspace_directory
            .clone()
            .expect("active turn workspace should be recorded"),
        ConversationState::Loading | ConversationState::Failed(_) => {
            panic!("conversation should stay ready during launch setup")
        }
    };
    let lease_path = Path::new(&leased_workspace)
        .parent()
        .expect("slot workspace should have a pool root")
        .join(".leases")
        .join("slot-1.json");
    let leased_branch = current_git_branch(&leased_workspace);

    for _ in 0..50 {
        runtime.poll_background_messages();
        let snapshot = runtime.app().parallel_mode_supervisor_snapshot();
        if snapshot.roster.active_count() == 1
            && snapshot.roster.entries[0].state_label == "failed"
            && snapshot.detail.session.as_ref().is_some_and(|detail| {
                detail.state_label == "failed"
                    && detail
                        .history
                        .iter()
                        .map(|entry| entry.state_label.as_str())
                        .collect::<Vec<_>>()
                        == vec![
                            "assigned",
                            "starting",
                            "running",
                            "reported_complete",
                            "ledger_refreshing",
                            "failed",
                        ]
            })
        {
            assert!(lease_path.exists());
            assert_eq!(current_git_branch(&leased_workspace), leased_branch);
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }

    panic!("timed out waiting for failed official completion state");
}
