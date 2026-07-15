# Agent Canvas 증거 원장

[English](../../../competitive/agent-canvas/evidence.md)

이 원장은 조사한 소스, 재현한 릴리스 게이트 출력, 공급자 문서, 추론, 검증되지 않은 실험을
구분한다. 제품 결론은 [analysis.md](analysis.md)에 있고 Akra 작업 결정은
[gap-matrix.md](gap-matrix.md)에 있다.

## 스냅샷

| 필드 | 값 |
| --- | --- |
| 제품 | OpenHands Agent Canvas |
| 공식 저장소 | <https://github.com/OpenHands/agent-canvas> |
| 릴리스 | [v1.2.1](https://github.com/OpenHands/agent-canvas/releases/tag/v1.2.1) |
| 릴리스 커밋 | `56d51c0767fb6fedc51c466f5138fdfc116a2707` |
| 릴리스 커밋 날짜 | 2026-07-10 17:52:12 +07:00 |
| 조사 날짜 | 2026-07-12 (Asia/Seoul) |
| Akra 기준선 | `prerelease`의 `0229ed71a541d91abc04ad13b188d28b9eafcaf8` |
| Akra 버전 | 1.3.5 |
| 소스 조사 환경 | Linux x86_64, 깨끗한 detached 소스 checkout |
| 릴리스 게이트 환경 | WSL2의 Ubuntu 24.04.2, x86_64, Node 24.14.1, npm 11.6.1 |

Agent Canvas 및 의존성 checkout은 공개되어 있었고 로컬에서 사용할 수 있는 깨끗한 detached
checkout이었다. Akra 기준선은 `origin/prerelease`에서 만든 깨끗한 worktree였다.

## 증거 분류

- `verified`: 변경 불가능한 소스 커밋에서 조사했거나, 릴리스 메타데이터에서 관찰했거나, 로컬에서
  재현함
- `documented`: 공식 프로젝트 문서에 명시되어 있지만 실행 중인 제품에서 재현하지 않음
- `proposed`: 미래, beta, experimental이라고 명시되어 있거나 이후 의존성 경로에만 존재함
- `inferred`: 검증된 사실에서 도출했지만 런타임 결과를 재현하지 않은 결론
- `unverified`: 필요한 실험이 누락된 주장 또는 비교

인벤토리 개수는 방향을 파악하기 위한 데이터이지 품질 점수가 아니다. 빌드 시간과 최대 RSS는 실행한
명령을 설명하며 대화형 제품 성능을 설명하지 않는다.

## 고정된 소스 집합

| 구성 요소 | 태그 또는 스냅샷 | 커밋 | 역할 |
| --- | --- | --- | --- |
| Agent Canvas | v1.2.1 | `56d51c0767fb6fedc51c466f5138fdfc116a2707` | 출시된 브라우저 제품 |
| OpenHands software-agent-sdk | v1.33.0 | `7ef44d10b2132125833ce2269559b3960f402805` | 출시된 Agent Server/ACP 런타임 |
| OpenHands Automation | 1.1.4 | `7e05cde624fee2dad9f9a8ed2b654d9ba4cc3e71` | 출시된 Automation 서비스 |
| Zed Codex ACP | v0.16.0 | `bb590500e8646f6daf879b8b3c6a659fbd29017d` | 출시된 내장 Codex ACP 브리지 |
| OpenHands software-agent-sdk | v1.35.0 | `9028562e2d5eda76de662ec9b7584125760eb83f` | 이후 경로, Canvas v1.2.1에 출시되지 않음 |
| Agent Client Protocol Codex ACP | v1.1.2 | `8aff492d4b033ff2c02ad3b9d591994d57617463` | 이후 app-server 브리지 |

조사 당시 Canvas `main` 브랜치는 `90223f99f43a977ec454c0fc15e93642b7053075`, SDK `main`
브랜치는 `cf6c2a3a4ace65b651ea29032ab6b4a74d7bb41a`였다. 이 이동하는 tip은 갱신 참고용으로만
기록했으며 출시된 제품에 관한 결론은 여기에 근거하지 않는다.

## 재현 명령

### 소스 Checkout

```bash
git clone --depth 1 --branch v1.2.1 \
  https://github.com/OpenHands/agent-canvas.git /tmp/akra-agent-canvas-v121-audit
git -C /tmp/akra-agent-canvas-v121-audit rev-parse HEAD
git -C /tmp/akra-agent-canvas-v121-audit describe --tags --exact-match
git -C /tmp/akra-agent-canvas-v121-audit log -1 --format='%cI %s'
git -C /tmp/akra-agent-canvas-v121-audit status --short

git clone --depth 1 --branch v1.33.0 \
  https://github.com/OpenHands/software-agent-sdk.git \
  /tmp/akra-software-agent-sdk-v133-audit
git clone --depth 1 --branch v1.35.0 \
  https://github.com/OpenHands/software-agent-sdk.git \
  /tmp/akra-software-agent-sdk-v135-audit
git clone --depth 1 --branch 1.1.4 \
  https://github.com/OpenHands/automation.git /tmp/akra-openhands-automation-114-audit
git clone --depth 1 --branch v0.16.0 \
  https://github.com/zed-industries/codex-acp.git /tmp/akra-zed-codex-acp-v016-audit
git clone --depth 1 --branch v1.1.2 \
  https://github.com/agentclientprotocol/codex-acp.git /tmp/akra-codex-acp-v112-audit
```

관찰된 Canvas 식별 정보:

```text
56d51c0767fb6fedc51c466f5138fdfc116a2707
v1.2.1
2026-07-10T17:52:12+07:00 chore(main): release 1.2.1 (#1646)
```

### 인벤토리

```bash
CANVAS=/tmp/akra-agent-canvas-v121-audit
cd "$CANVAS"

git ls-files | wc -l
git ls-files '*.ts' '*.tsx' | wc -l
git ls-files '*.ts' '*.tsx' | xargs wc -l | tail -1
git ls-files '*.ts' '*.tsx' '*.js' '*.mjs' '*.css' | wc -l
git ls-files '*.ts' '*.tsx' '*.js' '*.mjs' '*.css' \
  | xargs wc -l | tail -1
git ls-files '*.test.ts' '*.test.tsx' '*.spec.ts' '*.spec.tsx' | wc -l
git ls-files | rg '^tests/e2e/.+\.spec\.(ts|tsx)$' | wc -l
rg -n '\b(it|test|describe)\s*\(' \
  --glob '*.ts' --glob '*.tsx' --glob '*.js' --glob '*.mjs' | wc -l
```

관찰 결과:

| 인벤토리 | 개수 |
| --- | ---: |
| 추적된 파일 | 1,780 |
| 추적된 TypeScript 파일 (`.ts`, `.tsx`) | 1,525 |
| 추적된 TypeScript LOC | 197,327 |
| 추적된 TS/TSX/JS/MJS/CSS 파일 | 1,553 |
| 추적된 TS/TSX/JS/MJS/CSS LOC | 206,587 |
| TypeScript 경로명 기준 test/spec 파일 | 500 |
| E2E spec 파일 | 19 |
| 나열한 JS 계열 파일의 저장소 전체 `it`/`test`/`describe` 호출 표식 | 4,618 |

표식 개수는 텍스트 검색 결과이지 실행된 테스트 개수가 아니다. 테스트는 매개변수화되거나,
건너뛰거나, 생성되거나, 해당 표현식이 세지 못하는 방식으로 이름 붙을 수 있다.

### 릴리스 게이트

릴리스 게이트는 `.git` 메타데이터가 없는 별도의 내보낸 소스 트리
`/tmp/agent-canvas-v121-run`에서 실행했다.

```bash
cd /tmp/agent-canvas-v121-run
/usr/bin/time -v npm ci
/usr/bin/time -v npm run lint
/usr/bin/time -v npm test
/usr/bin/time -v npm run build
/usr/bin/time -v npm run build:lib
npm pack --dry-run --json
npm audit --json
npm audit --omit=dev --json
```

2026-07-12 관찰 결과:

| 게이트 | 결과 |
| --- | --- |
| `npm ci` | 통과; 1,301개 패키지 설치 |
| `npm run lint` | 46.62 s에 통과; 최대 RSS 3,362,648 KiB |
| `npm test` | 52.82 s에 통과; 최대 RSS 763,632 KiB |
| Vitest 결과 | 480개 파일 통과, 1개 건너뜀; 3,666개 테스트 통과, 5개 건너뜀, 9개 todo |
| `npm run build` | 8.76 s에 통과; 최대 RSS 1,759,796 KiB; 클라이언트 모듈 6,897개 |
| `npm run build:lib` | 12.83 s에 통과; 최대 RSS 1,581,092 KiB |
| `npm pack --dry-run --json` | 통과; 14,215,651바이트 tarball, 57,069,667바이트 압축 해제 크기, 9,439개 항목 |
| `npm audit --json` | exit 1; low 2개, moderate 11개, high 5개 advisory |
| `npm audit --omit=dev --json` | exit 1; low 2개, moderate 11개, high 4개 advisory |

Vitest는 향후 Vitest 릴리스에서 거부될 중첩 `vi.mock`/`vi.hoisted` 동작과 생성된 TypeScript
클라이언트에서 누락된 source-map 소스를 경고했다. 애플리케이션 빌드는 `node:http`/`node:https`의
브라우저 외부화, React Router 미래 플래그, minify 후 500 kB를 넘는 청크를 경고했다. 기록된 가장 큰
클라이언트 청크는 gzip 전 526.28 kB였고 library translation bundle은 gzip 전 1,303.12 kB였다.

advisory 합계는 특정 날짜의 lockfile 조사 결과다. 도달 가능성과 악용 가능성은 테스트하지 않았다.
pack prepare 단계는 검증 트리가 export였기 때문에 `.git`을 찾지 못했다고 보고했지만 성공적으로
종료했다.

## 변경 불가능한 소스 원장

<a id="product-packaging-and-boundary"></a>
### 제품, 패키징, 경계

- [README 제품 논지](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/README.md#L1-L45)는
  제어 센터 역할, 지원하는 에이전트 계열, 로컬/원격/클라우드 대상, Beta 상태, 배포 선택지를
  명시한다. 분류: `documented`.
- [아키텍처 경계](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/docs/architecture.md#L1-L31)는
  런타임, 샌드박스, 작업 공간, 이벤트 이력을 Canvas가 아니라 Agent Server에 할당한다. 소스
  경계의 분류: `verified`.
- [고정된 기본값](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/config/defaults.json#L1-L14)은
  Canvas 1.2.1을 Agent Server 1.33.0, Automation 1.1.4, 0.11 미만 Python ACP에 묶는다. 분류:
  `verified`.
- [패키지 스크립트](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/package.json#L74-L103)는
  `dev:docker`, `dev:dangerously-dockerless`, `dev:automation`을 생략하지만
  [아키텍처 가이드](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/docs/architecture.md#L45-L56)는
  이를 명시한다. 분류: `verified` 문서/소스 drift.
- [all-in-one 이미지 서비스](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/docker/Dockerfile#L3-L15)와
  [entrypoint 시작](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/docker/entrypoint.sh#L229-L263)은
  정적 프록시, Agent Server, 선택적 Automation 프로세스 토폴로지를 검증한다. 분류: `verified`.

<a id="conversation-creation-reconnect-and-workspaces"></a>
### 대화 생성, 재연결, 작업 공간

- [홈 launcher 상태와 제출](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/home/home-chat-launcher.tsx#L37-L115)은
  연결된 로컬 작업 공간 선택기를 `local_repo`로 초기화하는 반면 처음부터 시작하기는 작업 공간과
  모드를 생략한다. 분류: `verified`.
- [대화 생성](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/api/conversation-service/agent-server-conversation-service.api.ts#L394-L439)은
  명시적 override가 없는 연결된 작업 공간을 `local_repo`로 해석하지만, 생략된 작업 공간과 모드는
  `new_worktree`로 해석한다. `worktree`는 이 해석된 값을 따른다. 분류: `verified`.
- [Agent Server worktree 생성](https://github.com/OpenHands/software-agent-sdk/blob/7ef44d10b2132125833ce2269559b3960f402805/openhands-agent-server/openhands/agent_server/conversation_service.py#L62-L246)은
  `/tmp/conversation-worktrees/<conversation-id>/`, `openhands/<uuid>` 브랜치, 시스템 지침을 만든다.
  분류: `verified`.
- [Agent Server 삭제](https://github.com/OpenHands/software-agent-sdk/blob/7ef44d10b2132125833ce2269559b3960f402805/openhands-agent-server/openhands/agent_server/conversation_service.py#L904-L943)는
  대화 상태를 제거하지만 작업 공간 데이터는 명시적으로 보존한다. 조사한 소스 검색에서는 worktree
  정리 경로를 찾지 못했다. 보존의 분류: `verified`, 검색한 스냅샷을 넘어선 정리 부재의 분류:
  `inferred`.
- [클라이언트 대화 메타데이터](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/api/conversation-metadata-store.ts#L4-L84)는
  선택된 저장소, 브랜치, 작업 공간, 작업 공간 모드를 브라우저 로컬 스토리지에 보존한다.
  [Agent Server 어댑터](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/api/conversation-service/agent-server-conversation-service.api.ts#L404-L439)는
  런타임에 그에 대응하는 선택 소스 개념이 없다고 명시한다. 분류: `verified`.
- [대화 WebSocket 컨텍스트](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/contexts/conversation-websocket-context.tsx#L115-L174)는
  이벤트 시퀀스 추적과 하위 대화에 대한 단일 planning-agent 가정을 보여 준다. 분류: `verified`.
- [WebSocket 재연결](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/hooks/use-websocket.ts#L36-L154)은
  고정 재시도 지연과 재연결 루프를 사용한다. 분류: `verified`.
- [대화 목록 쿼리](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/hooks/query/use-paginated-conversations.ts#L8-L47)는
  10초마다 폴링한다. 분류: `verified`.

<a id="selected-conversation-ux"></a>
### 선택된 대화 UX

- [대화 분할](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/conversation/conversation-main/conversation-main.tsx#L21-L126)과
  [검사기 탭](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/conversation/conversation-tabs/conversation-tabs.tsx#L80-L145)은
  채팅, 파일, 플래너, 터미널, 브라우저, 태스크 화면을 검증한다. 분류: `verified`.
- [터미널 hook](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/hooks/use-terminal.ts#L88-L190)은
  입력이 비활성화된 xterm을 생성하고 관찰된 로그를 쓴다. 분류: `verified`.
- [브라우저 패널](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/browser/browser.tsx#L6-L25)은
  대화형 내장 브라우저 대신 스크린샷, URL, 외부에서 열기 컨트롤을 렌더링한다. 분류: `verified`.
- [파일 viewer](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/files-tab/file-content-viewer.tsx#L104-L227),
  [2,000개 파일 쿼리 한도](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/hooks/query/use-workspace-files.ts#L7-L34),
  [100개 파일 변경 한도](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/routes/changes-tab.tsx#L49-L119)는
  읽기 전용이며 제한된 파일/diff 화면을 정의한다. 분류: `verified`.
- [대화 레일](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/conversation-panel/conversation-panel.tsx#L220-L289)은
  고정, 그룹, 필터, 정렬, 활성 세션 탐색을 검증한다. 분류: `verified`.
- [상태 점 매핑](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/conversation-panel/conversation-status-dot.tsx#L24-L59)은
  idle 세션과 승인 대기 세션에 동일한 녹색 작업 표시를 부여한다. 분류: `verified`.
- [이벤트 축약](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/utils/handle-event-for-ui.ts#L147-L368)과
  [확인 컨트롤](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/shared/buttons/conversation-confirmation-buttons.tsx#L16-L132)은
  아래 설명한 ACP 손실 밖에서 광범위한 OpenHands 이벤트와 승인 렌더링을 검증한다. 분류:
  `verified`.
- [Goal interceptor](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/hooks/chat/use-goal-interceptor.ts#L9-L64)와
  [goal 상태](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/chat/goal-status-content.tsx#L22-L168)는
  Agent Server 판정 루프 컨트롤과 투영을 검증한다. UI의 분류는 `verified`이며 판정 품질에 대한
  분류는 아니다.
- [Git 액션 helper](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/utils/utils.ts#L458-L500)는
  자연어 pull, push, branch, create-PR 프롬프트를 반환한다.
  [Git 도구 메뉴](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/controls/git-tools-submenu.tsx#L28-L50)는
  해당 문자열을 message-to-send 상태에 삽입한다. 분류: `verified` prompt-only 제어.

<a id="released-codex-acp-path"></a>
### 출시된 Codex ACP 경로

- Canvas는 [ACP 하위 프로세스 relay](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/docs/ACP_AGENTS.md#L8-L31)를
  문서화한다. 고정된 SDK 레지스트리는
  [`@zed-industries/codex-acp@0.16.0`](https://github.com/OpenHands/software-agent-sdk/blob/7ef44d10b2132125833ce2269559b3960f402805/openhands-sdk/openhands/sdk/settings/acp_providers.py#L372-L440)을
  선택한다. 분류: `verified`.
- 출시된 브리지의
  [Cargo manifest](https://github.com/zed-industries/codex-acp/blob/bb590500e8646f6daf879b8b3c6a659fbd29017d/Cargo.toml#L20-L37)는
  ACP와 Codex `rust-v0.137.0` crate를 고정하고,
  [entry point](https://github.com/zed-industries/codex-acp/blob/bb590500e8646f6daf879b8b3c6a659fbd29017d/src/lib.rs#L15-L70)는
  내장 Codex 상태로 ACP를 제공한다. app-server 의존성이나 하위 프로세스는 없다. 분류:
  `verified`.
- [ACP 설정 해석](https://github.com/OpenHands/software-agent-sdk/blob/7ef44d10b2132125833ce2269559b3960f402805/openhands-sdk/openhands/sdk/settings/model.py#L1650-L1743)은
  명시적 명령이 없는 내장 프리셋을 고정 레지스트리 명령으로 바꾸고 사전 설치 바이너리로 다시 쓸
  수 있다. 분류: `verified`.
- [SDK 이벤트 브리지](https://github.com/OpenHands/software-agent-sdk/blob/7ef44d10b2132125833ce2269559b3960f402805/openhands-sdk/openhands/sdk/agent/acp_agent.py#L1200-L1318)는
  text, thought, usage, tool update를 처리하지만 ACP Plan update는 처리하지 않는다. 도구 진행은
  영속화 전에 축약된다. 분류: `verified`.
- 이전 브리지는 [ACP Plan update](https://github.com/zed-industries/codex-acp/blob/bb590500e8646f6daf879b8b3c6a659fbd29017d/src/thread.rs#L2708-L2723)를
  내보내지만 위 SDK 이벤트 브리지는 이를 소비하지 않는다. 따라서 출시된 경로에서는 Plan 이벤트가
  종단 간으로 사라진다. 이와 별개로 Canvas는 ACP 세션에서
  [Code/Plan 모드 전환을 숨긴다](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/chat/components/chat-input-actions.tsx#L81-L84).
  분류: `verified`.
- [권한 콜백](https://github.com/OpenHands/software-agent-sdk/blob/7ef44d10b2132125833ce2269559b3960f402805/openhands-sdk/openhands/sdk/agent/acp_agent.py#L1372-L1389)은
  사용자 상호작용 없이 첫 번째 옵션을 선택한다. 분류: `verified`.
- 출시된 브리지의 일반 권한 목록은
  [세션 범위 승인을 첫 번째로 둔다](https://github.com/zed-industries/codex-acp/blob/bb590500e8646f6daf879b8b3c6a659fbd29017d/src/thread.rs#L2339-L2347).
  이후 유지보수 브리지도
  [같은 순서](https://github.com/agentclientprotocol/codex-acp/blob/8aff492d4b033ff2c02ad3b9d591994d57617463/src/CodexApprovalHandler.ts#L172-L190)를
  사용한다. 명령/파일 요청은 첫 번째 옵션이 다를 수 있다. 분류: `verified`.
- [기본 Canvas 확인 설정](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/services/settings.ts#L5-L62)은
  확인을 비활성화한다. Canvas 어댑터는 그 설정을 바깥쪽 OpenHands
  [대화 요청](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/api/agent-server-adapter.ts#L930-L954)의
  [`NeverConfirm`](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/api/agent-server-adapter.ts#L477-L489)으로
  매핑하고 SDK 공급자 레지스트리는 full-access 모드를 사용한다. 어느 설정도 ACP
  `request_permission`을 운영자에게 전달하지 않는다. 그 콜백은 위의 별도 브리지 경로다. 분류:
  `verified`.
- [ACP 카드 내용](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/conversation-events/chat/event-content-helpers/get-acp-tool-call-content.ts#L70-L131)은
  래퍼의 구조화된 diff 내용이 아니라 원시 입력/출력을 읽는다. 분류: `verified`.
- [ACP 격리 한계](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/docs/ACP_AGENTS.md#L204-L212)는
  출시된 TypeScript 클라이언트가 `acp_isolate_data_dir`를 보낼 수 없으므로 동일 공급자 대화가 HOME
  상태를 공유할 수 있다고 말한다. 따라서 환경 로그인 공급자의 구성, 캐시, 잠금 파일을 공유할 수
  있다. 이와 별개로 SDK는 `CODEX_AUTH_JSON` 같은
  [등록된 파일 자격 증명을 구체화하여](https://github.com/OpenHands/software-agent-sdk/blob/7ef44d10b2132125833ce2269559b3960f402805/openhands-sdk/openhands/sdk/agent/acp_agent.py#L2149-L2224)
  대화별 스토리지에 저장하고 `CODEX_HOME`이 그곳을 가리키게 한다. Canvas 한계의 분류:
  `documented`, SDK 예외의 분류: `verified`.
- [ACP 하위 환경](https://github.com/OpenHands/software-agent-sdk/blob/7ef44d10b2132125833ce2269559b3960f402805/openhands-sdk/openhands/sdk/agent/acp_agent.py#L2300-L2347)은
  전체 상위 환경에서 시작하여 상속된 npm 변수와 선택된 충돌 변수만 제거하고 secret registry의
  모든 비파일 값을 추가한다.
  [파일 시크릿 권한](https://github.com/OpenHands/software-agent-sdk/blob/7ef44d10b2132125833ce2269559b3960f402805/openhands-sdk/openhands/sdk/agent/acp_agent.py#L2214-L2277)은
  제한된 디렉터리 및 파일 모드를 사용한다. 분류: `verified`.
- [Resume 폴백](https://github.com/OpenHands/software-agent-sdk/blob/7ef44d10b2132125833ce2269559b3960f402805/openhands-sdk/openhands/sdk/agent/acp_agent.py#L2516-L2581)은
  load protocol 오류 후 새 세션을 만들 수 있다. 분류: `verified`. 조용한 연속성 영향은 fault test
  전까지 `inferred`다.
- [Prompt 재시도](https://github.com/OpenHands/software-agent-sdk/blob/7ef44d10b2132125833ce2269559b3960f402805/openhands-sdk/openhands/sdk/agent/acp_agent.py#L3400-L3479)는
  선택된 전송 및 내부 실패를 재시도한다. 수락된 turn 중복은 response-loss injection을 실행하기
  전까지 `unverified` 위험이다.
- 출시된 브리지는
  [close, list, resume 세션 기능만 광고](https://github.com/zed-industries/codex-acp/blob/bb590500e8646f6daf879b8b3c6a659fbd29017d/src/codex_agent.rs#L452-L461)하고,
  [등록된 요청 handler](https://github.com/zed-industries/codex-acp/blob/bb590500e8646f6daf879b8b3c6a659fbd29017d/src/codex_agent.rs#L120-L307)에는
  fork 메서드가 없다. 분류: `verified` fork 부재.

### 이후 Codex ACP 경로, 출시되지 않음

- SDK 1.35는 레지스트리를
  [`@agentclientprotocol/codex-acp@1.1.2`](https://github.com/OpenHands/software-agent-sdk/blob/9028562e2d5eda76de662ec9b7584125760eb83f/openhands-sdk/openhands/sdk/settings/acp_providers.py#L378-L446)로
  전환하며 [Dockerfile](https://github.com/OpenHands/software-agent-sdk/blob/9028562e2d5eda76de662ec9b7584125760eb83f/openhands-agent-server/openhands/agent_server/docker/Dockerfile#L176-L195)은
  같은 버전을 설치한다. Canvas v1.2.1에 대한 분류: `proposed`.
- 유지보수 브리지는
  [`codex app-server`를 실행](https://github.com/agentclientprotocol/codex-acp/blob/8aff492d4b033ff2c02ad3b9d591994d57617463/src/CodexJsonRpcConnection.ts#L15-L42)하고
  [package manifest](https://github.com/agentclientprotocol/codex-acp/blob/8aff492d4b033ff2c02ad3b9d591994d57617463/package.json#L63-L69)는
  Codex `^0.144.0`을 선언한다. 이후 구성 요소에서는 `verified`, Canvas에 대해서는 `proposed`.
- 이벤트 handler는
  [Plan](https://github.com/agentclientprotocol/codex-acp/blob/8aff492d4b033ff2c02ad3b9d591994d57617463/src/CodexEventHandler.ts#L551-L561)을
  내보내지만 [이벤트 switch](https://github.com/agentclientprotocol/codex-acp/blob/8aff492d4b033ff2c02ad3b9d591994d57617463/src/CodexEventHandler.ts#L182-L221)에서는
  네이티브 diff/patch 알림을 무시한다. SDK 1.35의
  [Python 이벤트 브리지](https://github.com/OpenHands/software-agent-sdk/blob/9028562e2d5eda76de662ec9b7584125760eb83f/openhands-sdk/openhands/sdk/agent/acp_agent.py#L1221-L1339)는
  여전히 Plan을 소비하지 않는다. 이후 소스 경로의 분류: `verified`.
- 유지보수 브리지의
  [기능](https://github.com/agentclientprotocol/codex-acp/blob/8aff492d4b033ff2c02ad3b9d591994d57617463/src/CodexAcpServer.ts#L207-L229)과
  [등록된 handler](https://github.com/agentclientprotocol/codex-acp/blob/8aff492d4b033ff2c02ad3b9d591994d57617463/src/index.ts#L106-L132)는
  fork를 생략하지만 SDK 1.35의
  [`ask_agent()`](https://github.com/OpenHands/software-agent-sdk/blob/9028562e2d5eda76de662ec9b7584125760eb83f/openhands-sdk/openhands/sdk/agent/acp_agent.py#L3566-L3617)는
  `fork_session`을 호출한다. method-not-found 결과는 `inferred`이며 종단 간으로 재현하지 않았다.
- [선택적 app-server 로깅](https://github.com/agentclientprotocol/codex-acp/blob/8aff492d4b033ff2c02ad3b9d591994d57617463/src/CodexJsonRpcConnection.ts#L45-L60)은
  원시 프레임을 기록하고
  [프롬프트 로깅](https://github.com/agentclientprotocol/codex-acp/blob/8aff492d4b033ff2c02ad3b9d591994d57617463/src/CodexAcpServer.ts#L1425-L1429)은
  원시 프롬프트 데이터를 포함할 수 있다. 유출은 canary test 전까지 `inferred` opt-in 위험이다.
- [작업 공간 신뢰 주입](https://github.com/agentclientprotocol/codex-acp/blob/8aff492d4b033ff2c02ad3b9d591994d57617463/src/CodexAcpClient.ts#L379-L390)은
  구성된 root를 trusted로 표시한다. 이후 브리지의 분류: `verified`.

<a id="backend-registry-and-telemetry"></a>
### 백엔드 레지스트리와 텔레메트리

- [백엔드 레코드](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/api/backend-registry/types.ts#L1-L9)는
  호스트와 API 키를 포함하고,
  [스토리지](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/api/backend-registry/storage.ts#L80-L120)는
  레코드를 브라우저 로컬 스토리지에 보존한다. 분류: `verified`.
- [백엔드 상태 쿼리](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/hooks/query/use-backends-health.ts#L63-L299)는
  상태 저하와 복구 처리를 검증한다. 분류: `verified`.
- [익명 설치 텔레메트리](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/hooks/use-telemetry.ts#L29-L80)는
  DNT나 환경 정책으로 비활성화하지 않으면 일반 추적 동의 전에 설치 이벤트 하나를 보낼 수 있다.
  분류: `verified`.

<a id="automation-114"></a>
### Automation 1.1.4

- [실행 모델](https://github.com/OpenHands/automation/blob/7e05cde624fee2dad9f9a8ed2b654d9ba4cc3e71/openhands/automation/models.py#L121-L200)은
  상태, 오류, 대화, 샌드박스, 명령, 페이로드, 타임스탬프를 저장한다. 분류: `verified`.
- [키/값 암호화](https://github.com/OpenHands/automation/blob/7e05cde624fee2dad9f9a8ed2b654d9ba4cc3e71/openhands/automation/utils/kv.py#L175-L218)는
  SDK Fernet helper를 통해 자동화별 JSON 상태를 검증하고 암호화 및 복호화한다. 분류:
  `verified`.
- [자동화 생성](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/automations/create-instructions.tsx#L52-L95)은
  결정론적 생성 양식을 제출하는 대신 대화에서 번역된 프롬프트를 실행한다. 추천 recipe도 같은
  [에이전트 매개 프롬프트 경로](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/automations/recommended-automations-launcher.tsx#L82-L121)를
  사용한다. 분류: `verified`.
- 편집 모달은 일정 컨트롤을 노출하지만
  [이벤트 트리거 필드는 읽기 전용으로 렌더링](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/automations/detail/edit-automation-modal.tsx#L316-L428)한다.
  분류: `verified` 부분 편집 화면.
- [Scheduler](https://github.com/OpenHands/automation/blob/7e05cde624fee2dad9f9a8ed2b654d9ba4cc3e71/openhands/automation/scheduler.py#L144-L214),
  [dispatcher](https://github.com/OpenHands/automation/blob/7e05cde624fee2dad9f9a8ed2b654d9ba4cc3e71/openhands/automation/dispatcher.py#L347-L400),
  [watchdog](https://github.com/OpenHands/automation/blob/7e05cde624fee2dad9f9a8ed2b654d9ba4cc3e71/openhands/automation/watchdog.py#L225-L328)은
  영속 폴링, 백그라운드 디스패치, 시간 제한, 오래된 실행 조정을 검증한다. 분류: `verified`.
- [Webhook 보안 메모](https://github.com/OpenHands/automation/blob/7e05cde624fee2dad9f9a8ed2b654d9ba4cc3e71/openhands/automation/event_router.py#L1-L29)는
  HMAC 지원과 누락된 재전송/속도 제한 제어를 기록한다. 일치하는 delivery는 소스 이벤트 중복 제거
  키 없이 새 pending run을 만든다. 분류: `verified`.
- 백엔드는 6개의
  [실행 상태](https://github.com/OpenHands/automation/blob/7e05cde624fee2dad9f9a8ed2b654d9ba4cc3e71/openhands/automation/schemas.py#L200-L208)를
  노출하지만 Canvas의
  [실행 타입](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/types/automation.ts#L46-L68)은
  4개만 선언한다. [API는 상태를 그대로 전달](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/api/automation-service/automation-service.api.ts#L153-L174)하고
  [배지는 fallback 없이 구성을 역참조](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/components/features/automations/detail/run-status-badge.tsx#L13-L72)한다.
  분류: `verified` 계약 불일치. 런타임 `TypeError`는 `inferred`.
- 백엔드의
  [클라우드 동시성 경로](https://github.com/OpenHands/automation/blob/7e05cde624fee2dad9f9a8ed2b654d9ba4cc3e71/openhands/automation/dispatcher.py#L197-L208)는
  `SKIPPED`를 만들고
  [취소 endpoint](https://github.com/OpenHands/automation/blob/7e05cde624fee2dad9f9a8ed2b654d9ba4cc3e71/openhands/automation/router.py#L431-L522)는
  `CANCELLED`를 만든다. 이는 죽은 enum 값이 아니라 도달 가능한 백엔드 상태다. 분류: `verified`.
- 백엔드 응답에는 Canvas 실행 타입이 생략한
  [`timeout_at`, `sandbox_id`, `created_at`](https://github.com/OpenHands/automation/blob/7e05cde624fee2dad9f9a8ed2b654d9ba4cc3e71/openhands/automation/schemas.py#L625-L640)도
  있다. Canvas
  [Automation 서비스 메서드](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/src/api/automation-service/automation-service.api.ts#L56-L189)는
  목록, 갱신, 삭제, 디스패치, 실행 목록을 노출하지만 실행 취소 호출은 없다. 분류: `verified`.
- [배지 테스트](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/__tests__/components/automations/detail/run-status-badge.test.tsx#L7-L31)는
  Canvas의 4개 상태만 다룬다. 분류: `verified` 누락된 계약 커버리지.

<a id="quality-and-validation"></a>
### 품질과 검증

- [메인 CI](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/.github/workflows/ci.yml#L23-L99)는
  Ubuntu와 Windows에서 설치 및 애플리케이션 빌드를 실행한다. lint, test, library build, package
  verification은 Ubuntu `full_checks` 작업에서만 실행한다. 분류: `verified`.
- [커버리지 구성](https://github.com/OpenHands/agent-canvas/blob/56d51c0767fb6fedc51c466f5138fdfc116a2707/vite.config.ts#L417-L438)은
  있지만 이 조사에서는 임계값이나 메인 CI 커버리지 게이트를 찾지 못했다. 구성의 분류:
  `verified`, 소스 검색 후 저장소 전체 부재의 분류: `inferred`.
- 실시간 E2E 워크플로는 수동이거나 적격한 동일 저장소 pull request에 대한 레이블 게이트 방식이다.
  Mock-LLM과 Docker E2E에는 별도 워크플로가 있다. 워크플로 조사의 분류: `verified`. 이 조사에서는
  실행하지 않았다.

<a id="akra-baseline-evidence"></a>
## Akra 기준선 증거

- [App-server 어댑터](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/adapter/outbound/app_server/mod.rs#L380-L395)와
  [세션 카탈로그](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/adapter/outbound/app_server/mod.rs#L1135-L1175)는
  직접 공식 app-server 초기화, thread 작업, 세션 매핑을 검증한다. 분류: `verified`.
- [프로토콜 분류 테스트](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/adapter/outbound/app_server/protocol/contract_tests.rs#L314-L331)는
  app-server 알림에 명시적 처분이 없으면 실패한다. 분류: `verified`.
- 현재 Akra approval 도메인은
  [accept와 decline만 노출](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/domain/conversation.rs#L245-L262)하고,
  app-server 명령 요청에 한 turn용
  [accept와 decline이 모두 있어야 한다](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/adapter/outbound/app_server/approval.rs#L471-L490).
  file-change approval은
  [검사 불가로 분류](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/adapter/outbound/app_server/approval.rs#L17-L23)하여
  [거부](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/adapter/outbound/app_server/connection.rs#L1847-L1858)하며,
  해당 경로를 위한 타입 있는 양식이 없으므로
  [MCP elicitation도 거부](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/adapter/outbound/app_server/connection.rs#L1870-L1884)한다.
  직접 경계는 구조적인 것이며 완전한 결정 옵션 충실도는 출시되지 않았다. 분류: `verified`.
- [하위 환경 정책](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/adapter/outbound/app_server/connection.rs#L443-L520)은
  환경 변수를 필터링하고,
  [자격 증명 전달](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/adapter/outbound/app_server/connection.rs#L725-L773)은
  명시적인 API-key opt-in을 요구한다. 분류: `verified`.
- [병렬 roster](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/domain/parallel_mode/agent_session.rs#L14-L31)와
  [상세](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/domain/parallel_mode/agent_session.rs#L95-L121)는
  에이전트, 태스크, slot, 브랜치, 수명주기, 검증, 권한, distributor 투영을 검증한다. 분류:
  `verified`.
- [병렬 이벤트 무효화](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/application/service/parallel_mode/turn.rs#L348-L358)는
  도구 활동과 delta/completion 이벤트를 의도적으로 제외한다. 분류: `verified` 현재 활동 격차.
- 현재 대화 활동에는 완료된
  [file-change와 command-execution 종류](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/domain/conversation.rs#L192-L210)만
  있고, [공급자 stream channel](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/application/service/conversation_runtime_event.rs#L1-L20)은
  용량 8의 bounded synchronous channel이다. authority store는
  [runtime-event sequence](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/adapter/outbound/db/sqlite_planning_authority_adapter/runtime_projection.rs#L2564-L2603)를
  transactionally 할당한다. 분류: `verified`.
- [현재 전달 계약](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/docs/supersession/current-contract.md#L124-L164)은
  lease, 고정 범위, 기본 검토 PR, 명시적인 고위험 예외, 직렬화된 통합, 원격 검증, 정리 흐름을
  설명한다. 문서만 증거로 취급하지 않도록 소스 서비스를 조사했다. 현재 계약의 분류:
  `verified`.
- [네이티브 PR 스크립트](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/scripts/check_native_pr.sh#L1-L24)는
  TUI layering, Node surface, Rust format, test, clippy 게이트를 실행한다.
  [네이티브 PR 워크플로](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/.github/workflows/native-pr-checks.yml#L33-L176)는
  플랫폼 대상 테스트 작업과 광범위한 스크립트 게이트를 추가한다. 분류: `verified`.
- [Admin loopback bind](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/adapter/inbound/admin_api/mod.rs#L75-L125)는
  Akra에 아직 Canvas 방식 원격 백엔드 전환이 없음을 검증한다. 분류: `verified`.
- [CLI dispatch](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/src/adapter/inbound/cli.rs#L41-L150)는
  Admin, Telegram, planning-tool, parallel-tick 진입점을 노출한다. 저장소 전체 소스 검색에서는
  schedule trigger, signed webhook intake, durable automation run ledger를 찾지 못했다. 현재 CLI
  명령의 분류: `verified`, 소스 검색 후 저장소 전체 부재의 분류: `inferred`.
- [Diorama unit 할당](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/assets/admin/game/src/akra-diorama.ts#L611-L682),
  [연속 이동](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/assets/admin/game/src/akra-diorama.ts#L757-L780),
  [연속 패킷 애니메이션](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/assets/admin/game/src/akra-diorama.ts#L836-L876)은
  영속적인 작업 이벤트에서 파생되지 않는다. 분류: `verified` 진실성 격차.
- [시각 테스트](https://github.com/RefinedStone/codex-exec-loop/blob/0229ed71a541d91abc04ad13b188d28b9eafcaf8/scripts/capture_admin_graphic.mjs#L162-L178)는
  현재 idle 상태에서 canvas 픽셀이 바뀌도록 요구한다. 분류: `verified`.

<a id="audit-limits"></a>
## 조사 한계

이 조사에서는 다음을 수행하지 않았다.

- 인증된 Agent Server 및 Codex 대화 실행
- 브라우저에서 Automation `CANCELLED`/`SKIPPED` 배지 충돌 재현
- 대화 3개 동시 실행 또는 공유 HOME ACP race 유발
- 원격/클라우드 백엔드 failover 또는 공개 자체 호스팅 배포 실행
- cold start, 상호작용 지연 시간, 안정 상태 메모리 또는 동시 세션 scaling 측정
- ACP 권한 선택, response loss, 프로세스 충돌, 대규모 출력 또는 버전 drift 주입
- 모든 `npm audit` advisory가 production에서 도달 가능한지 검증
- OpenHands goal 판정과 Akra planning authority의 품질 비교
- 이후 SDK 1.35 또는 유지보수 Codex ACP 동작을 출시된 Canvas v1.2.1 동작으로 취급

가장 가치가 높은 누락 실험은 Akra와 유지보수 ACP 스택을 거치는 필드 보존, 순서, ID, 권한 결과,
실패 복구를 측정하는 golden app-server trace다. 그다음은 원시 샘플과 완전한 프로세스 트리 계산을
사용하는 동일 머신 인증 토폴로지 벤치마크다. 이것들이 있기 전까지 프로토콜 및 성능 주장은 위의
소스 사실로 범위를 제한한다.
