# Akra 커널 성질 목표

## 문서 상태

- 상태: **Proposed goals**. 2026-11-09 사용자와 논의해서 작성했다. 현재 제품 동작을 설명하지 않는다. 현재 구조의
  기준은 [architecture.md](../reference/architecture.md) 이다.
- 상위 계획: [18 Akra 커널 아키텍처](18-akra-kernel-architecture.md). 이 문서는 plan 18 의 K4~K6 에 **완료 기준이
  되는 성질**을 더한다. plan 18 의 결정(KP-01~07)은 바꾸지 않는다.
- 관련 설계: [K2 thread 세션](18-k2-thread-sessions.md) (Q-K2-2, Q-K2-3), [K3 커널 데몬](18-k3-kernel-daemon-protocol.md),
  [20 관찰-제안 구조](20-akra-observe-and-offer.md) (이 성질들이 받쳐야 하는 사용자 기능).
- 기준일: 2026-11-09 (Asia/Seoul)
- 기준 커밋: `09e898dd` (`origin/prerelease`, K3-3b·K3-4 머지 뒤).

## 1. 목적

plan 18 은 커널이 **돌아가게** 만든다: 화면 밖으로 꺼내고, 저장소당 하나로 두고, 프로토콜로 연다. 이 문서는 그
커널이 **어떤 성질을 가져야 하는가**를 정한다. 교과서 구조를 그대로 따르는 것이 목표가 아니다. Akra 의 목표(사람과
AI 가 함께 쓰는 상주 커널)에 맞는 성질을 먼저 정하고, 구조를 그 성질에 맞춘다.

## 2. 용어

| 용어 | 뜻 |
| --- | --- |
| Akra 커널 | 저장소 하나에 대해 상태와 판단 루프를 혼자 가지고 실행되는 단위. domain, application, core, 런타임 호스트, outbound adapter 를 포함한다. inbound adapter(TUI, Desktop, CLI, `kernel_protocol`, Gateway)는 커널 밖의 클라이언트 쪽이다 |
| 런타임 호스트 | 커널을 돌리는 장치. 한 주기(`run_host_cycle`)에서 입력을 처리하고, 타이머와 tick 을 돌리고, 결과를 나눈다 |
| 사건 연쇄 (edge-triggered) | 사건 하나가 다음 효과를 직접 내는 방식. 사건을 하나 놓치면 사슬이 끊긴다 |
| 맞추기 (reconcile, level-triggered) | 원하는 상태와 지금 상태를 비교해서 다음 행동을 고르는 방식. 같은 입력이면 같은 답이 나온다 |
| 유한 상태 기계 (FSM) | 가질 수 있는 상태와 허용된 전이를 정의한 것. "무엇이 가능한가"를 정한다. 맞추기는 "무엇을 할 것인가"를 정한다 |

## 3. 지금 구조의 진단 (2026-11-09)

### 3.1 일반적인 골격

지금 커널의 골격은 검증된 패턴의 조합이다. 방향은 맞다.

| 지금 구조 | 일반 패턴 |
| --- | --- |
| 스레드 하나(`KernelHost`)가 상태를 혼자 소유하고 입력을 차례로 처리 | 액터 모델 (Erlang `gen_server`) |
| 입력 → reducer → 이벤트와 효과 | Elm 아키텍처, Redux. 기능적 core 와 명령형 shell |
| 효과는 밖에서 실행하고 완료를 입력으로 되돌림 | 효과를 데이터로 다루기 |
| 결과를 여러 클라이언트에 나눔 | pub/sub 투영 (Jupyter 커널, LSP 서버) |

### 3.2 추출 과정의 흔적

TUI 에서 단계적으로 떼어내서(strangler) 기존 동작은 깨지 않았다. 대신 아래 흔적이 남았다. 의도한 비용이지만 정리
계획이 필요하다.

| 흔적 | 근거 | 비용 |
| --- | --- | --- |
| 루프가 세 겹이다: `KernelHost` → `run_host_cycle` → core 드라이버 | `src/composition/kernel_host.rs`, `native_client_runtime.rs:853`, `src/core/runtime/driver.rs:93` | 구조를 이해하기 어렵다 |
| 입력함이 셋(클라이언트, core, 병렬 완료)이고 50ms 마다 깨어난다 | `kernel_host.rs:19` `HOST_WAIT`, `native_client_runtime.rs:61` | 명령이 최대 50ms 늦다. 할 일이 없어도 깨어난다 |
| 타이머가 매 주기 Tick 이다 (화면 프레임 주기에서 옴) | `run_host_cycle` 의 `CoreInput::Tick`, `ParallelTick` | 타이머가 늘면 매번 모든 시계를 본다 |
| 상태 기계가 둘이다 (core 와 병렬 제어부) | K2 Q-K2-3 | 두 상태가 메시지로 맞춰야 한다 |
| 배경 세션을 `mem::swap` 으로 보는 자리와 바꿔 처리한다 | `conversation_turn_reducer.rs:266`, `:291` | 공유 상태를 건드리는 코드가 늘면 실수하기 쉽다 |
| 대화 턴은 모든 thread 를 통틀어 하나만 돈다 | `conversation_turn_reducer.rs:648`, K2 §3.3, Q-K2-2 | Akra 대화를 동시에 여러 개 돌리지 못한다 |
| 다음 행동을 사건 처리 코드 안에서 이어 붙인다 (턴 종료 → 평가 → 자동 후속, 원격 준비 완료 → 제출) | K1, K3-3b 리뷰 (TUI 취소가 원격 준비를 끊음, 병렬 모드 중복 실행), `controller.rs` 의 `ManualPromptPrepared` 처리 | 사건 하나를 놓치거나 순서가 바뀌면 사슬이 끊긴다. 재시작하면 이어지지 않는다 (원격 준비의 대상 thread 는 메모리에만 있다) |
| 준비 → 제출 규칙이 둘이다 (core 와 TUI) | `submit_prepared_thread_prompt`, TUI `apply_manual_prompt_intake_outcome` | 한쪽만 고치면 동작이 갈린다. K6 에서 TUI 쪽을 지운다 |
| 이름: 커널의 실체가 `NativeClientRuntime` 이고, `core` 가 헥사곤 밖에 있다 | plan 18 §2.3, §5.9 | 용어가 헷갈린다 |

## 4. 목표 성질

이 다섯 가지가 이 문서의 완료 기준이다. 각 성질은 시험으로 확인한다.

| ID | 성질 | 지금 | 확인 방법 |
| --- | --- | --- | --- |
| P1 | **재시작해도 이어진다.** 커널을 `kill -9` 해도 다시 켜면 하던 일(평가, 자동 후속, 승인 대기, 병렬 배분)을 정확히 이어 간다 | 판단 상태가 메모리에만 있어 대부분 잃는다 | 시뮬레이션에서 무작위 시점에 죽였다 살린다. 결과가 죽이지 않은 실행과 같다 |
| P2 | **경합을 재현할 수 있다.** 경합 결함을 시험이 같은 순서로 다시 만든다 | 리뷰어가 코드를 읽고 찾는다 | 결정적 시뮬레이션이 seed 하나로 실패를 재현한다 |
| P3 | **스스로를 설명한다.** 클라이언트(특히 AI)가 "무엇을 할 수 있나, 무슨 일이 있었나, 왜 거절됐나"를 조회한다 | `describe` 는 있다. 기록과 거절 이유 코드가 없다 | 에이전트가 문서 없이 `describe` 와 기록 조회만으로 작업을 끝낸다 |
| P4 | **동시성이 정책이다.** "대화 턴 1개"가 코드의 가정이 아니라 설정값이다 | worker 구조에 박혀 있다 | 정책 값만 바꿔 thread 두 개의 턴을 함께 돌린다 |
| P5 | **엔진이 바뀌어도 커널은 그대로다.** Codex 와 Claude 는 adapter 차이이다 | 커널 어휘가 app-server 에 가깝다 | Claude adapter(plan 17 R7)를 더할 때 커널 코드 변경이 없다 |

## 5. 설계 방향

### 5.1 기록이 진실이다 (가벼운 이벤트 기록)

- 명령을 검증한 결과를 **도메인 사건**으로 추가 전용 기록에 쓴다. 예: `TurnStarted`, `TurnCompleted`,
  `EvaluationDecided`, `ApprovalRequested`, `ApprovalResolved`, `PromptRejected { reason }`.
- 판단 상태는 그 기록의 투영이다. 투영 스냅샷은 SQLite 에 두고, 기록은 그 옆 표에 둔다 (plan 18 §5.5).
- 기록하지 않는 것: 스트림의 글자 단위 delta, 화면 상태, 대화 원본. 대화 원본은 계속 Codex 가 소유한다 (plan 17
  결정 A+). 이 기록은 Akra 의 판단 기록이지 대화 사본이 아니다.
- 얻는 것: 재시작 복구(P1), 감사(K-D4), 커서 구독으로 `Resync` 제거, "무슨 일이 있었나" 조회(P3).

### 5.2 이어 붙이지 말고 맞춘다 (FSM + reconcile)

두 단계로 나눈다.

1. **FSM:** thread 의 턴 뒤 상태를 명시적인 enum 하나로 모은다. 지금은 여러 필드와 `Option` 조합으로 흩어져
   있다. 불가능한 상태 조합을 타입이 막는다.
2. **Reconciler:** 순수 함수 `next_action(원하는 상태, 지금 상태) -> 행동 0~1개` 를 둔다. 원하는 상태는 정책(자동
   후속 켬, 최대 턴, 병렬 재개)이다. 런타임 호스트는 상태가 바뀔 때마다 이 함수를 부른다.

```text
원하는 상태 : 자동 후속 켬, 최대 3턴, 평가되지 않은 턴 없음
지금 상태   : 마지막 턴 완료, 평가 없음, 후속 1/3 사용
→ 차이: 마지막 턴이 평가되지 않았다 → 다음 행동: 평가 시작
```

- 같은 입력이면 같은 답이다(멱등). 사건을 놓쳐도 다음 비교에서 같은 차이가 보인다.
- 재시작하면 5.1 의 투영을 읽고 다시 비교하면 된다(P1).
- 타이머는 "마감 시각"이라는 상태가 된다. 호스트는 가장 가까운 마감까지만 잠든다. 매 주기 Tick 이 필요 없다.
- 첫 적용 대상: 턴 뒤 처리(평가 → 경로 결정 → 자동 후속). 결함이 가장 많았던 곳이다.

#### 5.2.1 Q2 조각 (2026-11-10)

| 조각 | 내용 | 상태 |
| --- | --- | --- |
| Q2-1 | 턴 뒤 평가·경로 단계(평가 대기, 평가 중, 경로 대기, 끝남)를 enum `PostTurnState` 하나로 모은다 (FSM 1단계). 동작은 바꾸지 않는다 | 구현됨 |
| Q2-2 | 호스트의 이벤트 사슬(`PostTurnEvaluationDue`, `PostTurnContinuationRoutingRequested` 를 꺼내 다음 단계를 부름)을 상태 비교로 바꾼다. 매 dispatch 뒤 `PostTurnState` 를 보고 다음 행동 0~1개를 고른다 | 구현됨 |
| Q2-3 | 원격 준비가 큐에 남긴 작업을 쉴 때 다시 내는 일을 같은 규칙에 넣는다 (K3 §3.2) | 구현됨 |
| Q2-4 | 입력함 셋(클라이언트, core, 병렬 완료)을 우편함 하나와 깨우기로 바꾼다 (50ms polling 제거) | 제안 |

Q2-1 결과 (`src/core/app/post_turn_state.rs`):

- 이전에는 같은 단계를 두 곳이 따로 가졌다. thread session 의 `pending_post_turn_evaluation`(평가 대기)과
  `in_flight_post_turn_evaluation`(평가 중), runtime 권한의 `snapshot.post_turn`(평가 중, 경로 대기, 끝남)과
  `pending_post_turn_route`(경로 데이터)이다. "평가 중"은 두 곳에 있었고 함께 바꿨다.
- 이제 runtime 권한이 `PostTurnState` (`Idle`, `Due`, `Evaluating`, `AwaitingRoute`, `Settled`) 하나를 갖는다.
  `snapshot.post_turn` 은 그 투영이고 `set_post_turn` 만 둘을 바꾼다. 경로 대기는 경로 데이터를 담으므로 "경로 대기인데
  데이터가 없다"는 조합이 사라졌다.
- 허용된 전이는 `PostTurnState::may_follow` 가 정한다. 상태를 바꾸는 곳은 `replace_post_turn` 하나이고, 시험 빌드에서
  허용되지 않은 전이를 `debug_assert` 로 막는다. production 경로에는 `unreachable!` 이 없다.
- 남은 턴 뒤 상태 보유자 (아직 `PostTurnState` 밖에 있다):

  | 보유자 | 위치 | 옮길 조각 (제안) |
  | --- | --- | --- |
  | 자동 후속 단계 `AutoFollowPhase` 와 `completed_auto_turns` | `conversation_runtime.rs` | Q2-2. 경로 결정(`Settled { AutoSubmit }`)과 `AutoFollowPhase::Queued` 는 같은 사실이라 함께 바뀐다. 지금은 `classify_turn_admission` 이 실행 중에 둘을 같이 확인한다 |
  | 평가 결과 무효화 gate `post_turn_continuation_gate` | `ThreadSession` | 옮기지 않는다. 상태가 아니라 worker 와 공유하는 세대 번호이다 |
  | 스트림 쪽 종료·평가 표시 (`terminal`, `last_applied_post_turn_evaluation_id`) | `TurnStreamState` (`turn_stream.rs`) | Q2-2 에서 판단한다. 평가 입장(`can_start_post_turn_evaluation`)과 완료 수락이 이 값을 본다 |

- 이전 동작을 그대로 둔 어색한 점 (Q2-2 에서 다시 본다):
  - 새 턴 시작(`begin_turn`), 대화 무효화, 평가 취소는 평가 대기를 지우지 않는다.
  - 턴 종료는 이전 평가 대기를 덮어쓴다. 호스트가 평가 대기를 바로 꺼내므로 실행 중인 커널에서는 생기지 않는다.
    호스트 없이 core 를 직접 돌리는 시험(`left_conversation_keeps_its_post_turn_lease_across_aba`)에서는 생긴다.
- 독립 리뷰 반영: 대화를 떠날 때 만드는 새 보는 자리(`detached_view`)가 runtime 권한을 복사하면서 평가 대기도
  복사했다. 이전처럼 빈 상태에서 시작하게 고쳤다 (회귀 시험
  `a_due_evaluation_follows_its_session_when_the_client_leaves_the_conversation`). 구조 시험은 새 소유 관계(runtime
  권한의 `post_turn: PostTurnState`, reducer 에 평가 상태 필드 없음)를 확인한다.
- 확인: `check_native_pr.sh`, 커널 시뮬레이션 seed 5000개가 통과했다.
- 머지 뒤 두 축 리뷰(표준, 명세)의 지적 10건 중 9건을 고쳤다: panic 없는 상태 꺼내기, 전이 검사, 비교 helper, 이름
  (`has_unsettled_work`), re-export 제거, 평가 중 덮어쓰기 때 permit 무효화, 이 문서의 남은 보유자 표. "wrapper 를
  모두 지우는" 대안은 받지 않았다 (기존 wrapper 와 맞추는 쪽이 변경이 작다).

Q2-2 결과 (`PostTurnHostStep`, `NativeClientRuntime::reconcile_post_turn`):

- 호스트는 core dispatch 마다 `next_post_turn_host_step()` 을 묻고, 고른 단계를 하고, 다시 묻는다. 단계는 모든
  session 의 `PostTurnState` 에서 나온다.

  | 상태 (모든 session) | 단계 |
  | --- | --- |
  | 평가 대기 (먼저 본다, 이전 호스트와 같은 차례) | `StartEvaluation`: 병렬 값(사용 여부, epoch)을 붙여 평가를 시작한다 |
  | 경로 대기가 하나 | `RouteContinuation`: 병렬 제어부에 묻고, 경로를 정하고, AutoSubmit 이면 자동 후속 턴을 낸다 |
  | 경로 대기가 둘 이상 | `SettleWithoutContinuation`: 모두 한 번에 이어 가지 않고 끝낸다 (병렬 제어부를 부르지 않는다) |

- 표시 이벤트는 이제 결과를 넣을 자리일 뿐이다. 단계 결과는 그 표시 자리에 들어가므로 화면이 보는 이벤트 순서는
  이전과 같다 (예: 경로 결정 결과가 planning 새로 고침 시작보다 앞). 표시가 없으면 끝에 붙인다. 표시는 클라이언트에
  보내기 전에 지운다. 경로 표시의 `execution` 값은 이제 쓰지 않는다 (지우는 일은 이벤트 정리 때 한다, 제안).
- 바뀐 동작 (의도함):
  - 사건을 놓쳐도 이어진다. 호스트를 거치지 않은 core 입력의 표시를 잃어도, 다음 dispatch(호스트 주기의 `Tick`
    포함)가 상태에서 같은 단계를 다시 본다 (`missed_post_turn_markers_are_picked_up_on_the_next_dispatch`).
  - 오래된 표시나 중복 표시는 아무것도 하지 않는다 (상태가 단계를 정한다).
  - 경로 대기가 둘 이상이면 모두 끝낸다. 이전에는 보는 thread 의 경로만 끝내고, 다른 경로는 표시를 잃어 경로 대기에
    남을 수 있었다.
- 병렬 모드 값을 core 에 알리는 중첩 dispatch(`report_parallel_mode_to_core`)는 상태 비교를 하지 않는다. 바깥 dispatch
  가 한다. 그렇지 않으면 중첩 호출이 단계를 하고 그 결과 이벤트를 버릴 수 있다 (독립 리뷰 지적).
- 확인: 모든 시험(`cargo test --locked`), 커널 시뮬레이션 seed 5000개, 결함 주입(호스트가 표시가 있을 때만 단계를
  하게 바꾸면 회복 시험이 실패한다).

Q2-3 결과 (`PostTurnHostStep::ResubmitQueuedThreadPrompt`, `PlanningFeatureReducer::queued_thread_prompts`):

- 준비가 끝났는데 제출이 거절되면(다른 턴, 끝나지 않은 정지, 지금 받을 수 없음) core 가 준비 결과(thread, 작업 공간,
  handoff, 작업 id)를 큐에 둔다. 클라이언트는 이전처럼 `queued` 와 작업 id 를 받는다.
- 같은 상태 비교에 단계 하나를 더했다. 턴 뒤 단계가 없을 때만 본다. 그래서 자동 후속 턴이 먼저 이어 간다.

  | 상태 | 단계 |
  | --- | --- |
  | 남긴 프롬프트가 있고, 준비가 없고, 병렬 모드가 꺼졌고, 모든 session 이 쉰다 | `ResubmitQueuedThreadPrompt`: 먼저 남긴 것 하나를 다시 낸다. 결과는 `submitted` |
  | 남긴 프롬프트가 있고 병렬 모드가 켜졌다 | 같은 단계. 내지 않고 `queued`, `parallel_mode_started` 로 끝낸다 (slot 이 작업을 맡는다) |
  | 병렬 모드 값을 모른다 | 기다린다 |

- 다시 낼 때 planning 투영이 그 작업을 완료나 취소로 보이면 내지 않는다. 자동 후속 턴이 큐 머리로 같은 작업을 했을 수
  있기 때문이다. 투영이 그 작업을 모르면 낸다 (확실한 신호만 막는다).
- 다시 낼 때도 거절되면 남기지 않는다. 클라이언트는 `queued` 를 한 번 더 받고 작업은 큐에 남는다 (Q2-3 이전과 같다).
  반복을 막는 값이다.
- 남긴 프롬프트는 남은 일이다 (`has_remaining_work`). 커널을 내리지 않는다. 메모리에만 있어 재시작하면 잃는다 (Q3).
- 남긴 것 (제안): 완료로 보여 버린 프롬프트는 알리지 않는다. 이유 코드(예: `task_finished`)는 Q4 스펙과 함께 더한다.
  시뮬레이션은 아직 thread 지정 프롬프트를 모델하지 않는다 (§5.5.1 남은 범위).
- 확인: core 시험 3개(다시 냄, 병렬 모드로 넘김, 완료 작업은 버림), 호스트 시험 1개(막던 턴이 끝나는 dispatch 에서
  다시 낸다), 구조 시험(입력 adapter 는 이 명령을 만들지 않는다), 커널 시뮬레이션 seed 2000개.

### 5.3 thread 를 독립 액터로 (동시성은 정책)

```text
커널 = 감독자
 ├ thread 액터 T1, T2, …   각자 FSM 과 우편함
 ├ planning 액터           큐, 방향 (저장소 공유)
 ├ 병렬 액터               슬롯, lease
 └ 자원 정책               동시 턴 수 ≤ N, 예산, 승인 규칙
```

- `mem::swap` 대신 thread 마다 상태 기계를 둔다.
- "대화 턴 1개"는 자원 정책 `N = 1` 이 된다(P4). N 을 올리려면 턴 스트림 worker 를 thread 별로 나눠야 한다
  (K2 §3.3 의 이유).
- 병렬 제어부도 같은 체계에 들어와 상태 기계 둘 문제(Q-K2-3)가 풀린다.

### 5.4 스펙 하나가 커널의 얼굴 (K4 의 중심 계약)

- 명령, 조회, 사건, 거절 이유를 **데이터로 한 번만** 정의한다.
- 거기서 Rust 타입, 응용 프로토콜, CLI `--json`, MCP 도구, Desktop TS 타입, 스킬 문서를 생성한다 (plan 18 KP-04,
  KP-05).
- 거절은 문자열이 아니라 **이유 코드와 다음 행동**이다. 예: `busy` → `next: [wait_for(thread_activity), steer]`.
  첫 조각: `prompt_submission` 이벤트의 `reason` (K3 §3.7). 다음 행동 목록은 스펙과 함께 더한다.
- 5.1 의 사건 목록이 정해진 뒤에 하는 것이 효율적이다.

### 5.5 결정적 시뮬레이션 시험

- 5.1~5.3 이 되면 커널은 "입력 → 상태 → 효과(데이터)"인 기계가 된다.
- 가짜 시계, 가짜 app-server, 가짜 git 으로 입력 순서를 무작위로 바꿔 많이 돌린다. 실패하면 seed 하나로 재현한다
  (FoundationDB, TigerBeetle 방식). 불변식 예: 같은 작업이 두 번 실행되지 않는다, 승인 결정은 한 번만 쓴다, 커널이
  죽었다 살아도 결과가 같다.
- 이것은 지금의 커널 호스트 위에서 작게 시작할 수 있다. 이후 모든 리팩터링의 안전망이다.

#### 5.5.1 Q1 첫 조각 (2026-11-10, 구현됨)

위치: `src/composition/kernel_host/simulation.rs` (단위 시험). 커널 호스트 루프, 런타임, core 는 실제 코드이고
바깥 세계만 가짜이다.

| 부품 | 방법 |
| --- | --- |
| 가짜 시계 | 호스트 루프를 `handle_pending_inputs` 와 `run_cycle(now, wait)` 로 나눴다. 시뮬레이션은 `시작 시각 + 지난 시간` 과 `wait = 0` 을 준다. 시작 시각은 실제 시각보다 하루 뒤라서, 커널이 두 시계를 섞으면 재현 시험이 드러낸다 |
| 가짜 효과 실행기 | `NativeCoreRuntime` 이 실행기를 `Box<dyn CoreEffectExecutor + Send>` 로 갖는다. 시험 실행기는 효과를 기록만 하고, 완료는 시뮬레이션이 core 입력함으로 넣는다 (실제 worker 와 같은 길) |
| 가짜 app-server | 턴 수락, 시작 알림(작업 공간, `ThreadPrepared`, `TurnStarted`), 종료 알림(완료 또는 중단), 시작 전후의 스트림 실패. 중지 요청은 성공, 실패, 무효(`InvalidateStopRequest` 뒤에는 실행하지 않고 실패)로 답하고, 성공하면 요청이 가리키는 시작된 턴만 중단한다 |
| 가짜 평가기 | 턴 뒤 평가에 `QueueAutoPrompt` 또는 `SkipAutoFollow` 를 답한다. 요청의 문맥(한도, 중지 키워드)은 일부러 보지 않아 호스트와 core 의 두 번째 방어를 시험한다 |
| 순서 | seed 로 만든 난수(SplitMix64, 알고리즘을 시험에 고정)가 행동(수동 제출, 중지, 턴 진행, 완료 돌려주기, 클라이언트 떼기·붙기, 주기)과 완료 순서를 고른다 |

| 불변식 | 내용 |
| --- | --- |
| I1 | 가짜 app-server 에는 턴이 하나만 돈다 (모든 thread 를 통틀어) |
| I2 | 완료 턴 하나는 한 번만 평가된다 |
| I3 | 자동 후속은 평가가 프롬프트를 답한 완료 턴마다 한 번이고, 수동 턴 뒤 최대 턴 수를 넘지 않는다 |
| I4 | 행동을 멈추고 바깥 세계가 남은 일을 모두 돌려주면 커널에 남은 일이 없다 (`has_remaining_work == false`) |
| I5 | 바깥 세계에서 턴이 돌거나 평가가 진행 중이면 커널은 남은 일이 있다고 답한다 (커널을 내릴지 정하는 값, K3-5) |
| 재현 | 같은 seed 는 같은 기록을 만든다 (seed 8개) |

- 실행: PR 마다 seed 24개(약 0.05초). `AKRA_SIM_SEEDS=<개수>` 로 더 돌리고, 실패 보고의 `AKRA_SIM_SEED=<seed>` 로
  다시 만든다. 5000개 seed 가 약 5초에 통과했다.
- 결함 주입 확인 (seed 24개 중 잡은 수): 자동 후속 한도 하나 늘림 → I3 (12), 단일 턴 검사 두 곳 제거 → I1 (14), 호스트가
  평가를 시작하지 않음 → I4 (23), 평가 대기를 지우지 않음 → I4 (23), 진행 중 턴을 바쁨으로 보지 않음 → I5 (24). 단일
  턴 검사 한 곳만 빼거나 평가 시작을 두 번 보내는 결함은 core 의 두 번째 방어가 막아 커널 동작이 바뀌지 않았다.
- 발견: core 의 판단은 실제 시계를 읽지 않는다. `conversation_runtime.rs` 의 `Instant::now()` 6곳은 모두 표시용
  `started_at` 이다. 호스트의 시계(붙기 때 한가한 시각, 감독 점검 재사용 60초)는 아직 실제 시각을 읽는다.
- 병렬 보기를 정하지 않는다: 실행 중인 커널은 조립 때 병렬 보기의 작업 공간을 정한다. 정하면 클라이언트가 붙을 때마다
  병렬 제어부가 감독 점검을 실제 작업 스레드(`control_plane/effect_runner.rs`)에서 돌려, 완료 시점이 seed 를 따르지
  않는다 (시도했을 때 모든 seed 가 I4 로 실패했다). 병렬 제어부의 효과를 가짜로 바꾸는 일은 병렬 모드를 모델할 때 한다.
- 남은 범위 (제안): 병렬 모드와 병렬 보기, 승인, steer, thread 지정 프롬프트(원격 준비), 대화 전환(보는 thread 와
  background session), 커널 재시작(P1), 평가 실패, 같은 알림의 중복·지연 도착, `TurnRetrying`, core 쪽 단일 턴 확인
  (효과를 받을 때 core 의 `active_turn` 도 본다), 완료 턴은 결국 평가된다(I2 의 liveness), 야간 긴 실행(Q-Q4).

## 6. 진행 순서

다시 쓰지 않는다 (plan 18 KP-01). 기존 커널을 성질 하나씩 끌어올린다.

| 순서 | 단계 | 내용 | plan 18 연결 | 성질 |
| --- | --- | --- | --- | --- |
| 1 | Q0 | K3-5·K3-6 을 먼저 끝낸다(완료 2026-11-10). 실사용 불편 목록이 이 문서의 설계를 검증한다 | K3 | - |
| 2 | Q1 | 시뮬레이션 하네스를 작게 시작한다: 가짜 시계와 가짜 app-server 로 커널 호스트를 돌리고, 입력 순서를 seed 로 섞는다. 불변식 2~3개부터. 첫 조각 완료(2026-11-10, §5.5.1) | K3 와 병행 | P2 |
| 3 | Q2 | 턴 뒤 처리를 FSM + reconcile 로 바꾼다. 원격 준비가 큐에 남긴 작업을 쉴 때 다시 내는 일도 같은 reconcile 규칙에 넣는다 (K3 §3.2). 입력함을 우편함 하나로 합치고 깨우기 방식으로 바꾼다 (50ms polling 제거). 조각은 §5.2.1, Q2-1~Q2-3 완료(2026-11-10) | K5 전 | P1 일부, P2 |
| 4 | Q3 | 사건 기록을 K5 의 기반으로 정한다. 커서 구독, 승인 객체, 우편함·깨우기, 연산 영수증을 그 위에 세운다. 타이머를 마감 시각으로 바꾼다 | K5 | P1, P3 |
| 5 | Q4 | 명령·사건·거절 이유 스펙과 생성기 | K4 | P3 |
| 6 | Q5 | thread 액터, 자원 정책(동시 턴 N), 병렬 제어부 합치기, 루프 한 겹으로 접기, 이름 정리 (`NativeClientRuntime` → `Kernel`, core 해체) | K6, plan 18 §5.9 | P4 |
| 7 | Q6 | 엔진 adapter 경계 점검: 커널 어휘를 엔진 중립으로 두고 Claude adapter 를 더한다 | plan 17 R7 | P5 |

K4 와 K5 의 순서는 plan 18 표와 반대이다. 사건 목록(K5)이 정해져야 스펙(K4)을 한 번에 만들 수 있기 때문이다
(제안, §8 Q-Q1). 단계 순서는 KP 결정이 아니므로 KP 를 바꾸지 않는다. plan 18 §6 표에 이 제안을 적었다.

## 7. 하지 않는 것

- **완전한 이벤트 소싱은 하지 않는다.** 모든 delta 를 기록하거나 기록만으로 모든 것을 재구성하지 않는다. 의미 있는
  사건만 기록하고 투영 스냅샷을 함께 둔다.
- **커널을 새로 쓰지 않는다.** 모든 단계는 기존 커널을 한 성질씩 바꾸는 변경이고, 기존 시험이 통과해야 한다.
- **대화 원본을 복사하지 않는다.** 원본은 Codex(app-server)가 소유한다.
- **교과서 용어를 위해 구조를 바꾸지 않는다.** 바꾸는 이유는 항상 성질(P1~P5) 중 하나여야 한다.

## 8. 미결 질문

| ID | 질문 | 기본값 (답이 없을 때) |
| --- | --- | --- |
| Q-Q1 | K4(스펙)와 K5(사건 기록)의 순서를 바꿀 것인가 | 바꾼다. 사건 목록을 먼저 정한다 |
| Q-Q2 | 사건 기록을 planning SQLite 와 같은 DB 에 둘 것인가 | 같은 DB 의 다른 표. 저장소 단위가 같기 때문이다 |
| Q-Q3 | 동시 턴 N 의 기본값 | 1 을 유지한다. 실사용에서 필요해지면 올린다 (Q-K2-2) |
| Q-Q4 | 시뮬레이션을 CI 에서 몇 번 돌릴 것인가 | PR 마다 짧게(고정 seed 몇 개), 야간에 길게 |

## 9. 근거

- 2026-11-09 대화: 커널 용어 정의, 루프 구조가 일반적인가에 대한 검토, reconcile 과 FSM 의 관계.
- 2026-11-09 리뷰 결과: K3-3b(TUI 취소가 원격 준비를 끊음, 병렬 모드 중복 실행), K3-4(커널을 버려도 병렬 작업
  스레드가 남음), #2194(Gateway 재시작 경합). 모두 사건 연쇄와 상태 소유가 흩어진 데서 왔다.
- 코드: §3.2 표의 위치.
