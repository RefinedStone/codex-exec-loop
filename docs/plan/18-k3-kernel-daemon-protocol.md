# K3 상세 설계: 커널 데몬과 응용 프로토콜

## 문서 상태

- 상태: **Proposed execution plan**. 현재 제품 동작을 설명하지 않는다.
- 상위 계획: [18 Akra 커널 아키텍처](18-akra-kernel-architecture.md) §4.1, §4.4, §5.3~§5.6, §6 K3. 결정
  KP-03, KP-04, KP-06 (승인 2026-11-07). Q-K3, Q-K6 답 (2026-11-07).
- 이전 단계: [K1](18-k1-headless-decision-loop.md), [K2](18-k2-thread-sessions.md) (완료 2026-11-08).
  K2-5(클라이언트 상태 타입 분리)를 이 단계로 옮겼다.
- 관련 계획: [17 Akra Desktop](17-akra-desktop/README.md) D-11 (얇은 Gateway, `akra/*` 이름 공간).
- 기준일: 2026-11-08 (Asia/Seoul)
- 기준 커밋: `70c99aed` (`origin/prerelease`).
- 범위: 커널(지금의 `NativeClientRuntime` + core)을 화면 프로세스 밖의 데몬에서 돌리고, 클라이언트가 응용
  프로토콜로 명령·조회·구독한다. 명령 경로 통일(K4), 이벤트 기록·커서·승인 객체(K5), 얇은 TUI(K6)는 범위
  밖이다.

## 1. 목표

1. 커널이 화면 없이 상주한다. 클라이언트가 없어도 턴, 평가, 자동 후속, 병렬 작업, PR 검증이 돈다.
2. 클라이언트 여럿이 같은 커널에 붙는다. 각자 다른 대화를 볼 수 있다. 결과는 요청한 클라이언트에게 간다.
3. 모든 원격 클라이언트는 하나의 응용 프로토콜(JSON-RPC)을 쓴다. 첫 원격 클라이언트는 Desktop 이다.
4. 같은 작업 공간에서 커널이 둘 돌지 않는다 (K-D3, K-D5 해소).

## 2. 현재 상태 (2026-11-08 코드 조사)

### 2.1 이미 있는 것

| 항목 | 위치 | K3 에서 쓰는 방법 |
| --- | --- | --- |
| 사용자별 상주 데몬 `akra-gateway` | `src/bin/akra-gateway.rs`, `src/adapter/inbound/gateway/` | 커널을 이 프로세스에 올린다 |
| 로컬 전용 endpoint: unix `$AKRA_HOME/run/gateway.sock`(0600), Windows named pipe(소유자 전용, 원격 거절) | `gateway/listener.rs` | 그대로 쓴다. TCP 는 열지 않는다 |
| 접속마다 WebSocket JSON-RPC, `akra/*` 는 Gateway 가 직접 답한다 | `gateway/router.rs`, `session.rs` | `akra/kernel/*` 를 더한다 |
| 화면 없이 도는 런타임 호스트 `run_host_cycle(now, wait, max_inputs)` | `composition/native_client_runtime.rs` (K1-6) | 데몬의 커널 스레드가 `wait > 0` 으로 돌린다 |
| thread 별 session, 보지 않는 thread 의 결과 차단 | core `ConversationTurnFeatureReducer` (K2-3, K2-4) | 클라이언트별 보기로 일반화한다 |
| 일이 남은 thread 요약 `ThreadActivityChanged` | core (K2-4) | 그대로 프로토콜 이벤트가 된다 |

### 2.2 막는 것

| 제약 | 근거 |
| --- | --- |
| core 계약 타입에 직렬화가 없다. core 는 `serde_json` 을 쓰면 안 된다 | `tests/architecture_boundaries.rs` core 금지 목록 |
| 보는 thread 가 커널에 하나뿐이다. 클라이언트가 둘이면 서로의 보기를 바꾼다 | `ConversationTurnFeatureReducer.session`, `in_flight_conversation_load`, `ReadModelFeatureReducer` 슬롯 |
| 결과가 호출자 한 명에게 반환값으로만 간다. 구독자 분배가 없다 | `NativeClientPort::dispatch_client_event` |
| 커널 하나가 작업 공간 하나(`current_dir`)로 조립된다 | `production::build_native_tui_application` |
| TUI 가 자기 프로세스 안에 커널을 갖는다. 데몬도 커널을 가지면 같은 작업 공간에 둘이 돈다 | K-D3, K-D5 |
| app-server 연결은 커널마다 자기 stdio 자식을 쓴다 | `ports.app_server_adapter` |

## 3. 설계

### 3.1 커널 호스트 (`KernelHost`)

- composition 이 작업 공간 하나에 커널 하나를 조립한다 (`build_kernel_for_workspace(root)`).
- 커널은 전용 OS 스레드에서 돈다. 반복: 클라이언트 입력함을 비우고, `run_host_cycle(now, 50ms, …)` 를 돌리고,
  결과를 구독자에게 나눈다. 커널 상태는 이 스레드만 바꾼다.
- 클라이언트는 `KernelClientHandle` 로 붙는다: `send(command) -> Admission`, `query(query) -> Response`,
  이벤트 수신 채널. 비동기 Gateway 와는 채널로만 만난다. core 와 application 은 tokio 를 모른다.

### 3.2 클라이언트 세션 (K2-5 를 여기서 한다)

- 커널은 `ClientId` 마다 `ClientView` 를 둔다: 보는 thread, 대화 로드 절차, 조회 모델 슬롯(peek, review
  center, queue, directions), 작업자 패널 기록, 수동 프롬프트 준비.
- thread session 은 모두 같은 자격이다. "보는 자리"는 커널에서 없어지고, 클라이언트마다 보는 thread id 가
  된다. K2-3 의 `background_sessions` 와 바꿔 넣기는 thread id 맵으로 바뀐다.
- 이벤트는 대상이 붙어 나간다: `Thread(thread_id)`, `Client(client_id)`, `Kernel`. 호스트는 thread 이벤트를
  그 thread 를 보는 클라이언트에게, 클라이언트 이벤트를 그 클라이언트에게, 커널 이벤트를 모두에게 보낸다.
- 같은 프로세스의 TUI 는 `ClientId` 하나를 가진 클라이언트가 된다. 화면 동작은 지금과 같다.

### 3.3 응용 프로토콜 v0 (`akra/kernel/*`)

JSON-RPC 2.0. 전송은 Gateway 의 기존 접속을 그대로 쓴다. 직렬화 타입과 core 타입 사이의 변환은 inbound
adapter `src/adapter/inbound/kernel_protocol/` 에 둔다. core 는 직렬화를 모른다.

| 메서드 | 종류 | 내용 |
| --- | --- | --- |
| `akra/kernel/attach` | 요청 | `{ workspace, client_name }` → `{ client_id, protocol_version, kernel }`. 그 작업 공간의 커널에 붙는다 (없으면 만든다) |
| `akra/kernel/describe` | 요청 | 메서드, 명령, 이벤트 목록과 버전 (§4.3 자기 설명) |
| `akra/kernel/command` | 요청 | `{ command, idempotency_key? }` → `{ admission }`. 오래 걸리는 명령은 받아들임만 답하고 결과는 이벤트로 온다 |
| `akra/kernel/query` | 요청 | 세션 목록, 보는 대화, thread 요약, planning 투영 |
| `akra/kernel/event` | 알림 (서버→클라이언트) | `{ target, event }`. attach 뒤 자동으로 받는다 |
| `akra/kernel/detach` | 요청 | 클라이언트 세션을 닫는다. 접속이 끊겨도 같다 |

v0 명령은 화면 흐름 단계가 아니라 한 번에 끝나는 도메인 명령이다 (§5.5): 세션 목록 읽기, 대화 열기,
새 초안, 프롬프트 제출(준비 + 제출을 하나로), steer, 정지, 승인 결정, 자동 후속 설정, 이름 바꾸기, planning
초기화(확인 필수, K-D1 의 TUI 부분). 나머지 명령은 K4 에서 명령 스펙으로 옮길 때 더한다.

호출자 신원은 접속이다 (로컬 소유자만 접속할 수 있다). 멱등 키는 같은 클라이언트의 같은 키 재전송을 한 번만
처리한다. 원격 인증은 만들지 않는다 (Q-K6).

### 3.4 작업 공간당 커널 하나

- Gateway 는 작업 공간 루트(정규화한 경로)마다 커널을 하나 둔다. 첫 attach 때 만들고, 클라이언트가 모두
  떠나도 일이 남아 있으면(턴, 평가, 병렬, PR 검증) 계속 돈다.
- 작업 공간 잠금: 커널을 돌리는 프로세스는 `.codex-exec-loop/` 아래 커널 잠금 파일을 OS 파일 잠금으로 잡는다.
  잠금을 못 잡으면 그 작업 공간의 커널을 만들지 않고 이유를 답한다. 이것으로 TUI 와 데몬이 같은 작업 공간에서
  스케줄러와 병렬 제어를 두 번 돌리지 않는다 (K-D5). 병렬 정지가 다른 프로세스에 닿지 않는 문제(K-D3)는
  커널이 하나뿐이라 생기지 않는다.

### 3.5 TUI 와의 관계 (K6 전까지)

- TUI 는 지금처럼 자기 프로세스 안에 커널을 갖고, 같은 잠금을 잡는다.
- 데몬이 이미 그 작업 공간의 커널을 갖고 있으면 TUI 는 시작할 때 이유를 보이고 끝난다. 그 반대도 같다.
- TUI 가 데몬 커널에 원격 클라이언트로 붙는 일은 K6(얇은 TUI)에서 한다.

### 3.6 경계 테스트

- core 는 지금처럼 `serde_json` 과 adapter 를 모른다.
- `kernel_protocol` 만 직렬화 타입을 core 타입으로 바꾼다. Gateway 는 `kernel_protocol` 을 거쳐서만 커널에
  닿는다.
- 커널 상태는 커널 스레드만 바꾼다 (`KernelHost` 밖에서 `NativeClientRuntime` 을 직접 부르지 않는다).

## 4. 단계 (PR 단위)

| 단계 | 내용 | 기존 동작 | 검증 |
| --- | --- | --- | --- |
| K3-1 | §3.3 프로토콜 v0 의 타입과 변환 (`kernel_protocol`). 전송 없음 | 유지 | 직렬화 왕복 시험, 모든 v0 명령의 core 변환 시험, `describe` 내용 고정 |
| K3-2 | §3.1 `KernelHost` 와 `KernelClientHandle`. 클라이언트 하나 | 유지 | 화면 없는 시험: 핸들로 명령을 보내고 턴 종료 → 평가 → 자동 후속 이벤트를 받는다 |
| K3-3 | §3.2 클라이언트 세션과 대상이 붙은 이벤트. TUI 는 클라이언트 하나 | 유지 | 두 클라이언트가 다른 대화를 보고, 각자 자기 결과만 받는 core·호스트 시험 |
| K3-4 | §3.4 작업 공간 잠금과 작업 공간당 커널 | 바뀜: 같은 작업 공간에 커널 둘이 안 돈다 | 잠금 경합 시험, TUI 시작 시 거절 메시지 |
| K3-5 | Gateway 가 `akra/kernel/*` 를 커널 호스트에 연결 | 확장 | Gateway 시험: attach → command → event, 접속 끊김 → detach |
| K3-6 | Desktop 이 `akra/kernel/*` 로 세션 활동과 자동 후속 상태를 보인다 | 확장 | Desktop 시험, 숨긴 창 텍스트 캡처 |

규모: plan 18 추정 5~10일을 유지한다. K3-3 이 가장 크다 (K2-3 의 보는 자리 구조를 클라이언트별로 바꾼다).

## 5. 위험

| 위험 | 대응 |
| --- | --- |
| 커널 스레드와 비동기 Gateway 사이에서 이벤트가 밀린다 | 클라이언트마다 상한이 있는 채널. 넘치면 그 클라이언트에 "다시 조회" 이벤트를 보내고 오래된 증분은 버린다 |
| 데몬 커널이 오래 돌며 app-server 자식이 죽는다 | 지금 adapter 의 재시작 규칙을 쓴다. 커널 상태는 thread 별 실패로 끝낸다 |
| 작업 공간 잠금이 남아 다음 실행이 막힌다 | OS 파일 잠금이라 프로세스가 끝나면 풀린다. 잠금 파일 내용(pid, 시작 시각)은 진단용이다 |
| 프로토콜 v0 가 K4 명령 스펙과 어긋난다 | v0 는 `protocol_version` 을 알린다. K4 에서 스펙으로 생성하면서 v1 로 올린다 |
| 두 클라이언트가 같은 thread 에 동시에 명령을 보낸다 | 커널 스레드 하나가 차례로 처리한다. 지금의 correlation 규칙이 늦은 명령을 거절한다 |

## 6. 미결

| ID | 질문 | 기본값 (답이 없을 때) |
| --- | --- | --- |
| Q-K3-1 | 커널을 어느 프로세스에 둘 것인가 | `akra-gateway`. 이미 사용자별 데몬이고 endpoint 보안이 있다 |
| Q-K3-2 | 첫 원격 클라이언트 | Desktop. 이미 Gateway 에 붙어 있다 |
| Q-K3-3 | K3 동안 TUI 는 어떻게 하는가 | 자기 프로세스 커널을 유지하고 작업 공간 잠금을 함께 쓴다. 데몬 커널에 붙는 일은 K6 |
| Q-K3-4 | 커널 단위 | 작업 공간당 하나 |
| Q-K3-5 | 이벤트 전달 | K3 는 접속 중 실시간 알림만. 영구 기록과 커서는 K5 |
