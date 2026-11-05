# 참고: 현재 Akra

상태: 조사 기록 · 조사일: 2026-11-05 · 기준 커밋: `0732aea8` (`origin/prerelease`)

현재 동작의 공식 설명은 [architecture.md](../../../reference/architecture.md) 와
[current-product.md](../../../reference/current-product.md) 이다. 이 문서는 이 계획에 필요한 부분만
요약한다.

## 1. 바이너리

| 바이너리 | 소스 | 역할 |
| --- | --- | --- |
| `akra` | `src/bin/akra.rs` | 기본 TUI 와 CLI |
| `akra-admin` | `src/bin/akra-admin.rs` | 관리 웹 (axum, HTTP + SSE) |
| `akra-telegram` | `src/bin/akra-telegram.rs` | Telegram 입구 |

입구마다 바이너리를 둔다. 같은 application 서비스를 공유한다.

## 2. 계층과 크기

의존 방향은 `adapter/inbound -> core 또는 application -> domain` 이다. 아키텍처 경계 테스트
(`tests/architecture_boundaries.rs`)가 이 방향을 검사한다.

| 경로 | 줄 수 (인라인 테스트 포함) | 이 계획과의 관계 |
| --- | --- | --- |
| `src/adapter/inbound/tui` | 81,325 | 유지한다 (D-08) |
| `src/adapter/inbound/admin_api` | 13,684 | 대화 전송에는 쓰지 않는다 (D-09) |
| `src/adapter/inbound/telegram_bot` | 4,364 | 유지한다 |
| `src/adapter/inbound/cli` | 308 | 유지한다 |
| `src/adapter/outbound` | 70,625 | app-server 실행 코드를 Gateway 가 재사용한다 |
| `src/application` | 119,699 | MVP 에서는 바꾸지 않는다 |
| `src/core` | 22,513 | MVP 에서는 바꾸지 않는다 |
| `src/domain` | 22,960 | MVP 에서는 바꾸지 않는다 |
| `src/composition` | 9,844 | Gateway 조립 함수를 추가한다 |

측정 명령: 디렉터리별 `*.rs` 파일의 줄 수 합계 (2026-11-05).

## 3. 대화 경로

### 대화 포트

`src/application/port/outbound/interactive_turn_runtime_port.rs` 의 `InteractiveTurnRuntimePort`:

| 메서드 | 용도 |
| --- | --- |
| `runtime_control_truth` | 실행 상태 조회 |
| `load_conversation_snapshot` | 스레드 기록 조회 |
| `request_stop_all_sessions` | 모든 세션 중지 |
| `resolve_approval_request` | 승인 응답 |
| `steer_turn` | 진행 중인 턴에 지시 추가 |
| `run_new_thread_stream` | 새 스레드 시작과 첫 턴 스트리밍 |
| `run_turn_stream` | 기존 스레드의 턴 스트리밍 |

### 대화 이벤트

`src/domain/conversation_stream.rs` 의 `ConversationStreamEvent` 16종:
`AttachmentObserved`, `ThreadPrepared`, `TurnStarted`, `RuntimeEnvelopeObserved`,
`ItemLifecycleObserved`, `ProgressiveActivityObserved`, `StatusUpdated`, `AgentMessageCompleted`,
`ToolActivity`, `ApprovalReviewUpdated`, `ApprovalRequested`, `ApprovalResolved`,
`TurnInterruptRequestFailed`, `TurnRetrying`, `TurnTerminal`, `Failed`.

Gateway 는 이 포트와 이벤트를 쓰지 않는다 (D-11). TUI 와 하네스는 계속 쓴다.

## 4. app-server 실행 방식

구현: `src/adapter/outbound/app_server/connection.rs`, 조립: `src/composition/production.rs`
(`CodexAppServerAdapter`).

| 항목 | 내용 |
| --- | --- |
| 전송 | stdio, 한 줄에 JSON 메시지 하나 |
| 실행 인자 | `app-server -c shell_environment_policy.inherit="..." -c shell_environment_policy.ignore_default_excludes=false -c allow_login_shell=false` |
| 환경 변수 | 허용 목록(`PATH`, `HOME` 등)만 전달한다. `OPENAI_API_KEY`, `CODEX_API_KEY` 는 전달하지 않는다 |
| 실행 파일 | `src/trusted_executable.rs` 의 `pinned_codex_command` 가 PATH 에서 codex 를 찾아 신뢰 여부를 확인하고 고정한다 |
| initialize | `clientInfo` + `capabilities {experimentalApi: false}` |
| 프로토콜 타입 | Codex 크레이트에 의존하지 않고 직접 정의한다. `deny_unknown_fields` 를 쓰지 않아서 새 필드를 무시한다 |
| 개발자 지시문 | 일반 대화 스레드는 `developer_instructions: None`, 계획 작업자 스레드는 전용 지시문을 넣는다 (`src/adapter/outbound/app_server/mod.rs`) |

Gateway 의 upstream 실행은 위 설정을 그대로 재사용한다 (P-02). 다른 점은 `--listen unix://<경로>`
인자뿐이다.

## 5. 관리 웹 (admin_api)

- axum 기반이다. 실시간 갱신은 SSE(`/api/admin/akra/stream`)이다.
- 계획·작업·대시보드 API 가 있다 (`/api/planning/*`, `/api/admin/akra/*`).
- 관리 화면을 위한 표면이다. 대화의 실시간 전송에는 쓰지 않는다 (D-09).

## 6. 설정과 저장 위치

| 항목 | 위치 |
| --- | --- |
| 전역 설정 | `$AKRA_HOME/config.toml` (기본 `~/.akra/config.toml`) |
| 프로젝트 설정 | `<저장소>/.akra/config.toml` |
| 계획 데이터 | `<저장소>/.codex-exec-loop/` (`planning-authority.db` 등) |

Akra 의 하네스는 프로세스 하나가 저장소 하나를 다룬다는 전제로 작성되었다. D-10 에 따라 하네스 연동(R5)
때 이 전제를 바꿔야 한다.

## 7. 빌드와 테스트 기준선 (2026-11-05, 이 Mac)

| 항목 | 결과 |
| --- | --- |
| 툴체인 | Rust 1.95.0 (`rust-toolchain.toml` 과 같음) |
| `cargo build --release` | 성공, 약 2분 19초 |
| `akra doctor` | "planning workspace is healthy" |
| `cargo test --release` | 테스트 3,126개 중 3,099 통과, 25 실패, 2 무시. 약 720초 |
| 실패 테스트 재실행 | 샌드박스 밖에서 실패 항목만 다시 실행하자 **다른 테스트가 실패했다** |

실패는 실행마다 바뀌었다. 모두 `parallel_mode` (worktree 풀, PR 분배기) 와 git·curl 을 실제로
실행하는 테스트, 그리고 admin PR 검증 보드 테스트 하나였다. 병렬 실행의 타이밍 문제로 보인다. 대화
경로와 Gateway 작업 영역에는 해당하지 않는다. 하네스 연동(R5) 전에 원인을 정리한다 (K-02).

## 8. 열린 작업 (2026-11-05)

| 브랜치 | prerelease 대비 | 마지막 커밋 |
| --- | --- | --- |
| `codex/akra-config-layers` | 1 앞섬, 39 뒤처짐 | 2026-08-11 |
| `fix/thread-start-model` | 1 앞섬, 0 뒤처짐 | 2026-08-26 |

열린 PR 은 없다. 두 브랜치의 처리 방법은 사용자에게 확인한다.
