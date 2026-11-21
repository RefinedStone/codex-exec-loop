# 29-D7. 위임 실행: akra-tunnel 을 core 와 frontend 로 나누기

상태: **Proposed**. 현재 제품 동작을 설명하지 않는다. · 기준일: 2026-11-21 · 기준 커밋: akra-tunnel `7cd0de6` (`master`),
`33a12b72` (`origin/prerelease`) · 상위: [plan 29 위임 실행](29-akra-delegated-execution.md) (§8.2, §8.4)

이 조각의 일은 akra-tunnel 저장소 (`akra-devs/akra-tunnel`) 에서 한다. 서브 이슈도 그 저장소에 둔다.

## 목표

akra-tunnel 을 core 라이브러리와 frontend 로 나눈다 (사용자의 방향, 2026-11-21). Akra Desktop 은 core 를 외부
라이브러리로 import 하고 자기 frontend 를 만든다. akra-tunnel 의 launcher 는 같은 core 의 다른 frontend 이다.

## 지금

코드에서 확인했다 (2026-11-21, `7cd0de6`).

- `src/` 에는 Electron import 가 없다. Electron import 는 `launcher/` 의 4개 파일에 있다: `main.mjs`, `preload.cjs`,
  `continuation-browser-host.mjs`, `image-browser-host.mjs`.
- 이음매가 이미 있다: `ContinuationBrowser` (`observe`, `send`, `reconnect`, `retryLoad`),
  `sendContinuationInput({ read, insertText, assertActive, focus })`, `AkraTunnelService`.
- 그러나 라이브러리로 쓸 수 없다.
  - 패키지가 하나이고 `private` 이다. `exports` 가 없다.
  - 페이지를 읽는 코드 (`*-dom.mjs`, `continuation-input.mjs`) 가 `launcher/` 에 있다. 타입이 없다.
  - `continuation-browser-host.mjs` 한 함수에 보기 관리, broker 서버, 10초 tick, 발견 루프가 섞여 있다.
  - cloud 와 push 를 읽는 두 모듈이 Electron 의 `webContents.debugger` 를 직접 받는다.
- core 가 하는 일은 기존 대화의 감시와 재개이다. `start` 는 있는 대화의 `chatUrl` 을 요구한다. 새 대화에 글 작업을 보내는
  일은 없다. 새 대화와 임시 대화를 여는 흐름은 이미지 작업자에만 있다.
- `sharp` 와 `playwright-core` 는 이미지와 외부 브라우저 계열의 6개 파일만 쓴다. continuation 계열은 `zod`, MCP SDK,
  `commander`, node 내장만 쓴다.
- broker 의 pipe 이름은 home 에서 나온다 (`continuationPipe`). MCP 런타임은 그 pipe 로 `call_observed` 를 보낸다.
- 런타임 설치는 `src/<이름>.ts` 의 경로와 파일 해시로 설치본을 검증한다 (`WORKER_OVERLAY_PATHS`, `host-runtime.ts`).
- tunnel 의 감시 (watchdog, `powerMonitor`) 는 `launcher/main.mjs` 에 있다.

## 서브 이슈

| ID | 결과 하나 | 바뀌는 곳 | 선행 |
| --- | --- | --- | --- |
| D7-1 | 패키지 나누기: `core` 와 `launcher`. core 는 Electron 을 import 하지 않는다. 시험이 이것을 막는다 | `package.json`, 폴더 구조, 런타임 overlay 의 경로 | - |
| D7-2 | 페이지 키트를 core 로: DOM 읽기, 입력 순서, cloud 와 push 해석. `PageHandle` 과 `CdpSession` interface, 타입 | `launcher/*-dom.mjs`, `continuation-input.mjs`, `continuation-cloud-snapshot.mjs`, `continuation-discovery-network.mjs` | D7-1 |
| D7-3 | host 가르기: broker 서버, `dispatch`, tick, 발견 루프를 core 로. frontend 는 보기와 handle 만 준다. launcher 가 새 core 로 돈다 | `continuation-browser-host.mjs` | D7-2 |
| D7-4 | host 임대: home 하나에 host 하나. 둘째 frontend 는 client 로 돈다 | core, launcher | D7-3 |
| D7-5 | 글 작업 보내기 (chat job): 새 대화, 임시 대화, 지정한 대화. 표식, 전달 확인, 결과 글 읽기, `requestId` 멱등 | core | D7-3 |
| D7-6 | 배포: 버전, 고정하는 방법, Akra Desktop 의 의존성 | 릴리스 절차 | D7-1 |

## 계약

- **core 가 가지는 것.** 페이지 읽기와 입력, 세션 감시와 복구, 발견과 요청 간격, broker, 세션의 저장, 계약 타입, tunnel
  런타임 서비스.
- **frontend 가 주는 것.** 보기 만들기 (partition, 숨긴 창, 이동 제한, 새 창 거절), `PageHandle`, 전원과 잠금의 사건, 화면.
- `PageHandle` 은 작다: `evaluate(fn, args)`, `insertText`, `focus`, `load(url)`, `url()`, 그리고 선택으로 `CdpSession`.
- core 는 `electron` 을 import 하지 않는다. Electron 의 버전에 묶이지 않는다. akra-tunnel 은 `^44`, Akra Desktop 은
  `45.2.3` 이다.
- core 패키지의 의존성에 `sharp` 와 `playwright-core` 가 없다. 그것을 쓰는 파일은 다른 패키지에 둔다.
- 상태 값의 이름은 그대로 둔다 (관찰 단계, 보내기 확인). Akra 의 adapter 가 공통 값으로 옮긴다 (D3 의 표).
- chat job 은 한 번만 보낸다. 같은 `requestId` 는 같은 작업이다. 보내기의 응답을 잃으면 `send_uncertain` 이다. 다시 보내지
  않는다.
- chat job 의 복구 한도는 호출자가 준다 (`maxResumes`). 한도를 넘으면 core 는 멈추고 그 사실을 알린다.
- host 임대는 먼저 잡은 frontend 가 가진다. 임대가 없는 frontend 는 보내지 않는다. 죽은 host 의 임대는 pid 로 가린다.
- launcher 의 동작은 바뀌지 않는다. 기존 smoke 시험이 통과한다.

## 완료 기준

- [ ] core 에 `electron` import 가 0건이다. 시험이 이것을 확인한다.
- [ ] launcher 가 새 core 로 돈다. `smoke:continuation`, `smoke:continuation-broker`, `smoke:launcher` 가 통과한다.
- [ ] 가짜 `PageHandle` 로 관찰, 보내기, 복구의 시험이 Electron 없이 돈다.
- [ ] 두 frontend 가 같은 home 으로 뜨면 host 는 하나이다. 둘째는 보내기가 0회이다.
- [ ] chat job 이 fixture 페이지에서 새 대화를 열고, 보내고, 표식 뒤의 답을 읽는다.
- [ ] core 만 설치하면 `sharp` 와 `playwright-core` 가 설치되지 않는다.
- [ ] 런타임 overlay 의 해시 검증이 새 경로에서 통과한다. 사용자가 다시 설치해야 하면 릴리스 글에 적는다.

## 검증

- `command`: `npm run verify`, 기존 smoke 시험, 가짜 `PageHandle` 의 단위 시험.
- 실제 계정: chat job 은 사용자가 허락한 한 번.

## 제외

- 이미지 작업자의 재설계. chat job 은 새로 만들고 공통 부분만 뽑는다.
- tunnel 감시를 frontend 밖으로 옮기는 것 (plan 29 Q-DE16).
- 외부 브라우저 (Playwright) 경로의 변경.

## 미결정

| ID | 질문 | 기본값 |
| --- | --- | --- |
| Q-D7-1 | core 를 같은 저장소의 workspace 로 두는가, 새 저장소로 떼는가 | 같은 저장소의 workspace. 런타임 overlay 가 같은 파일을 쓴다 |
| Q-D7-2 | 배포 방법 | 버전을 단 tarball 과 무결성 값. npm 공개는 저장소의 공개 여부에 따른다 |
| Q-D7-3 | 보기를 만드는 코드 (숨긴 창, 이동 제한) 도 나누는가 | 선택 모듈로 둔다. Electron 을 인자로 받는다. frontend 가 쓰지 않아도 된다 |
| Q-D7-4 | 파일을 옮기면 런타임 schema 가 오른다. 받아들이는가 | 예. overlay 에 들어가는 파일의 경로를 한 번만 바꾼다 |
| Q-D7-5 | host 를 뺏는 명령을 두는가 (launcher 가 쥔 host 를 Akra Desktop 이 가져온다) | 두지 않는다. 사용자가 한쪽을 끈다 |
