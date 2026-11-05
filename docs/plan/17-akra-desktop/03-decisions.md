# 03. 결정 기록

상태: Proposed · 기준일: 2026-11-05

이 문서는 이 계획의 결정 기준이다. 다른 문서와 내용이 다르면 이 문서를 따른다.

## 상태 정의

| 상태 | 뜻 | 작업자 행동 |
| --- | --- | --- |
| 승인 | 사용자가 승인했다 | 그대로 따른다. 바꾸려면 사용자와 다시 논의한다 |
| 권장 | 사용자가 반대하지 않았지만 명시적으로 승인하지도 않았다 | 해당 단계를 시작하기 전에 사용자에게 확인한다 |
| 제안 | 작업자가 정한 설계안이다 | 해당 단계를 시작하기 전에 사용자에게 확인한다. 더 나은 안이 있으면 근거와 함께 제시한다 |
| 제외 | 하지 않기로 했다 | 다시 제안하지 않는다 |
| 대체됨 | 다른 결정으로 바뀌었다 | 새 결정을 따른다 |

## 요약

| ID | 결정 | 상태 | 날짜 |
| --- | --- | --- | --- |
| D-01 | 출발점은 Akra + Electron | 승인 | 2026-11-05 |
| D-02 | 엔진은 Codex 먼저, Claude 는 나중 | 승인 | 2026-11-05 |
| D-03 | 클라우드 에이전트(dots) 비공개 백엔드 연결 | 제외 | 2026-11-05 |
| D-04 | 저장소는 Akra 저장소 안 `desktop/` | 승인 | 2026-11-05 |
| D-05 | 기준 데이터 A+ | 승인 | 2026-11-05 |
| D-06 | Gateway 는 항상 실행되는 데몬 | 승인 | 2026-11-05 |
| D-07 | free 플랜 대응 보류 | 승인 | 2026-11-05 |
| D-08 | TUI 유지 | 승인 | 2026-11-05 |
| D-09 | 전송은 WebSocket + JSON-RPC, main 이 접속 | 승인 | 2026-11-05 |
| D-10 | 컴퓨터당 데몬 1개, 작업공간은 Codex Project | 승인 | 2026-11-05 |
| D-11 | 얇은 Gateway | 승인 | 2026-11-05 |
| D-12 | MVP 는 대화 부분만 | 승인 | 2026-11-05 |
| D-13 | Codex 사용량 최소화 | 승인 | 2026-11-05 |
| D-14 | RefinedStone 신원, 회사 계정과 분리 | 승인 | 2026-11-05 |
| D-15 | macOS 와 Windows 를 함께 지원 | 승인 | 2026-11-05 |
| D-16 | 사이드바는 작업 폴더(스레드 cwd) 묶음 | 승인 | 2026-11-05 |
| P-01 | Gateway 는 새 바이너리 `akra-gateway` | 승인 | 2026-11-05 |
| P-02 | upstream 은 Gateway 전용 자식 프로세스 | 승인 | 2026-11-05 |
| P-03 | 클라이언트 접속 1개 = upstream 접속 1개 | 승인 | 2026-11-05 |
| P-04 | 로컬 IPC 만 사용 (macOS unix 소켓, Windows named pipe), TCP 미사용 | 승인 | 2026-11-05 |
| P-05 | 데스크톱 기술 조합 | 제안 | 2026-11-05 |
| P-06 | TS 타입은 generate-ts 산출물을 커밋 | 제안 | 2026-11-05 |
| P-07 | initialize 는 `experimentalApi: false` | 제안 | 2026-11-05 |
| P-08 | 기존 TUI·하네스는 자체 app-server 유지 | 제안 | 2026-11-05 |
| P-09 | upstream 접속은 `codex app-server proxy` | 승인 | 2026-11-05 |

## 승인된 결정

### D-01 출발점은 Akra + Electron

- 결정: Akra 를 확장하고 Electron 데스크톱 앱을 새로 만든다. 코드 전체를 사용자가 소유한다.
- 이유:
  - Akra 는 이미 app-server 클라이언트이고 외부 루프(L4)를 가진다. 나중에 하네스를 연결할 수 있다.
  - Electron 은 Chromium 을 포함한다. 앱 안 브라우저, Chrome 쿠키 가져오기, CDP 조작이 가능하다.
  - 공식 Codex 앱과 Orca 가 같은 구조(Electron + React)이다. 참고할 자료가 많다.
- 검토한 대안 ([alternatives.md](reference/alternatives.md)):
  - Orca 플러그인: 가장 빠르다. 그러나 화면과 구조를 Orca 가 소유한다.
  - Tauri: macOS 에서 WebKit 을 쓴다. CDP 가 없고 Chrome 쿠키 형식과 다르다.
  - 순수 Rust GUI (gpui, iced 등): 앱 안 브라우저를 넣을 수 없다.
  - Akra TUI 그대로 사용: 브라우저와 업무 패널을 넣을 수 없다.
- 영향: 매일 쓸 수준까지 약 3~4개월이 걸린다 (추정). Akra 코드(운영 코드 약 15만 줄)를 함께
  유지한다.

### D-02 엔진은 Codex 먼저, Claude 는 나중

- 결정: MVP 와 초기 단계는 Codex 엔진만 연결한다. Claude 는 앱 완성 후 확장으로 붙인다.
- 이유: Akra 는 이미 Codex app-server 클라이언트이다. Claude Code 는 엔진 소스가 비공개이고,
  CLI 의 stream-json 형식에 맞춘 별도 어댑터가 필요하다.
- 영향: 앱의 프로토콜은 app-server 형식이다 (D-11). Claude 는 app-server 형식을 흉내 내는 백엔드로
  붙인다 ([R7](roadmap/R7-claude-backend.md)).

### D-03 클라우드 에이전트 비공개 백엔드 연결 (제외)

- 결정: OpenAI dots 처럼 클라우드에서 실행되는 에이전트의 비공개 백엔드에 연결하지 않는다.
- 이유: 공개 문서가 없다. 연결하려면 공식 앱을 역분석해야 한다. 약관 위험, 계정 위험, 잦은
  변경이 있다.
- 영향: 이런 에이전트는 공식 앱에서 쓴다. 공식 API 가 공개되면 그때 다시 논의한다. 공식 앱의 해당
  내부 구조는 이 저장소에 기록하지 않는다.

### D-04 저장소는 Akra 저장소 안 `desktop/`

- 결정: 별도 저장소를 만들지 않는다. Akra 저장소 루트에 `desktop/` 을 추가한다.
- 이유:
  - Gateway(Rust)와 데스크톱(TS)의 변경을 커밋 하나로 함께 낸다.
  - 데스크톱 빌드가 같은 저장소의 Gateway 바이너리를 함께 빌드한다.
  - Codex 저장소도 엔진과 프로토콜 타입 생성을 한 저장소에서 한다.
- 상태 메모: 2026-11-05 사용자가 승인했다 (Q-10 해결).
- 영향: Akra 의 Rust 중심 규칙과 CI 가 `desktop/` 에도 적용된다. `desktop/` 전용 규칙과 검사 경로를
  따로 둔다 ([P2](phases/P2-desktop-shell.md)).

### D-05 기준 데이터 A+

- 결정: 원본 데이터(Project, Thread, Turn, Item, Section)는 Codex app-server 가 저장하고 소유한다.
  Akra 는 파생 데이터만 소유한다: 요약본, compact 정책, 스레드 사이의 연결, 하네스 데이터.
- 이유:
  - app-server 를 감싸는 구조에서는 모델 문맥을 Codex 가 소유한다. Akra 가 대화를 따로 저장해도
    문맥은 바뀌지 않는다.
  - 같은 대화를 두 곳에 저장하면 두 기록이 어긋난다.
  - 자체 compact 도 A+ 안에서 할 수 있다 ([R6](roadmap/R6-context-compaction.md)).
- 기각한 대안: Akra 가 대화의 기준이 되는 안 (B). 문맥을 바꾸지 못하면서 저장만 이중이 된다.
- 영향: 앱의 목록과 기록 화면은 app-server 에서 읽는다. Akra 고유 정보는 Project metadata 의
  `akra.*` 키나 각 저장소의 `.codex-exec-loop/` 에 둔다.

### D-06 Gateway 는 항상 실행되는 데몬

- 결정: Gateway 는 로그인 동안 계속 실행되는 데몬이다. 앱은 데몬에 접속만 한다.
- 이유: 앱을 닫아도 진행 중인 턴이 계속되어야 한다. 하네스와 다른 입구(Telegram 등)도 나중에 같은
  데몬을 쓸 수 있다.
- 기각한 대안: 앱이 Gateway 를 실행하고 앱 종료 때 함께 끄는 안.
- 영향: 데몬 시작 방식이 필요하다 (Q-05, [R8](roadmap/R8-packaging-and-operations.md)). MVP 에서는
  앱이 접속에 실패하면 데몬을 시작하는 방식으로 충분하다. 앱을 닫은 뒤 턴이 계속되는지는 Q-09 로
  확인한다.

### D-07 free 플랜 대응 보류

- 결정: 사용량 절약 기능(기본 모델 낮춤, 자동 실행 차단, 사용량 경고)을 지금은 만들지 않는다.
- 영향: 필요해지면 다시 논의한다. 개발 중 사용량은 D-13 으로 관리한다.

### D-08 TUI 유지

- 결정: Akra 의 TUI 를 삭제하지 않는다. 데스크톱이 안정된 뒤에 다시 판단한다.
- 이유: TUI 를 지워도 얻는 것이 없다. 삭제 작업에는 위험이 있다.

### D-09 전송은 WebSocket + JSON-RPC, main 이 접속

- 결정: Akra Desktop 과 Gateway 사이는 WebSocket 위의 JSON-RPC 이다. renderer 는 Gateway 에 직접
  접속하지 않는다. Electron main 이 접속하고 renderer 와는 IPC 로 주고받는다.
- 이유:
  - 명령과 이벤트가 같은 연결을 쓰므로 순서가 보장된다 (예: steer 직후의 이벤트).
  - 브라우저의 출처당 HTTP/1.1 연결 수 제한(6개)이 없다.
  - app-server 와 같은 형식이라 변환 코드가 없다.
  - 접속 경로와 권한을 main 에만 둔다.
- 기각한 대안: HTTP POST + SSE (현재 admin_api 방식). admin 웹을 위한 방식이고, 명령과 이벤트가
  다른 연결이라 순서 보장이 약하다.
- 참고: 화면의 부드러움은 전송 방식보다 렌더링(프레임 단위 묶음, 가상 스크롤)이 더 크게 좌우한다
  ([P3](phases/P3-mvp-conversation.md)).

### D-10 컴퓨터당 데몬 1개, 작업공간은 Codex Project

- 결정: Gateway 데몬은 컴퓨터에 하나이다. 여러 저장소를 다룬다. 작업공간은 Codex 의 Project
  (`project/*`)를 그대로 쓴다. Akra 고유 정보는 Project metadata 의 `akra.*` 키에 둔다.
- 이유: app-server 하나가 여러 Project 와 cwd 를 처리한다. 공식 Codex 앱도 같은 구조이다. Project 는
  경로를 여러 개 가질 수 있어서 관련 저장소를 하나로 묶을 수 있다.
- 영향: Akra 의 하네스 코드는 "프로세스 하나 = 저장소 하나"를 전제로 한다. 하네스 연동
  ([R5](roadmap/R5-harness-integration.md))에서 이 전제를 바꿔야 한다. MVP 에는 영향이 없다.

### D-11 얇은 Gateway

- 결정: Gateway 는 app-server 프로토콜을 바꾸지 않고 전달한다. 새 기능은 `akra/*` 이름 공간으로만
  추가한다. 특정 메서드를 가로채는 interceptor 자리를 둔다. MVP 에서 interceptor 는 비어 있다.
- 이유:
  - 코드가 가장 적다. Codex 기능을 빠짐없이 쓴다.
  - TS 타입을 Codex 가 생성한다 (`codex app-server generate-ts`).
  - 나중에 동작을 바꿀 지점이 interceptor 하나로 모인다.
- 기각·보류한 대안: Akra 고유의 엔진 중립 프로토콜, 핵심만 엔진 중립으로 바꾸는 혼합안.
- 영향: 앱은 Codex 프로토콜에 묶인다. Claude 는 app-server 형식의 백엔드 어댑터로 붙인다 (R7).
  P0 실측으로 app-server 가 다중 접속을 직접 지원함을 확인했다. 그래서 Gateway 는 접속 1:1 중계로
  충분하다 (P-03).

### D-12 MVP 는 대화 부분만

- 결정: MVP 는 공식 Codex 앱의 대화 부분만 만든다: 사이드바(Project → Thread), 대화 화면, 입력,
  중단과 steer, 승인.
- 이유: 가장 자주 쓰는 기능부터 대체한다. 하네스 연동 방식은 아직 정하지 않았다 (Q-01).
- 영향: Gateway 에는 `akra/*` 자리만 둔다. 다른 기능은 R1~R8 에서 추가한다.

### D-13 Codex 사용량 최소화

- 결정: 개발과 검증에서 모델을 호출하는 요청을 보내지 않는다. 필요하면 사용자에게 목적과 횟수를
  알리고 허락을 받는다.
- 모델을 호출하는 요청 (허락 필요): `turn/start`, `turn/steer`, `thread/compact/start`, 그리고 턴을
  시작시키는 모든 요청.
- 허락 없이 해도 되는 일: `initialize`, `account/read`, `thread/list`, `thread/read`,
  `project/list`, `model/list` 같은 조회 요청. 빌드, 단위 테스트, 가짜 서버 시험, 녹화 파일 재생.
- 허락받은 실측의 규칙: 가장 짧은 프롬프트와 낮은 추론 강도를 쓴다. 가능하면 `ephemeral: true`
  스레드를 쓴다. 실측 중 접속 메시지를 녹화해서 이후 시험에 재사용한다.

### D-14 RefinedStone 신원, 회사 계정과 분리

- 결정: Akra 저장소의 커밋과 원격 쓰기는 repo-local RefinedStone 신원으로만 한다. 회사 계정, 회사
  자격 증명, 회사 데이터는 이 저장소와 Akra 기본 설정에 넣지 않는다. 자격 증명 값은 출력하지 않는다.
- 근거: Akra 저장소 규칙 ([development.md](../../reference/development.md) "GitHub Identity").
- 영향: 업무 패널([R4](roadmap/R4-work-panels.md))이 회사 계정을 쓸 때에도 자격 증명은 OS 키체인에만
  저장한다.

### D-15 macOS 와 Windows 를 함께 지원

- 결정: Gateway 와 Akra Desktop 은 macOS 와 Windows 를 함께 지원한다. Linux 는 미정이다 (Q-06).
- 이유: 사용자가 Windows 에서도 작업한다. Akra 는 이미 Windows 검증 경로
  (`scripts/check_native_pr.ps1`)와 Windows 전용 의존성(`windows-sys`)을 가진다.
- 근거: 2026-11-05 Windows 11 실측 ([P0](phases/P0-preflight-verification.md) §6).
  - `codex app-server --listen unix://` 는 Windows 에서도 동작한다. 소켓 디렉터리가 현재 사용자 전용이
    아니면 `socket directory is not private to the current user` 오류로 시작하지 않는다.
  - Electron main 의 기반인 Node 는 Windows 에서 unix 소켓에 접속하지 못한다 (`EACCES`). Node 의
    `net.connect(path)` 는 Windows 에서 named pipe 만 다룬다.
  - Windows 의 CPython 에는 `socket.AF_UNIX` 가 없다. P0 의 Python 스크립트는 Windows 에서 실행되지 않는다.
  - `codex app-server proxy --sock <경로>` 는 stdio 와 소켓 사이에서 바이트를 그대로 전달한다. 접속 3개가
    동시에 `initialize` 와 `thread/list` 에 성공했다.
- 영향: P-04 를 OS 별로 나눈다. upstream 접속 방식은 P-09 이다. 권한, 종료 신호, 데몬 시작 방식은
  OS 별로 처리한다 ([P1](phases/P1-gateway-daemon.md), [R8](roadmap/R8-packaging-and-operations.md)).

### D-16 사이드바는 작업 폴더(스레드 cwd) 묶음

- 결정: P3 사이드바는 Codex Project 대신 `thread/list` 의 스레드를 cwd(작업 폴더)로 묶어 보여 준다.
  P-07(안정 API)을 유지한다 (Q-12).
- 이유: codex 0.159 에서 `project/*` 요청은 실험 API(`experimentalApi: true`)에만 있다. 실험 API 는 변경이
  잦아 앱이 더 자주 깨진다.
- 영향: D-10 의 "작업공간은 Codex Project" 는 `project/*` 가 안정 API 로 오면 적용한다. 그때까지 새 스레드는
  작업 폴더(cwd)만 고른다. 스레드의 `projectId` 필드는 안정 API 에도 있으므로 나중에 묶음 기준만 바꾼다.

## 제안

P-01~P-04 와 P-09 는 2026-11-05 사용자가 승인했다. 나머지는 해당 단계 전에 확인한다.

### P-01 Gateway 는 새 바이너리 `akra-gateway`

- 제안: `src/bin/akra-gateway.rs` 를 추가한다.
- 이유: Akra 는 입구마다 바이너리를 둔다 (`akra`, `akra-admin`, `akra-telegram`).
- 대안: `akra gateway` 하위 명령.
- 영향: `tests/binary_entrypoints.rs` 에 새 바이너리 시험을 추가한다.

### P-02 upstream 은 Gateway 전용 자식 프로세스

- 제안: Gateway 가 `codex app-server --listen unix://<경로>` 를 직접 실행하고 감시한다. Codex 관리형
  데몬(`codex app-server daemon`)은 쓰지 않는다.
- 이유: 수명, 버전, 보안 설정을 Akra 가 통제한다. Akra 의 기존 실행 코드(보안 `-c` 설정, 환경 변수
  허용 목록, 신뢰 실행 파일 확인)를 그대로 쓴다 ([akra-current.md](reference/akra-current.md) §4).
- Windows: 소켓 디렉터리는 현재 사용자 전용 ACL 이어야 한다. 아니면 app-server 가 시작하지 않는다 (D-15).

### P-03 클라이언트 접속 1개 = upstream 접속 1개

- 제안: Gateway 는 클라이언트 접속마다 upstream 접속을 하나씩 연다. initialize 와 요청 ID 는
  클라이언트와 app-server 가 직접 처리한다. Gateway 는 메시지 본문을 바꾸지 않는다. 예외는 `akra/*`
  메서드와 interceptor 대상뿐이다.
- 근거: P0 실측. app-server 는 접속마다 독립 세션을 만든다
  ([codex-app-server.md](reference/codex-app-server.md) §5).
- 효과: Gateway 에 요청 ID 재매핑, 알림 분배, initialize 대행이 필요 없다.

### P-04 로컬 IPC 만 사용

- 제안: Gateway 는 OS 의 로컬 IPC 로만 접속을 받는다. TCP 포트를 열지 않는다.
  - macOS: unix 소켓 `$AKRA_HOME/run/gateway.sock` (기본 `~/.akra/run/`). 디렉터리 권한은 0700, 소켓은
    0600 이다.
  - Windows: named pipe `\\.\pipe\akra-gateway-<사용자 식별자>`. 현재 사용자에게만 접근을 허용하는 보안
    설명자로 만든다. `first_pipe_instance` 로 이름 선점을 막고, 원격 클라이언트를 거부한다.
- 이유: OS 접근 통제가 인증을 대신한다. 별도 인증 토큰이 필요 없다.
- 주의:
  - macOS 의 unix 소켓 경로 한도는 104바이트이다. 시작할 때 길이를 검사한다.
  - Windows named pipe 의 기본 권한은 Everyone 읽기를 허용한다. 권한을 반드시 명시한다.
  - Windows 에서 Node 는 unix 소켓에 접속하지 못한다 (D-15). 그래서 Windows 의 클라이언트 구간은 named
    pipe 만 쓴다.

### P-05 데스크톱 기술 조합

- 제안: electron-vite, React, TypeScript, 상태 관리 zustand, 패키지 관리자 npm.
- 이유: npm 은 Akra 저장소 루트와 같다 (`package-lock.json`). 나머지는 Orca 가 쓰는 조합이다.
- 버전: P2 시작 시점의 안정 버전으로 고정한다. 참고 조합(2026-11 Orca): Electron 43, React 19,
  Vite 8, TypeScript 7.

### P-06 TS 타입은 generate-ts 산출물을 커밋

- 제안: `codex app-server generate-ts` 산출물을 `desktop/src/shared/protocol/` 에 커밋한다. 생성에
  쓴 codex 버전을 같은 위치의 파일에 기록한다.
- 이유: 빌드할 때 codex 가 없어도 된다. 버전 차이를 리뷰에서 볼 수 있다.
- 연결 규칙: Gateway 는 upstream 버전을 `akra/gateway/info` 로 알린다. 앱은 두 버전이 다르면 경고한다.

### P-07 initialize 는 `experimentalApi: false`

- 제안: Akra Desktop 은 안정 API 로 시작한다. 실험 기능이 필요하면 그 기능의 단계에서 켠다.
- 근거: Akra TUI 도 `experimentalApi: false` 로 initialize 한다.
- 참고: humanCommand 승인은 `experimentalApi: true` 와 `requiredHumanApproval: true` 가 필요하다.
  관리형 규칙을 쓰지 않는 개인 환경에서는 발생하지 않는다.

### P-08 기존 TUI·하네스는 자체 app-server 유지

- 제안: 기존 TUI, admin, Telegram, 하네스 경로는 지금처럼 자체 app-server 프로세스(stdio)를 쓴다.
  Gateway 의 upstream 과 합치지 않는다.
- 이유: MVP 범위를 줄인다. 기존 동작을 바꾸지 않는다.
- 영향: app-server 프로세스가 두 개가 될 수 있다. 둘 다 같은 `~/.codex` 를 쓰므로 대화 목록은 양쪽에서
  보인다. 통합은 [R5](roadmap/R5-harness-integration.md) 에서 판단한다.

### P-09 upstream 접속은 `codex app-server proxy`

- 제안: Gateway 는 클라이언트 Session 마다 `codex app-server proxy --sock <upstream 소켓>` 을 자식
  프로세스로 실행한다. 그 stdin/stdout 위에서 WebSocket 으로 upstream 과 통신한다. macOS 와 Windows 가 같은
  경로를 쓴다.
- 이유:
  - tokio 는 Windows 에서 unix 소켓 클라이언트를 제공하지 않는다. proxy 를 쓰면 새 크레이트가 필요 없다.
  - `--sock` 이 Gateway 가 실행한 app-server 를 가리키므로 P-02 와 맞는다. Codex 관리형 데몬은 필요 없다.
  - 2026-11-05 Windows 실측에서 접속 3개가 동시에 성공했다 ([P0](phases/P0-preflight-verification.md) §6).
- 비용: 접속마다 codex 프로세스가 1개 늘어난다 (대기 중 약 6MB, Windows 실측).
- 주의: proxy 는 바이트만 전달한다. 클라이언트가 WebSocket 핸드셰이크부터 직접 해야 한다. 실측 중 Node 의
  `kill` 뒤에도 proxy 가 남은 경우가 있었다. 자식 프로세스는 Akra 의 `ManagedChild`(Windows job object)로
  묶는다.
- 대안: Windows 전용 unix 소켓 크레이트(예: `uds_windows`, 동기 API)를 비동기로 감싼다. macOS 는 tokio
  `UnixStream` 을 직접 쓸 수 있다.
