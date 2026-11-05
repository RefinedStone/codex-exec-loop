# 참고: 공개 Codex app-server

상태: 조사 기록 · 조사일: 2026-11-05

## 출처와 버전

| 항목 | 값 |
| --- | --- |
| 저장소 | <https://github.com/openai/codex> (Apache-2.0) |
| 프로토콜 조사 기준 | main 브랜치 `3a2788e` (2026-11-04) |
| 라우팅 코드 조사 기준 | 2026-11-05 의 main 브랜치 |
| 설치된 CLI | `codex-cli 0.149.1` (`/opt/homebrew/bin/codex`, Akra 가 PATH 로 찾는 실행 파일) |
| 공식 앱 번들 CLI | `codex-cli 0.166.0-alpha.6` (참고용, 공식 데스크톱 앱 안에 포함) |
| 기존 경쟁 분석 | [docs/competitive/upstream-codex/README.md](../../../competitive/upstream-codex/README.md) (2026-08 기준) |

## 1. 실행 형태

| 명령 | 설명 |
| --- | --- |
| `codex app-server` | 기본 전송은 stdio 이다. 한 줄에 JSON-RPC 메시지 하나를 쓴다 (NDJSON). Akra 의 현재 방식이다 |
| `codex app-server --listen unix://PATH` | unix 소켓에서 접속을 받는다. 메시지는 **WebSocket 프레임**이다 (P0 실측). 여러 접속을 받는다. Windows 에서도 동작하며, 소켓 디렉터리가 현재 사용자 전용이어야 한다 (2026-11-05 실측) |
| `codex app-server --listen ws://IP:PORT` | TCP WebSocket 으로 접속을 받는다 |
| `codex app-server daemon start/stop/restart/version` | Codex 관리형 데몬. 제어 소켓은 `~/.codex/app-server-control/app-server-control.sock` 이다 |
| `codex app-server proxy [--sock PATH]` | stdio 와 unix 소켓 사이에서 바이트를 그대로 전달한다. 클라이언트는 stdio 위에서 WebSocket 핸드셰이크부터 해야 한다. `--sock` 을 생략하면 Codex 관리형 데몬의 제어 소켓에 접속한다 (2026-11-05 Windows 실측) |
| `codex app-server generate-ts` | 프로토콜의 TypeScript 타입을 생성한다 (실험 기능) |
| `codex app-server generate-json-schema` | 프로토콜의 JSON Schema 를 생성한다 (실험 기능) |
| `-c key=value`, `--enable`, `--disable` | 설정을 덮어쓴다 |

## 2. 접속 수명

1. 클라이언트가 `initialize` 를 보낸다. params 는 `clientInfo {name, version}` 와 `capabilities` 이다.
2. 응답 result 의 키: `codexHome`, `platformFamily`, `platformOs`, `userAgent` (P0 실측).
3. 클라이언트가 `initialized` 알림을 보낸다.
4. 이후 요청을 보낸다.

`capabilities` 의 주요 필드 (`InitializeCapabilities`):

| 필드 | 뜻 |
| --- | --- |
| `experimentalApi` | 실험 메서드와 필드를 받는다 |
| `requiredHumanApproval` | humanCommand 승인을 받는다. `experimentalApi` 가 필요하다 |
| `optOutNotificationMethods` | 이 접속에서 받지 않을 알림 이름 목록 |
| `requestAttestation` | 증명 요청을 받는다 |
| `extensions` | MCP 확장 설정 |

## 3. 데이터 모델

### Project (`v2/project.rs`)

| 필드 | 뜻 |
| --- | --- |
| `id`, `name` | 식별자, 이름 |
| `roots` | 경로 목록 `[{path}]`. 저장소 여러 개를 하나의 Project 로 묶을 수 있다 |
| `metadata` | 문자열 맵. Akra 는 `akra.*` 키를 쓴다 (D-10) |
| `position` | 사이드바 순서 |
| `createdAt`, `updatedAt`, `recencyAt` | 시각. `recencyAt` 은 최근 스레드 활동 시각이다 |

메서드: `project/list` (cursor, limit, sortKey, sortDirection), `project/read`, `project/create`
(idempotencyKey 필수), `project/import`, `project/update`, `project/move`, `project/delete`.
알림: `project/changed`, `thread/project/updated`.

### Thread

- `thread/start` 의 주요 params: `cwd`, `projectId`, `model`, `approvalPolicy`, `sandbox`, `config`,
  `baseInstructions`, `developerInstructions`, `ephemeral`, `runtimeWorkspaceRoots`, `serviceName`.
- 0.149.1 의 thread 필드 (P0 실측): `agentNickname`, `agentRole`, `canAcceptDirectInput`,
  `cliVersion`, `createdAt`, `cwd`, `ephemeral`, `extra`, `forkedFromId`, `gitInfo`, `historyMode`,
  `id`, `modelProvider`, `name`, `parentThreadId`, `path`, `preview`, `projectId`, `recencyAt`,
  `section`, `sectionEnteredAt`, `sessionId`, `source`, `status`, `threadSource`, `turns`,
  `updatedAt`.

### Section

`threadSection/create`, `threadSection/list`, `threadSection/update`, `threadSection/delete`,
`thread/section/move`, 알림 `thread/section/changed`.

## 4. 주요 메서드와 알림

공개 프로토콜(`common.rs`)에는 요청과 알림 이름이 약 271개 있다. 공식 데스크톱 앱은 그중 약 255개를
쓴다 ([codex-desktop-app.md](codex-desktop-app.md)).

| 분류 | 이름 |
| --- | --- |
| 스레드 | `thread/start`, `thread/resume`, `thread/fork`, `thread/list`, `thread/read`, `thread/archive`, `thread/unarchive`, `thread/name/set`, `thread/unsubscribe`, `thread/settings/update`, `thread/items/list`, `thread/turns/list`, `thread/compact/start` |
| 턴 | `turn/start`, `turn/steer`, `turn/interrupt` |
| 작업공간 | `project/*`, `threadSection/*` |
| 기타 조회 | `model/list`, `config/read`, `account/read` |
| 턴 알림 | `turn/started`, `turn/completed`, `turn/diff/updated`, `turn/plan/updated` |
| 아이템 알림 | `item/started`, `item/completed`, `item/agentMessage/delta`, `item/reasoning/summaryTextDelta`, `item/commandExecution/outputDelta`, `item/fileChange/outputDelta` |
| 스레드 알림 | `thread/status/changed`, `thread/tokenUsage/updated`, `thread/compacted`, `thread/readState/changed` |
| 서버 요청 | `item/commandExecution/requestApproval`, `item/fileChange/requestApproval`, `item/permissions/requestApproval`, `item/tool/requestUserInput`, `mcpServer/elicitation/request` |
| 요청 해결 알림 | `serverRequest/resolved` |

정확한 params 와 필드는 `generate-ts` 산출물을 기준으로 한다.

## 5. 다중 접속 규칙

| 항목 | 규칙 | 근거 |
| --- | --- | --- |
| 접속별 세션 | 접속마다 initialize 를 따로 한다. 요청 ID 는 접속별로 독립이다 | P0 실측 (0.149.1) |
| 스레드 알림 | 그 스레드를 구독한 접속에만 보낸다 | `app-server/src/outgoing_message.rs` (`ThreadScopedOutgoingMessageSender`) |
| 구독 시작 | `thread/start`, `thread/resume`, `thread/fork` | `app-server/README.md`, `thread_state.rs` |
| 구독 종료 | `thread/unsubscribe` 또는 접속 종료 | `app-server/README.md`, `thread_state.rs` |
| 전역 알림 | 모든 접속에 보낸다 | `outgoing_message.rs` (`send_global_server_notification`) |
| 승인 요청 | 그 스레드를 구독한 모든 접속에 보낸다 | `outgoing_message.rs` (`send_request_to_connections`) |
| humanCommand 승인 | `experimentalApi` 와 `requiredHumanApproval` 를 선언한 구독 접속에만 보낸다. 그런 접속이 없으면 명령을 거부한다 | `app-server/README.md`, `requires_human_approval`, `request_processors/initialize_processor.rs` |
| 요청 해결 | 요청이 해결되면 구독 접속 모두에 `serverRequest/resolved {threadId, requestId}` 를 보낸다 | `outgoing_message.rs` (`resolve_server_request`) |
| replay | 스레드를 새로 구독한 접속에 대기 중인 요청을 다시 보낸다 | `outgoing_message.rs` (`replay_requests_to_connection_for_thread`) |
| 턴 상태 변경 | 대기 중인 요청을 오류로 끝낸다 | `outgoing_message.rs` (`abort_pending_server_requests`) |
| 알림 거부 | 접속마다 `optOutNotificationMethods` 로 고른다 | `InitializeCapabilities` |

아직 확인하지 않은 것:

- 두 접속이 같은 승인 요청에 거의 동시에 답할 때 두 번째 답의 처리 (Q-03)
- 구독 접속이 모두 끊긴 스레드의 진행 중인 턴이 계속되는지 (Q-09)
- 0.149.1 의 라우팅이 위의 main 브랜치 규칙과 같은지

## 6. 문맥 제어 지점 (R6 근거)

| 지점 | 내용 |
| --- | --- |
| 설정 `compact_prompt` | compact 할 때 쓰는 요약 지시문 |
| 설정 `model_auto_compact_token_limit` | 자동 compact 를 시작하는 문맥 크기 |
| `thread/compact/start` | compact 를 원하는 시점에 시작한다. 끝나면 `thread/compacted` 알림이 온다 |
| `thread/items/list`, `thread/turns/list`, `thread/read` | 기록을 읽는다 |
| `thread/fork` | 스레드를 갈라서 새 스레드를 만든다 |
| `thread/start` 의 `developerInstructions` | 새 스레드에 지시문을 넣는다. 직접 만든 요약을 넣을 수 있다 |

모델에 들어가는 문맥 자체는 엔진(L2)이 구성한다. 클라이언트는 위 지점으로만 개입할 수 있다.

## 7. 버전 사이의 차이 (P0 실측)

| 응답 | 0.149.1 | 0.166.0-alpha.6 에서 추가된 키 |
| --- | --- | --- |
| `thread/list` result | `backwardsCursor`, `data`, `nextCursor` | `readStates` |
| thread 객체 | §3 목록 | `daybreakEnabled`, `environments`, `model`, `originator`, `reasoningEffort` |
| `account/read` result | `account`, `requiresOpenaiAuth` | `workspaceRouting` |

모든 차이는 필드 추가였다. 모르는 필드를 무시하는 클라이언트는 영향을 받지 않는다.

## 8. 참고한 소스 파일

- `codex-rs/app-server/README.md`
- `codex-rs/app-server/src/outgoing_message.rs`
- `codex-rs/app-server/src/thread_state.rs`
- `codex-rs/app-server/src/request_processors/initialize_processor.rs`
- `codex-rs/app-server-protocol/src/protocol/common.rs`
- `codex-rs/app-server-protocol/src/protocol/v2/project.rs`
- `codex-rs/app-server-protocol/src/protocol/v2/thread.rs`
- `codex-rs/app-server-protocol/schema/typescript/InitializeCapabilities.ts`
- `codex-rs/app-server-client/src/remote.rs` (원격 WebSocket 클라이언트: `websocket_url`, `auth_token`)
