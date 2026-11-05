# P1. Gateway 데몬

상태: **다음 작업** (제안) · 기준일: 2026-11-05 · 규모: Rust 수백~1,000줄과 테스트, 약 1주

## 1. 목표

`akra-gateway` 데몬을 만든다. 데몬은 unix 소켓에서 클라이언트 접속을 받고, 접속마다 Codex
app-server 접속을 하나씩 열어 메시지를 그대로 중계한다.

## 2. 선행 조건

- Q-10: 모노레포(D-04)를 사용자에게 확인한다.
- P-01~P-04 를 사용자에게 확인한다. 더 나은 안이 있으면 근거와 함께 제시한다.
- [06 인계 가이드](../06-handoff-guide.md) 의 저장소 절차대로 전용 worktree 를 만든다.

## 3. 범위

| 범위 안 | 범위 밖 |
| --- | --- |
| `akra-gateway` 바이너리 | interceptor 의 실제 동작 (자리만 만든다) |
| unix 소켓 Listener (WebSocket) | `akra/gateway/info` 외의 `akra/*` 메서드 |
| 클라이언트 접속마다 upstream 접속 1개 | OS 서비스 등록 (launchd 등, R8) |
| upstream app-server 실행, 감시, 재시작 | TUI·하네스의 app-server 통합 (P-08) |
| `akra/gateway/info` | TCP 접속 |
| 단일 인스턴스, 경로·권한, 종료 처리 | 데스크톱 앱 (P2) |
| 녹화 옵션 (로컬 전용) | |
| 테스트와 수동 검증 | |

## 4. 설계

### 4.1 구성 요소

| 구성 요소 | 책임 |
| --- | --- |
| GatewayListener | `$AKRA_HOME/run/gateway.sock` 에서 접속을 받고 WebSocket 핸드셰이크를 한다 |
| GatewaySession | 클라이언트 접속 하나를 맡는다. upstream 접속 하나를 연다. 양방향 전달 작업 두 개를 실행한다 |
| MessageRouter | 클라이언트 메시지의 `method` 를 읽고 `akra/*` 처리, interceptor, 전달 중 하나를 고른다 |
| AkraMethodHandler | `akra/*` 요청에 응답한다 |
| UpstreamSupervisor | app-server 를 실행하고 감시한다. 종료되면 재시작한다 |
| FrameRecorder | 녹화 옵션이 켜져 있으면 메시지를 파일에 기록한다 |

### 4.2 메시지 처리 규칙

1. 클라이언트에서 온 텍스트 프레임은 JSON 으로 읽는다. `method` 필드만 확인한다.
2. `method` 가 `akra/` 로 시작하는 요청이면 Gateway 가 응답한다. upstream 으로 보내지 않는다.
3. 그 밖의 메시지는 바이트를 바꾸지 않고 upstream 으로 보낸다.
4. upstream 에서 온 메시지는 바이트를 바꾸지 않고 클라이언트로 보낸다.
5. 방향마다 작업을 하나만 두어서 메시지 순서를 유지한다.
6. JSON 으로 읽을 수 없는 메시지도 그대로 전달한다. 판단은 app-server 가 한다.
7. ping/pong 은 구간마다 처리한다. close 는 반대쪽으로 전파한다.

interceptor 자리 (제안, MVP 에서는 모두 "그대로 전달"):

```rust
enum ClientMessageAction { Forward, Replace(String), Respond(serde_json::Value) }
enum UpstreamMessageAction { Forward, Replace(String), Drop }
```

### 4.3 `akra/gateway/info`

- 요청: `{"jsonrpc":"2.0","id":1,"method":"akra/gateway/info"}`
- 응답 result (제안): `gatewayVersion`, `codexVersion`, `startedAt`, `upstreamRestarts`
- initialize 전에도 응답한다. app-server 의 initialize 규칙과 무관하다.

### 4.4 upstream 관리

- 실행 파일: `pinned_codex_command` 로 PATH 의 codex 를 찾고 신뢰 여부를 확인한다.
- 실행 인자: Akra 의 기존 보안 `-c` 설정 + `--listen unix://$AKRA_HOME/run/codex-app-server.sock`.
- 환경 변수: Akra 의 기존 허용 목록을 그대로 쓴다.
- 기존 실행 코드(`src/adapter/outbound/app_server/connection.rs`)에서 실행 설정 부분을 함수로 분리해서
  재사용한다. 기존 stdio 경로의 동작은 바꾸지 않는다.
- 시작 확인: upstream 소켓에 접속할 수 있을 때까지 기다린다 (제한 시간 10초).
- 버전: 시작할 때 `codex --version` 을 한 번 실행해서 기록한다.
- 종료 감지: 자식 프로세스가 끝나면 모든 클라이언트 접속을 close 코드 1012 (service restart) 로 닫는다.
  1초에서 시작해 두 배씩 늘려 최대 30초 간격으로 재시작한다. 60초 동안 정상이면 간격을 초기화한다.
- 정지: SIGINT, SIGTERM 을 받으면 클라이언트 접속을 닫고 자식 프로세스를 종료하고 소켓 파일을 지운다.

### 4.5 단일 인스턴스, 경로, 권한

- `$AKRA_HOME/run` 을 권한 0700 으로 만든다. 소켓은 0600 이다.
- 소켓 경로가 103바이트를 넘으면 시작하지 않는다 (macOS 한도 104바이트, 끝의 NUL 포함).
- `$AKRA_HOME/run/gateway.lock` 에 파일 잠금을 건다. 잠금을 얻지 못하면 다른 인스턴스가 실행 중이다.
  안내를 출력하고 종료한다.
- 잠금을 얻었는데 소켓 파일이 남아 있으면 이전 실행의 흔적이다. 지우고 다시 만든다.

### 4.6 로그와 녹화

- 로그는 `$AKRA_HOME/log/` 에 쓴다. 메서드 이름, 크기, 접속 번호, 시각만 기록한다. 메시지 본문은 대화
  내용을 포함하므로 기록하지 않는다.
- 녹화: 환경 변수(예: `AKRA_GATEWAY_RECORD=1`)가 켜져 있을 때만 접속 메시지를 JSONL 로 기록한다.
  위치는 `$AKRA_HOME/recordings/`, 파일 권한은 0600 이다.
- 녹화 파일은 실제 대화를 담는다. 저장소에 커밋하지 않는다. 시험용 픽스처는 내용을 바꾼 사본만 쓴다.

### 4.7 의존성 (제안)

- `tokio-tungstenite`: unix 소켓 위의 WebSocket 서버(`accept_async`)와 클라이언트(`client_async`)
- `futures-util`: 스트림 처리
- 현재 Akra 에는 WebSocket 의존성이 없다. axum 에도 `ws` 기능이 켜져 있지 않다.
- 의존성을 추가하면 `Cargo.lock` 을 함께 커밋한다. CI 는 `cargo test --locked` 와
  `cargo clippy --locked --all-targets --all-features -- -D warnings` 를 실행한다.
- 저장소에는 자동 라이선스 검사가 없다. 추가하는 크레이트의 라이선스가 MIT 또는 Apache-2.0 인지 직접
  확인한다.

### 4.8 모듈 위치와 아키텍처 규칙

Akra 의 아키텍처 테스트(`tests/architecture_boundaries.rs`)를 통과해야 한다. 확인할 규칙은 다음과 같다.

- 의존 방향: `adapter/inbound -> core 또는 application -> domain`
- outbound 어댑터는 outbound 포트를 구현하고 서비스에 의존하지 않는다.
- 경계 가드가 `src` Rust 파일의 90% 이상을 덮어야 한다.

배치안 (작업 1 에서 확정):

| 위치 | 내용 |
| --- | --- |
| `src/bin/akra-gateway.rs` | 진입점 |
| `src/adapter/inbound/gateway/` | Listener, Session, Router, AkraMethodHandler |
| `src/application/port/outbound/` | upstream 접속을 여는 포트 |
| `src/adapter/outbound/app_server/` | upstream 실행과 접속 구현 |
| `src/composition/` | Gateway 조립 |

순수 중계 경로에 application 서비스가 필요한지 판단한다. 규칙을 지키는 가장 작은 구조를 고른다.

## 5. 작업 순서

1. **구조 확정:** 아키텍처 테스트를 읽고 모듈 위치와 포트를 정한다. 결과를 사용자에게 확인한다.
2. **실행 코드 분리:** 기존 app-server 실행 설정을 재사용할 수 있게 분리한다. 기존 테스트가 그대로
   통과해야 한다.
3. **Listener 와 Session:** 클라이언트 접속마다 upstream 접속을 열고 양방향으로 전달한다.
4. **`akra/gateway/info`.**
5. **수명 관리:** 단일 인스턴스, 경로·권한 검사, 재시작, 종료 처리.
6. **녹화 옵션.**
7. **테스트:**
   - 단위: 라우팅 규칙(`akra/*` 와 전달 구분), 경로 길이 검사, 재시작 간격 계산
   - 통합 (가짜 upstream): 다중 클라이언트, 순서 보장, upstream 재시작 시 접속 종료, close 전파
   - 통합 (실제 codex, 턴 없음): 접속 2개의 initialize 와 `thread/list`. 로컬에서만 실행하도록 표시한다
   - `tests/binary_entrypoints.rs` 에 새 바이너리의 도움말과 오류 계약을 추가한다
8. **수동 검증:** `scripts/multi_client_check.py --socket ~/.akra/run/gateway.sock` 을 실행한다.
9. **실측 (사용자 허락 후):** 아래 §6 절차로 Q-03 과 Q-09 를 확인하고 녹화 파일을 얻는다.

## 6. 실측 절차 (턴 사용, 허락 필요)

사용자에게 "턴 2~3회, 짧은 프롬프트, 낮은 추론 강도"를 알리고 허락을 받는다.

1. 녹화 옵션을 켜고 Gateway 를 시작한다.
2. 접속 A 와 B 를 연다. 둘 다 initialize 한다.
3. A 에서 `thread/start` 를 보낸다. B 에서 같은 스레드를 `thread/resume` 한다.
4. A 에서 승인이 필요한 짧은 턴을 시작한다 (예: 파일 하나를 만드는 요청, 승인 정책은 매번 확인).
5. 승인 요청이 A 와 B 에 모두 오는지 확인한다.
6. A 와 B 가 거의 동시에 답한다. 두 번째 답의 응답과 `serverRequest/resolved` 를 기록한다 (Q-03).
7. 두 번째 턴을 시작하고, 진행 중에 A 와 B 를 모두 닫는다. 잠시 뒤 새 접속에서 `thread/resume` 한다.
   턴이 끝까지 진행되었는지 확인한다 (Q-09).
8. 시험 스레드를 archive 한다.
9. 결과를 [05](../05-risks-and-open-questions.md) 의 Q-03, Q-09 에 기록한다.

## 7. 완료 기준

1. `akra-gateway` 를 실행하면 `$AKRA_HOME/run/gateway.sock` 이 권한 0600 으로 생긴다.
2. `multi_client_check.py --socket` 이 Gateway 소켓에서 성공한다.
3. `akra/gateway/info` 가 codex 버전을 돌려준다.
4. upstream 을 강제로 종료하면 Gateway 가 재시작하고 클라이언트 접속이 close 코드 1012 로 닫힌다.
5. 두 번째 `akra-gateway` 실행은 안내를 출력하고 종료한다.
6. 아키텍처 테스트, 바이너리 진입점 테스트, 기존 app-server 어댑터 테스트가 통과한다. 불안정 테스트는
   기준선([akra-current.md](../reference/akra-current.md) §7)과 비교한다.
7. `node scripts/agent-plan.mjs` 가 출력한 검사를 통과한다.
8. (허락받은 경우) 실측 결과를 Q-03, Q-09 에 기록했다.

## 8. 위험

| 위험 | 대응 |
| --- | --- |
| 아키텍처 규칙 때문에 구조가 커진다 | 작업 1 에서 가장 작은 구조를 정하고 확인받는다 |
| 기존 실행 코드 분리 중 TUI 경로가 깨진다 | 분리 전후로 app-server 어댑터 테스트를 실행한다 |
| 0.149.1 의 다중 접속 동작이 main 브랜치와 다르다 | 실측(§6)으로 확인한다 |
| 구독자가 없으면 턴이 멈춘다 (Q-09) | Gateway 가 자체 접속으로 진행 중인 스레드를 구독해서 유지한다 |
