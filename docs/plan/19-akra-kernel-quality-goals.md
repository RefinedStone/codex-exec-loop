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

#### 5.1.1 Q3 조각 (2026-11-11)

Q-Q1, Q-Q2 는 기본값으로 정했다 (§8): 사건 기록(K5)을 스펙(K4)보다 먼저 하고, 기록은 planning SQLite 의 다른 표에
둔다.

기록 방식:

- core 가 판단하는 자리에서 판단 사건(`KernelEvent`)을 기록 buffer 에 쌓는다. 보지 않는 thread 의 판단도 쌓는다.
  화면용 이벤트는 보지 않는 thread 의 결과를 감추므로, 화면용 이벤트에서 사건을 거꾸로 만들지 않는다. buffer 는
  화면용 이벤트와 따로 있으므로 클라이언트에 새지 않는다 (Q3-2. Q3-1 은 사건을 화면용 이벤트에 섞었다가 호스트가
  꺼냈다).
- 호스트(`NativeClientRuntime`)가 dispatch 마다 `take_kernel_records` 로 buffer 를 비워 기록기에 넘긴다. 기록기가
  없으면(TUI) 버린다.
- 기록기는 전용 스레드에서 차례로 쓴다. 쓰기가 실패해도 커널은 멈추지 않는다 (경고 로그). 커널 스레드는 SQLite 잠금을
  기다리지 않는다.
- 표: `kernel_events(sequence, kind, thread_id, payload_json, recorded_at)`. `payload_json` 은 `KernelEvent` 의 JSON
  이다. schema version 은 올리지 않는다. 올리면 이전 binary(병렬 slot 의 akra 등)가 이 DB 를 열지 못한다. 표는 기록
  adapter 가 처음 쓰거나 읽을 때 `CREATE TABLE IF NOT EXISTS` 로 만든다.

| 조각 | 내용 | 성질 | 상태 |
| --- | --- | --- | --- |
| Q3-1 | 기록 저장소(`KernelEventLogPort`, SQLite adapter, 기록기)와 첫 사건: 남긴 thread 지정 프롬프트의 남김과 끝남(`ThreadPromptQueued`, `ThreadPromptSettled`). 커널을 조립할 때 끝나지 않은 남김을 기록에서 읽어 core 에 되돌린다 | P1 첫 조각 | 구현됨 |
| Q3-2 | 판단 사건을 더한다: 턴 제출(자동 후속 포함)·시작·끝, 턴 뒤 경로 결정, thread 지정 프롬프트 거절(이유 코드). 승인 사건은 Q3-5 로 옮겼다 | P3 | 구현됨 |
| Q3-3 | 기록 조회와 보존: `akra/kernel/query` 의 `history { after, limit }`, 남김 종류가 아닌 사건은 최근 10,000개만 남긴다 | P3 | 구현됨 |
| Q3-3b | 커서 구독: 커널이 기록 sequence 를 정하고 `recorded` 이벤트로 실시간으로 보낸다. 넘친 클라이언트는 마지막 sequence 뒤를 `history` 로 따라잡는다. 턴 끝의 종류(완료, 중단, 실패, 모름)를 terminal receipt 에서 더한다 | P3 | 구현됨 |
| Q3-4 | 타이머를 마감 시각으로 바꾼다. core 와 병렬 제어부가 다음 마감을 알리고, 호스트는 그때까지 잔다. Q2-4 의 250ms 주기 깨우기를 없앤다 | P2 | 구현됨 |
| Q3-5 | 승인 객체: 승인 요청과 결정을 사건으로 기록하고(Q3-2 에서 옮김), 재시작 뒤 기다리는 승인을 보인다 | P1, P3 | 제안 |
| Q3-6 | 연산 영수증(`op_id`, 상태, `next`)과 멱등 키 | P3 | 제안 |
| Q3-7 | 시뮬레이션에 재시작을 모델한다: 무작위 시점에 커널을 버리고 기록으로 다시 조립한다 | P1, P2 | 구현됨 (기록을 다 쓴 뒤의 재시작) |

Q3-1 결과 (`KernelEvent`, `KernelEventLogPort`, `KernelEventRecorder`, `AppCommand::RestoreQueuedThreadPrompts`):

- core 는 남긴 프롬프트(Q2-3)를 큐에 넣을 때 `ThreadPromptQueued` 를 낸다. 다시 내거나(`submitted`), 병렬 slot 에
  맡기거나(`parallel_slots`), 완료 작업이라 버리거나(`task_finished`), 다시 거절될 때(`rejected_again`)
  `ThreadPromptSettled` 를 낸다. 둘은 작업 id 로 짝을 맞춘다. 남긴 프롬프트 타입은 domain 으로 옮겼다
  (`domain::kernel_event::QueuedThreadPrompt`). core 의 큐와 기록이 같은 값을 쓴다.
- 기록기와 복구는 화면 없는 커널(`build_kernel_for_workspace`)에만 붙인다. thread 지정 프롬프트는 커널 프로토콜
  클라이언트만 보내므로 남김은 그 커널에서만 생긴다. TUI 런타임은 판단 사건을 버린다.
- 커널을 조립할 때 저장소 잠금을 잡은 뒤 그 작업 공간의 기록을 읽는다. 끝나지 않은 남김을 `RestoreQueuedThreadPrompts`
  로 core 에 넣고, core 는 Q2-3 과 같은 규칙으로 쉴 때 한 번 다시 낸다. 기록을 읽지 못하면 되돌리지 않고 기록만 했다.
  Q3-3b 부터는 기록을 붙이지 않는다 (sequence 를 모르면 쓸 때 부딪힌다).
- 기록기는 공유 worker 경계(`spawn_worker_with_panic_fallback`)의 스레드에서 쓴다. 런타임을 버리면 남은 사건을 다 쓰고
  끝난다. 커널은 그 뒤에 저장소 잠금을 풀므로 다음 커널은 모든 사건을 읽는다.
- 남김과 끝남 사이에 프로세스가 죽으면, 다시 조립할 때 그 프롬프트를 한 번 더 낸다. 그 사이에 작업이 끝났으면 planning
  투영이 완료를 보일 때만 막는다 (Q2-3 의 확실한 신호 규칙). 드문 중복 실행 위험이다.
- 복구는 그 작업 공간의 사건을 모두 읽었다. Q3-2 에서 남김 종류(`THREAD_PROMPT_QUEUE_KINDS`)만 읽게 바꿨다.
- 확인: domain 시험 2개(끝나지 않은 남김, JSON 왕복), adapter 시험 1개(순서 유지, 모르는 사건 건너뜀), 기록기 시험
  1개(버릴 때 다 씀, 실패한 쓰기를 넘김), core 시험(복구 뒤 다시 냄, Q2-3 시험 3개에 기록 확인), 호스트 시험 1개(새 커널이
  기록에서 끝나지 않은 남김만 다시 내고 그 끝남을 기록), 구조 시험(입력 adapter 는 복구 명령을 만들지 않는다), 커널
  시뮬레이션 seed 2000개, Windows Rust gate.

Q3-2 결과 (`TurnSubmitted`, `TurnStarted`, `TurnEnded`, `PostTurnRouted`, `ThreadPromptRejected`):

| 사건 | 내는 자리 | 내용 |
| --- | --- | --- |
| `turn_submitted` | `handle_input` 의 출구. 결과의 `SubmitTurn` 효과 | thread id(새 대화는 빈 값), 제출 세대 번호, 낸 쪽(`manual`, `manual_intake`, `auto_follow`) |
| `turn_started`, `turn_ended` | `handle_input` 의 출구. thread 요약(`ThreadActivityChanged`)의 이전 값과 새 값의 차이 | thread id, 턴 id. 끝을 시작보다 먼저 낸다 |
| `post_turn_routed` | `ResolvePostTurnContinuation` 처리 | thread id, 완료 턴 id, 경로(`auto_submit`, `parallel_consumed`, `no_continuation`) |
| `thread_prompt_rejected` | thread 지정 프롬프트의 받아들임 | thread id, 이유(`busy`, `preparation_active`, `unknown_thread`, `parallel_mode`, `parallel_mode_unknown`) |

- 모든 제출 경로(사람, intake, thread 지정, 자동 후속)가 `SubmitTurn` 효과를 내므로 제출은 한 자리에서 기록한다.
  `handle_input` 은 자기를 다시 부르고(자동 후속 제출) 바깥 결과가 안쪽 효과를 담는다. 그래서 이미 쌓은 제출은 다시
  쌓지 않는다.
- 턴의 시작과 끝은 thread 요약에서 얻는다. 요약은 보지 않는 thread 를 담으므로 모든 thread 의 턴을 본다. 끝의 종류(완료,
  중단, 실패)는 아직 가르지 않았다. Q3-3b 에서 더했다.
- buffer 는 `ConversationTurnFeatureReducer` 에 둔다. `CoreController` 는 typed slice 8개만, `AppState` 는 스냅샷 필드
  하나만 가지도록 구조 시험이 막기 때문이다. 이 reducer 는 클라이언트에 알릴 thread 요약도 가진다.
- 기록기는 커널의 작업 공간 하나에 쓴다. 복구도 그 작업 공간에서 읽는다. Q3-1 은 사건마다 그 사건의 작업 공간에 썼다.
- 기록은 지우지 않고 늘어났다. Q3-3 에서 보존 한도를 정했다.
- 확인: domain 시험(턴 차이, 모든 종류의 JSON 왕복), adapter 시험(종류 거르기), 기록기 시험(실패한 묶음 다음 묶음을
  씀), core 시험 1개(제출·시작·끝·경로·거절이 buffer 에만 쌓임), 호스트 시험(복구 뒤 다시 낸 제출과 끝남을 기록),
  시뮬레이션 불변식 I6(아래), 커널 시뮬레이션 seed 2000개, Windows Rust gate. 결함 주입: 제출 중복 거르기를 빼면 I6 가
  seed 24개 모두에서 잡는다.

Q3-3 결과 (`KernelQuery::History`, `KernelEventLogPort::load_kernel_events_after`):

- 클라이언트는 `akra/kernel/query` 에 `{ "type": "history", "after": <cursor>, "limit": <n> }` 를 보낸다. 처음에는
  `after` 를 비운다 (0). 답은 `{ "type": "history", "records": [...], "next_after": <cursor> }` 이다. 사건 하나는
  `sequence`, `recorded_at`, `event` 이고, `event.kind` 가 종류이다. `limit` 의 기본값은 100, 한도는 500 이다.
- 다음 조회는 답의 `next_after` 를 `after` 로 보낸다. 이 binary 가 모르는 사건은 건너뛰지만 cursor 는 그 뒤로 간다. 더
  읽을 것이 없으면 받은 cursor 를 그대로 돌려준다. 기록이 없는 커널(TUI, 시험)은 빈 쪽을 돌려준다.
- 조회는 Gateway 접속에서 커널 스레드로 가고, 커널 스레드가 기록을 읽는다 (`KernelHostInput::History`). 기록기는 따로
  쓰므로 방금 낸 사건은 아직 없을 수 있다.
- 보존: 사건을 쓸 때 남김 종류(`thread_prompt_queued`, `thread_prompt_settled`)가 아닌 사건은 최근 10,000개만 남긴다
  (`RETAINED_KERNEL_EVENTS`). 남김 종류는 재시작 복구에 필요해서 지우지 않는다. 지워진 범위보다 오래된 cursor 는 남은 가장
  오래된 사건부터 받는다.
- shortcut: 기록을 커널 스레드에서 읽는다 (한 쪽은 최대 500개). 읽기가 느려지면 읽기 전용 연결로 옮긴다.
- Desktop 은 아직 이 조회를 쓰지 않는다 (`desktop/src/renderer/kernel/protocol.ts` 에 타입이 없다).
- 확인: adapter 시험(cursor 와 쪽 크기, 모르는 사건을 건너뛴 cursor, 보존 한도와 남김 보존), 프로토콜 시험(조회 해석,
  답의 JSON 모양, 기록 없는 커널의 빈 쪽, `describe` 의 `history`), 커널 호스트 시험(쪽마다 읽기), Gateway 시험(접속을
  거친 조회), 구조 시험(facade 의 읽기 전용 `kernel_event_history`), Windows Rust gate.

Q3-3b 결과 (`KernelEvent::Recorded`, `KernelHostEvent::Recorded`, `TurnEndOutcome`):

- 저장소 커널은 저장소 잠금으로 하나뿐이고, 그 저장소의 `kernel_events` 를 쓰는 것도 그 커널뿐이다. 그래서 커널이
  sequence 를 정한다. 기록을 붙일 때 마지막 sequence(`last_kernel_event_sequence`)를 읽고, dispatch 마다 꺼낸 사건에
  다음 수와 시각을 붙인다. 기록기는 그 값 그대로 쓴다. 실시간 이벤트와 기록이 같은 sequence 를 가진다.
- 커널 호스트는 dispatch 결과를 나눈 뒤 번호를 붙인 기록을 꺼내(`take_published_kernel_records`) 모든 클라이언트에
  보낸다. 프로토콜 이벤트는 `{ "type": "recorded", "sequence", "recorded_at", "event": { "kind", ... } }` 이다.
- 따라잡기: 클라이언트는 받은 `sequence` 를 기억한다. `resync` 를 받거나 다시 붙으면 그 값을 `after` 로 `history` 를
  읽는다. 붙은 뒤의 실시간 이벤트와 `history` 가 겹칠 수 있으므로 `sequence` 로 겹침을 지운다. attach 에 cursor 를
  넣는 방식은 쓰지 않았다. 붙은 뒤 한 번 읽는 것으로 같다.
- 쓰기가 실패하면 클라이언트가 이미 받은 sequence 가 기록에 없을 수 있다 (경고 로그). 다음 사건은 다음 수를 쓴다.
- 턴 끝: 그 dispatch 의 입력이 terminal receipt(완료, 중단, 실패, 모름)나 스트림 실패(실패)이면 `turn_ended` 에
  `outcome` 을 붙인다. 대화 무효화처럼 다른 일로 요약에서 빠진 턴은 비운다. 턴은 한 번에 하나만 돌므로 한 입력이 끝낼
  수 있는 턴은 하나이다. 이전 기록(`outcome` 없음)도 그대로 읽힌다.
- Desktop 은 모르는 이벤트 종류를 넘긴다 (`kernelConnection.ts` 의 `default`). 이 이벤트를 그리는 일은 아직 없다.
- 확인: domain 시험(이전 기록 읽기, `outcome` JSON), adapter 시험(명시한 sequence, 마지막 sequence 읽기), 프로토콜
  시험(`recorded` JSON 모양, `describe`), 커널 호스트 시험(기록의 마지막 41 뒤로 42 를 실시간으로 받음), core 시험(완료한
  턴의 `outcome`), 시뮬레이션 I6 에 sequence 검사(1 부터 빠짐없이 늘어남), seed 2000개, Windows Rust gate.

Q3-4 결과 (`next_deadline`, `KernelHostLoop::next_wait`, `run_host_cycle_with_clock`):

- 커널 호스트는 250ms 마다 깨지 않는다. 다음 마감 시각까지만 자고, 마감이 없으면 입력이 깨울 때까지 잔다.

  | 시계 일 | 마감이 있는 조건 | 마감 |
  | --- | --- | --- |
  | GitHub 리뷰 폴링 (core) | 대상과 설정이 있고 폴링이 진행 중이 아니다 | 예약된 시각 (설정 직후는 바로) |
  | 배분 정리 재시도 (병렬 제어부) | 끝나지 않은 정리가 있고 제어 효과가 진행 중이 아니다 | 마지막 재시도 + 1초 |
  | 감독 새로 고침 (병렬 제어부) | 감독 화면을 보는 클라이언트가 있고, epoch 가 열려 있고, 진행 중인 제어 효과가 없다 | 마지막 새로 고침 + 1초 |
  | 입력 상한에 걸린 주기 | 한 주기가 입력 상한(64개)에서 멈췄다. 입력함에 남은 입력은 호스트를 다시 깨우지 않는다 | 바로 |
  | 배분 깨움 확인 (병렬 제어부) | 병렬 모드가 켜져 있고 epoch 가 열려 있고 투영이 준비되었다 | 마지막 확인 + 1초 |

- 규칙: 마감은 `tick` 이 실제로 일을 할 수 있을 때만 낸다. 진행 중인 효과 때문에 `tick` 이 기다리는 일은 마감에서
  뺀다. 그 효과의 완료 입력이 호스트를 깨운다. 마감이 있는데 `tick` 이 시각을 넘기지 않으면 호스트가 쉬지 않고 돈다.
  시험이 "마감 시각의 tick 뒤에는 마감이 없거나 뒤로 간다"를 확인한다.
- 250ms 주기에 숨어 있던 의존 넷을 찾아 고쳤다.
  1. 시계를 기다리기 전에 읽었다. 이제 기다린 뒤에 읽는다 (`run_host_cycle_with_clock`). 오래 잔 뒤에도 `Tick` 의
     시각이 맞다.
  2. 넘친 클라이언트의 `Resync` 재시도가 다음 주기에 기댔다. `Resync` 가 남아 있으면 250ms 만 잔다
     (`HOST_RESYNC_RETRY_WAIT`).
  3. 한가해진 시각을 주기 끝에 기록했다. 이제 자기 전에 기록한다. 그렇지 않으면 오래 자는 동안 커널을 내릴 수 없다.
  4. 병렬 모드 값은 주기마다 다시 읽었다. 확인하니 이 값은 커널 스레드만 바꾼다 (명령과 완료 이벤트 처리 안). 그래서
     바꿀 것이 없다.
- 안전망: 마감이 없어도 30초에 한 번은 깬다 (`HOST_IDLE_WAIT`). 마감을 알리지 않는 시계 일이 생겨도 그 일이 멈추지
  않는다. 주기가 아니라 방어선이다.
- 달라진 점: GitHub 리뷰 폴링의 간격은 "완료 직전의 tick" 에서 잰다. 이전에는 tick 이 250ms 마다 있어 거의 완료
  시각이었다. 이제 폴링 중에는 tick 이 없으므로 폴링을 시작한 시각에서 잰다. 폴링이 걸린 시간만큼 간격이 짧아진다.
- 클라이언트가 붙어 있어도 epoch 가 없으면 깨지 않는다 (새로 고침 명령이 버려지므로 마감을 내지 않는다). 병렬 모드의
  epoch 가 열려 있는 동안은 1초마다 깬다 (이전 250ms).
- `Tick` 은 마감이 없어도 주기마다 넣는다. 그 dispatch 가 병렬 완료 뒤의 병렬 모드 값을 core 에 알리고 턴 뒤 단계를
  다시 본다.
- TUI 와 시뮬레이션은 바뀌지 않는다. 둘 다 기다림 0 으로 주기를 부른다.
- 시뮬레이션은 이제 실제 호스트처럼 돈다: 입력이 깨웠거나 마감 시각이 되었을 때만 주기를 돈다. 30초 안전망은 흉내
  내지 않는다. 그래서 깨우지도 않고 마감도 내지 않는 일이 커널에 있으면 I4 가 잡는다. seed 5000개가 통과했다.
- 안전망에 기대는 드문 경우 (독립 리뷰, 고치지 않음): 작업 스레드가 마지막 이벤트를 보낸 뒤에 실행 수를 줄이므로
  한가함 기록이 최대 30초 늦을 수 있다. core 입력함의 `notify_one` 이 기다리는 보내는 쪽으로 갈 수 있다 (보내는
  스레드가 17개 넘게 막혀야 한다). 턴 뒤 단계가 한 dispatch 의 상한(8)을 넘으면 다음 깨움에서 돈다.
- 남은 일 (제안): GitHub 리뷰 폴링이 커널로 오면 간격의 기준을 완료 시각으로 맞춘다.
- 확인: core 시험 1개(마감 조건과 tick 뒤 이동), 병렬 제어부 시험 1개(한가하거나 epoch 가 없으면 마감 없음, tick 뒤
  진행 중에는 없음, 완료 뒤 1초), 런타임 시험 1개(입력 상한에 걸리면 바로, 비우면 없음), 커널 호스트 시험 2개(한가하면
  30초, `Resync` 가 남으면 250ms; 시계를 기다린 뒤에 읽음), 기존 한가함 시험(주기 끝 기록이면 실패).

Q3-7 결과 (`src/composition/kernel_host/simulation.rs`, 시험만 바꿨다):

- 시뮬레이션이 thread 지정 프롬프트를 다룬다: 클라이언트가 세션 목록의 thread 에 프롬프트를 보내고, 가짜 planner 가
  준비(작업 하나와 실행할 프롬프트)를 답한다. 다른 턴이 도는 동안 준비가 끝나면 커널은 프롬프트를 큐에 남기고 쉴 때
  다시 낸다 (Q2-3).
- 시뮬레이션이 무작위 시점에 커널을 다시 켠다 (seed 의 반, 최대 2번). 커널을 버려 기록기가 남은 사건을 다 쓰게 한
  뒤, 같은 기록으로 새 커널을 조립한다. 이전 커널의 진행 중 턴과 완료는 바깥 세계에서도 버린다. 지금 커널은 그 일을
  기록에 남기지 않기 때문이다 (P1 의 남은 부분: 턴, 평가, 승인, 준비 중인 thread 지정 프롬프트). 준비 중에 다시 켜면
  받아들인 프롬프트를 잃고, 작업만 planning 큐에 남는다. 실제 Gateway 도 커널을 멈춘 뒤 app-server 를 멈춘다.
- 불변식 I7: thread 지정 프롬프트는 커널을 다시 켜도 app-server 에 많아야 한 번, 자기 thread 에만 닿는다. 끝에
  기록에 열린 남김이 없고, "다시 내서 끝남"으로 기록한 남김은 app-server 가 정확히 한 번 받았다. 잃은 프롬프트(준비
  중의 재시작, 다시 거절됨)는 이 불변식이 잡지 않는다.
- 가짜 중지는 실제 adapter 처럼 지금 도는 턴을 중단한다 (보지 않는 thread 의 턴 포함). 이전에는 중지 요청이 가리킨
  턴만 중단했다.
- 가짜 클라이언트의 수동 턴을 TUI 처럼 고쳤다: 보는 대화를 이어 가거나, 새 대화를 열고 낸다 (준비 취소를 먼저
  보낸다. 그 취소가 thread 지정 프롬프트의 준비에 닿으면 위반이다, K3-3b 의 회귀). 이전에는 보지 않는
  thread 에도 냈다. 실제 클라이언트는 그렇게 하지 않는다 (TUI 는 대화를 먼저 열고, 커널 프로토콜에는 이 명령이 없다).
  thread 지정 프롬프트와 섞이면 같은 thread 에 session 이 둘 생겼다.
- 기존 불변식을 여러 thread 에 맞췄다. I3 은 자동 후속 수를 thread 마다 센다 (한도는 session 마다 따로이다). I5 는
  커널이 무효로 만든 평가를 세지 않고 (중지하면 커널은 평가를 버리고 그 worker 의 결과를 받지 않는다), 준비 중인
  thread 지정 프롬프트를 남은 일로 센다. I6 은 제출
  번호를 커널마다 따로 본다 (새 커널은 1 부터 다시 센다). sequence 는 재시작을 넘어 1 부터 빠짐없이 늘어나야 한다.
- 결함 주입 (seed 1000개): 되돌리지 않음 → I7 "큐에 남은 채 끝남" (41개), 두 번 되돌림 → I7 "2번 닿음".
- 도달: 큐에 남은 채 재시작해 되돌리는 경로는 정해진 순서의 시험
  (`kernel_simulation_restores_a_queued_prompt_after_a_restart`)이 항상 지난다. 되돌리지 않는 결함을 넣으면 이 시험이
  실패한다. 무작위 시나리오에서는 seed 의 몇 % 에서 나온다. 그 경우를 늘리려고 준비 중에는 턴을 먼저 시작하고,
  프롬프트가 큐에 남았을 때 재시작을 더 자주 고른다.
- 확인: seed 5000개 통과.

시뮬레이션이 찾은 커널 결함 (고치지 않았다, 제안):

- **중지가 끝나기 전에 새 대화를 열면 커널이 돌고 있는 턴을 잊는다.** 순서: 턴이 도는 중에 `RequestStopAllSessions`,
  그 중지가 끝나기 전에 `InvalidateConversationLoad`. `reduce_conversation_invalidation` 은 중지가 진행 중인
  session 을 뒤로 보내지 않고 그 자리에서 지운다 (`active_turn` 을 비우고 중지 요청을 무효로 만든다). 그 턴은
  app-server 에서 끝까지 돌고, 커널은 남은 일이 없다고 답하며, 다른 턴을 함께 시작할 수 있다. 다른 대화를 여는
  `LoadConversation` 은 같은 경우에 중지가 끝날 때까지 미룬다 (`Deferred`).
- 재현: `submit_manual_turn` 의 `may_leave` 조건을 빼면 seed 1000개 중 26개가 I5(일이 있는데 없다고 답함) 또는
  I1(두 번째 턴)로 실패한다. 예: seed 15 (I5), seed 496 (I1).
- 이 순서는 TUI 에서만 가능하다 (커널 프로토콜에는 `InvalidateConversationLoad` 가 없다). 실제 가능성은 낮다. 중지
  worker 는 곧 끝나므로 그 사이에 새 대화를 열어야 한다. 실제로는 중지가 이미 나갔으면 턴은 중단되고, 커널만 그 턴을
  잊는다. 시뮬레이션은 이 순서를 피한다 (끝나지 않은 중지 시도가 있을 수 있으면 새 대화를 열지 않는다).
  고치면 그 조건을 지운다. 고치는 방향: 새 대화도 `LoadConversation` 처럼 중지가 끝날 때까지 미루거나, 중지가 진행
  중인 session 도 뒤로 보낸다.
- 커널은 보지 않는 thread 에 낸 `SubmitTurn` 을 막지 않는다. 같은 thread 에 session 이 둘이 되면 뒤의 session 의 자동
  후속 취소가 보는 session 에 가서, 뒤의 session 이 `Queued` 로 남고 커널이 한가해지지 않는다. 지금 클라이언트는
  이 입력을 보내지 않는다. 막는 일은 명령 스펙(Q4)에서 정한다.

커널 관찰 (확인하지 않음, 제안): 되돌린 프롬프트는 기록을 붙이는 dispatch 안에서 바로 다시 나간다. 그때 planning
투영이 비어 있어 "완료한 작업은 내지 않는다"는 확인(Q2-3)이 걸리지 않을 수 있다. 가짜 planner 는 투영을 답하지 않아
시뮬레이션이 이것을 보지 못한다.

모델하지 않은 것 (제안): 가짜 planner 의 다른 응답(준비 실패, 글 불일치, 완료한 작업), 보는 thread 에 온 thread 지정
프롬프트, worker 가 요청을 보내기 전에 끝난 종료, 기록을 다 쓰지 못한 강제 종료. 남김과 끝남 사이에 죽으면 프롬프트를 한 번 더 낸다 (Q3-1 의
알려진 중복 위험). 이것을 보려면 기록기의 쓰기를 가짜로 바꿔야 한다.

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
| Q2-4 | 입력함 셋(클라이언트, core, 병렬 완료)을 우편함 하나와 깨우기로 바꾼다 (50ms polling 제거) | 구현됨 (깨우기만 하나로, 아래) |

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
  시뮬레이션은 Q3-7 부터 thread 지정 프롬프트를 모델한다.
- 확인: core 시험 3개(다시 냄, 병렬 모드로 넘김, 완료 작업은 버림), 호스트 시험 1개(막던 턴이 끝나는 dispatch 에서
  다시 낸다), 구조 시험(입력 adapter 는 이 명령을 만들지 않는다), 커널 시뮬레이션 seed 2000개.

Q2-4 결과 (`CoreInputWaker`, `KernelInbox`):

- 설계와 다른 점: 입력함 셋은 합치지 않았다. core 입력함은 자리 제한(보내는 쪽이 기다림)과 진행 표시 병합 규칙을
  가진다. 클라이언트 입력과 병렬 완료를 그 안에 넣으면 그 규칙을 다시 정해야 한다. 대신 깨우기를 하나로 했다.
- 커널 스레드는 전과 같이 core 입력함에서만 기다린다. `CoreInputWaker::wake` 는 입력 없이 그 기다림을 끝낸다.
  기다리기 전에 깨워도 표시가 남아 잃지 않는다. 깨우기는 보내는 쪽 수에 들지 않아 입력함의 끊김 판단을 바꾸지 않는다.
- 클라이언트 입력함(`KernelInbox::send`)과 병렬 완료(`NativeParallelModeControlPlaneEventSink`)는 넣은 뒤 깨운다.
  core 입력은 이전부터 condvar 로 깨웠다.
- (Q3-4 에서 없앴다.) 기다림의 최대 시간(`HOST_WAIT`)은 50ms 에서 250ms 로 늘렸다. 이제 입력이 아니라 시계 일(`Tick`, 병렬 tick)의
  주기이다. 그 일의 간격(GitHub 리뷰 폴링, 감독 새로 고침, 배분 깨움 확인, 정리 재시도)은 1초 이상이다. 일이 없어도
  이 주기로 깨는 것은 남는다. 타이머를 마감 시각으로 바꾸는 Q3 에서 없앤다.
- TUI 는 자기 화면 루프에서 `run_host_cycle(now, 0, …)` 를 부르므로 바뀌지 않는다. 시뮬레이션도 기다림 0 을 쓴다.
- 확인: 입력함 시험 2개(입력 없이 깨움, 먼저 깨운 것을 잃지 않음, 깨우기가 끊김을 막지 않음), 커널 호스트 시험
  1개(명령 10개가 `HOST_WAIT * 4` 안에 끝난다). 결함 주입: 클라이언트 입력함의 깨우기를 빼면 명령 10개가 2.59초
  걸려 실패한다.

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
| I3 | 자동 후속은 평가가 프롬프트를 답한 완료 턴마다 한 번이고, thread 마다 사람이 낸 턴 뒤 최대 턴 수를 넘지 않는다 |
| I4 | 행동을 멈추고 바깥 세계가 남은 일을 모두 돌려주면 커널에 남은 일이 없다 (`has_remaining_work == false`) |
| I5 | 바깥 세계에서 턴이 돌거나 평가가 진행 중이면 커널은 남은 일이 있다고 답한다 (커널을 내릴지 정하는 값, K3-5) |
| I6 | 판단 사건 기록이 맞다: 같은 턴 제출은 한 번만 기록되고, 같은 턴은 한 번 시작하고 한 번 끝나며 끝은 시작 뒤에 온다 (Q3-2). sequence 는 1 부터 빠짐없이 늘어난다 (Q3-3b). 시뮬레이션은 커널에 메모리 기록을 붙이고, 커널을 버린 뒤 읽는다 |
| I7 | thread 지정 프롬프트는 커널을 다시 켜도 app-server 에 많아야 한 번, 자기 thread 에만 닿는다. 끝에 기록에 열린 남김이 없고, "다시 내서 끝남"으로 기록한 남김은 app-server 가 정확히 한 번 받았다 (Q3-7) |
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
- (Q3-7 에서 thread 지정 프롬프트, 새 대화 열기, 기록을 다 쓴 뒤의 커널 재시작을 더했다.)
- 남은 범위 (제안): 병렬 모드와 병렬 보기, 승인, steer, 다른 대화 열기(`LoadConversation`), 기록을 다 쓰지 못한
  강제 종료(P1), 평가 실패, 같은 알림의 중복·지연 도착, `TurnRetrying`, core 쪽 단일 턴 확인
  (효과를 받을 때 core 의 `active_turn` 도 본다), 완료 턴은 결국 평가된다(I2 의 liveness), 야간 긴 실행(Q-Q4).

## 6. 진행 순서

다시 쓰지 않는다 (plan 18 KP-01). 기존 커널을 성질 하나씩 끌어올린다.

| 순서 | 단계 | 내용 | plan 18 연결 | 성질 |
| --- | --- | --- | --- | --- |
| 1 | Q0 | K3-5·K3-6 을 먼저 끝낸다(완료 2026-11-10). 실사용 불편 목록이 이 문서의 설계를 검증한다 | K3 | - |
| 2 | Q1 | 시뮬레이션 하네스를 작게 시작한다: 가짜 시계와 가짜 app-server 로 커널 호스트를 돌리고, 입력 순서를 seed 로 섞는다. 불변식 2~3개부터. 첫 조각 완료(2026-11-10, §5.5.1) | K3 와 병행 | P2 |
| 3 | Q2 | 턴 뒤 처리를 FSM + reconcile 로 바꾼다. 원격 준비가 큐에 남긴 작업을 쉴 때 다시 내는 일도 같은 reconcile 규칙에 넣는다 (K3 §3.2). 입력함을 우편함 하나로 합치고 깨우기 방식으로 바꾼다 (50ms polling 제거). 조각은 §5.2.1, Q2-1~Q2-4 완료(2026-11-10, Q2-4 는 깨우기만 하나로 했다) | K5 전 | P1 일부, P2 |
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
| Q-Q1 | K4(스펙)와 K5(사건 기록)의 순서를 바꿀 것인가 | 바꾼다. 사건 목록을 먼저 정한다. **기본값 적용 (2026-11-11, Q3)** |
| Q-Q2 | 사건 기록을 planning SQLite 와 같은 DB 에 둘 것인가 | 같은 DB 의 다른 표. 저장소 단위가 같기 때문이다. **기본값 적용 (2026-11-11, `kernel_events`)** |
| Q-Q3 | 동시 턴 N 의 기본값 | 1 을 유지한다. 실사용에서 필요해지면 올린다 (Q-K2-2) |
| Q-Q4 | 시뮬레이션을 CI 에서 몇 번 돌릴 것인가 | PR 마다 짧게(고정 seed 몇 개), 야간에 길게 |

## 9. 근거

- 2026-11-09 대화: 커널 용어 정의, 루프 구조가 일반적인가에 대한 검토, reconcile 과 FSM 의 관계.
- 2026-11-09 리뷰 결과: K3-3b(TUI 취소가 원격 준비를 끊음, 병렬 모드 중복 실행), K3-4(커널을 버려도 병렬 작업
  스레드가 남음), #2194(Gateway 재시작 경합). 모두 사건 연쇄와 상태 소유가 흩어진 데서 왔다.
- 코드: §3.2 표의 위치.
