// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::application::service::parallel_mode::{
    ParallelModeOfficialCompletionReport, ParallelModeOrchestratorTrigger, ParallelModeService,
};
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::domain::parallel_mode::ParallelModeSlotLeaseRequest;

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
/*
학습 주석: 이 요청 타입은 TUI가 "대화 스트림을 시작한다"는 한 가지 동작을
application 계층으로 넘길 때 필요한 입력을 담습니다. 평소에는 현재 workspace와
thread_id를 그대로 사용하지만, 병렬 모드에서는 `slot_lease_request`가 함께 들어와
"먼저 빈 슬롯 worktree를 빌린 뒤 그 worktree에서 새 thread를 시작하라"는 의미가
됩니다. 그래서 이 타입은 대화 런타임과 병렬 슬롯 런타임 사이의 작은 경계 객체입니다.
*/
// 학습 주석: `struct`는 여러 값을 하나의 의미 있는 데이터 묶음으로 다루기 위한 Rust의 구조체 정의입니다.
pub struct ParallelTurnStreamLaunchRequest {
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub workspace_directory: String,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub thread_id: Option<String>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub prompt: String,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub slot_lease_request: Option<ParallelModeSlotLeaseRequest>,
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
/*
학습 주석: launch outcome은 실제 스트림 실행에 사용할 요청을 다시 돌려줍니다.
병렬 슬롯을 빌린 경우 `request.workspace_directory`가 원래 저장소에서 슬롯 worktree로
바뀌고 `thread_id`가 `None`이 됩니다. 슬롯마다 별도의 Codex thread를 시작해야
주 저장소의 대화와 슬롯 대화가 섞이지 않기 때문입니다.

`invalidate_supervisor_snapshot`은 TUI 캐시 무효화 신호입니다. lease 획득은 감독자
패널의 슬롯 상태를 바꾸므로, 상위 화면은 이 값을 보고 최신 pool/roster/detail을
다시 읽습니다.
*/
// 학습 주석: `struct`는 여러 값을 하나의 의미 있는 데이터 묶음으로 다루기 위한 Rust의 구조체 정의입니다.
pub struct ParallelTurnStreamLaunchOutcome {
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub request: ParallelTurnStreamLaunchRequest,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub launch_notice: Option<String>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub invalidate_supervisor_snapshot: bool,
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
/*
학습 주석: 스트림 이벤트 outcome은 "대화 런타임에서 관측한 이벤트가 슬롯 lease
상태를 바꾸었는가"를 알려 줍니다. `ThreadPrepared`는 thread id를 lease에 기록하고,
`TurnStarted`는 슬롯을 running 상태로 전환합니다. 이 구분이 있어야 시작 직전 실패와
실행 중 실패를 다르게 처리할 수 있습니다.
*/
// 학습 주석: `struct`는 여러 값을 하나의 의미 있는 데이터 묶음으로 다루기 위한 Rust의 구조체 정의입니다.
pub struct ParallelTurnStreamEventOutcome {
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub runtime_notice: Option<String>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub invalidate_supervisor_snapshot: bool,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub turn_started_observed: bool,
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug, Clone, PartialEq, Eq)]
// 학습 주석: `struct`는 여러 값을 하나의 의미 있는 데이터 묶음으로 다루기 위한 Rust의 구조체 정의입니다.
pub struct ParallelTurnStreamCompletionOutcome {
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub runtime_notice: Option<String>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub invalidate_supervisor_snapshot: bool,
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Clone)]
/*
학습 주석: 이 서비스는 TUI의 conversation stream lifecycle과 `ParallelModeService`의
slot lease 상태 기계를 이어 주는 얇은 application 서비스입니다. 대화 런타임은
ThreadPrepared, TurnStarted, terminal failure 같은 스트림 사건을 알고 있고,
병렬 모드 서비스는 lease 파일과 planning authority 상태를 알고 있습니다. 이 타입은
두 세계가 서로의 세부 구현을 직접 알지 않도록 상태 전이 호출만 번역합니다.
*/
// 학습 주석: `struct`는 여러 값을 하나의 의미 있는 데이터 묶음으로 다루기 위한 Rust의 구조체 정의입니다.
pub struct ParallelModeTurnService {
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    parallel_mode_service: ParallelModeService,
}

// 학습 주석: `impl` 블록은 특정 타입이나 trait 구현에 속한 함수들을 한곳에 묶습니다.
impl std::fmt::Debug for ParallelModeTurnService {
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .debug_struct("ParallelModeTurnService")
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .finish_non_exhaustive()
    }
}

// 학습 주석: `impl` 블록은 특정 타입이나 trait 구현에 속한 함수들을 한곳에 묶습니다.
impl ParallelModeTurnService {
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn new(parallel_mode_service: ParallelModeService) -> Self {
        Self {
            parallel_mode_service,
        }
    }

    /*
    학습 주석: 스트림을 실제로 띄우기 전 병렬 슬롯 lease가 필요한지 판단하는 진입점입니다.
    `slot_lease_request`가 없으면 일반 대화이므로 입력 요청을 그대로 돌려주고, 있으면
    `ParallelModeService::acquire_slot_lease`로 비어 있는 슬롯을 하나 확보합니다.

    lease가 성공하면 반환 요청을 슬롯 worktree 기준으로 다시 작성합니다. 여기서
    `thread_id`를 비우는 것이 중요합니다. 기존 thread를 재사용하면 root workspace의
    대화 기록과 슬롯 작업 대화가 연결될 수 있으므로, 슬롯 작업은 항상 leased worktree의
    새 thread로 시작합니다.
    */
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn prepare_stream_launch(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        request: ParallelTurnStreamLaunchRequest,
    ) -> Result<ParallelTurnStreamLaunchOutcome, String> {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let Some(slot_lease_request) = request.slot_lease_request.clone() else {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return Ok(ParallelTurnStreamLaunchOutcome {
                request,
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                launch_notice: None,
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                invalidate_supervisor_snapshot: false,
            });
        };

        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let lease = self
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .parallel_mode_service
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .acquire_slot_lease(&request.workspace_directory, slot_lease_request)?;
        // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
        Ok(ParallelTurnStreamLaunchOutcome {
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            request: ParallelTurnStreamLaunchRequest {
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                workspace_directory: lease.worktree_path.clone(),
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                thread_id: None,
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                prompt: request.prompt,
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                slot_lease_request: None,
            },
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            launch_notice: Some(format!(
                "slot lease acquired before stream launch / slot: {} / agent: {} / task: {}",
                lease.slot_id, lease.agent_id, lease.task_id
            )),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            invalidate_supervisor_snapshot: true,
        })
    }

    /*
    학습 주석: 대화 스트림은 여러 이벤트를 순서대로 방출하지만, 병렬 슬롯 상태에 의미가
    있는 이벤트는 일부뿐입니다. `ThreadPrepared`는 Codex app-server가 새 thread id를
    확정했다는 뜻이므로 lease에 thread id를 저장합니다. `TurnStarted`는 실제 turn이
    시작되었다는 뜻이므로 슬롯을 running으로 바꿉니다.

    이 함수가 `turn_started_observed`를 따로 반환하는 이유는 완료 처리에서 "시작도 못한
    lease"와 "시작한 뒤 실패한 turn"을 구분하기 위해서입니다. 시작 전 실패는 슬롯을 즉시
    release할 수 있지만, 시작 후 성공은 공식 완료/검증/통합 큐 단계로 이어져야 합니다.
    */
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn sync_stream_event(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_directory: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        event: &ConversationStreamEvent,
    ) -> ParallelTurnStreamEventOutcome {
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if let ConversationStreamEvent::ThreadPrepared { thread_id, .. } = event {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return match self
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .parallel_mode_service
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .record_workspace_slot_thread_prepared(workspace_directory, thread_id)
            {
                // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
                Ok(Some(_)) => ParallelTurnStreamEventOutcome {
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    runtime_notice: None,
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    invalidate_supervisor_snapshot: true,
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    turn_started_observed: false,
                },
                // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
                Ok(None) => ParallelTurnStreamEventOutcome {
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    runtime_notice: None,
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    invalidate_supervisor_snapshot: false,
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    turn_started_observed: false,
                },
                // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
                Err(error) => ParallelTurnStreamEventOutcome {
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    runtime_notice: Some(format!(
                        "slot lease thread-prepared transition failed: {error}"
                    )),
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    invalidate_supervisor_snapshot: false,
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    turn_started_observed: false,
                },
            };
        }

        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if !matches!(event, ConversationStreamEvent::TurnStarted { .. }) {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return ParallelTurnStreamEventOutcome {
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                runtime_notice: None,
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                invalidate_supervisor_snapshot: false,
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                turn_started_observed: false,
            };
        }

        // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
        match self
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .parallel_mode_service
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .mark_workspace_slot_running(workspace_directory)
        {
            // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
            Ok(Some(_)) => ParallelTurnStreamEventOutcome {
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                runtime_notice: None,
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                invalidate_supervisor_snapshot: true,
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                turn_started_observed: true,
            },
            // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
            Ok(None) => ParallelTurnStreamEventOutcome {
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                runtime_notice: None,
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                invalidate_supervisor_snapshot: false,
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                turn_started_observed: true,
            },
            // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
            Err(error) => ParallelTurnStreamEventOutcome {
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                runtime_notice: Some(format!("slot lease running transition failed: {error}")),
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                invalidate_supervisor_snapshot: false,
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                turn_started_observed: true,
            },
        }
    }

    /*
    학습 주석: 스트림 종료 시점에는 단순히 lease를 없애면 안 됩니다. turn이 시작되기도
    전에 실패했다면 슬롯 worktree에 의미 있는 작업이 없으므로 lease를 release합니다.
    반대로 turn이 정상적으로 시작되고 실패 이벤트 없이 끝났다면, 슬롯은 곧바로 reusable이
    아니라 "공식 완료를 기다리는 작업 결과"가 됩니다. 이후 official completion이 작업
    요약/검증 결과를 기록하고 distributor queue에 넘깁니다.

    `saw_failed_before_turn_started`, `saw_failed_event`, `terminal_failure_observed`를
    나눠 받는 것은 스트림 이벤트의 실패 시점이 슬롯 정리 정책을 바꾸기 때문입니다.
    */
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn finalize_stream_completion(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_directory: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        saw_turn_started: bool,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        saw_failed_before_turn_started: bool,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        saw_failed_event: bool,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        terminal_failure_observed: bool,
    ) -> ParallelTurnStreamCompletionOutcome {
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if should_release_unstarted_slot_lease(
            saw_turn_started,
            saw_failed_before_turn_started,
            terminal_failure_observed,
        ) {
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return match self
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .parallel_mode_service
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .release_workspace_slot_lease_after_failed_start(workspace_directory)
            {
                // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
                Ok(Some(lease)) => ParallelTurnStreamCompletionOutcome {
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    runtime_notice: Some(format!(
                        "slot lease released after startup failure / slot: {} / agent: {}",
                        lease.slot_id, lease.agent_id
                    )),
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    invalidate_supervisor_snapshot: true,
                },
                // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
                Ok(None) => ParallelTurnStreamCompletionOutcome {
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    runtime_notice: None,
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    invalidate_supervisor_snapshot: false,
                },
                // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
                Err(error) => ParallelTurnStreamCompletionOutcome {
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    runtime_notice: Some(format!(
                        "slot lease release failed after startup failure: {error}"
                    )),
                    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                    invalidate_supervisor_snapshot: false,
                },
            };
        }

        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if should_mark_cleanup_pending_after_success(
            saw_turn_started,
            saw_failed_event,
            terminal_failure_observed,
        ) {
            // Successful leased turns now wait for post-turn official completion/distributor
            // orchestration before cleanup and slot return.
            // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
            return ParallelTurnStreamCompletionOutcome {
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                runtime_notice: None,
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                invalidate_supervisor_snapshot: false,
            };
        }

        ParallelTurnStreamCompletionOutcome {
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            runtime_notice: None,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            invalidate_supervisor_snapshot: false,
        }
    }

    /*
    학습 주석: official completion은 슬롯 agent가 낸 결과를 root planning authority의
    언어로 다시 정리하는 단계입니다. 여기서는 root turn id, refresh 순서, 최종 답변,
    검증 요약을 `ParallelModeService`에 넘겨 슬롯 lease를 "공식 완료 진행 중" 상태로
    전환합니다. 이 단계를 거쳐야 distributor가 어떤 task 결과를 어떤 순서로 통합할지
    안정적으로 판단할 수 있습니다.
    */
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn begin_official_completion(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_directory: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        root_turn_id: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        refresh_order: Option<u64>,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        latest_main_reply: Option<&str>,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        validation_summary: Option<&str>,
    ) -> Result<Option<ParallelModeOfficialCompletionReport>, String> {
        self.parallel_mode_service
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .begin_workspace_official_completion(
                workspace_directory,
                root_turn_id,
                refresh_order,
                latest_main_reply,
                validation_summary,
                None,
            )
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn reserve_official_completion_refresh_order(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_directory: &str,
    ) -> Result<Option<u64>, String> {
        self.parallel_mode_service
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .reserve_workspace_official_completion_refresh_order(workspace_directory)
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn mark_official_completion_failed(&self, workspace_directory: &str, failure_detail: &str) {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let _ = self
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .parallel_mode_service
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .mark_workspace_official_completion_failed(workspace_directory, failure_detail);
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn mark_official_completion_refreshing(&self, workspace_directory: &str) -> Option<String> {
        // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
        match self
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .parallel_mode_service
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .mark_workspace_official_completion_refreshing(workspace_directory)
        {
            // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
            Ok(_) => None,
            // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
            Err(error) => Some(format!(
                "official completion refreshing state could not be recorded: {error}"
            )),
        }
    }

    /*
    학습 주석: official completion이 성공하면 슬롯 결과는 commit-ready 상태가 되고,
    distributor queue에 들어갑니다. 즉, agent worktree에서 나온 변경을 바로 통합하지
    않고 queue record로 한 번 직렬화합니다. 병렬 agent는 여러 개지만 prerelease 통합은
    한 줄로 처리해야 충돌과 순서 의존성을 관리할 수 있기 때문입니다.

    마지막 `run_orchestrator_tick`은 방금 enqueue한 결과를 계기로 queue 처리를 한 번 더
    진행하게 합니다. 이 덕분에 사용자가 별도 새로고침을 누르지 않아도 공식 완료 직후
    통합 오케스트레이션이 이어질 수 있습니다.
    */
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn finalize_official_completion_success(
        &self,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        workspace_directory: &str,
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        authority_refresh_outcome: &str,
    ) -> Vec<String> {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let mut notices = Vec::new();
        // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
        if let Err(error) = self
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .parallel_mode_service
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .mark_workspace_commit_ready(workspace_directory, authority_refresh_outcome)
        {
            notices.push(format!(
                "commit-ready state could not be recorded after official refresh: {error}"
            ));
        }
        // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
        match self
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .parallel_mode_service
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .enqueue_workspace_commit_ready_result(workspace_directory)
        {
            // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
            Ok(Some(item)) => notices.push(format!(
                "commit-ready result entered the distributor queue / agent: {} / task: {} / state: {}",
                item.source_agent,
                item.task_title,
                item.queue_state.label()
            )),
            // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
            Ok(None) => {}
            // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
            Err(error) => {
                notices.push(format!(
                    "distributor enqueue failed after official refresh: {error}"
                ));
                // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
                return notices;
            }
        }

        // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
        match self.parallel_mode_service.run_orchestrator_tick(
            workspace_directory,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            ParallelModeOrchestratorTrigger::PlanningRefreshCompleted,
        ) {
            // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
            Ok(tick_result) => notices.extend(tick_result.notices),
            // 학습 주석: `Result`의 `Ok`는 성공 값을, `Err`는 실패 정보를 담아 호출자가 오류를 처리하게 합니다.
            Err(error) => notices.push(format!(
                "orchestrator tick failed after official refresh: {error}"
            )),
        }

        notices
    }
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn should_release_unstarted_slot_lease(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    saw_turn_started: bool,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    saw_failed_before_turn_started: bool,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    terminal_failure_observed: bool,
) -> bool {
    saw_failed_before_turn_started || (!saw_turn_started && terminal_failure_observed)
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn should_mark_cleanup_pending_after_success(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    saw_turn_started: bool,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    saw_failed_event: bool,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    terminal_failure_observed: bool,
) -> bool {
    saw_turn_started && !saw_failed_event && !terminal_failure_observed
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[cfg(test)]
// 학습 주석: `mod` 선언은 Rust 파일/하위 모듈을 현재 모듈 트리에 연결하는 입구 역할을 합니다.
mod tests {
    // 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
    use std::fs;
    // 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
    use std::process::Command;
    // 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
    use std::sync::Arc;
    // 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
    use std::time::{SystemTime, UNIX_EPOCH};

    // 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
    use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
    // 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
    use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
    // 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
    use crate::adapter::outbound::github::GithubAutomationAdapter;
    // 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
    use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
    // 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
    use crate::application::service::parallel_mode::ParallelModeService;

    // 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
    use super::{
        ParallelModeTurnService, should_mark_cleanup_pending_after_success,
        should_release_unstarted_slot_lease,
    };

    // 학습 주석: `struct`는 여러 값을 하나의 의미 있는 데이터 묶음으로 다루기 위한 Rust의 구조체 정의입니다.
    struct TempGitWorkspace {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        root: String,
    }

    // 학습 주석: `impl` 블록은 특정 타입이나 trait 구현에 속한 함수들을 한곳에 묶습니다.
    impl TempGitWorkspace {
        // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
        fn new(prefix: &str) -> Self {
            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let unique_suffix = SystemTime::now()
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .duration_since(UNIX_EPOCH)
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .expect("system clock should be valid")
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .as_nanos();
            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let root = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            fs::create_dir_all(&root).expect("temp git workspace should be created");
            run_git(&root, &["init"]);
            run_git(&root, &["config", "user.name", "RefinedStone"]);
            run_git(&root, &["config", "user.email", "chem.en.9273@gmail.com"]);
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            fs::write(root.join("README.md"), "temp repo\n")
                // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
                .expect("temp git workspace seed file should write");
            run_git(&root, &["add", "README.md"]);
            run_git(&root, &["commit", "-m", "Initial commit"]);

            Self {
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                root: root.display().to_string(),
            }
        }
    }

    // 학습 주석: `impl` 블록은 특정 타입이나 trait 구현에 속한 함수들을 한곳에 묶습니다.
    impl Drop for TempGitWorkspace {
        // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
        fn drop(&mut self) {
            // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn create_temp_directory(prefix: &str) -> String {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let unique_suffix = SystemTime::now()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .duration_since(UNIX_EPOCH)
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .expect("system clock should be valid")
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .as_nanos();
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let root = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        fs::create_dir_all(&root).expect("temp directory should be created");
        root.display().to_string()
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn run_git(repo_root: &std::path::Path, args: &[&str]) {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let status = Command::new("git")
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .current_dir(repo_root)
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .args(args)
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .status()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .expect("git command should launch");
        assert!(
            status.success(),
            "git command failed: git {}",
            args.join(" ")
        );
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn test_parallel_mode_service() -> ParallelModeService {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        ParallelModeService::new(
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            Arc::new(SqlitePlanningAuthorityAdapter::new()),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            Arc::new(GithubAutomationAdapter::new()),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            Arc::new(GitParallelModeRuntimeAdapter::new()),
        )
    }

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[test]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn startup_failure_requests_unstarted_slot_release() {
        assert!(should_release_unstarted_slot_lease(false, true, true));
    }

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[test]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn running_turn_does_not_request_unstarted_slot_release() {
        assert!(!should_release_unstarted_slot_lease(true, false, true));
    }

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[test]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn successful_running_turn_is_cleanup_candidate() {
        assert!(should_mark_cleanup_pending_after_success(
            true, false, false
        ));
    }

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[test]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn turn_started_without_slot_lease_keeps_snapshot_steady() {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let workspace = TempGitWorkspace::new("parallel-turn-no-lease");
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let service = ParallelModeTurnService::new(test_parallel_mode_service());

        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let outcome = service.sync_stream_event(
            &workspace.root,
            &ConversationStreamEvent::TurnStarted {
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                turn_id: "turn-1".to_string(),
            },
        );

        assert!(!outcome.invalidate_supervisor_snapshot);
        assert!(outcome.turn_started_observed);
        assert!(outcome.runtime_notice.is_none());
    }

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[test]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn thread_prepared_without_slot_lease_keeps_snapshot_steady() {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let workspace = TempGitWorkspace::new("parallel-thread-prepared-no-lease");
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let service = ParallelModeTurnService::new(test_parallel_mode_service());

        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let outcome = service.sync_stream_event(
            &workspace.root,
            &ConversationStreamEvent::ThreadPrepared {
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                thread_id: "thread-1".to_string(),
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                title: "Temp".to_string(),
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                cwd: workspace.root.clone(),
            },
        );

        assert!(!outcome.invalidate_supervisor_snapshot);
        assert!(!outcome.turn_started_observed);
        assert!(outcome.runtime_notice.is_none());
    }

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[test]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn official_completion_refreshing_failure_becomes_runtime_notice() {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let workspace = create_temp_directory("parallel-turn-refresh-failure");
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let service = ParallelModeTurnService::new(test_parallel_mode_service());

        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let notice = service.mark_official_completion_refreshing(&workspace);

        assert!(notice.as_deref().is_some_and(|value| {
            value.contains("official completion refreshing state could not be recorded")
        }));
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let _ = fs::remove_dir_all(workspace);
    }

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[test]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn official_completion_finalize_surfaces_commit_ready_transition_failure() {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let workspace = create_temp_directory("parallel-turn-commit-ready-failure");
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let service = ParallelModeTurnService::new(test_parallel_mode_service());

        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let notices = service
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .finalize_official_completion_success(&workspace, "official ledger refresh succeeded");

        assert!(notices.iter().any(|notice| {
            notice.contains("commit-ready state could not be recorded after official refresh")
        }));
        assert!(notices.iter().any(|notice| {
            notice.contains("distributor enqueue failed after official refresh")
        }));
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let _ = fs::remove_dir_all(workspace);
    }
}
