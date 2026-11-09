# P2. Electron 골격과 Gateway 연결

상태: 완료 (Windows 2026-11-05, macOS 2026-11-09 실기 확인) · 기준일: 2026-11-09 · 규모: 약 1주

## 1. 목표

`desktop/` 에 Electron 앱의 골격을 만든다. 앱은 Gateway 에 접속하고 initialize 를 한 뒤 codex 버전과
Project 목록을 표시한다. 이후 화면 작업(P3)에 필요한 연결, 보안 설정, 타입, 시험 도구를 모두 갖춘다.

## 2. 선행 조건

- P1 완료 (Gateway 가 동작한다)
- P-05, P-06, P-07 확인

## 3. 범위

| 범위 안 | 범위 밖 |
| --- | --- |
| electron-vite + React + TypeScript 골격, npm | 대화 화면 (P3) |
| BrowserWindow 보안 설정 | 패키징, 서명, 자동 업데이트 (R8) |
| main 의 Gateway 접속 관리와 재접속 | 터미널, 브라우저 (R2, R3) |
| 데몬이 없을 때 `akra-gateway` 시작 | |
| preload 브리지 (메시지 버스) | |
| renderer 의 JSON-RPC 클라이언트 | |
| 프로토콜 타입 생성 스크립트 | |
| `desktop/AGENTS.md`, CI 범위, 검사 스크립트 | |
| 가짜 Gateway 와 기본 시험 | |

## 4. 설계

### 4.1 디렉터리

```text
desktop/
+- package.json, package-lock.json      npm, 버전 고정
+- electron.vite.config.ts
+- tsconfig*.json                       strict
+- AGENTS.md                            desktop 전용 규칙
+- src/main/                            창, Gateway 접속, 데몬 시작
+- src/preload/                         window.akra 브리지
+- src/renderer/                        React UI
+- src/shared/protocol/                 generate-ts 산출물 + CODEX_VERSION (손으로 고치지 않는다)
+- scripts/generate-protocol.mjs        타입 생성
+- test/fake-gateway/                   녹화 재생 서버, 픽스처
```

`desktop/` 은 루트 `package.json` 과 분리된 독립 패키지로 둔다. 루트 패키지는 npm 배포용이다.

### 4.2 창 보안 설정

```ts
new BrowserWindow({
  webPreferences: {
    preload,               // src/preload 빌드 결과
    sandbox: true,
    contextIsolation: true,
    nodeIntegration: false,
  },
});
```

- Content-Security-Policy 를 설정한다. 원격 스크립트를 허용하지 않는다.
- `will-navigate` 로 앱 밖 이동을 막는다. `setWindowOpenHandler` 에서 http(s) 링크는
  `shell.openExternal` 로 기본 브라우저에 연다.

### 4.3 main 의 Gateway 접속

- 라이브러리: `ws`. macOS 는 `ws+unix:///<절대경로>:/` 형식을 쓴다. Windows 는 `createConnection` 옵션으로
  named pipe 접속(`net.connect("\\\\.\\pipe\\akra-gateway-<사용자 식별자>")`)을 넘긴다 (P-04).
- 창 하나마다 접속 하나를 연다 (04 §4).
- 접속이 끊기면 1초에서 시작해 최대 30초 간격으로 다시 접속한다. 상태(연결 중, 연결됨, 끊김)를
  renderer 에 알린다.
- 접속에 실패하면 `akra-gateway` 를 분리(detached) 실행하고 다시 접속한다.
  - 개발 중 경로: 저장소의 `target/release/akra-gateway` 또는 `target/debug/akra-gateway`
  - 설정으로 경로를 바꿀 수 있게 한다
- main 은 메시지를 해석하지 않는다. 창의 메시지를 그 창의 접속으로, 접속의 메시지를 그 창으로 보낸다.

### 4.4 preload 브리지

```ts
// renderer 에서 쓰는 전부
window.akra = {
  send(text: string): void,                          // JSON-RPC 메시지 한 개
  onMessage(listener: (text: string) => void): () => void,
  onConnectionState(listener: (state: ConnectionState) => void): () => void,
};
```

- 메시지는 문자열 그대로 주고받는다. 파싱은 renderer 가 한다.
- 큰 메시지(`thread/read` 결과 등)의 크기를 측정한다. 문제가 생기면 조각 전송을 추가한다.

### 4.5 renderer 의 JSON-RPC 클라이언트

- 요청 ID 카운터, 대기 중 요청 표, 알림 처리기 목록, 서버 요청 처리기를 가진다.
- 연결이 다시 맺어지면 initialize 를 다시 하고, 대기 중 요청을 오류로 끝낸다.
- 타입은 `src/shared/protocol/` 의 생성 타입을 쓴다.

### 4.6 프로토콜 타입 생성

```bash
codex app-server generate-ts --out <임시 디렉터리>
```

- `scripts/generate-protocol.mjs` 가 위 명령을 실행하고 결과를 `src/shared/protocol/` 에 복사한다.
- 생성에 쓴 codex 버전을 `src/shared/protocol/CODEX_VERSION` 에 쓴다.
- 기본값은 안정 API 이다. `--experimental` 은 쓰지 않는다 (P-07).
- 생성 파일은 손으로 고치지 않는다.

### 4.7 Akra 저장소와의 통합

- `scripts/ci-scope.mjs` 는 docs, Rust, Node 경로가 아닌 파일이 바뀌면 전체 CI 로 분류한다. `desktop/`
  을 위한 범위를 추가한다. 이 파일을 바꾸는 PR 은 전체 CI 를 실행한다.
- `desktop/` 검사 스크립트를 추가한다: 타입 검사, 린트, 단위 테스트. `scripts/agent-plan.mjs` 가 이
  검사를 출력하게 한다.
- `.github/` 워크플로에 데스크톱 작업을 추가한다.
- `desktop/AGENTS.md` 규칙:
  - renderer 에서 Node API 를 쓰지 않는다
  - main 과 renderer 사이에는 메시지 버스만 쓴다
  - main 에 대화 로직을 넣지 않는다
  - 생성 타입을 손으로 고치지 않는다
  - Codex 사용량 정책(D-13)을 따른다

### 4.8 가짜 Gateway

- Node `ws` 서버로 만든다. 녹화 파일(JSONL)을 읽어 요청에 맞는 응답과 알림을 재생한다.
- P1 의 녹화 옵션으로 얻은 실제 기록을 바탕으로 픽스처를 만든다. 대화 내용은 바꾼 사본만 커밋한다.
- 화면 시험(P3)은 모두 가짜 Gateway 로 실행한다. 턴을 쓰지 않는다.

## 5. 작업 순서

1. 골격 생성과 버전 고정
2. 보안 설정과 창 생성
3. main 의 접속 관리, 재접속, 데몬 시작
4. preload 브리지
5. renderer 의 JSON-RPC 클라이언트와 시험 화면 (연결 상태, codex 버전, Project 목록)
6. 타입 생성 스크립트와 첫 생성
7. CI 범위, 검사 스크립트, `desktop/AGENTS.md`
8. 가짜 Gateway 와 시험

## 6. 완료 기준

1. `npm run dev` 로 창이 열린다.
2. Gateway 가 실행 중이면 codex 버전과 Project 목록이 표시된다.
3. Gateway 가 없으면 앱이 시작하고 접속한다.
4. Gateway 를 재시작하면 앱이 다시 접속하고 initialize 를 다시 한다.
5. renderer 에서 `require`, `process` 를 쓸 수 없다.
6. 생성 타입과 Gateway 의 codex 버전이 다르면 경고가 표시된다.
7. 단위 시험과 가짜 Gateway 시험이 통과한다. 턴을 쓰지 않는다.
8. `node scripts/agent-plan.mjs` 가 출력한 검사를 통과한다.

## 7. 진행 기록

### 2026-11-05 골격 구현

- 진행 방식: 사용자 지시로 P-05, P-06, P-07 을 제안대로 적용하고 먼저 진행했다. 피드백은 후행으로 반영한다.
- 버전 (2026-11-05 고정): electron 45.2.3, electron-vite 6.0.0, vite 8.3.6, react 19.3.1, typescript 7.0.2,
  zustand 5.0.16, ws 8.22.1, vitest 5.1.0.
- 설계와 다르게 구현한 부분:
  - §4.3: macOS 와 Windows 모두 `ws` 의 `createConnection` 에 `net.connect(<주소>)` 를 넘긴다. Node 의
    `net.connect` 가 unix 소켓과 named pipe 를 같은 방식으로 다루므로 `ws+unix:` 형식이 필요 없다.
  - §4.3: Windows 사용자 SID 는 시스템 경로의 `whoami.exe /user` 로 읽는다. Git Bash 의 PATH 에서는 `whoami` 가
    coreutils 로 잡혀 실패했다.
  - §4.6: 생성 타입은 `src/shared/protocol/generated/`, 생성에 쓴 codex 버전은 `src/shared/protocol/version.ts` 에
    둔다 (renderer 가 import 할 수 있게 TS 상수로 둔다).
  - §4.7: 린트 도구는 아직 넣지 않았다. strict TypeScript 두 설정(main·preload·시험, renderer)으로 시작한다.
    CI 는 `desktop/` 를 Node 범위로 분류하고 `check_node_surfaces.sh`, `check_native_pr.ps1` 이 의존성 설치,
    audit, 타입 검사, 시험, 빌드를 실행한다.
  - 시험 화면의 목록은 Project 대신 작업 폴더(스레드 cwd 묶음)이다. codex 0.159 의 안정 API 에는
    `project/list` 가 없다 (Q-12).
- 완료 기준 확인 (Windows 11, codex-cli 0.159.0, 턴 없음, Playwright 로 빌드한 앱 실행):

| 기준 | 결과 |
| --- | --- |
| 1. 창 열림 | 제목 `Akra Desktop` 창이 열렸다 |
| 2. 버전과 목록 | `codex-cli 0.159.0`, 작업 폴더 목록이 표시됐다 |
| 3. Gateway 자동 시작 | Gateway 가 없는 상태에서 앱이 `target/debug/akra-gateway.exe` 를 시작하고 접속했다 |
| 4. 재접속 | Gateway 를 강제 종료하자 `끊김` 이 표시되고, 앱이 데몬을 다시 시작해 접속과 initialize 를 다시 했다 |
| 5. renderer 격리 | renderer 에서 `typeof require`, `typeof process` 가 `undefined`, `window.akra` 는 함수 3개뿐이다 |
| 6. 버전 경고 | 버전이 같으면 경고가 없다. 다른 경우는 단위 시험으로 확인했다 |
| 7. 시험 | vitest 17개 통과 (JSON-RPC 클라이언트, 주소, 재접속 간격, 가짜 Gateway 위 세션·재접속·데몬 시작) |
| D-06 | 앱 창을 닫으면 앱은 끝나고 Gateway 는 계속 실행된다 |

- 남은 일: macOS 실기 확인, Q-12 결정, 완료 기준 8 (`agent-plan` 검사는 PR 에서 실행). → 모두 끝남: Q-12 는
  2026-11-05 에 cwd 묶음으로 결정, 완료 기준 8 은 머지된 PR 의 CI 로 확인, macOS 는 아래 기록.

### 2026-11-09 macOS 실기 확인

- 환경: macOS 26.5 (Apple silicon), codex-cli 0.167.0, Electron 45.2.3, Node 24.3.0. 빌드한 앱을 Playwright 로
  숨긴 창(`AKRA_DESKTOP_HIDDEN=1`)으로 띄워 실제 Gateway 에 붙였다. 턴을 쓰지 않았다.

| 기준 | 결과 |
| --- | --- |
| 1. 창 열림 | 제목 `Akra Desktop` 창이 열렸다 |
| 2. 버전과 목록 | `codex-cli 0.167.0`, 작업 폴더별 스레드 498개가 표시됐다 |
| 3. Gateway 자동 시작 | Gateway 가 없는 상태에서 앱이 `target/debug/akra-gateway` 를 시작하고 접속했다 |
| 4. 재접속 | Gateway 를 `SIGKILL` 로 끝내자 `끊김` → `연결됨` 으로 돌아왔고 새 Gateway 가 떴다. 처음에는 실패했다: 고아 upstream 때문에 새 Gateway 가 시작하지 못했다 (P1 §9 2026-11-09 결함). 수정 뒤 통과 |
| 5. renderer 격리 | `typeof require`, `typeof process` 가 `undefined`. `window.akra` 는 bridge 함수만 있다 |
| 6. 버전 경고 | 생성 타입(0.159.0)과 Gateway(0.167.0)가 달라 경고가 표시됐다 |
| 7. 시험 | `npm run check`, vitest 92개 통과. `npm run e2e` 화면 시험 23개 통과 (승인 흐름, 여러 창, 실제 셸 터미널, 브라우저) |
| D-06 | 앱 창을 닫으면 앱은 끝나고 Gateway 는 계속 실행된다 |

- 후속 (선택): codex 0.167.0 기준으로 `npm run generate:protocol` 을 다시 돌리면 경고가 없어진다. 타입 차이를
  검토해야 하므로 이 확인에서는 하지 않았다.
