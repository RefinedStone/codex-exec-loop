# K2 상세 설계: thread 별 세션 상태와 클라이언트 상태 분리

## 문서 상태

- 상태: **Proposed execution plan**. 현재 제품 동작을 설명하지 않는다.
- 상위 계획: [18 Akra 커널 아키텍처](18-akra-kernel-architecture.md) §3.2 제약 1·3, §5.4, §6 K2. 결정 KP-03
  (승인 2026-11-07).
- 이전 단계: [K1 설계](18-k1-headless-decision-loop.md) (완료 2026-11-07).
- 기준일: 2026-11-08 (Asia/Seoul)
- 기준 커밋: `71c0f98e` (`origin/prerelease`). K1 이 모두 들어간 뒤이다.
- 범위: 같은 프로세스 안에서 core 의 대화·턴 상태를 thread 별로 나눈다. 한 클라이언트가 보는 대화와 커널이
  관리하는 턴 상태를 분리한다. 데몬, 프로토콜, 클라이언트 여러 개는 K3 이다.

## 1. 목표

1. 대화를 여는 일이 실행 중인 턴을 지우지 않는다. 대화를 여는 일은 "보는 thread"만 바꾼다 (KP-03).
2. 다른 thread 의 턴은 보지 않는 동안에도 끝까지 처리된다. 턴 종료 → 평가 → 자동 후속 턴도 이어진다.
3. 커널 상태(모든 클라이언트가 공유)와 클라이언트 상태(보는 대화, 조회 화면)를 타입으로 나눈다.
4. TUI 의 대화 전환 금지 규칙을 없앤다. 이것은 TUI 에 남은 결정 규칙이기도 하다.

## 2. 현재 상태 (2026-11-08 코드 조사)

### 2.1 대화를 열면 턴 상태가 지워진다

`ConversationTurnFeatureReducer`(`src/core/app/conversation_turn_reducer.rs:122-143`)는 대화 하나 기준이다.
`admit_conversation_load`(`:221-259`)와 `complete_conversation_load`(`:281-311`)가 아래 값을 모두 지운다.

| 값 | 뜻 |
| --- | --- |
| `conversation_runtime` (`ConversationRuntimeAuthority`) | 진행 중 턴, 승인, 자동 후속, 턴 종료 후 상태, planning handoff |
| `turn_stream_state` (`TurnStreamState`) | 스트림 투영 (thread, 항목 생명주기, 진행 표시, 종료) |
| `active_stop_request`, `active_turn_steer` | 정지, steer 진행 |
| `approval_review_persistence` | 승인 검토 저장 |
| `post_turn_messages`, `pending_post_turn_evaluation`, `in_flight_post_turn_evaluation` | 평가 입력과 평가 진행 (K1-4) |
| `guarded_session_rename_stream` | 이름 바꾸기와 겹친 스트림 |

지운 뒤에 도착한 이전 턴의 스트림 이벤트는 correlation 이 맞지 않아 버려진다
(`apply_correlated_turn_stream_event`, `:386-393`). app-server 의 턴은 계속 돌지만 Akra 는 결과를 모른다.
승인 요청은 화면에 나오지 않고 기한이 지나면 거절된다 (`adapter/outbound/app_server/connection.rs:2761-2767`).

### 2.2 TUI 가 전환을 막아서 결함이 드러나지 않는다

TUI 는 턴이 실행 중이거나 대화가 바쁘면 새 초안과 대화 전환을 거절한다
(`tui/app/conversation_intents.rs:78-111`, "turn still running; wait for completion before switching
sessions"). 그래서 지금 제품에서 턴이 사라지지는 않는다. 대신 아래 제약이 있다.

- 턴이 도는 동안 다른 대화를 볼 수 없다. 자동 후속 루프가 길면 그동안 다른 대화를 볼 수 없다.
- 이 규칙은 TUI 에만 있다. core 를 쓰는 다른 입구(K3 이후)는 같은 보호를 받지 못한다.

### 2.3 단수형 상태와 공유 슬롯

| 상태 | 위치 | K2 후 |
| --- | --- | --- |
| 위 §2.1 의 값들 | `ConversationTurnFeatureReducer` | thread 별 |
| 대화 로드 절차 (`in_flight_conversation_load`, `deferred_conversation_load`) | 같은 reducer | 클라이언트 |
| `AppSnapshot.conversation` (`ConversationSnapshot`) | `snapshot.rs:11` | 클라이언트 (보는 대화) |
| `AppSnapshot.conversation_runtime` | `snapshot.rs:12` | 보는 thread 의 값 + 전체 thread 요약 |
| 조회 모델 슬롯 (peek, review center, queue, directions) | `read_model_reducer.rs:8-15` | 클라이언트 |
| 작업자 패널 기록 (`reset_worker_panel_history`) | `PlanningFeatureReducer` | 클라이언트 |
| 세션 목록, planning 작업, 큐 변경, GitHub 리뷰, 설정 | 각 reducer | 커널 (지금과 같음) |
| 세대 번호 (`next_*_generation`) | 각 reducer | 커널 (모든 thread 에서 유일해야 한다) |

### 2.4 결정이 이벤트에 섞여 있다

호스트(`NativeClientRuntime`)는 core 결과의 `AppEvent` 목록에서 내부 요청(`PostTurnEvaluationDue`, 경로
결정 요청)을 꺼내서 처리한다 (`composition/native_client_runtime.rs:664-`, `take_internal_post_turn_routing_requests`).
thread 가 여럿이 되면 이런 내부 요청이 동시에 여러 개 나올 수 있다. 지금 코드는 "정확히 하나"
(`exact_single_post_turn_routing_request`)를 가정한다.

## 3. 설계

### 3.1 `ThreadSession`: thread 하나의 턴 상태

§2.1 의 값을 구조체 하나로 묶는다. 이름은 Kotlin 식으로 읽히게 `ThreadSession` 으로 한다.

```rust
pub(super) struct ThreadSession {
    runtime: ConversationRuntimeAuthority,
    turn_stream: TurnStreamState,
    stop_request: Option<ActiveStopRequest>,
    turn_steer: Option<ActiveTurnSteer>,
    approval_review_persistence: ApprovalReviewPersistenceCoordinator,
    post_turn_messages: PostTurnMessages,
    pending_post_turn_evaluation: Option<PendingPostTurnEvaluation>,
    in_flight_post_turn_evaluation: Option<ActivePostTurnEvaluation>,
    post_turn_continuation_gate: PostTurnContinuationGate,
    guarded_session_rename_stream: Option<(TurnSubmissionCorrelation, SessionRenameCorrelation)>,
    turn_options: ConversationTurnOptions,
}
```

세대 번호는 reducer 에 남긴다. 모든 thread 에서 correlation 이 유일해야 하기 때문이다.

### 3.2 `ThreadSessions`: 맵과 보는 thread

```rust
pub(super) struct ThreadSessions {
    by_thread: BTreeMap<String, ThreadSession>,
    draft: ThreadSession,        // thread id 가 아직 없는 새 초안
    viewed: ViewedThread,        // Draft 또는 Thread(id)
}
```

- 새 초안의 턴은 `draft` 에서 시작한다. `ThreadPrepared` 로 thread id 가 정해지면 맵으로 옮긴다.
- 스트림 이벤트, 평가 완료, 승인 결과는 correlation 의 turn 으로 주인 session 을 찾는다. 보는 thread 와
  관계없이 처리한다.
- 대화를 열면 `viewed` 만 바꾼다. 실행 중이 아닌 session 은 열 때 새로 만든다. 실행 중인 session 은 그대로
  둔다. 다시 열면 진행 상태가 그대로 보인다.
- 끝난 session(실행 중 턴, 승인, 평가가 모두 없음)은 보는 thread 가 아니면 맵에서 지운다. 맵은 실행 중인
  thread 수만큼만 커진다.

### 3.3 한 번에 실행하는 턴 수

K2 에서는 대화 턴을 **동시에 하나만** 실행한다. 다른 thread 에서 턴이 도는 동안 새 턴 제출은 core 가
거절한다 ("another session is running"). 이유:

- app-server 연결과 턴 스트림 worker 가 지금 턴 하나를 기준으로 짜여 있다.
- 병렬 작업은 이미 병렬 모드(슬롯)가 맡는다.

동시 실행은 Q-K2-2 로 남긴다.

### 3.4 스냅샷: 보는 thread + 전체 요약

`AppSnapshot.conversation_runtime` 은 보는 thread 의 값이다. 새 필드 `thread_activity` 는 실행 중이거나
승인을 기다리는 다른 thread 의 요약이다.

```rust
pub struct ThreadActivitySummary {
    pub thread_id: String,
    pub running: bool,
    pub approval_pending: bool,
    pub auto_follow_active: bool,
}
```

TUI 는 이 값으로 세션 목록에 "실행 중", "승인 대기" 표시를 그린다. 승인은 그 대화를 열어서 결정한다.
기한이 지나면 지금처럼 거절된다.

### 3.5 내부 요청을 이벤트와 분리한다

`CoreDispatchOutcome` 에 `host_requests: Vec<HostRequest>` 를 더한다. 평가 시작 요청과 경로 결정 요청은
여기에만 넣는다. `AppEvent` 에서 내부 요청 변형을 지운다. 호스트는 목록을 차례로 모두 처리한다.
"정확히 하나" 가정을 없앤다.

### 3.6 커널 상태와 클라이언트 상태

- 커널: `ThreadSessions` 의 맵과 draft, 세션 목록, planning, GitHub 리뷰, 설정, 세대 번호.
- 클라이언트(`ClientView`): 보는 thread, 대화 로드 절차, `ConversationSnapshot`, 조회 모델 슬롯, 작업자 패널
  기록, 수동 프롬프트 준비.
- K2 에서는 클라이언트가 하나라서 `ClientView` 도 하나이다. 타입만 나눈다. 클라이언트 여러 개와 "결과를
  요청자에게 전달"은 프로토콜이 생기는 K3 에서 한다. 클라이언트가 하나일 때는 눈에 보이는 차이가 없어
  시험할 수 없기 때문이다.
- `revision` 은 하나로 둔다. 나누는 일은 K3 의 구독 설계와 함께 한다.

### 3.7 GitHub 리뷰 폴링 설정 (K1-6 에서 넘어온 일)

지금은 TUI 가 첫 화면을 그린 뒤 설정 값(TUI 환경 상태)으로 `SetupGithubReviewPolling` 을 보낸다. 작업 공간이
바뀌면 다시 보낸다.

- 설정 값은 composition 이 읽어서 호스트에 넘긴다.
- 호스트가 시작 점검 성공 뒤와 작업 공간 변경 뒤에 설정을 시작한다. 첫 화면 조건은 없앤다. 폴링은
  백그라운드 effect 라 화면을 막지 않는다.

### 3.8 경계 테스트

- inbound 는 대화 전환을 막는 규칙을 갖지 않는다 (`turn still running; wait for completion before switching
  sessions` 문자열 금지).
- core 의 `ThreadSession` 필드 목록과 `ClientView` 필드 목록을 고정한다. 클라이언트 상태가 커널로 섞이지 않게
  한다.
- inbound 는 `SetupGithubReviewPolling` 을 보내지 않는다.

## 4. 단계 (PR 단위)

| 단계 | 내용 | 기존 동작 | 검증 |
| --- | --- | --- | --- |
| K2-1 (K2-3 에 합침 2026-11-08) | §3.5 내부 요청 분리. 구현 전 검토에서 바꿨다: `CoreDispatchOutcome` 을 직접 만드는 곳이 81곳이라 필드를 더하는 비용이 크다. 각 입력의 결과에는 내부 요청이 많아야 하나이다(효과는 worker 가 따로 완료를 보낸다). 실제로 필요한 변경은 호스트가 보는 thread 의 스냅샷 대신 요청의 thread 로 경로를 확인하는 것이고, 이것은 K2-3 에서 한다 | 유지 | K2-3 에서 |
| K2-2 (완료 2026-11-08) | §3.1 `ThreadSession` 으로 필드를 묶는다. 아직 session 은 하나이다. 필드 이름은 바꾸지 않았다 (`conversation_runtime`, `turn_stream_state` 등): 변경을 작게 두고 이름 정리는 맵이 생기는 K2-3 에서 함께 본다 | 유지 | 기존 core 테스트 전부. `ConversationTurnFeatureReducer` 와 `ThreadSession` 필드 목록 경계 테스트 |
| K2-3 (완료 2026-11-08) | 대화를 열거나 새 초안을 열 때 일이 남은 session(턴, 자동 후속, 평가, 정지, steer)은 `background_sessions` 로 보내고, 보는 자리에는 자동 후속 설정과 턴 옵션만 이어받은 새 session 을 둔다. 다시 그 thread 를 열면 그 session 이 돌아온다. 설계와 다른 점: (1) 맵 대신 보는 session 하나와 목록을 쓴다. 주인은 thread 가 아니라 correlation 으로 찾고, 처리할 때만 보는 자리와 바꿔 넣는다. 그래서 기존 메서드 본문을 그대로 쓴다. (2) 승인 검토 저장은 한 줄로 차례대로 해야 해서 session 밖(공유)에 둔다. (3) K2-1 의 필요한 부분: 호스트가 경로를 기다리는 thread 의 runtime 값(`post_turn_route_runtime`)으로 경로를 고른다. (4) 평가 대기 턴 꺼내기와 평가 시작을 한 session 안에서 함께 한다(`start_due_post_turn_evaluation`). (5) 자동 후속 정책 변경(최대 턴, 일시 정지, 병렬 재개)은 모든 session 에 적용한다. (6) 보지 않는 thread 의 스트림 투영, steer 결과, 승인 결정 결과는 클라이언트에 보내지 않는다. 정지 요청은 지금처럼 app-server 의 모든 세션을 멈춘다 | core 는 전환을 허용. TUI 의 금지는 그대로라 화면 동작은 유지 | core 시험: 실행 중 다른 대화를 열고 돌아오면 같은 턴의 응답으로 평가한다. 다른 대화를 여는 중의 평가 완료는 뒤에서 경로까지 간다. 호스트 시험: 새 초안을 연 뒤에도 남겨 둔 대화의 자동 후속 턴이 제출된다. 기존 시험 7개는 "전환하면 버린다" 대신 "주인 session 에서 끝까지 처리한다"로 바꿨다 |
| K2-4 | §3.4 `thread_activity`. TUI 의 전환 금지를 지우고 세션 목록에 실행·승인 표시를 그린다 | **바뀜**: 턴 실행 중 다른 대화를 볼 수 있다 | TUI 시험, 터미널 캡처. 다시 열면 진행 상태가 보인다 |
| K2-5 | §3.6 `ClientView` 로 클라이언트 상태를 옮긴다 (대화 로드 절차, 조회 모델, 작업자 패널 기록) | 유지 | 필드 목록 경계 테스트 |
| K2-6 | §3.7 GitHub 폴링 설정을 호스트로 | 첫 화면 조건 없어짐 | 헤드리스 시험: TUI 없이 폴링 설정과 폴링이 시작된다 |

규모: plan 18 추정 10~15일을 유지한다. K2-3 이 가장 크다 (reducer 1,504줄 중 대부분이 session 접근을 바꾼다).

## 5. 위험

| 위험 | 대응 |
| --- | --- |
| 보지 않는 thread 의 자동 후속 턴이 사용자가 모르게 계속 돈다 | K2-4 의 실행 표시. 정지 명령(`RequestStopAllSessions`)은 모든 session 에 적용한다 |
| correlation 으로 주인을 찾지 못한 이벤트가 버려진다 | 지금도 버린다. 버린 수를 로그로 남기고 시험으로 경로를 고정한다 |
| 초안 session 이 thread id 를 받기 전에 사용자가 다른 대화를 연다 | draft session 은 턴이 끝날 때까지 남긴다. 다시 열 방법은 세션 목록의 새 thread 이다 |
| 승인 대기를 놓쳐 기한이 지나 거절된다 | 지금과 같은 기한 규칙. K2-4 의 승인 대기 표시로 줄인다. 알림은 K5(주의, 우편함) 범위이다 |
| TUI 의 전환 금지에 기대던 다른 TUI 코드가 있다 | K2-4 에서 `has_running_turn` 사용처(약 12곳)를 보는 thread 기준으로 점검한다 |

## 6. 미결

| ID | 질문 | 기본값 (답이 없을 때) |
| --- | --- | --- |
| Q-K2-1 | 턴 실행 중에 다른 대화를 보게 할 것인가 (K2-4 의 동작 변경) | 허용한다. KP-03 의 뜻과 같다 |
| Q-K2-2 | 서로 다른 thread 의 대화 턴을 동시에 실행할 것인가 | K2 에서는 하나만. 병렬 작업은 병렬 모드가 맡는다 |
| Q-K2-3 | 병렬 제어부를 core 상태 기계에 합칠 것인가 (지금은 core 와 따로 tick·상태를 갖는다) | K2 에서는 합치지 않는다. K3 에서 데몬 하나가 둘을 소유한 뒤 다시 본다 |
