# R10. 블루프린트 화면으로 옮기기

상태: 진행 중 · 기준일: 2026-11-12 · 선행: P3, R1~R4 · 규모: 약 1~2주

## 목표

실제 앱(`desktop/src/renderer/`)을 UX 블루프린트(`desktop/src/renderer/blueprint/`)의 화면으로 옮긴다.
지금 데이터가 있는 기능만 옮긴다. 데이터나 서버 기능이 없는 것은 만들지 않고, 화면에도 넣지 않는다.

## 규칙

- 동작은 바꾸지 않는다. `DesktopSession`, `desktopStore`, `transcript`, main, preload 의 계약은 그대로 둔다.
  화면에 필요한 값만 더한다.
- 블루프린트는 계속 mock 으로 둔다 (`session/`, `rpc/`, `window.akra` 를 읽지 않는다). 두 화면이 함께 쓰는
  것(CSS, 아이콘)은 `design/` 으로 옮긴다.
- 동작하지 않는 버튼은 넣지 않는다. 블루프린트의 `notConnected` 버튼은 실제 앱에 옮기지 않는다.
- `data-testid` 는 유지한다. 자리가 바뀐 것만 e2e 를 고친다.
- 턴을 만드는 요청은 사용자가 보낸 메시지에서만 나간다 (D-13). 시험은 가짜 Gateway 를 쓴다.
- S1~S5 는 PR 하나이다. 조각을 따로 넣으면 옛 모양과 새 모양의 CSS 가 섞인 화면이 중간에 생긴다. S6 의 항목은
  각각 PR 하나이다. PR 마다 `npm run check`, `npx vitest run`, `npm run e2e` 를 통과한다.

## 기능 대응표

| 블루프린트 | 실제 데이터 | 조각 |
| --- | --- | --- |
| 전체 틀 (사이드바, 위 막대, 본문, 오른쪽 패널) | 현재 `App.tsx` 의 구성요소 | S1 |
| 오른쪽 패널 탭: 변경, Akra, 브라우저, 업무, 문서 | `WorkingTreePanel`, `KernelPanel`, `BrowserPanel`, GitHub·Jira·Notion 패널, `DocumentsPanel` | S1 |
| 패널 너비 조절, 사이드바 접기, 밝은/어두운 화면, 단축키 목록, 알림 | 화면 상태만 필요 | S1 |
| 터미널 영역 (위 막대 버튼, `Ctrl/⌘+J`) | `TerminalDock` | S1 |
| 사이드바 상태 표시: 진행 중, 승인 대기 | `activeThreadIds`, `pendingApprovals` | S2 |
| 사이드바 상태 표시: 실패 | `Thread.status` 의 `systemError` | S2 |
| 사이드바 상태 표시: 읽지 않음 | 이 창이 본 진행 → 끝 변화 (창 메모리) | S2 |
| 스레드 줄의 브랜치, 지난 시간 | `Thread.gitInfo.branch`, `updatedAt` | S2 |
| 연결 상태와 codex 버전 | `connection`, `gateway` | S2 |
| 위 막대: 폴더 › 제목, 브랜치, 변경 줄 수 | 사이드바 목록, `gitStatus` | S2 |
| 대화: 사용자 메시지, 도구 호출 묶음, 생각, 변경 카드 | `ThreadItem`, `turnDiffFiles` | S3 |
| 대화: 계획 체크 목록 | `turn/plan/updated` | S3 |
| 대화: 진행 표시와 지난 시간, 턴 요약(걸린 시간), 복사, 맨 아래로 | `Turn.startedAt`, `Turn.durationMs` | S3 |
| 입력창: 모양, 높이 자동, 보내기/중단 버튼 | 현재 `Composer` | S4 |
| 입력창 위 승인 카드, `Ctrl/⌘+Enter` 허용, `Ctrl/⌘+Backspace` 거부 | 현재 `ApprovalPrompt` | S4 |
| 입력창 위 계획 진행 줄 | `turn/plan/updated` | S4 |
| 컨텍스트 사용량 표시 | `thread/tokenUsage/updated` | S4 |
| 새 스레드 화면 (폴더 선택, 첫 메시지, 최근 스레드) | `thread/start` 다음 `turn/start` | S5 |
| 현황판 | S2 의 상태 | S5 |
| 검색과 명령 (`Ctrl/⌘+K`) | 사이드바 목록, 화면 동작 | S5 |
| 대기 메시지, "지금 지시" | 창 메모리의 대기 목록, `turn/steer` | S6 |
| 모델, 추론 수준, 권한 선택 | `model/list`, `turn/start` 의 `model`·`effort`·`approvalPolicy` | S6 |
| `@` 파일 언급 | `fuzzyFileSearch` | S6 |
| 변경 줄에 메모, 다음 메시지에 첨부 | 화면 상태만 필요 | S6 |

## 옮기지 않는 것 (데이터나 서버 기능이 없음)

| 블루프린트 | 없는 것 |
| --- | --- |
| 고정, 완료로 정리, 미루기 | 스레드별 저장 값. Gateway 장부가 필요하다 |
| Worktree 에서 실행 선택 | 스레드용 worktree 를 만드는 서비스 |
| 홈의 제안, 커널 제안의 수락/무시 | 관찰-제안 (plan 20) |
| 스레드별 PR 배지, PR 만들기, 스테이지, 커밋 | 스레드와 PR 의 연결, git 쓰기 서비스 |
| 변경 범위 (마지막 턴, 스테이지 전, 브랜치 전체) | 범위별 diff. 지금은 작업 트리 전체만 읽는다 |
| 여기로 되돌리기 | 되돌리기 요청의 검증 |
| `/` 명령, 파일 첨부 | 명령 목록, 첨부 경로 |
| 브라우저 요소 선택 | R9 |

이 항목은 각 기능의 계획이 승인된 뒤에 따로 옮긴다.

## 조각

### S1. 전체 틀

- `blueprint/blueprint.css` 와 `blueprint/icons.tsx` 를 `design/` 으로 옮긴다. 블루프린트와 실제 앱이 함께 읽는다.
- `styles.css` 에서 블루프린트와 이름이 겹치는 규칙을 지운다. 아직 옮기지 않은 구성요소의 규칙은 둔다.
- 화면 상태 저장소 `session/uiStore.ts` 를 만든다: 보기, 사이드바, 패널, 터미널, 팔레트, 단축키 목록, 화면 색, 알림.
- `App.tsx` 를 블루프린트의 틀로 바꾼다. `GatewayBar` 는 위 막대가 된다. 버전 경고와 세션 오류는 위 막대 아래 줄에 둔다.
- 오른쪽 패널은 탭 다섯 개이다. `업무` 탭은 GitHub, Jira, Notion 을 구분 버튼으로 바꾼다. `변경` 탭은
  `WorkingTreePanel` 의 내용을 항상 펼쳐서 보여 준다.
- `TerminalDock` 은 열림 상태를 저장소에서 받는다. `Ctrl+\`` 는 유지하고 `Ctrl/⌘+J` 를 더한다.
- e2e: 패널 여는 단계가 바뀐 시험만 고친다 (Notion, GitHub, Jira 는 `업무` 탭을 먼저 연다).

### S2. 사이드바와 위 막대

- `WorkspaceThread` 에 `branch`, `failed` 를 더한다. 저장소에 `unreadThreadIds` 를 더한다.
- 순수 함수 `threadStatus(thread, active, approvals, unread)` 를 만들고 단위 시험을 둔다. 우선순위는
  승인 대기 › 진행 중 › 실패 › 읽지 않음 › 없음이다.
- 읽지 않음: 열려 있지 않은 스레드가 진행에서 끝으로 바뀌면 표시한다. 스레드를 열면 지운다. 창을 닫으면 없어진다.
- 사이드바 아래에 연결 상태, codex 버전, 단축키 버튼, 화면 색 버튼을 둔다.
- 위 막대에 폴더 › 제목, 브랜치, 변경 줄 수를 둔다. 변경 줄 수는 턴이 끝날 때와 스레드를 열 때 `gitStatus` 로 읽는다.

### S3. 대화

- 순수 함수 `turnBlocks(items)` 를 만든다: 사용자 메시지, 생각, 작업 묶음(명령·파일 변경·도구 호출이 이어진 것),
  답, 그 밖의 아이템. 단위 시험을 둔다.
- `transcript` 가 `turn/plan/updated` 를 턴의 `plan` 에 넣는다. 계획 카드는 이 값을 그린다.
- 진행 표시는 `Turn.startedAt` 부터 센다. 끝난 턴은 `durationMs` 를 보여 준다. 토큰 수는 턴별 값이 없으므로 넣지 않는다.
- 가상 스크롤, "이전 턴 더 보기", 맨 아래 따라가기는 유지한다. "맨 아래로" 버튼을 더한다.
- 주의: `fix/review-followups-desktop-2` 가 `ConversationView.tsx`, `desktopSession.ts` 를 고친다. 그 PR 이 먼저 들어가면 rebase 한다.

### S4. 입력창

- 승인 카드, 계획 진행 줄, 입력창을 한 영역으로 묶는다. 승인의 세 가지 답과 파일 변경 diff 는 유지한다.
- `transcript` 가 `thread/tokenUsage/updated` 를 받는다. 컨텍스트 사용량은 `last.totalTokens / modelContextWindow` 이다.
  창 크기 값이 없으면 표시하지 않는다.

### S5. 새 스레드 화면, 현황판, 검색과 명령

- 새 스레드 화면에서 보내면 `thread/start` 다음에 `turn/start` 를 보낸다. 사이드바의 `+` 는 그 폴더를 고른 새 스레드 화면을 연다.
  빈 스레드는 만들지 않는다.
- 업무 패널의 "넘기기"는 새 스레드 화면의 입력창을 채운다. 지금의 `composerDraft` 를 대신한다.
- 현황판은 S2 의 상태로 스레드를 네 열에 놓는다.

### S6. 창 안에서 새로 만드는 동작

각 항목은 따로 PR 이다. 시작 전에 동작을 이 문서에 적는다.

- 대기 메시지: 턴이 진행 중이면 대기 목록에 넣고, 턴이 끝나면 차례로 보낸다. "지금 지시"는 `turn/steer` 이다.
  대기 목록은 창 메모리에 둔다 (창을 닫으면 없어진다). 커널이 대기 목록을 갖게 되면 그쪽으로 옮긴다.
- 모델, 추론 수준, 권한 선택: `model/list` 로 읽고 다음 `turn/start` 에 넣는다.
- `@` 파일 언급: `fuzzyFileSearch`.
- 변경 줄 메모: 다음 메시지 끝에 `경로:줄 메모` 로 붙인다.

## 검증

- 조각마다: `cd desktop && npm run check && npx vitest run && npm run e2e`, `git diff --check`.
- 화면 확인은 숨김 창(`AKRA_DESKTOP_HIDDEN=1`)의 스크린샷으로 한다. 사용자 화면에 창을 띄우지 않는다.

## 진행 기록

### 2026-11-12: 계획

- 블루프린트 화면과 실제 앱을 비교해서 위 대응표를 만들었다.

### 2026-11-12: S1~S5

- 공용 파일: `design/app.css`(이전 `blueprint/blueprint.css`), `design/icons.tsx`, `design/Picker.tsx`. `styles.css` 에는
  블루프린트에 없는 부분(파일 변경 목록, 터미널, 패널 내용)만 남겼다.
- 화면 상태는 `session/uiStore.ts` 에 있다. 처음 화면은 새 스레드 화면이다.
- 사이드바: 상태 계산은 `sidebar/threadStatus.ts` 이다. `desktopStore` 에 `unreadThreadIds` 와 스레드의 `branch`, `failed` 를 더했다.
- 대화: 묶기는 `conversation/turnBlocks.ts` 이다. 명령의 `commandActions` 가 읽기나 검색 하나이면 그 종류로 보여 준다.
  `transcript` 는 `turn/plan/updated`, `thread/tokenUsage/updated`, 턴의 `startedAt`·`durationMs` 를 받는다.
- 입력창: 승인 카드와 계획 진행 줄이 입력창 위에 붙는다. `Ctrl/⌘+Backspace` 거부는 입력창에 글이 있으면 동작하지 않는다
  (낱말 지우기와 겹친다).
- 새 스레드: 보낼 때 `thread/start` 다음 `turn/start` 를 보낸다. 업무 패널의 넘기기는 새 스레드 화면의 입력창을 채운다.
  스레드별 초안 값(`composerDraft`)은 지웠다.
- 변경 패널: 작업 트리 목록을 스레드를 열 때와 턴이 끝날 때 읽는다 (이전: 영역을 펼쳤을 때만). 위 막대의 줄 수에 쓴다.
- 단축키: `Ctrl/⌘+N` 은 새 스레드이다. 새 창은 `Ctrl/⌘+Shift+N` 으로 옮겼다. 터미널 안의 `Ctrl` 조합은 셸에 준다 (`Ctrl+\`` 제외).
- 목록 응답에 `status` 가 없는 스레드도 읽는다 (시험 녹화가 그렇다. 이전에는 세션 오류가 보였다).
- e2e 변경: Notion 은 `work-toggle` 다음 `notion-toggle`, 보내기 버튼은 `title`, 명령 카드는 접힌 작업 묶음(`item-activity`)으로 확인한다.
- 검증 (Windows): `npm run check`, `npx vitest run` (134 중 3 건너뜀), `npm run e2e` (PASS 40, 터미널 시험은 Windows 에서 건너뜀).
  숨김 창 스크린샷으로 새 스레드 화면, 승인 카드, 업무 패널, 검색과 명령, 어두운 화면을 확인했다.

남은 것과 알려진 한계:

- macOS 실기 확인, 터미널 e2e (`AKRA_E2E_TERMINAL=1`), 실제 턴에서의 계획 카드와 컨텍스트 표시 확인.
- 브라우저 패널이 열려 있으면 검색과 명령, 단축키 목록이 페이지 뒤에 가려진다 (페이지는 main 의 WebContentsView 이다).
- 현황판은 열마다 최근 30개까지, 검색과 명령은 50개까지 그린다.
- S6 은 시작하지 않았다.
