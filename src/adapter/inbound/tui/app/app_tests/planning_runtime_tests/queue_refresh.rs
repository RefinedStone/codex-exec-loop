use std::thread;
use std::time::Duration;

use super::super::{
    ConversationInputState, ConversationMessage, ConversationMessageKind, ConversationRuntimeEvent,
    ConversationState, ConversationStreamEvent, PlanningTaskHandoff, StartupState,
    TASK_LEDGER_FILE_PATH, bootstrap_active_planning_workspace, create_temp_workspace,
    enable_queue_idle_review_and_enqueue, make_test_app, ready_conversation,
    sample_startup_diagnostics,
};

#[test]
fn proposed_only_refresh_promotes_top_proposal_and_queues_auto_followup() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let workspace_dir = create_temp_workspace("planning-proposal-followup-app");
    bootstrap_active_planning_workspace(&workspace_dir);
    enable_queue_idle_review_and_enqueue(&workspace_dir);
    let planning_dir = std::path::Path::new(&workspace_dir)
        .join(".codex-exec-loop")
        .join("planning");
    std::fs::write(
        planning_dir.join("task-ledger.json"),
        r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-proposal-1",
      "direction_id": "general-workstream",
      "direction_relation_note": "Follow-up option offered in the latest answer.",
      "title": "Draft a Korea-specific Chinese-chef job entry guide",
      "description": "Expand the answer into a Korea-specific hiring guide.",
      "status": "proposed",
      "base_priority": 70,
      "dynamic_priority_delta": 0,
      "priority_reason": "First follow-up branch from the latest answer.",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "llm",
      "last_updated_by": "llm",
      "source_turn_id": null,
      "updated_at": "2026-04-13T00:00:00Z"
    },
    {
      "id": "task-proposal-2",
      "direction_id": "general-workstream",
      "direction_relation_note": "Alternate follow-up option offered in the latest answer.",
      "title": "Create a beginner 3-month Chinese-cooking training plan",
      "description": "Turn the answer into a 3-month training plan.",
      "status": "proposed",
      "base_priority": 65,
      "dynamic_priority_delta": 0,
      "priority_reason": "Second follow-up branch from the latest answer.",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "llm",
      "last_updated_by": "llm",
      "source_turn_id": null,
      "updated_at": "2026-04-13T00:00:00Z"
    }
  ]
}"#,
    )
    .expect("task ledger should write");

    codex_port
        .new_thread_stream_behavior
        .lock()
        .expect("new-thread stream behavior mutex poisoned")
        .events = vec![
        ConversationStreamEvent::ThreadPrepared {
            thread_id: "planner-thread-1".to_string(),
            title: "Planner".to_string(),
            cwd: workspace_dir.clone(),
        },
        ConversationStreamEvent::AgentMessageCompleted {
            item_id: "planner-item-1".to_string(),
            phase: None,
            text: "planner refreshed the queue".to_string(),
        },
        ConversationStreamEvent::TurnCompleted {
            turn_id: "planner-turn-1".to_string(),
            changed_planning_file_paths: vec![TASK_LEDGER_FILE_PATH.to_string()],
        },
    ];

    let mut conversation = ready_conversation();
    conversation.auto_follow_state.template_state.selected_index = 0;
    conversation.cwd = workspace_dir.clone();
    conversation.draft_workspace_directory = workspace_dir.clone();
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-main".to_string());
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    conversation.replace_planning_runtime_snapshot(
        app.planning
            .runtime
            .load_runtime_snapshot_or_invalid(&workspace_dir),
    );
    app.conversation_state = ConversationState::ready(conversation);

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
        ConversationStreamEvent::TurnCompleted {
            turn_id: "turn-main".to_string(),
            changed_planning_file_paths: Vec::new(),
        },
    ));

    let mut turn_calls = Vec::new();
    for _ in 0..20 {
        turn_calls = codex_port
            .turn_calls
            .lock()
            .expect("turn call mutex poisoned")
            .iter()
            .map(|(_, prompt)| prompt.clone())
            .collect::<Vec<_>>();
        if !turn_calls.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };

    assert_eq!(turn_calls.len(), 1);
    assert!(
        turn_calls[0].contains("Draft a Korea-specific Chinese-chef job entry guide"),
        "auto follow-up prompt should target the promoted proposal: {}",
        turn_calls[0]
    );
    assert_eq!(
        conversation
            .planning_runtime_snapshot
            .queue_head()
            .map(|task| task.task_id.as_str()),
        Some("task-proposal-1"),
        "status={}, notices={:?}",
        conversation.status_text,
        conversation.runtime_notices
    );
    assert!(
        app.planner_worker_panel_state
            .last_host_detail
            .as_deref()
            .is_some_and(|detail: &str| detail.contains("host promoted top follow-up proposal"))
    );
    assert_eq!(
        conversation.status_text,
        "auto follow-up submitted / turn 1/3 / template: builtin next-task"
    );
    assert_eq!(
        app.planner_worker_panel_state.last_queue_summary.as_deref(),
        Some("next task: Draft a Korea-specific Chinese-chef job entry guide")
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn repeated_builtin_next_task_refresh_warns_once_before_pausing() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let workspace_dir = create_temp_workspace("planning-repeated-next-task");
    bootstrap_active_planning_workspace(&workspace_dir);
    let planning_dir = std::path::Path::new(&workspace_dir)
        .join(".codex-exec-loop")
        .join("planning");
    std::fs::write(
        planning_dir.join("task-ledger.json"),
        r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-repeat-1",
      "direction_id": "general-workstream",
      "direction_relation_note": "Current next task.",
      "title": "Rust 입문 8주 커리큘럼 구체화",
      "description": "Expand the roadmap into a week-by-week curriculum.",
      "status": "ready",
      "base_priority": 80,
      "dynamic_priority_delta": 0,
      "priority_reason": "Current top executable task.",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "llm",
      "last_updated_by": "llm",
      "source_turn_id": "turn-prev",
      "updated_at": "2026-04-13T00:00:00Z"
    }
  ]
}"#,
    )
    .expect("task ledger should write");

    codex_port
        .new_thread_stream_behavior
        .lock()
        .expect("new-thread stream behavior mutex poisoned")
        .events = vec![
        ConversationStreamEvent::ThreadPrepared {
            thread_id: "planner-thread-1".to_string(),
            title: "Planner".to_string(),
            cwd: workspace_dir.clone(),
        },
        ConversationStreamEvent::AgentMessageCompleted {
            item_id: "planner-item-1".to_string(),
            phase: None,
            text: "planner refreshed the queue".to_string(),
        },
        ConversationStreamEvent::TurnCompleted {
            turn_id: "planner-turn-1".to_string(),
            changed_planning_file_paths: vec![TASK_LEDGER_FILE_PATH.to_string()],
        },
    ];

    let mut conversation = ready_conversation();
    conversation.auto_follow_state.template_state.selected_index = 0;
    conversation.cwd = workspace_dir.clone();
    conversation.draft_workspace_directory = workspace_dir.clone();
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-main".to_string());
    conversation.last_planning_task_handoff = Some(PlanningTaskHandoff {
        task_id: "task-repeat-1".to_string(),
        task_title: "Rust 입문 8주 커리큘럼 구체화".to_string(),
        direction_id: "general-workstream".to_string(),
        progress_note: String::new(),
        combined_priority: 80,
        updated_at: "2026-04-13T00:00:00Z".to_string(),
        status_label: "ready".to_string(),
    });
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    conversation.replace_planning_runtime_snapshot(
        app.planning
            .runtime
            .load_runtime_snapshot_or_invalid(&workspace_dir),
    );
    app.conversation_state = ConversationState::ready(conversation);

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
        ConversationStreamEvent::TurnCompleted {
            turn_id: "turn-main".to_string(),
            changed_planning_file_paths: Vec::new(),
        },
    ));

    let mut turn_calls = Vec::new();
    for _ in 0..20 {
        turn_calls = codex_port
            .turn_calls
            .lock()
            .expect("turn call mutex poisoned")
            .iter()
            .map(|(_, prompt)| prompt.clone())
            .collect::<Vec<_>>();
        if !turn_calls.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };

    assert_eq!(turn_calls.len(), 1);
    assert_eq!(
        conversation.status_text,
        "auto follow-up submitted / turn 1/3 / template: builtin next-task"
    );
    assert_eq!(conversation.repeated_planning_queue_head_count, 1);
    assert!(
        conversation
            .runtime_notices
            .iter()
            .any(|notice| { notice.contains("allowing one more auto turn before pausing") })
    );
    assert!(
        conversation
            .planning_runtime_snapshot
            .auto_followup_pause_reason()
            .is_none()
    );
    assert!(conversation.messages.iter().any(|message| {
        message
            .text
            .contains("다음 queued task 1개를 이어서 진행합니다.")
    }));
    assert!(
        app.planner_worker_panel_state
            .last_notice_detail
            .as_deref()
            .is_some_and(|detail: &str| detail.contains("allowing one more auto turn"))
    );
    assert_eq!(
        app.planner_worker_panel_state
            .last_operation_label
            .as_deref(),
        Some("refresh")
    );
    assert_eq!(
        app.planner_worker_panel_state.last_response.as_deref(),
        Some("planner refreshed the queue")
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn second_repeated_builtin_next_task_refresh_pauses_after_warning_budget_is_exhausted() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let workspace_dir = create_temp_workspace("planning-repeated-next-task-pause");
    bootstrap_active_planning_workspace(&workspace_dir);
    let planning_dir = std::path::Path::new(&workspace_dir)
        .join(".codex-exec-loop")
        .join("planning");
    std::fs::write(
        planning_dir.join("task-ledger.json"),
        r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-repeat-1",
      "direction_id": "general-workstream",
      "direction_relation_note": "Current next task.",
      "title": "Rust 입문 8주 커리큘럼 구체화",
      "description": "Expand the roadmap into a week-by-week curriculum.",
      "status": "ready",
      "base_priority": 80,
      "dynamic_priority_delta": 0,
      "priority_reason": "Current top executable task.",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "llm",
      "last_updated_by": "llm",
      "source_turn_id": "turn-prev",
      "updated_at": "2026-04-13T00:00:00Z"
    }
  ]
}"#,
    )
    .expect("task ledger should write");

    codex_port
        .new_thread_stream_behavior
        .lock()
        .expect("new-thread stream behavior mutex poisoned")
        .events = vec![
        ConversationStreamEvent::ThreadPrepared {
            thread_id: "planner-thread-1".to_string(),
            title: "Planner".to_string(),
            cwd: workspace_dir.clone(),
        },
        ConversationStreamEvent::AgentMessageCompleted {
            item_id: "planner-item-1".to_string(),
            phase: None,
            text: "planner refreshed the queue".to_string(),
        },
        ConversationStreamEvent::TurnCompleted {
            turn_id: "planner-turn-1".to_string(),
            changed_planning_file_paths: vec![TASK_LEDGER_FILE_PATH.to_string()],
        },
    ];

    let mut conversation = ready_conversation();
    conversation.auto_follow_state.template_state.selected_index = 0;
    conversation.cwd = workspace_dir.clone();
    conversation.draft_workspace_directory = workspace_dir.clone();
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-main".to_string());
    conversation.last_planning_task_handoff = Some(PlanningTaskHandoff {
        task_id: "task-repeat-1".to_string(),
        task_title: "Rust 입문 8주 커리큘럼 구체화".to_string(),
        direction_id: "general-workstream".to_string(),
        progress_note: String::new(),
        combined_priority: 80,
        updated_at: "2026-04-13T00:00:00Z".to_string(),
        status_label: "ready".to_string(),
    });
    conversation.repeated_planning_queue_head_count = 1;
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    conversation.replace_planning_runtime_snapshot(
        app.planning
            .runtime
            .load_runtime_snapshot_or_invalid(&workspace_dir),
    );
    app.conversation_state = ConversationState::ready(conversation);

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
        ConversationStreamEvent::TurnCompleted {
            turn_id: "turn-main".to_string(),
            changed_planning_file_paths: Vec::new(),
        },
    ));

    thread::sleep(Duration::from_millis(50));

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };

    assert!(
        codex_port
            .turn_calls
            .lock()
            .expect("turn call mutex poisoned")
            .is_empty()
    );
    assert_eq!(conversation.repeated_planning_queue_head_count, 2);
    assert_eq!(
        conversation.status_text,
        "turn completed / auto follow-up paused: planning queue repeated the previous task"
    );
    assert!(
        conversation
            .planning_runtime_snapshot
            .auto_followup_pause_reason()
            .is_some_and(|reason: &str| reason.contains("previously handed-off task"))
    );
    assert!(
        app.planner_worker_panel_state
            .last_host_detail
            .as_deref()
            .is_some_and(|detail: &str| detail.contains("progress_note"))
    );

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn refreshed_queue_head_with_same_task_id_but_new_progress_note_still_submits_auto_followup() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let workspace_dir = create_temp_workspace("planning-repeated-next-task-updated");
    bootstrap_active_planning_workspace(&workspace_dir);
    let planning_dir = std::path::Path::new(&workspace_dir)
        .join(".codex-exec-loop")
        .join("planning");
    std::fs::write(
        planning_dir.join("task-ledger.json"),
        r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-repeat-1",
      "direction_id": "general-workstream",
      "direction_relation_note": "Current next task.",
      "title": "Rust 입문 8주 커리큘럼 구체화",
      "description": "Expand the roadmap into a week-by-week curriculum.",
      "status": "ready",
      "base_priority": 80,
      "dynamic_priority_delta": 0,
      "priority_reason": "Current top executable task.",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "llm",
      "last_updated_by": "llm",
      "source_turn_id": "turn-prev",
      "updated_at": "2026-04-13T00:00:00Z"
    }
  ]
}"#,
    )
    .expect("task ledger should write");

    codex_port
        .new_thread_stream_behavior
        .lock()
        .expect("new-thread stream behavior mutex poisoned")
        .events = vec![
        ConversationStreamEvent::ThreadPrepared {
            thread_id: "planner-thread-1".to_string(),
            title: "Planner".to_string(),
            cwd: workspace_dir.clone(),
        },
        ConversationStreamEvent::AgentMessageCompleted {
            item_id: "planner-item-1".to_string(),
            phase: None,
            text: "planner refreshed the queue with an updated task".to_string(),
        },
        ConversationStreamEvent::TurnCompleted {
            turn_id: "planner-turn-1".to_string(),
            changed_planning_file_paths: vec![TASK_LEDGER_FILE_PATH.to_string()],
        },
    ];
    codex_port
        .new_thread_stream_behavior
        .lock()
        .expect("new-thread stream behavior mutex poisoned")
        .planning_file_writes = vec![(
        TASK_LEDGER_FILE_PATH.to_string(),
        r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-repeat-1",
      "direction_id": "general-workstream",
      "direction_relation_note": "Current next task was updated after the latest reply.",
      "progress_note": "주차별 목표는 유지하고 실습 구성을 다음 단계로 이어감.",
      "title": "Rust 입문 8주 커리큘럼 구체화",
      "description": "Expand the roadmap into a week-by-week curriculum.",
      "status": "ready",
      "base_priority": 80,
      "dynamic_priority_delta": 0,
      "priority_reason": "Current top executable task.",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "llm",
      "last_updated_by": "llm",
      "source_turn_id": "turn-main",
      "updated_at": "2026-04-13T00:00:00Z"
    }
  ]
}"#
        .to_string(),
    )];

    let mut conversation = ready_conversation();
    conversation.auto_follow_state.template_state.selected_index = 0;
    conversation.cwd = workspace_dir.clone();
    conversation.draft_workspace_directory = workspace_dir.clone();
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-main".to_string());
    conversation.last_planning_task_handoff = Some(PlanningTaskHandoff {
        task_id: "task-repeat-1".to_string(),
        task_title: "Rust 입문 8주 커리큘럼 구체화".to_string(),
        direction_id: "general-workstream".to_string(),
        progress_note: String::new(),
        combined_priority: 80,
        updated_at: "2026-04-13T00:00:00Z".to_string(),
        status_label: "ready".to_string(),
    });
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        "latest answer",
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    conversation.replace_planning_runtime_snapshot(
        app.planning
            .runtime
            .load_runtime_snapshot_or_invalid(&workspace_dir),
    );
    app.conversation_state = ConversationState::ready(conversation);

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
        ConversationStreamEvent::TurnCompleted {
            turn_id: "turn-main".to_string(),
            changed_planning_file_paths: Vec::new(),
        },
    ));

    let mut turn_calls = Vec::new();
    for _ in 0..20 {
        turn_calls = codex_port
            .turn_calls
            .lock()
            .expect("turn call mutex poisoned")
            .iter()
            .map(|(_, prompt)| prompt.clone())
            .collect::<Vec<_>>();
        if !turn_calls.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    let ConversationState::Ready(conversation) = &app.conversation_state else {
        panic!("conversation should remain ready");
    };

    assert_eq!(turn_calls.len(), 1);
    assert_eq!(
        conversation.status_text,
        "auto follow-up submitted / turn 1/3 / template: builtin next-task"
    );
    assert!(
        conversation
            .planning_runtime_snapshot
            .auto_followup_pause_reason()
            .is_none()
    );
    assert_eq!(conversation.repeated_planning_queue_head_count, 0);

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}

#[test]
fn builtin_next_task_refresh_passes_full_latest_agent_reply_to_hidden_planner_prompt() {
    let (mut app, codex_port) = make_test_app();
    app.startup_state = StartupState::Ready(sample_startup_diagnostics("/tmp/root", true));
    let workspace_dir = create_temp_workspace("planning-refresh-full-latest-reply");
    bootstrap_active_planning_workspace(&workspace_dir);
    enable_queue_idle_review_and_enqueue(&workspace_dir);

    codex_port
        .new_thread_stream_behavior
        .lock()
        .expect("new-thread stream behavior mutex poisoned")
        .events = vec![
        ConversationStreamEvent::ThreadPrepared {
            thread_id: "planner-thread-1".to_string(),
            title: "Planner".to_string(),
            cwd: workspace_dir.clone(),
        },
        ConversationStreamEvent::AgentMessageCompleted {
            item_id: "planner-item-1".to_string(),
            phase: None,
            text: "planner refreshed the queue".to_string(),
        },
        ConversationStreamEvent::TurnCompleted {
            turn_id: "planner-turn-1".to_string(),
            changed_planning_file_paths: Vec::new(),
        },
    ];

    let latest_reply = [
        "시험 최소 범위에 맞추면 아래 목차가 깔끔합니다.",
        "",
        "**강의명**",
        "`CKA 합격을 위한 쿠버네티스 네트워크 최소 핵심`",
        "",
        "1. 강의 소개: CKA에서 네트워크가 왜 중요한가, 어디까지 알면 충분한가",
        "2. 네트워크 기초 15분 압축: IP, Port, TCP/UDP, CIDR, DNS만 빠르게 정리",
        "3. 쿠버네티스 네트워크의 3가지 기본 원칙: Pod IP, Pod 간 통신, Node 간 통신 관점 이해",
        "4. Pod 네트워크 이해: Pod IP가 붙는 방식, 같은 노드와 다른 노드 간 통신 흐름, CNI는 무엇인가",
        "5. Service 핵심: ClusterIP, NodePort, LoadBalancer 차이와 시험에서 보는 포인트",
        "6. Service가 실제로 연결되는 방식: selector, endpoints, kube-proxy를 아주 얕고 실전적으로 이해",
        "7. 클러스터 DNS: CoreDNS, Service 이름으로 통신하는 방식, FQDN과 네임스페이스 개념",
        "8. Ingress 기초: Ingress가 필요한 이유, Service와의 관계, 시험에서 알아야 할 정도만",
        "9. NetworkPolicy 핵심: ingress/egress, allow 기준 사고방식, 자주 나오는 정책 해석법",
        "10. 트러블슈팅 패턴: Pod to Pod, Pod to Service, DNS 문제를 어떤 순서로 확인할지",
        "11. 시험용 필수 명령어: kubectl get svc, kubectl get endpoints, kubectl describe, nslookup, dig, curl, ping 활용",
        "12. 실습 1: Pod 간 통신 확인",
        "13. 실습 2: Service 연결 확인과 endpoint 문제 찾기",
        "14. 실습 3: DNS 조회 실패 문제 해결",
        "15. 실습 4: NetworkPolicy 적용 전후 통신 비교",
        "16. 시험 직전 암기 포인트 정리: 꼭 기억할 개념, 자주 헷갈리는 차이점, 문제 풀이 순서",
        "",
        "빼도 되는 내용도 정해두면 강의가 더 선명합니다.",
        "",
        "- OSI 7계층 상세 설명",
        "- 라우팅 프로토콜 심화",
        "- iptables/IPVS 내부 동작 심화",
        "- CNI 플러그인 구현 디테일",
        "- BGP, VXLAN 심화",
    ]
    .join("\n");
    let latest_user_request =
        "CKA 네트워크 강의를 만들 건데 시험 최소 범위에 맞는 목차부터 정리해줘";

    let mut conversation = ready_conversation();
    conversation.auto_follow_state.template_state.selected_index = 0;
    conversation.cwd = workspace_dir.clone();
    conversation.draft_workspace_directory = workspace_dir.clone();
    conversation.input_state = ConversationInputState::StreamingTurn;
    conversation.active_turn_id = Some("turn-main".to_string());
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::User,
        latest_user_request,
        None,
        None,
    ));
    conversation.messages.push(ConversationMessage::new(
        ConversationMessageKind::Agent,
        latest_reply.clone(),
        Some("final_answer".to_string()),
        Some("agent-1".to_string()),
    ));
    conversation.replace_planning_runtime_snapshot(
        app.planning
            .runtime
            .load_runtime_snapshot_or_invalid(&workspace_dir),
    );
    app.conversation_state = ConversationState::ready(conversation);

    app.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamUpdated(
        ConversationStreamEvent::TurnCompleted {
            turn_id: "turn-main".to_string(),
            changed_planning_file_paths: Vec::new(),
        },
    ));

    let mut hidden_prompts = Vec::new();
    for _ in 0..20 {
        hidden_prompts = codex_port
            .new_thread_calls
            .lock()
            .expect("new-thread call mutex poisoned")
            .iter()
            .map(|(_, prompt)| prompt.clone())
            .collect::<Vec<_>>();
        if !hidden_prompts.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    assert_eq!(hidden_prompts.len(), 1);
    assert!(hidden_prompts[0].contains("latest operator request:"));
    assert!(hidden_prompts[0].contains(latest_user_request));
    assert!(hidden_prompts[0].contains("main session latest reply:"));
    assert!(hidden_prompts[0].contains("5. Service 핵심"));
    assert!(hidden_prompts[0].contains("- BGP, VXLAN 심화"));
    assert!(!hidden_prompts[0].contains("worker received full text"));

    std::fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
}
