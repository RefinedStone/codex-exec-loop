# K3 상세 설계: 커널 데몬과 응용 프로토콜

## 문서 상태

- 상태: **완료 (2026-11-10)**, K3-1~K3-6. 남긴 것은 §3.7 과 각 단계의 "설계와 다른 점"에 있다. §3.2 는 2026-11-08 에 개정했다 (클라이언트별 보기 대신
  thread 를 지정하는 명령). §3.4 는 2026-11-09 에 개정했다 (커널 단위를 폴더에서 저장소로). §1, §3.1, §3.3 은
  2026-11-09 K3 리뷰 두 번 뒤 v0 코드에 맞췄다 (§3.7). K3-5 리뷰 반영(2026-11-10)과 커널 내리기 정책은 §3.7 에
  적었다. 현재 제품 동작을 설명하지 않는다.
- 상위 계획: [18 Akra 커널 아키텍처](18-akra-kernel-architecture.md) §4.1, §4.4, §5.3~§5.6, §6 K3. 결정
  KP-03, KP-04, KP-06 (승인 2026-11-07). Q-K3, Q-K6 답 (2026-11-07).
- 이전 단계: [K1](18-k1-headless-decision-loop.md), [K2](18-k2-thread-sessions.md) (완료 2026-11-08).
  K2-5(클라이언트 상태 타입 분리)를 이 단계로 옮겼다가, §3.2 개정 때 K6 으로 다시 옮겼다.
- 관련 계획: [17 Akra Desktop](17-akra-desktop/README.md) D-11 (얇은 Gateway, `akra/*` 이름 공간).
- 기준일: 2026-11-08 (Asia/Seoul)
- 기준 커밋: `70c99aed` (`origin/prerelease`).
- 범위: 커널(지금의 `NativeClientRuntime` + core)을 화면 프로세스 밖의 데몬에서 돌리고, 클라이언트가 응용
  프로토콜로 명령·조회·구독한다. 명령 경로 통일(K4), 이벤트 기록·커서·승인 객체(K5), 얇은 TUI(K6)는 범위
  밖이다.

## 1. 목표

1. 커널이 화면 없이 상주한다. 클라이언트가 없어도 턴, 평가, 자동 후속, 병렬 작업, PR 검증이 돈다.
2. 클라이언트 여럿이 같은 커널에 붙는다. 각자 thread 를 지정해 명령한다 (§3.2). 명령을 바로 거절하면 그 이유는
   요청한 클라이언트의 명령 응답으로 간다. 그 밖의 결과 이벤트는 모든 클라이언트에 간다. thread 에 대한 이벤트는
   thread id 를 담는다. 요청한 클라이언트만 받는 이벤트 대상 지정은 클라이언트별 보기와 함께 K6 에서 한다.
3. 모든 원격 클라이언트는 하나의 응용 프로토콜(JSON-RPC)을 쓴다. 첫 원격 클라이언트는 Desktop 이다.
4. 같은 저장소에서 커널이 둘 돌지 않는다 (K-D3, K-D5 해소). 저장소의 worktree 들은 한 커널을 쓴다 (§3.4).

## 2. 현재 상태 (2026-11-08 코드 조사)

### 2.1 이미 있는 것

| 항목 | 위치 | K3 에서 쓰는 방법 |
| --- | --- | --- |
| 사용자별 상주 데몬 `akra-gateway` | `src/bin/akra-gateway.rs`, `src/adapter/inbound/gateway/` | 커널을 이 프로세스에 올린다 |
| 로컬 전용 endpoint: unix `$AKRA_HOME/run/gateway.sock`(0600), Windows named pipe(소유자 전용, 원격 거절) | `gateway/listener.rs` | 그대로 쓴다. TCP 는 열지 않는다 |
| 접속마다 WebSocket JSON-RPC, `akra/*` 는 Gateway 가 직접 답한다 | `gateway/router.rs`, `session.rs` | `akra/kernel/*` 를 더한다 |
| 화면 없이 도는 런타임 호스트 `run_host_cycle(now, wait, max_inputs)` | `composition/native_client_runtime.rs` (K1-6) | 데몬의 커널 스레드가 `wait > 0` 으로 돌린다 |
| thread 별 session, 보지 않는 thread 의 결과 차단 | core `ConversationTurnFeatureReducer` (K2-3, K2-4) | 원격 클라이언트는 보는 자리를 쓰지 않고 thread 를 지정한다 (§3.2) |
| 일이 남은 thread 요약 `ThreadActivityChanged` | core (K2-4) | 그대로 프로토콜 이벤트가 된다 |

### 2.2 막는 것

| 제약 | 근거 |
| --- | --- |
| core 계약 타입에 직렬화가 없다. core 는 `serde_json` 을 쓰면 안 된다 | `tests/architecture_boundaries.rs` core 금지 목록 |
| 보는 thread 가 커널에 하나뿐이다. 클라이언트가 둘이면 서로의 보기를 바꾼다 | `ConversationTurnFeatureReducer.session`, `in_flight_conversation_load`, `ReadModelFeatureReducer` 슬롯 |
| 결과가 호출자 한 명에게 반환값으로만 간다. 구독자 분배가 없다 | `NativeClientPort::dispatch_client_event` |
| 커널 하나가 작업 공간 하나(`current_dir`)로 조립된다. planning 기준 데이터는 저장소 단위이다: 한 저장소의 모든 worktree 가 같은 `~/.akra/projects/<repo>-<hash>/runtime/planning-authority.db` 를 쓴다 | `production::build_native_tui_application`, `sqlite_planning_authority_adapter/workspace_paths.rs` `resolve_repository_paths` |
| TUI 가 자기 프로세스 안에 커널을 갖는다. 데몬도 커널을 가지면 같은 작업 공간에 둘이 돈다 | K-D3, K-D5 |
| app-server 연결은 커널마다 자기 stdio 자식을 쓴다 | `ports.app_server_adapter` |
| 병렬 제어부 tick 은 화면이 알린 병렬 보기(`ParallelViewChanged`, 작업 공간)가 있어야 돈다. 데몬 커널에는 이것을 알릴 화면이 없다 | `native_client_runtime.rs` `NativeClientEvent::ParallelTick` |

## 3. 설계

### 3.1 커널 호스트 (`KernelHost`)

- composition 이 저장소 하나에 커널 하나를 조립한다 (`build_kernel_for_workspace(folder)`, §3.4).
- 커널은 전용 OS 스레드에서 돈다. 반복: 클라이언트 입력함을 비우고, 다음 마감 시각까지 기다리는 주기를 돌리고,
  결과를 구독자에게 나눈다. 커널 상태는 이 스레드만 바꾼다. 입력함 셋은 넣을 때 커널 스레드를 깨우므로 입력은 이
  대기를 기다리지 않는다 (plan 19 Q2-4). 마감이 없으면 입력이 올 때까지 잔다 (plan 19 Q3-4, 이전에는 250ms 주기).
- 클라이언트는 `KernelClientHandle` 로 붙는다: `send_command(command) -> Option<Vec<AppEvent>>`,
  `query(query) -> Response`, 이벤트 수신 채널. `send_command` 는 커널이 그 명령을 처리할 때까지 기다리고, 그
  처리에서 나온 core 이벤트를 돌려준다. 프로토콜은 이것으로 명령 응답의 거절 이유를 만든다. 같은 이벤트는 모든
  클라이언트에도 간다. `None` 은 커널이 끝난 경우이다. 비동기 Gateway 와는 채널로만 만난다. core 와 application 은 tokio 를 모른다.

### 3.2 thread 를 지정하는 명령 (2026-11-08 개정)

처음 설계는 클라이언트마다 보는 thread 와 조회 슬롯을 두는 것이었다 (K2-5 를 여기서). K3-2 뒤 조사에서 두 가지를
확인해 바꿨다.

- Desktop 은 대화 기록을 Gateway 를 거쳐 app-server 에서 직접 읽는다. 커널의 "보는 대화"(`ConversationSnapshot`,
  스트림 투영)는 TUI 화면용이다. 원격 클라이언트에 필요한 것은 thread 요약, 자동 후속, 승인, planning 이다.
- 그래서 원격 클라이언트는 보기를 바꾸지 않고 thread 나 승인 id 를 지정해서 명령한다.

규칙:

- 커널의 보는 자리는 같은 프로세스의 TUI 것으로 남는다. 원격 클라이언트는 이것을 바꾸는 명령(대화 열기, 새
  초안)을 보내지 않는다. v0 프로토콜에서 이 두 명령을 뺀다.
- 승인 결정은 승인 id 로, steer 는 thread id 로 그 thread 의 session 을 찾는다. 보지 않는 thread 의 결과도
  처리한다. 받아들인 결과는 thread 요약의 변화로 알리고, 거절은 명령 응답의 `rejection` 으로 알린다.
- thread 요약(`ThreadActivityChanged`)은 일이 남은 모든 thread(보는 thread 포함)를 담고, 진행 중 턴 id 와 기다리는
  승인 요청을 함께 담는다.
- 프롬프트 제출은 thread 를 지정한다. core 가 준비(planning 조각 붙이기)와 제출을 이어서 한다 (K3-3b). core 가
  모르는 thread(보는 thread, 일이 남은 thread, 읽은 세션 목록 어디에도 없다)와 일하는 중인 thread 는 준비 전에
  거절한다. 준비가 planning 작업을 만들기 때문이다. 턴은 모든 thread 를 통틀어 하나만 돌기 때문에 다른 thread 에서
  턴이 돌아도 거절한다. 병렬 모드가 켜져 있거나 호스트가 병렬 모드 값을 아직 알리지 않았어도 거절한다. 호스트는
  core 입력을 넣기 전마다 병렬 제어부 값을 보고, 바뀌었으면 호스트 전용 `SetParallelModeEnabled` 로 알린다.
- 준비가 끝나면 core 는 병렬 모드를 다시 본다. 그 사이에 병렬 모드가 켜졌으면 턴을 내지 않는다
  (`queued`, `parallel_mode_started`). 작업은 큐에 있으므로 병렬 slot 이 맡는다.
- 준비하는 사이에 다른 턴(예: 자동 후속)이 시작하면 제출은 거절된다 (`queued`, `turn_started_during_preparation`).
  준비가 만든 planning 작업은 큐에 남는다. TUI 의 큐에만 넣기(QueueOnly)와 같은 상태이므로 잃는 작업은 없다. 이벤트는
  그 작업 id(`task_id`)를 담고, 클라이언트는 같은 프롬프트를 다시 보내지 않는다. 다시 보내면 작업이 하나 더 생긴다.
  그 사이에 준비 칸을 잡아 다른 턴을 막지 않는 이유는 자동 후속 턴을 잃지 않기 위해서이다. 큐에 남은 작업을 쉴 때
  다시 내는 일은 사건 연쇄가 아니라 맞추기로 한다. 커널이 쉬면 한 번 다시 내고 `submitted` 를 알린다 (plan 19
  Q2-3, 2026-11-10 구현).
- 클라이언트별 보기(K2-5)는 TUI 가 원격 클라이언트가 되는 K6 으로 옮긴다.

### 3.3 응용 프로토콜 v0 (`akra/kernel/*`)

JSON-RPC 2.0. 전송은 Gateway 의 기존 접속을 그대로 쓴다. 직렬화 타입과 core 타입 사이의 변환은 inbound
adapter `src/adapter/inbound/kernel_protocol/` 에 둔다. core 는 직렬화를 모른다.

| 메서드 | 종류 | 내용 |
| --- | --- | --- |
| `akra/kernel/attach` | 요청 | `{ workspace, client_name }` → `{ client_id, protocol_version, kernel }`. `kernel` 은 저장소 신원이다. 그 폴더가 속한 저장소의 커널에 붙는다 (없으면 만든다, §3.4) |
| `akra/kernel/describe` | 요청 | 메서드, 명령, 이벤트 목록, 판단 사건 기록의 종류(`records`, plan 19 Q3-5)와 버전 (§4.3 자기 설명) |
| `akra/kernel/command` | 요청 | `{ command }` → `{ processed, rejection? }`. 커널이 처리한 뒤 답한다. 바로 거절한 명령(`steer_turn`, `decide_approval`)은 `rejection` 에 이유 코드를 담는다. `submit_prompt` 의 진행과 거절은 `prompt_submission` 이벤트가 이유 코드와 함께 알린다 |
| `akra/kernel/query` | 요청 | 세션 목록, 자동 후속 설정, thread 요약, planning 큐, 병렬 작업 상태(K3-6), 판단 사건 기록(`history`, plan 19 Q3-3). 보는 대화의 턴 상태는 TUI 화면용이라 보내지 않는다 (§3.2) |
| `akra/kernel/event` | 알림 (서버→클라이언트) | `{ event }`. attach 뒤 자동으로 받는다. 대상 지정은 K6 (§1) |
| `akra/kernel/detach` | 요청 | 클라이언트 세션을 닫는다. 접속이 끊겨도 같다. 답 뒤에는 이벤트가 오지 않는다 |

v0 명령은 화면 흐름 단계가 아니라 한 번에 끝나는 도메인 명령이다 (§5.5): 세션 목록 읽기, 프롬프트 제출(준비 +
제출을 하나로), steer, 정지, 승인 결정, 자동 후속 설정, 이름 바꾸기, planning
초기화(확인 필수. 확인 값은 core 명령에 실려 planning 서비스까지 간다), planning 다시 읽기(K3-6). 나머지 명령은 K4 에서 명령 스펙으로 옮길 때 더한다.

호출자 신원은 접속이다 (로컬 소유자만 접속할 수 있다). 멱등 키는 v0 에 없다. 재전송을 가리려면 처리한 명령을
기억해야 하므로 사건 기록과 함께 K5 에서 더한다. 그때까지 `idempotency_key` 를 보내면 모르는 필드로 거절한다.
원격 인증은 만들지 않는다 (Q-K6).

### 3.4 저장소당 커널 하나 (2026-11-09 개정)

처음 설계는 정규화한 폴더 경로마다 커널을 두고, 잠금 파일을 `<folder>/.codex-exec-loop/` 아래에 두었다. 이것은
git worktree 에서 틀린다. planning 기준 데이터는 저장소 단위이다. 한 저장소의 모든 worktree 는
`git rev-parse --git-common-dir` 로 같은 저장소 이름 공간에 닿고, 같은 `planning-authority.db` 를 쓴다 (큐와 병렬
상태를 함께 쓴다). 폴더마다 커널을 두면 두 worktree 의 커널이 같은 큐를 두 번 돌린다. 사용자가 이 개정을
승인했다 (2026-11-09).

규칙:

- 커널 단위는 저장소이다. 저장소의 기준은 planning 기준 데이터와 같은 저장소 신원과 이름 공간이다
  (`SqlitePlanningAuthorityAdapter::resolve_authority_location_from_workspace`). git 이 아닌 폴더는 정규화한 폴더
  경로가 기준이다.
- 커널 잠금: `$AKRA_HOME/projects/<namespace>/runtime/kernel.lock` 을 OS 파일 잠금(`File::try_lock`, `gateway.lock`
  과 같은 방법)으로 잡는다. 파일 안의 pid, 시작 시각, 폴더는 진단용이다. git 이 아닌 폴더의 `runtime` 은 planning
  이 그 폴더에 쓰는 관리 디렉터리(`$AKRA_HOME/projects/<folder>-<hash>/runtime`)이다. 폴더 안에는 아무것도
  만들지 않는다.
- 잠금을 쥔 프로세스만 결정 고리(턴 종료 평가, 자동 후속, 병렬 제어, PR 검증 스케줄러)를 돌린다. 같은 저장소의
  두 번째 프로세스는 같은 worktree 든 다른 worktree 든 커널을 만들지 않고 이유를 답한다. 사용자는 같은 저장소에서
  TUI 와 Desktop 을 동시에 거의 쓰지 않는다고 확인했다. 그래서 K6 전까지는 거절로 충분하다.
- 병렬 슬롯 worktree(`akra-agent/slot-*`)는 그 커널이 관리하는 작업 공간이고 커널이 아니다. 잠금을 잡지 않는다.
- 커널을 조립할 때 요청한 폴더를 병렬 보기로 정한다. 그래서 화면 없이도 병렬 tick(깨우기 폴링, 정리 재시도)이
  돈다. 화면이 없으므로 `supervisor_view_active` 는 `false` 이다. 클라이언트가 붙어 있는 동안에는 `true` 이다 (K3-6). supervisor 새로 고침은 화면에 보이기 위한
  주기적 점검이고, TUI 도 병렬 화면이 닫혀 있으면 `false` 를 보낸다. 배분과 정리는 이 값과 관계없이 돈다.
- Gateway 는 저장소 신원마다 커널을 하나 둔다 (K3-5). 첫 attach 때 만들고, 클라이언트가 모두 떠나도 일이 남아
  있으면(턴, 평가, 병렬) 계속 돈다. 붙은 클라이언트도 남은 일도 없이 10분이 지나면 내린다 (§3.7 K3-5 리뷰).
  커널의 작업 공간은 저장소의 대표 checkout 이다. 병렬 pool 안의 worktree 에서 붙으면 거절한다.
- 병렬 정지가 다른 프로세스에 닿지 않는 문제(K-D3)는 모든 입구가 커널 하나를 거칠 때(K3-5, K4) 생기지 않는다.

### 3.5 TUI 와의 관계 (K6 전까지)

- TUI 는 지금처럼 자기 프로세스 안에 커널을 갖고, 시작할 때 시작 폴더의 저장소 잠금을 잡는다. 잠금은 TUI 가
  끝날 때까지 쥔다.
- 다른 프로세스(다른 TUI 나 데몬 커널)가 그 저장소의 잠금을 쥐고 있으면 TUI 는 화면을 열기 전에 이유(쥔
  프로세스의 pid, 폴더, 잠금 경로)를 보이고 끝난다 (종료 코드 1). 그 반대도 같다.
- 잠금 파일을 만들지 못하는 오류(경합이 아닌 오류)는 TUI 를 막지 않는다. 경고를 기록하고 잠금 없이 시작한다
  (기존 동작 유지). 데몬 커널은 이 경우에도 만들지 않는다.
- 알려진 한계: TUI 는 시작 뒤 초안의 폴더를 다른 저장소로 바꿀 수 있다. 잠금은 시작 폴더의 저장소만 덮는다.
- admin(`akra admin`, `akra-admin`), Telegram(`akra telegram`, `akra-telegram`), `akra parallel tick` 은 잠금을
  잡지 않는다. 이 프로세스는 턴 종료 평가, 자동 후속, 주기적 병렬 tick 을 돌리지 않는다. 명령을 받을 때만 자기
  병렬 제어부를 움직인다(`akra parallel tick` 은 cron 용 1회 배분). 병렬 제어부를 조립하면서 PR 검증 스케줄러
  스레드를 시작한다. 스케줄러는 기록마다 DB lease 를 잡으므로 같은 PR 을 두 번 폴링하지 않는다. 이 입구들은
  K4 에서 커널 명령 경로로 옮긴다.
- TUI 가 데몬 커널에 원격 클라이언트로 붙는 일은 K6(얇은 TUI)에서 한다.

### 3.6 경계 테스트

- core 는 지금처럼 `serde_json` 과 adapter 를 모른다.
- `kernel_protocol` 만 직렬화 타입을 core 타입으로 바꾼다. Gateway 는 `kernel_protocol` 을 거쳐서만 커널에
  닿는다.
- 커널 상태는 커널 스레드만 바꾼다 (`KernelHost` 밖에서 `NativeClientRuntime` 을 직접 부르지 않는다).
- 병렬 모드 값처럼 호스트만 아는 값은 호스트 전용 명령(`SetParallelModeEnabled`, `EvaluateCompletedTurn`)에 싣는다.
  inbound adapter 는 이 명령을 만들지 않는다.

### 3.7 K3 리뷰 반영 (2026-11-09)

K3-1~K3-4 를 plan 19·20 방향과 함께 리뷰했다. 고친 것:

- 커널 잠금 adapter 가 SQLite planning adapter 를 직접 부르지 않는다. composition 이 runtime 디렉터리를 준비해서
  넘긴다.
- 클라이언트 명령(`SubmitThreadPrompt`)에 병렬 모드 값이 없다. 호스트가 병렬 제어부 값을 호스트 전용
  `SetParallelModeEnabled` 로 core 에 알린다. core 는 받을 때와 준비가 끝난 뒤 두 번 본다. 값을 모르면
  `parallel_mode_unknown` 으로 거절한다.
- `prompt_submission` 은 이유 코드를 담는다 (plan 19 P3 의 첫 조각). 준비 실패도 취소, 본문 불일치, 초안 검토
  필요, planning 거절로 나눈다. 작업이 큐에 남으면 `queued` 와 `task_id` 를 알린다.
- steer 와 승인 결정의 거절은 명령 응답의 `rejection` 으로 요청한 클라이언트에 간다. 명령 응답의 `accepted` 는
  뜻대로 `processed` 로 바꿨다.
- `conversation_runtime` 이벤트·조회를 `auto_follow`(모든 thread 에 같은 설정)로 줄였다. 보는 대화의 턴 상태는 TUI
  화면용이므로 내보내지 않는다.
- v0 에서 동작하지 않던 `idempotency_key` 를 뺐다. attach 응답에 `kernel` 을 더했다. §1, §3.1, §3.3 을 코드에 맞췄다.
- TUI 저장소 잠금과 thread 지정 프롬프트의 병렬 모드 확인을 `docs/reference` 에 적었다.

리뷰에서 반려한 것: 시험 코드가 SQLite adapter 로 runtime 위치를 만드는 것(경계 규칙은 운영 코드에만 걸린다),
`kernel` 필드 이름(커널은 저장소당 하나라 뜻이 같다), 시험 helper 중복, 잠금 디렉터리 준비가 DB adapter 에 있는
것(사용자 전용 디렉터리 보안 루틴을 그 파일과 함께 쓴다), 전송 타입 이름 규칙(K4 생성 때 정한다).

남긴 것 (계획된 단계에서 한다):

- 준비 → 제출 규칙이 core(`submit_prepared_thread_prompt`)와 TUI(`apply_manual_prompt_intake_outcome`)에 둘 있다.
  TUI 쪽은 K6 에서 지운다.
- 원격 준비의 대상 thread(`addressed_thread_id`)는 메모리에만 있다. 재시작하면 잃는다 (plan 19 P1, Q3).
- steer, 승인, 원격 제출이 모두 `with_owning_session`(`mem::swap`)을 거친다. thread 액터(plan 19 Q5)에서 바꾼다.
- 명령 하나를 더할 때 enum, 이름 목록, 변환, 시험을 함께 고친다. 스펙 생성(K4, plan 19 Q4)이 대신한다.
- `NativeClientRuntime`·`NativeTuiApplicationComposition` 이름은 plan 18 §5.9 대로 마지막에 바꾼다.

#### K3-5 리뷰 반영 (2026-11-10)

K3-5 병합 범위(`526f93a3..43e1d5c5`)를 리뷰했다. 지적 8개를 모두 코드와 실패하는 시험으로 확인했다(결함 아님으로
본 것은 없다). 고친 것:

1. Gateway 종료 순서. 접속 작업이 목록 `Arc` 를 쥐고, 바이너리는 `exit` 로 끝나 소멸자가 돌지 않았다. 그래서
   `drop(kernels)` 는 커널을 멈추지 못했고 upstream app-server 가 먼저 멈췄다. 이제 `RepositoryKernelRegistry::shutdown`
   이 커널마다 병렬 자동화를 멈추고, 호스트 스레드를 기다리고, 잠금을 푼 뒤 붙기를 거절한다. Gateway 는 이것을
   upstream 보다 먼저 부른다. 시험: `shutdown_stops_kernels_that_sessions_still_share`,
   `gateway_stop_stops_kernels_before_the_upstream_app_server`.
2. 읽기 고리 막힘. 접속의 읽기 고리가 커널 호출을 기다려 승인 답과 중단 전달이 멈췄다. 이제 접속마다 작업 하나가 커널
   요청을 차례대로 처리하고 답을 보내기 함으로 보낸다(같은 접속의 요청 순서와 id 는 그대로). 커널 호출은 시간 상한이
   있다(붙기 60초, 명령·조회 10초, 넘으면 `-32013`). 목록 잠금은 자리를 찾을 때만 잡고, 저장소마다 자리 잠금을 둔다.
   그래서 커널 만들기는 같은 저장소의 붙기만 기다리게 하고, 두 worktree 는 여전히 한 커널을 쓴다. 시험:
   `a_waiting_kernel_attach_does_not_block_app_server_forwarding`, `a_slow_kernel_start_does_not_block_another_repository`.
3. 늦은 작업 스레드의 slot 반환. epoch 가 닫힌 뒤 시작하지 않는 작업 스레드가 worktree 경로로 slot 을 돌려줬다.
   이제 lease 세대(`finalize_stream_completion_for_lease`)로 막는다. 시험:
   `a_late_worker_does_not_release_a_slot_that_a_new_epoch_leased_again`.
4. 커널 작업 공간. 처음 붙은 경로가 커널의 병렬 작업 공간이 됐고, slot worktree 에서 붙어도 커널을 만들었다. 이제
   작업 공간은 저장소의 대표 checkout(planning 기준 데이터의 `canonical_repo_root`)이고, 병렬 pool 안에서 붙으면
   이유와 함께 거절한다. 대표 checkout 이 없는 저장소(bare 저장소, 대표 checkout 의 `.git` 이 없음)는 worktree 마다
   기준이 달라 처음 붙은 worktree 에 따라 병렬 pool 위치가 바뀐다. 그래서 그런 저장소의 붙기는 이유와 함께 거절한다
   (두 번째 리뷰). 시험: `a_kernel_attached_from_a_linked_worktree_works_in_the_primary_checkout`,
   `a_parallel_slot_worktree_does_not_create_a_kernel`, `a_repository_without_a_primary_checkout_does_not_create_a_kernel`.
5. 접속 정리 2초. 커널 세션이 보내기 함 송신자를 쥐어, 클라이언트가 닫을 때마다 쓰기 작업이 2초 상한까지 기다렸다.
   이제 커널 요청 작업을 먼저 끝낸다. 시험: `a_client_close_ends_the_session_without_waiting_for_the_flush_timeout`.
6. 떼어 낸 뒤 이벤트. 이전 이벤트 스레드가 최대 200ms 더 이벤트를 보냈고, 다시 붙으면 이벤트가 두 번 왔다. 이제
   떼어 내기와 다시 붙기는 호스트에서 클라이언트를 떼고 이벤트 스레드가 끝난 뒤 답한다. 스레드는 멈춤 표시를 본 뒤
   보내지 않고, 보내기 함이 차도 멈춤 표시를 본다. 다시 붙기는 이전 붙음을 먼저 떼므로, 새 붙기가 실패하면 그 접속은
   붙지 않은 상태가 된다. 시험: `no_kernel_event_reaches_a_client_after_its_detach_response`,
   `attaching_again_does_not_duplicate_kernel_events`.
7. proxy 핸드셰이크 상한. 답하지 않는 proxy 가 커널의 app-server 호출을 모두 멈췄다. 이제 10초 상한이 있고, stderr
   읽기는 핸드셰이크 전에 시작한다. 시험: `the_handshake_gives_up_when_the_proxy_never_answers`.
8. 커널 내리기(승인한 정책). 붙은 클라이언트가 없고 남은 일이 없는 채로 10분(`KERNEL_IDLE_UNLOAD_AFTER`)이 지난
   커널을 Gateway 가 30초마다 찾아 1번과 같은 순서로 내린다. 그러면 TUI 가 잠금을 잡을 수 있고, 다음 붙기가 커널을
   다시 만든다. 남은 일은 지금 커널 상태로 정한다(`NativeClientRuntime::has_remaining_work`): core 의 바쁜 thread
   session(턴, 대기 승인, 자동 후속, 평가, 정지, steer)과 진행 중인 planning 작업(프롬프트 준비, 큐 변경, 작업 공간
   작업), 병렬 제어부의 켜진 병렬 모드, 열린 automation epoch, 진행 중인 제어 효과, 도는 배분 작업 스레드(automation
   guard 의 새 수). 시험: `an_idle_kernel_unloads_and_the_next_attach_starts_it_again`,
   `a_kernel_is_idle_only_without_clients_and_work`, `remaining_work_keeps_the_kernel_busy`.

남긴 것:

- PR 검증 스케줄러와 GitHub 리뷰 폴링은 남은 일로 보지 않는다(승인한 정책 목록 밖). 병렬 모드가 켜져 있으면 커널은
  내려가지 않는다. 꺼진 저장소의 폴링은 커널과 함께 멈추고 다음 붙기에서 다시 시작한다.
- 병렬 제어부의 정리 재시도 목록(`unsettled_dispatch_cleanups`)은 프로세스 메모리이고, 설계상 재시작 때 잃어도
  기준 데이터에서 다시 찾는다. 그래서 남은 일로 보지 않는다.
- 종료와 내리기는 이미 도는 배분 작업 스레드를 기다리지 않는다(epoch 를 닫을 뿐이다). Gateway 종료 때 그 slot 턴은
  upstream 이 멈추면서 전송 오류로 끝나고 실패로 기록될 수 있다. 내리기는 작업 스레드가 없을 때만 한다.
- 시간을 넘긴 명령은 커널이 나중에 처리할 수 있다. 클라이언트는 조회로 결과를 확인한다.

## 4. 단계 (PR 단위)

| 단계 | 내용 | 기존 동작 | 검증 |
| --- | --- | --- | --- |
| K3-1 (완료 2026-11-08) | §3.3 프로토콜 v0 의 타입과 변환 (`src/adapter/inbound/kernel_protocol/`). 전송 없음. 명령 10개(세션 목록, 대화 열기, 새 초안, 이름 바꾸기, steer, 정지, 승인 결정, 자동 후속 최대 턴, 자동 후속 멈춤, planning 초기화), 조회 3개, 굵은 이벤트 4개(세션, 대화 실행 상태, thread 요약, 다시 맞추기). 설계와 다른 점: 프롬프트 제출은 core 가 준비 뒤 제출을 이어서 해야 하므로 K3-3 에서 더한다. 필드 없는 명령도 `{}` 로 둔다(serde 가 unit variant 의 모르는 필드를 거절하지 못한다) | 유지 | 직렬화 왕복 시험, 모든 v0 명령의 core 변환 시험, 확인 없는 초기화 거절, 모르는 명령·필드 거절, `describe` 내용 고정. 경계 테스트: 이 adapter 는 application·composition·outbound 를 모른다 |
| K3-2 (완료 2026-11-08) | §3.1 `KernelHost`(`src/composition/kernel_host.rs`) 와 `KernelClientHandle`. 커널 스레드는 클라이언트 입력함을 비운 뒤 `run_host_cycle(now, 50ms, 64)` 을 돌리고 core 이벤트와 병렬 제어부 이벤트를 모든 클라이언트에 나눈다(대상별 분배는 K3-3. 병렬 이벤트는 2026-11-09 에 더했다). 클라이언트 채널은 1024 개 상한이고, 넘치면 `Resync` 를 보낸 뒤 이어서 보낸다. 밀린 `Resync` 는 새 이벤트가 없어도 다음 주기에 보낸다. 조회는 스냅샷과 마지막 thread 요약을 준다. 알려진 한계: 클라이언트 명령은 커널 대기(최대 50ms)가 끝나야 처리된다. 운영 코드 연결은 K3-5 이다 | 유지 | 화면 없는 시험: 핸들로 명령을 보내고 조회하고, 턴 종료 → 평가 시작 → 자동 후속 제출 이벤트를 받고, 후속 턴이 실행된다. 느린 클라이언트는 커널을 막지 않고 `Resync` 를 받는다. 경계 테스트: 호스트는 tokio 와 inbound adapter 를 모른다 |
| K3-3a (완료 2026-11-08) | §3.2 승인 결정은 승인 id 로, steer 는 thread id 로 주인 session 을 찾는다. thread 요약에 보는 thread, 진행 중 턴 id, 기다리는 승인을 넣는다. v0 에서 대화 열기·새 초안을 뺀다 | 유지 (TUI 의 세션 목록 표시는 보는 thread 에도 붙는다) | core 시험: 보지 않는 thread 의 승인을 결정하고 steer 한다 |
| K3-3b (완료 2026-11-09) | §3.2 thread 를 지정한 프롬프트 제출. core 명령 `SubmitThreadPrompt` 가 준비(`PrepareManualPrompt` 와 같은 효과) 뒤 제출(`SubmitTurn` 과 같은 admission)을 잇는다. 그 thread 의 session 이 없으면 보는 session 의 설정으로 뒤에 새로 만든다. 보는 자리는 바꾸지 않는다. 프로토콜 `submit_prompt` 와 진행 이벤트 `prompt_submission`(preparing, submitted, rejected, failed). 설계와 다른 점: (1) 모르는 thread 는 거절한다. 아는 thread 는 보는 thread, 일이 남은 thread, 읽은 세션 목록(첫 페이지)의 thread 이다. (2) planning 초안 검토가 필요한 준비 결과는 원격에서 검토할 수 없어 실패로 알린다. (3) 병렬 모드가 켜져 있으면 준비 전에 거절한다(`RejectedParallelMode`). 병렬 작업 intake 는 아직 TUI 에만 있고, main session 턴으로 내면 같은 작업이 병렬 slot 에도 나갈 수 있다. 사용 여부는 호스트가 병렬 제어부 값으로 채운다. 이미지 첨부는 없다. (4) TUI 는 지금처럼 자기 준비 → 제출 경로를 쓴다. (5) 준비 칸은 TUI 와 같이 쓴다. TUI 가 대화를 바꿀 때 보내는 준비 취소는 thread 를 지정한 준비를 취소하지 않는다 | 확장 | core 시험: 보지 않는 thread 에 제출하면 턴 효과가 나오고 보는 자리는 쉰다. 모르는 thread, 일하는 중, 병렬 모드, 다른 준비 진행 중, 준비 실패, TUI 의 대화 전환 뒤에도 원격 준비가 제출된다. 프로토콜 왕복·변환·이벤트·`describe`. 호스트 시험: 명령 → 턴 실행 |
| K3-4 (완료 2026-11-09) | §3.4 저장소 잠금과 저장소당 커널. 설계와 다른 점: 커널 단위를 폴더에서 저장소로 고쳤다(§3.4 개정). 잠금은 `$AKRA_HOME/projects/<namespace>/runtime/kernel.lock` 이다. composition `build_kernel_for_workspace(folder)` 가 잠금을 먼저 잡고, 런타임을 조립하고, 병렬 보기(그 폴더, `supervisor_view_active = false`)를 정한 뒤 `KernelHost` 를 시작한다. 커널을 버리면 호스트 스레드가 끝난 뒤 잠금이 풀린다. TUI 도 시작할 때 같은 잠금을 잡는다. 저장소별 커널 목록(registry)은 Gateway 가 커널의 수명(클라이언트가 떠난 뒤)과 비동기 접속 사이의 공유를 정해야 하므로 K3-5 로 옮겼다. 데몬 커널의 GitHub 리뷰 폴링 설정도 K3-5 에서 넘긴다 | 바뀜: 같은 저장소에서 TUI 둘, 또는 TUI 와 데몬 커널이 함께 돌지 않는다. 두 번째 TUI 는 이유를 보이고 끝난다 | 잠금 경합 시험(다른 worktree 에서 두 번째 잠금 거절, 다른 저장소는 둘 다 성공, 쥔 쪽이 버리면 풀림), 조립 때 병렬 보기 설정, 커널 시작 뒤 잠금 유지, TUI 시작 거절 메시지 |
| K3-5 (완료 2026-11-10) | Gateway 가 `akra/kernel/*` 를 커널 호스트에 연결한다. Q-K3-6 을 확인했다(공유할 수 있다). Gateway 는 `RepositoryKernelRegistry` 를 가지고, 세션은 `akra/kernel/*` 를 `gateway/kernel.rs` 로 보낸다(attach, describe, command, query, detach, event 알림). 붙기 전 요청은 `-32011`, 커널에 닿지 못하면 `-32012`, 커널이 시간 안에 답하지 않으면 `-32013`(2026-11-10 리뷰 뒤). 커널의 app-server adapter 는 `codex app-server proxy` 위 WebSocket (`proxy_transport.rs`)으로 Gateway upstream 을 쓴다. 조회 응답은 같은 이름의 이벤트와 같은 모양이다. 먼저 한 일: 커널 종료 순서와 저장소별 커널 목록(아래), 리뷰에서 막은 epoch 확인 틈. (1) 커널 종료 순서. `NativeClientRuntime` 을 버리면 병렬 제어부의 자동화 epoch 를 모두 닫는다(저장한 병렬 모드 설정은 그대로). 커널은 호스트 스레드에서 런타임을 버린 뒤 잠금을 푼다. 이전 작업 스레드(orchestrator tick, 배분, 작업 실행)는 다음 epoch 확인에서 멈춘다. 리뷰(2026-11-10)에서 확인 뒤 guard 없이 바꾸는 경로 세 개를 막았다: 배분된 작업 스레드는 턴 시작 전에 epoch 를 다시 보고 slot 을 돌려준다, distributor 큐 넣기는 epoch commit 안에서 한다, slot 턴 뒤 planning 갱신은 epoch 에 묶인 continuation permit 으로 결과를 적용한다(갱신 자체는 오래 걸리므로 잠금을 쥐지 않는다). 남은 틈: 배분 확인과 턴 시작 사이의 아주 짧은 틈(코드 `shortcut`). 시작하지 않는 작업 스레드의 slot 반환은 K3-5 리뷰 뒤 lease 세대로 막았다(§3.7). 이미 시작한 slot 작업 턴은 끝까지 돌지만 그 뒤 큐를 바꾸지 않는다. (2) 저장소별 커널 목록 `RepositoryKernelRegistry`. 처음 붙을 때 커널을 만들고, 같은 저장소의 다른 worktree 는 같은 커널에 붙는다. 클라이언트가 모두 떠나도 커널은 남는다(자동 후속과 병렬 배분은 클라이언트 없이 돈다). Gateway 가 끝날 때 `shutdown` 이 upstream 보다 먼저 커널을 멈추고 잠금을 푼다. 붙은 클라이언트도 남은 일도 없이 10분이 지난 커널은 내린다(K3-5 리뷰, §3.7). 설계와 다른 점: 병렬 제어부 이벤트는 아직 보내지 않는다(K3-6). 알려진 한계: 계정 사용량 한도로 실제 턴을 커널 경로로 돌려 보지 못했다. 공유 app-server 에서 승인 요청이 어느 연결로 가는지(커널, 또는 thread 를 resume 한 Desktop 도) 확인하지 못했다 | 확장 | Gateway 시험: attach → command → event, 붙기 전 거절, detach 뒤 거절, 조회 응답 모양. proxy 전송: 핸드셰이크 검사 단위 시험, 실제 codex 로 초기화·계정·세션 목록(`--ignored`). 호스트를 버리면 epoch 가 닫힌다, epoch 가 닫힌 뒤 배분된 작업 스레드는 턴을 시작하지 않는다, 큐 넣기 중에 epoch 가 닫히면 결과는 commit-ready 로 남는다, epoch permit 은 epoch 와 함께 무효가 된다, 목록은 저장소당 커널 하나를 두고 클라이언트가 떠나도 잠금을 쥐며 목록을 버리면 잠금을 푼다. 남은 시험: 커널을 실제로 다시 만들어 이전 작업 스레드가 배분하지 않음(Gateway 연결 때). K3-5 리뷰 시험은 §3.7 |
| K3-6 (완료 2026-11-10) | Desktop 의 "Akra" 패널이 `akra/kernel/*` 로 일이 남은 대화(thread 요약), 자동 후속 설정, 병렬 작업 상태, planning 큐를 보인다. 패널이 열려 있고 열린 스레드가 있을 때만 그 폴더의 저장소 커널에 붙고, 닫으면 뗀다. 붙어 있는 동안 그 저장소의 TUI 는 열리지 않기 때문이다. 프로토콜에 명령 `refresh_planning`, 조회와 이벤트 `planning`, `parallel` 을 더했다. 호스트는 병렬 제어부 이벤트를 모아 지금 상태(`ParallelHostView`: 사용 여부, 마지막 감독 스냅샷, 상태 문구)를 갖고, 바뀌면 통째로 보낸다. planning 투영은 core 스냅샷에서 바뀌면 보낸다. 설계와 다른 점: (1) 데몬 커널은 화면이 없어 감독 스냅샷을 만들지 않았다. 이제 클라이언트가 붙어 있는 동안 `supervisor_view_active` 를 켜고, 첫 클라이언트가 붙을 때 pool 을 바꾸지 않는 점검(`InspectSupervisor`)을 한 번 한다. 주기적 새로 고침은 병렬 epoch 가 열려 있을 때만 돈다. 점검은 TUI 의 병렬 화면처럼 push 준비 점검(GitHub 쓰기 신원 확인)을 한다. (2) 병렬 제어부의 planning 다시 읽기 요청은 클라이언트가 붙어 있을 때만 따른다. 클라이언트는 붙은 뒤 `refresh_planning` 을 보낸다. (3) 병렬 agent, 병합 큐, PR 검증의 `state` 는 TUI 표시 이름을 snake_case 로 바꾼 문자열이다. 열거형은 명령 스펙(K4)에서 고정한다. 리뷰 반영(2026-11-10): 같은 저장소의 다른 폴더로 다시 붙으면 Gateway 가 먼저 떼므로, 점검은 최근 60초 안에 했으면 다시 하지 않는다(`SUPERVISOR_INSPECTION_REUSE`). Desktop 은 같은 커널에 다시 붙으면 보던 상태를 둔다. 병렬 모드가 꺼지면 epoch 가 닫혀 감독 스냅샷이 더 새로 고쳐지지 않으므로 이전 스냅샷을 버리고, 클라이언트가 있으면 다시 점검한다. 커널 작업 공간이 아닌 병렬 이벤트는 버린다. Desktop 은 붙은 뒤의 조회 실패에도 붙음을 유지하고 메시지만 보인다. 감독 보기 값은 `ParallelViewChanged` 로 바꾼다(런타임 공개 API 경계) | 확장 | Rust: 프로토콜 왕복과 모양 시험, 호스트 시험(병렬 상태를 한 번에 보냄, planning 변경, 감독 보기 켜고 끄기), Gateway 조회 시험. Desktop: 연결 단위 시험 11개, 가짜 Gateway 화면 시험(`test/e2e/kernel-panel.mjs`, 숨긴 창 글 캡처). 실제 Gateway 와 저장소 커널(임시 `AKRA_HOME`, 턴 없음)에서 숨긴 창으로 패널 글을 확인했다 (2026-11-10, macOS) |

규모: plan 18 추정 5~10일을 유지한다. K3-3 이 가장 컸다 (thread 를 지정한 명령과 원격 프롬프트 제출).

## 5. 위험

| 위험 | 대응 |
| --- | --- |
| 커널 스레드와 비동기 Gateway 사이에서 이벤트가 밀린다 | 클라이언트마다 상한이 있는 채널. 넘치면 그 클라이언트에 "다시 조회" 이벤트를 보내고 오래된 증분은 버린다 |
| 데몬 커널이 오래 돌며 app-server 자식이 죽는다 | 지금 adapter 의 재시작 규칙을 쓴다. 커널 상태는 thread 별 실패로 끝낸다 |
| 저장소 잠금이 남아 다음 실행이 막힌다 | OS 파일 잠금이라 프로세스가 끝나면 풀린다. 잠금 파일 내용(pid, 시작 시각, 폴더)은 진단용이다 |
| 프로토콜 v0 가 K4 명령 스펙과 어긋난다 | v0 는 `protocol_version` 을 알린다. K4 에서 스펙으로 생성하면서 v1 로 올린다 |
| 두 클라이언트가 같은 thread 에 동시에 명령을 보낸다 | 커널 스레드 하나가 차례로 처리한다. 지금의 correlation 규칙이 늦은 명령을 거절한다 |

## 6. 미결

| ID | 질문 | 기본값 (답이 없을 때) |
| --- | --- | --- |
| Q-K3-1 | 커널을 어느 프로세스에 둘 것인가 | `akra-gateway`. 이미 사용자별 데몬이고 endpoint 보안이 있다 |
| Q-K3-2 | 첫 원격 클라이언트 | Desktop. 이미 Gateway 에 붙어 있다 |
| Q-K3-3 | K3 동안 TUI 는 어떻게 하는가 | 자기 프로세스 커널을 유지하고 저장소 잠금을 함께 쓴다. 데몬 커널에 붙는 일은 K6 |
| Q-K3-4 | 커널 단위 | **답함 (2026-11-09):** 저장소당 하나. worktree 들은 planning 기준 데이터를 함께 쓰므로 한 커널을 쓴다. git 이 아닌 폴더는 폴더당 하나 (§3.4) |
| Q-K3-5 | 이벤트 전달 | K3 는 접속 중 실시간 알림만. 영구 기록과 커서는 K5 |
| Q-K3-6 | 데몬 커널이 실행한 턴을 Desktop 이 실시간으로 보려면, 커널과 Desktop 이 같은 app-server 를 써야 한다. 지금 커널은 자기 stdio app-server 를, Desktop 은 Gateway upstream app-server 를 쓴다 | **답함 (2026-11-10): 공유할 수 있다.** Gateway upstream 에 `app-server proxy` 연결 둘을 열었다. 연결 B 가 `thread/resume` 한 뒤 연결 A 의 턴 알림(`turn/started`, `item/started`, `item/completed`, `error`, `turn/completed`)을 받았다. 합류 전에는 전역 알림(`thread/started`, `thread/status/changed`)만 받았다. 임시(ephemeral) thread 는 첫 턴 전에 `thread/resume` 할 수 없다(`no rollout found`). 한계: 계정 사용량 한도로 턴이 모델 응답 없이 끝나서 `item/agentMessage/delta` 는 보지 못했다. 같은 구독 경로로 오는 것으로 본다. 확인 시험: `gateway_upstream/shared_app_server_probe.rs` (`--ignored`). 결론: 커널은 Gateway upstream 을 쓴다. 커널의 app-server 전송을 stdio JSON-RPC 에서 `app-server proxy` 위 WebSocket 으로 바꾼다 (K3-5). Desktop 은 커널이 돌리는 thread 를 `thread/resume` 해서 실시간으로 본다 |
