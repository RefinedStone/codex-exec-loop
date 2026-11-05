# 04. 목표 구조

상태: Proposed · 기준일: 2026-11-05

이 문서는 MVP(P1~P3) 이후의 목표 구조이다. 결정의 근거는 [03 결정 기록](03-decisions.md) 에 있다.

## 1. 구성도

```text
+------------------------ Akra Desktop (Electron) ------------------------+
| renderer (창마다)                                                        |
|   React UI, 대화 상태, JSON-RPC 요청 생성                                 |
|   권한 없음: sandbox, contextIsolation, Node 사용 불가                    |
|        | window.akra.* (preload 가 노출한 함수만)                          |
| preload                                                                  |
|        | IPC (메시지 버스)                                                  |
| main (Node)                                                              |
|   창마다 Gateway 접속 1개, 데몬 확인과 시작, OS 기능                       |
+--------+-----------------------------------------------------------------+
         | WebSocket, unix 소켓 $AKRA_HOME/run/gateway.sock
+--------v------------- akra-gateway (데몬, 컴퓨터당 1개) ------------------+
| Listener -> Session(클라이언트마다) -> Router                             |
|   akra/*       -> Gateway 가 직접 응답                                     |
|   interceptor  -> MVP 에서는 없음                                          |
|   그 외        -> 이 Session 의 upstream 접속으로 그대로 전달               |
| Upstream supervisor: app-server 실행, 감시, 재시작                         |
+--------+-----------------------------------------------------------------+
         | WebSocket, unix 소켓 $AKRA_HOME/run/codex-app-server.sock
         | (클라이언트 Session 마다 upstream 접속 1개)
+--------v------------- codex app-server (Gateway 의 자식 프로세스) --------+
| 공식 엔진. Project, Thread, Turn, Item 을 ~/.codex 에 저장한다             |
+--------------------------------------------------------------------------+

변경 없는 기존 경로: akra(TUI), akra-admin, akra-telegram -> 각자 app-server (stdio)
```

## 2. 프로세스와 책임

| 프로세스 | 수 | 책임 | 하지 않는 일 |
| --- | --- | --- | --- |
| renderer | 창마다 1개 | 화면, 대화 상태, 요청 생성, 승인 응답 | 파일·프로세스·네트워크 직접 접근 |
| main | 1개 | 창 관리, Gateway 접속, 데몬 시작, 메시지 중계 | 프로토콜 해석 (중계만 한다) |
| akra-gateway | 컴퓨터당 1개 | 접속 수용, upstream 관리, `akra/*` 처리 | 대화 저장, 메시지 수정 (interceptor 제외) |
| codex app-server | Gateway 당 1개 | 엔진 실행, 원본 데이터 저장, 알림 라우팅 | - |

## 3. 데이터 소유 (D-05)

| 데이터 | 소유자 | 저장 위치 | 접근 방법 |
| --- | --- | --- | --- |
| Project, Thread, Turn, Item, Section | Codex app-server | `~/.codex` | `project/*`, `thread/*`, `threadSection/*` |
| Project 와 Akra 의 연결 정보 | Akra | Codex Project metadata 의 `akra.*` 키 | `project/update` |
| 계획 큐, 작업, 슬롯 | Akra | 각 저장소의 `.codex-exec-loop/` | `akra/*` (R5) |
| 요약본, 스레드 연결 (compact) | Akra | `$AKRA_HOME` (R6 에서 결정) | `akra/*` (R6) |
| Gateway 설정, 로그, 녹화 파일 | Akra | `$AKRA_HOME` | 파일 |
| 창 크기, 마지막 선택 | Akra Desktop | Electron `userData` | 파일 |

## 4. 연결 규칙

1. main 은 창 하나마다 Gateway 접속을 하나 연다.
2. Gateway 는 클라이언트 접속 하나마다 upstream 접속을 하나 연다 (P-03).
3. initialize 는 renderer 가 만든다. main 과 Gateway 는 그대로 전달한다. 접속마다 따로 한다.
4. 요청 ID 는 접속 안에서만 유일하면 된다. Gateway 와 main 은 ID 를 바꾸지 않는다.
5. 창이 어떤 스레드의 이벤트를 받으려면 그 창의 접속에서 `thread/start`, `thread/resume`,
   `thread/fork` 중 하나를 보낸다. 이것이 구독이다. 보지 않는 스레드는 `thread/unsubscribe` 로
   구독을 끝낸다.
6. `akra/*` 메서드는 Gateway 가 응답한다. upstream 으로 보내지 않는다.
7. Gateway 는 그 밖의 메시지를 바이트 그대로 전달한다. 메시지마다 `method` 필드만 읽는다.

## 5. 승인 요청 (여러 창)

app-server 의 규칙 ([codex-app-server.md](reference/codex-app-server.md) §5):

- 승인 요청은 그 스레드를 구독한 모든 접속에 간다.
- 한 접속이 답하면 app-server 는 요청을 닫는다. 그리고 구독 접속 모두에
  `serverRequest/resolved {threadId, requestId}` 를 보낸다.
- 스레드를 새로 구독한 접속에는 대기 중인 요청을 다시 보낸다 (replay).
- 턴 상태가 바뀌면 대기 중인 요청은 오류로 끝난다.

Akra Desktop 의 규칙:

- 승인 대화상자는 요청을 받은 모든 창에 표시한다.
- `serverRequest/resolved` 를 받으면 그 대화상자를 닫는다.
- 두 창이 거의 동시에 답할 때의 처리는 실측으로 확인한다 (Q-03). 확인 전에는 먼저 도착한 답이
  쓰인다고 가정하고, 두 번째 답의 오류 응답은 무시한다.

## 6. 주요 흐름

### 6.1 앱 시작

1. main 이 `$AKRA_HOME/run/gateway.sock` 에 접속한다.
2. 접속에 실패하면 main 이 `akra-gateway` 를 시작하고 다시 접속한다. 나중에 OS 서비스로 바꾼다 (R8).
3. main 이 창을 만들고, 그 창의 Gateway 접속을 연다.
4. renderer 가 `initialize` 를 보내고 `initialized` 알림을 보낸다.
5. renderer 가 `akra/gateway/info` 로 upstream codex 버전을 확인한다. 생성 타입의 버전과 다르면
   경고를 표시한다.
6. renderer 가 `project/list` 와 `thread/list` 로 사이드바를 그린다.

### 6.2 대화

1. 사용자가 스레드를 고른다. renderer 가 `thread/resume` 을 보낸다. 응답으로 기록을 받고, 구독이
   시작된다.
2. 사용자가 메시지를 보낸다. renderer 가 `turn/start` 를 보낸다.
3. app-server 가 `turn/started`, `item/started`, `item/*` delta, `item/completed`,
   `turn/completed` 를 보낸다.
4. renderer 는 delta 를 화면 프레임 단위로 묶어서 그린다.
5. 진행 중에 사용자가 추가 지시를 하면 `turn/steer`, 중단하면 `turn/interrupt` 를 보낸다.

### 6.3 새 스레드

1. 사용자가 Project 를 고른다. Project 의 root 가 여러 개이면 cwd 로 쓸 root 를 고른다.
2. renderer 가 `thread/start {projectId, cwd}` 를 보낸다. 이 접속은 새 스레드를 구독한다.

### 6.4 앱 종료와 재시작

1. 앱을 닫으면 main 이 Gateway 접속을 닫는다. Gateway 는 그 Session 의 upstream 접속을 닫는다.
2. 진행 중인 턴은 app-server 에서 계속되어야 한다. 구독자가 없는 스레드의 턴이 계속되는지는 아직
   확인하지 않았다 (Q-09). 계속되지 않으면 Gateway 가 자체 접속으로 진행 중인 스레드를 구독해서
   유지한다.
3. 앱을 다시 열면 `thread/resume` 으로 기록과 현재 상태를 받는다. 대기 중인 승인 요청은 replay 로
   다시 온다.

## 7. 장애 처리

| 상황 | Gateway | main | renderer |
| --- | --- | --- | --- |
| upstream 종료 | 재시작한다 (대기 시간을 점점 늘린다). 기존 클라이언트 접속을 닫는다 | 다시 접속한다 | initialize 를 다시 하고, 보던 스레드를 resume 한다 |
| Gateway 종료 | - | 다시 접속한다. 계속 실패하면 데몬을 시작한다 | "연결 끊김"을 표시한다 |
| 클라이언트 비정상 종료 | 그 Session 의 upstream 접속을 닫는다 | - | - |
| 소켓 경로가 104바이트 초과 | 시작하지 않고 오류를 출력한다 | 오류를 표시한다 | - |
| Gateway 가 이미 실행 중 | 새 인스턴스는 시작하지 않는다 | 기존 데몬에 접속한다 | - |

## 8. 보안

- Gateway 는 unix 소켓만 쓴다. `$AKRA_HOME/run` 권한은 0700, 소켓은 0600 이다. TCP 포트를 열지 않는다.
- upstream 실행은 Akra 의 기존 보안 설정을 그대로 쓴다
  ([akra-current.md](reference/akra-current.md) §4):
  - `-c shell_environment_policy.inherit=...`
  - `-c shell_environment_policy.ignore_default_excludes=false`
  - `-c allow_login_shell=false`
  - 환경 변수 허용 목록, `OPENAI_API_KEY`·`CODEX_API_KEY` 제거
  - PATH 의 codex 실행 파일 신뢰 확인
- renderer 는 `sandbox: true`, `contextIsolation: true`, `nodeIntegration: false` 이다. preload 는
  필요한 함수만 노출한다.
- renderer 창에는 앱 자체 화면만 로드한다. 외부 링크는 기본 브라우저로 연다. 앱 안 브라우저(R3)는
  별도의 보기와 세션을 쓴다.
- Codex 로그인 정보는 codex 가 관리한다 (`~/.codex`). Akra Desktop 과 Gateway 는 이를 읽지 않는다.

## 9. 버전 정합성

- upstream 버전은 Gateway 가 실행한 codex 의 버전이다.
- `desktop/src/shared/protocol/` 에는 타입을 생성한 codex 버전을 기록한다 (P-06).
- 앱은 시작할 때 두 버전을 비교한다. 다르면 경고만 하고 계속한다. P0 에서 버전 사이의 차이는 대부분
  필드 추가였다.
- codex 업그레이드 절차:
  1. codex 를 업그레이드한다.
  2. `generate-ts` 를 다시 실행한다.
  3. 타입 차이를 검토한다. 깨지는 변경이 있으면 앱을 고친다.
  4. Gateway 를 재시작한다.

## 10. 저장소 배치 (제안)

```text
codex-exec-loop/
+- src/bin/akra-gateway.rs                 Gateway 진입점 (P-01)
+- src/adapter/inbound/gateway/            Listener, Session, akra/* 처리 (위치는 P1 에서 확정)
+- src/adapter/outbound/app_server/        기존 app-server 실행 코드 재사용과 --listen 확장
+- src/composition/                        Gateway 조립
+- desktop/                                Akra Desktop (D-04)
|  +- package.json, electron.vite.config.ts
|  +- src/main/                            Gateway 접속, 창, 데몬 시작
|  +- src/preload/                         window.akra 브리지
|  +- src/renderer/                        React UI
|  +- src/shared/protocol/                 generate-ts 산출물과 버전 파일 (P-06)
|  +- scripts/                             타입 생성, 녹화 재생 도구
+- docs/plan/17-akra-desktop/              이 계획
```

Gateway 의 모듈 위치는 `tests/architecture_boundaries.rs` 의 규칙을 지켜야 한다. P1 의 첫 작업에서
확정한다.

## 11. 확장 지점

| 지점 | 용도 | 단계 |
| --- | --- | --- |
| `akra/*` | Gateway 정보, 하네스 기능, 요약본 | P1, R5, R6 |
| interceptor | compact 제어, 지시문 추가 | R6 |
| 백엔드 선택 | 스레드마다 엔진 선택 (Codex, Claude) | R7 |
| main 서비스 | 터미널, 브라우저, 업무 패널 | R2, R3, R4 |
