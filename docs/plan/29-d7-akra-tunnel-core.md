# 29-D7. 위임 실행: akra-tunnel 을 akra-tunnel-core 와 frontend 로 나누기

상태: **Proposed**. 현재 제품 동작을 설명하지 않는다. · 기준일: 2026-11-21 · 기준 커밋: akra-tunnel `7cd0de6` (`master`),
`33a12b72` (`origin/prerelease`) · 상위: [plan 29 위임 실행](29-akra-delegated-execution.md) (§8.2, §8.4)

이 조각의 일은 두 저장소에서 한다: 새 저장소 `RefinedStone/akra-tunnel-core` (public. 사용자의 결정, 2026-11-21) 와
akra-tunnel (`akra-devs/akra-tunnel`, private). 서브 이슈는 그 일이 있는 저장소에 둔다.

등록한 이슈 (2026-11-21, `RefinedStone/akra-tunnel-core`): 상위는
[#1](https://github.com/RefinedStone/akra-tunnel-core/issues/1) 이다. D7-1 ~ D7-6 은 #3 ~ #8 (S1 ~ S6) 이다. Q-D7-6 과
Q-D7-7 은 결정 이슈 [#2](https://github.com/RefinedStone/akra-tunnel-core/issues/2) 이다. akra-tunnel 저장소 쪽의 일
(의존성 전환, 런타임 overlay 의 경로) 은 아직 이슈가 없다.

## 이름

- 이름은 `akra-tunnel-core` 이다. `core` 는 헥사고날의 뜻으로 쓴다: host 를 모르는 안쪽이다. port 를 정의하고 frontend 가
  그 port 를 구현한다. `playwright-core` 와 `puppeteer-core` 가 브라우저를 빼고 쓰는 쪽에 맡기는 것과 같은 관계이다.
- 범위는 akra-tunnel 에서 frontend 를 뺀 전부이다: 대화 세션, tunnel 런타임, broker. 대화 세션만 담으면 이 이름은 틀린다.
- `core` 는 층의 이름이다. 영역의 이름은 한 단계 아래에 둔다: `akra-tunnel-core/chat`, `akra-tunnel-core/tunnel`.
- 공개 API 의 이름은 Akra 의 규칙과 맞춘다. frontend 가 구현하는 것은 `Port` 로 끝난다 (`PagePort`, `CdpPort`).
  akra-tunnel-core 가 주는 것은 `Service` 로 끝난다 (`ChatSessionService`, `TunnelRuntimeService`).
- 공개 API 에 구현의 역사를 드러내지 않는다: `continuation`, `image worker`, `launcher`.
- Akra 의 문서와 코드는 `core` 로 줄여 쓰지 않는다. Akra 의 `src/core` 는 다른 것이다 (plan 18 §2.3).

## 목표

akra-tunnel 을 akra-tunnel-core 라이브러리와 frontend 로 나눈다 (사용자의 방향, 2026-11-21). Akra Desktop 은 akra-tunnel-core 를 외부
라이브러리로 import 하고 자기 frontend 를 만든다. akra-tunnel 의 launcher 는 같은 akra-tunnel-core 의 다른 frontend 이다.

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
- akra-tunnel-core 가 하는 일은 기존 대화의 감시와 재개이다. `start` 는 있는 대화의 `chatUrl` 을 요구한다. 새 대화에 글 작업을 보내는
  일은 없다. 새 대화와 임시 대화를 여는 흐름은 이미지 작업자에만 있다.
- `sharp` 와 `playwright-core` 는 이미지와 외부 브라우저 계열의 6개 파일만 쓴다. continuation 계열은 `zod`, MCP SDK,
  `commander`, node 내장만 쓴다.
- broker 의 pipe 이름은 home 에서 나온다 (`continuationPipe`). MCP 런타임은 그 pipe 로 `call_observed` 를 보낸다.
- 런타임 설치는 `src/<이름>.ts` 의 경로와 파일 해시로 설치본을 검증한다 (`WORKER_OVERLAY_PATHS`, `host-runtime.ts`).
- tunnel 의 감시 (watchdog, `powerMonitor`) 는 `launcher/main.mjs` 에 있다.

## 서브 이슈

| ID | 결과 하나 | 바뀌는 곳 | 선행 |
| --- | --- | --- | --- |
| D7-1 | 새 저장소 `RefinedStone/akra-tunnel-core` 를 만들고 Electron 을 모르는 코드를 옮긴다. akra-tunnel 은 그것을 의존성으로 쓴다. 시험이 Electron import 를 막는다 | 새 저장소, akra-tunnel 의 `package.json`, 런타임 overlay 의 경로 | - |
| D7-2 | 페이지 키트를 akra-tunnel-core 로: DOM 읽기, 입력 순서, cloud 와 push 해석. `PageHandle` 과 `CdpSession` interface, 타입 | `launcher/*-dom.mjs`, `continuation-input.mjs`, `continuation-cloud-snapshot.mjs`, `continuation-discovery-network.mjs` | D7-1 |
| D7-3 | host 가르기: broker 서버, `dispatch`, tick, 발견 루프를 akra-tunnel-core 로. frontend 는 보기와 handle 만 준다. launcher 가 새 akra-tunnel-core 로 돈다 | `continuation-browser-host.mjs` | D7-2 |
| D7-4 | host 임대: home 하나에 host 하나. 둘째 frontend 는 client 로 돈다 | akra-tunnel-core, launcher | D7-3 |
| D7-5 | 글 작업 보내기 (chat job): 새 대화, 임시 대화, 지정한 대화. 표식, 전달 확인, 결과 글 읽기, `requestId` 멱등 | akra-tunnel-core | D7-3 |
| D7-6 | 배포: 버전, 고정하는 방법, Akra Desktop 의 의존성 | 릴리스 절차 | D7-1 |

## 계약

- **akra-tunnel-core 가 가지는 것.** 페이지 읽기와 입력, 세션 감시와 복구, 발견과 요청 간격, broker, 세션의 저장, 계약 타입, tunnel
  런타임 서비스.
- **frontend 가 주는 것.** 보기 만들기 (partition, 숨긴 창, 이동 제한, 새 창 거절), `PageHandle`, 전원과 잠금의 사건, 화면.
- `PageHandle` 은 작다: `evaluate(fn, args)`, `insertText`, `focus`, `load(url)`, `url()`, 그리고 선택으로 `CdpSession`.
- akra-tunnel-core 는 `electron` 을 import 하지 않는다. Electron 의 버전에 묶이지 않는다. akra-tunnel 은 `^44`, Akra Desktop 은
  `45.2.3` 이다.
- akra-tunnel-core 패키지의 의존성에 `sharp` 와 `playwright-core` 가 없다. 그것을 쓰는 파일은 다른 패키지에 둔다.
- 상태 값의 이름은 그대로 둔다 (관찰 단계, 보내기 확인). Akra 의 adapter 가 공통 값으로 옮긴다 (D3 의 표).
- chat job 은 한 번만 보낸다. 같은 `requestId` 는 같은 작업이다. 보내기의 응답을 잃으면 `send_uncertain` 이다. 다시 보내지
  않는다.
- chat job 의 복구 한도는 호출자가 준다 (`maxResumes`). 한도를 넘으면 akra-tunnel-core 는 멈추고 그 사실을 알린다.
- host 임대는 먼저 잡은 frontend 가 가진다. 임대가 없는 frontend 는 보내지 않는다. 죽은 host 의 임대는 pid 로 가린다.
- launcher 의 동작은 바뀌지 않는다. 기존 smoke 시험이 통과한다.
- 공개 저장소의 규칙. akra-tunnel 은 지금 private 이다. 그래서 이 저장소가 첫 공개이다.
  - 새 이력으로 시작한다. akra-tunnel 의 이력을 push 하지 않는다.
  - `docs/analysis/` 와 배포 기록, 실제 계정의 시험 결과를 옮기지 않는다.
  - 커밋의 신원은 RefinedStone 이다. 첫 커밋 전에 저장소의 git 설정에 둔다 (plan 17 D-14).
  - 제3자 코드와 patch 의 고지를 옮긴다 (`THIRD_PARTY_NOTICES.md`).

## 완료 기준

- [ ] akra-tunnel-core 에 `electron` import 가 0건이다. 시험이 이것을 확인한다.
- [ ] launcher 가 새 akra-tunnel-core 로 돈다. `smoke:continuation`, `smoke:continuation-broker`, `smoke:launcher` 가 통과한다.
- [ ] 가짜 `PageHandle` 로 관찰, 보내기, 복구의 시험이 Electron 없이 돈다.
- [ ] 두 frontend 가 같은 home 으로 뜨면 host 는 하나이다. 둘째는 보내기가 0회이다.
- [ ] chat job 이 fixture 페이지에서 새 대화를 열고, 보내고, 표식 뒤의 답을 읽는다.
- [ ] akra-tunnel-core 만 설치하면 `sharp` 와 `playwright-core` 가 설치되지 않는다.
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
| Q-D7-1 | akra-tunnel-core 를 같은 저장소의 workspace 로 두는가, 새 저장소로 떼는가 | 결정 (사용자, 2026-11-21): 새 저장소 `RefinedStone/akra-tunnel-core`, public |
| Q-D7-2 | 배포 방법 | git 의존성. 태그와 커밋 SHA 로 고정한다. npm 등록은 뒤에 정한다 |
| Q-D7-3 | 보기를 만드는 코드 (숨긴 창, 이동 제한) 도 나누는가 | 선택 모듈로 둔다. Electron 을 인자로 받는다. frontend 가 쓰지 않아도 된다 |
| Q-D7-4 | 파일이 다른 저장소로 가면 런타임 overlay 는 의존성 패키지의 파일을 읽는다. 런타임 schema 가 오른다. 받아들이는가 | 예. overlay 에 들어가는 파일의 경로를 한 번만 바꾼다 |
| Q-D7-6 | akra-tunnel-core 를 Rust 로 옮기는가 | 지금은 옮기지 않는다. TypeScript 로 먼저 나눈다. 그 뒤에 세션 엔진 (약 57 KB, 시험 77개) 만 다시 본다. 측정 (2026-11-21, `7cd0de6`): `src/` 805 KB 중 약 460 KB 는 Node MCP 런타임 안에서 돌고, 페이지 JS 54 KB 는 브라우저 안에서 돈다. 페이지 JS 는 옮기지 못한다. 런타임 쪽은 Q-D7-7 에 달렸다 |
| Q-D7-7 | Node MCP 런타임 (cokacremote 와 akra-tunnel 의 overlay) 을 Rust 로 바꾸는가 | 정하지 않았다. 측정 (2026-11-21): cokacremote 자체는 116 KB, 도구 20개, PTY 없음. 설치된 런타임 718 KB 중 약 600 KB 는 akra-tunnel 의 overlay 이다. `tunnel-client` 는 `--mcp-command` 로 실행 파일 하나를 띄운다. 바꾸면 Node, Git, npm 의 요구와 설치 파이프라인이 없어진다. 바꾸는 방법은 Rust 서버가 앞에 서고 옮기지 않은 도구를 Node 서버로 넘기는 단계적 교체이다. 이것을 정하면 이 조각의 범위가 바뀐다 |
| Q-D7-5 | host 를 뺏는 명령을 두는가 (launcher 가 쥔 host 를 Akra Desktop 이 가져온다) | 두지 않는다. 사용자가 한쪽을 끈다 |
