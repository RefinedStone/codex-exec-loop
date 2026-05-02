// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::{build_detail_lines, build_distributor_lines, build_roster_lines};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterEntry, ParallelModeAgentRosterSnapshot,
    ParallelModeAgentSessionDetailSnapshot, ParallelModeAgentSessionHistoryEntry,
    ParallelModeCompletionFeedEntry, ParallelModeDistributorQueueItem,
    ParallelModeDistributorSnapshot, ParallelModeOrchestratorStatus, ParallelModePoolBoardSnapshot,
    ParallelModePoolSlotSnapshot, ParallelModePoolSlotState, ParallelModeQueueItemState,
    ParallelModeSupervisorDetailSnapshot, ParallelModeSupervisorSnapshot,
    ParallelModeSupervisorState,
};

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn distributor_lines_render_blocked_head_and_rebase_provenance() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let snapshot = ParallelModeDistributorSnapshot::new(
        vec![ParallelModeDistributorQueueItem::new(
            "agent-1",
            "Task One",
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            ParallelModeQueueItemState::Blocked,
            "akra-agent/slot-1/task-one",
            "def4567",
            "rebased branch `akra-agent/slot-1/task-one` could not be force-pushed: rejected",
        )],
        vec![ParallelModeCompletionFeedEntry::new(
            "merge queued",
            "distributor queue head is blocked",
        )],
        "blocked",
        "rebased branch `akra-agent/slot-1/task-one` could not be force-pushed: rejected",
    )
    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
    .with_head_blocked_detail(Some(
        "rebased branch `akra-agent/slot-1/task-one` could not be force-pushed: rejected"
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .to_string(),
    ))
    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
    .with_head_rebase_provenance(Some("rebased abc1234 -> def4567 onto `akra`".to_string()));

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let rendered = build_distributor_lines(&snapshot)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .into_iter()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(|line| line.to_string())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .collect::<Vec<_>>()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .join("\n");

    assert!(rendered.contains("blocked head: rebased branch"));
    assert!(rendered.contains("provenance: rebased abc1234 -> def4567 onto `akra`"));
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn distributor_lines_omit_duplicate_note_when_blocked_detail_matches() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let detail = "rebased branch `akra-agent/slot-1/task-one` could not be force-pushed: rejected";
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let snapshot = ParallelModeDistributorSnapshot::new(
        vec![ParallelModeDistributorQueueItem::new(
            "agent-1",
            "Task One",
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            ParallelModeQueueItemState::Blocked,
            "akra-agent/slot-1/task-one",
            "def4567",
            detail,
        )],
        vec![],
        "blocked",
        detail,
    )
    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
    .with_head_blocked_detail(Some(detail.to_string()));

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let rendered = build_distributor_lines(&snapshot)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .into_iter()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(|line| line.to_string())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .collect::<Vec<_>>()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .join("\n");

    assert!(!rendered.contains(&format!("note: {detail}")));
    assert!(rendered.contains(&format!("blocked head: {detail}")));
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn distributor_lines_render_head_marker_and_humanized_queue_state_labels() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let snapshot = ParallelModeDistributorSnapshot::new(
        vec![
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            ParallelModeDistributorQueueItem::new(
                "agent-1",
                "Task One",
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                ParallelModeQueueItemState::PrPending,
                "akra-agent/slot-1/task-one",
                "abc1234",
                "pull request #42 is open and waiting for merge",
            ),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            ParallelModeDistributorQueueItem::new(
                "agent-2",
                "Task Two",
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                ParallelModeQueueItemState::Queued,
                "akra-agent/slot-2/task-two",
                "def5678",
                "commit-ready result accepted into distributor queue",
            ),
        ],
        vec![],
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        ParallelModeQueueItemState::PrPending.label(),
        "pull request #42 is open and waiting for merge",
    );

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let rendered = build_distributor_lines(&snapshot)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .into_iter()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(|line| line.to_string())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .collect::<Vec<_>>()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .join("\n");

    assert!(rendered.contains("head: pr pending"));
    assert!(rendered.contains(
        "current: agent-1 / Task One / pr pending / akra-agent/slot-1/task-one / abc1234 / pull request #42 is open and waiting for merge"
    ));
    assert!(rendered.contains(
        "next: agent-2 / Task Two / queued / akra-agent/slot-2/task-two / def5678 / commit-ready result accepted into distributor queue"
    ));
    assert!(!rendered.contains("pr_pending"));
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn distributor_lines_render_orchestrator_status_details() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let snapshot = ParallelModeDistributorSnapshot::new(
        vec![ParallelModeDistributorQueueItem::new(
            "agent-1",
            "Task One",
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            ParallelModeQueueItemState::Blocked,
            "akra-agent/slot-1/task-one",
            "abc1234",
            "could not cherry-pick into `akra` cleanly",
        )],
        vec![],
        "blocked",
        "could not cherry-pick into `akra` cleanly",
    )
    // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
    .with_orchestrator_status(ParallelModeOrchestratorStatus {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        queue_head: "agent-1 / task-1 / blocked".to_string(),
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        barrier_state: "blocked".to_string(),
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        blocked_reason: Some("could not cherry-pick into `akra` cleanly".to_string()),
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        conflict_files: vec!["conflict.txt".to_string()],
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        held_queue_count: 2,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        integration_worktree_readiness: "blocked: dirty worktree".to_string(),
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        slot_return_wait_reason: Some(
            "slot `slot-1` stays running until the queue head is integrated".to_string(),
        ),
    });

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let rendered = build_distributor_lines(&snapshot)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .into_iter()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(|line| line.to_string())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .collect::<Vec<_>>()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .join("\n");

    assert!(rendered.contains("orchestrator head: agent-1 / task-1 / blocked"));
    assert!(rendered.contains("orchestrator barrier: blocked"));
    assert!(rendered.contains("orchestrator held queue: 2"));
    assert!(rendered.contains("integration worktree: blocked: dirty worktree"));
    assert!(rendered.contains("blocked reason: could not cherry-pick into `akra` cleanly"));
    assert!(rendered.contains("conflict files: conflict.txt"));
    assert!(
        rendered.contains(
            "slot return: slot `slot-1` stays running until the queue head is integrated"
        )
    );
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn roster_and_detail_lines_render_distinct_reported_and_official_labels() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let snapshot = ParallelModeSupervisorSnapshot::new(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        ParallelModeSupervisorState::Supervise,
        "/tmp/workspace",
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        ParallelModePoolBoardSnapshot::new(0, "/tmp/pool", "idle", Vec::new()),
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        ParallelModeAgentRosterSnapshot::new(
            vec![ParallelModeAgentRosterEntry::new(
                "agent-1",
                "Task One",
                "slot-1",
                "akra-agent/slot-1/task-one",
                "reported_complete",
                "reported",
                "agent reported completion",
            )],
            "empty",
        ),
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        ParallelModeSupervisorDetailSnapshot::new(
            Some(ParallelModeAgentSessionDetailSnapshot::new(
                "slot-1:task-1",
                "agent-1",
                "task-1",
                "Task One",
                "slot-1",
                Some("thread-1".to_string()),
                "/tmp/worktree",
                "akra-agent/slot-1/task-one",
                "2026-04-17T00:00:00Z",
                "commit_ready",
                "commit_ready",
                "official ledger refresh accepted the completion report",
                "tests passed",
                "official ledger refresh succeeded",
                Some("commit-ready result accepted into distributor queue".to_string()),
                vec![
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    ParallelModeAgentSessionHistoryEntry::new(
                        "reported_complete",
                        "2026-04-17T00:01:00Z",
                        "agent reported completion",
                    ),
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    ParallelModeAgentSessionHistoryEntry::new(
                        "commit_ready",
                        "2026-04-17T00:02:00Z",
                        "official ledger refresh accepted the completion report",
                    ),
                ],
                "2026-04-17T00:02:00Z",
            )),
            "empty",
        ),
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "official", "queued"),
        None,
    );

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let roster_rendered = build_roster_lines(&snapshot)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .into_iter()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(|line| line.to_string())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .collect::<Vec<_>>()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .join("\n");
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let detail_rendered = build_detail_lines(&snapshot)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .into_iter()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(|line| line.to_string())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .collect::<Vec<_>>()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .join("\n");

    assert!(roster_rendered.contains("reported / reported / agent reported completion"));
    assert!(!roster_rendered.contains("reported_complete"));
    assert!(detail_rendered.contains("selection: agent-1 / slot-1 / official"));
    assert!(detail_rendered.contains("completion: official"));
    assert!(
        detail_rendered.contains("2026-04-17T00:01:00Z / reported / agent reported completion")
    );
    assert!(detail_rendered.contains(
        "2026-04-17T00:02:00Z / official / official ledger refresh accepted the completion report"
    ));
    assert!(!detail_rendered.contains("commit_ready"));
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[test]
// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn roster_and_detail_lines_surface_missing_slot_worktree_health() {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let snapshot = ParallelModeSupervisorSnapshot::new(
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        ParallelModeSupervisorState::Supervise,
        "/tmp/workspace",
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        ParallelModePoolBoardSnapshot::new(
            3,
            "/tmp/pool",
            "reconcile blocked",
            vec![ParallelModePoolSlotSnapshot::new(
                "slot-1",
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                ParallelModePoolSlotState::Blocked,
                "akra-agent/slot-1/task-one",
                "akra-pool/slot-1 / lease exists but worktree is missing",
                "agent-1 / task-1",
            )],
        ),
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        ParallelModeAgentRosterSnapshot::new(
            vec![ParallelModeAgentRosterEntry::new(
                "agent-1",
                "Task One",
                "slot-1",
                "akra-agent/slot-1/task-one",
                "running",
                "1m 5s",
                "agent session is active in the leased slot",
            )],
            "empty",
        ),
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        ParallelModeSupervisorDetailSnapshot::new(
            Some(ParallelModeAgentSessionDetailSnapshot::new(
                "slot-1:task-1",
                "agent-1",
                "task-1",
                "Task One",
                "slot-1",
                Some("thread-1".to_string()),
                "/tmp/missing-worktree",
                "akra-agent/slot-1/task-one",
                "2026-04-17T00:00:00Z",
                "running",
                "in_progress",
                "agent session is active in the leased slot",
                "not checked yet",
                "not refreshed yet",
                None,
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                Vec::new(),
                "2026-04-17T00:01:05Z",
            )),
            "empty",
        ),
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queued"),
        None,
    );

    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let roster_rendered = build_roster_lines(&snapshot)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .into_iter()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(|line| line.to_string())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .collect::<Vec<_>>()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .join("\n");
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let detail_rendered = build_detail_lines(&snapshot)
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .into_iter()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .map(|line| line.to_string())
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .collect::<Vec<_>>()
        // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
        .join("\n");

    assert!(roster_rendered.contains("running / working 1m 5s"));
    assert!(roster_rendered.contains("slot blocked: lease exists but worktree is missing"));
    assert!(
        detail_rendered.contains("slot health: slot blocked: lease exists but worktree is missing")
    );
}
