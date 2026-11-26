# 33. Akra Desktop 사용자 흐름 (맡기기에서 정리하기까지)

상태: **Proposed**. 현재 제품 동작을 설명하지 않는다. · 기준일: 2026-11-26 (Asia/Seoul) ·
기준 커밋: `31d5279e` (`origin/prerelease`) ·
관련: [plan 26 Desktop 사용성](26-akra-desktop-usability.md), [plan 23 주의 항목](23-akra-attention-items.md),
[plan 24 증거](24-akra-evidence-harness.md), [plan 20 관찰과 제안](20-akra-observe-and-offer.md),
[R10 블루프린트](17-akra-desktop/roadmap/R10-blueprint-port.md), ADE 로드맵 (#2481)

## 한 줄 요약

Akra Desktop 에는 기능이 있다. 그러나 한 기능에서 다음 기능으로 가는 길을 화면이 말하지 않는다. 이 문서는 경쟁
앱 7 개의 실제 사용 순서를 7 단계로 적고, Akra 에서 흐름이 끊기는 곳과 고칠 화면을 정한다. 고칠 화면은 블루프린트
(`desktop/src/renderer/blueprint/`)에 mock 으로 넣었다.

## 범위

- [plan 26](26-akra-desktop-usability.md)은 기능이 있는지를 비교했다. 이 문서는 순서를 비교한다: 사용자가 무엇을
  누르고, 다음에 무엇을 보는가.
- 대상은 `desktop/` 이다. TUI 와 Admin 은 다루지 않는다.
- 다른 제품의 코드를 옮기지 않는다. 동작만 우리 말로 적는다.
- 판정이 필요한 값(누구의 차례인가, 왜 멈췄는가)은 kernel 이 준다 (plan 18, plan 23). Desktop 은 그린다.
- 턴을 만드는 요청은 사용자가 보낸 메시지에서만 나간다 (D-13). 이 문서의 모든 "넘기기"는 입력창을 채우기만 한다.

## 1. 조사 근거

| 제품 | 고정점 | 읽은 것 | 근거 등급 |
| --- | --- | --- | --- |
| Codex 앱 | 2026-11-26 의 공식 문서. 지금은 ChatGPT 데스크톱 앱의 Codex 모드이다 | app, code-review, git-worktrees, automations, commands, local-environment, settings, slash-commands, integrated-terminal, projects | `documented` |
| Claude Code Desktop | 2026-11-26 의 공식 문서, 2026-04-14 의 재설계 글 | desktop, desktop-quickstart, desktop-scheduled-tasks | `documented`. desktop 문서의 뒷부분은 요약으로 읽었다 |
| Cursor 3 Agents Window | 문서, 변경 기록 3.0 (2026-04-02), 3.3 (2026-05-07) | agents-window, worktrees, review, planning, cloud-agent | `documented`. 창 안의 단추와 단축키는 문서에 거의 없다 |
| Conductor | 문서, 변경 기록 0.101.0 (2026-11-25) 까지 | workflow, first-workspace, workspaces-and-branches, diff-viewer, checks, scripts, checkpoints, review-and-merge | `documented` |
| Antigravity 2.x | 2026-11-26 의 공식 문서, codelab | getting-started, projects, side-panel, review-changes, artifacts, artifact-review, agent-settings, browser | `documented` |
| Orca | v1.4.277 (2026-11-26) | README, 문서 (first-session, worktrees, notifications), 릴리스 노트 | `documented`. 소스 확인은 [Orca 조사](../competitive/orca/README.md)에 있다 |
| T3 Code | v0.1.12 (2026-11-25). OmO Desktop 의 UI 바탕이다 | README, 사용자 문서 (install, source-control, remote-access, thread-sidebar), 릴리스 노트 | `documented` |
| Akra | `31d5279e` 의 `desktop/` | 소스, [현재 제품 문서](../reference/current-product.md) | `verified/source` |

어느 앱도 실행하지 않았다. 화면 동작(`verified/runtime`)은 확인하지 않았다. 아래의 "실제 흐름"은 각 제품이 문서에
적은 순서이다. 출처 주소는 §9 에 있다.

## 2. 일곱 단계

일곱 앱의 문서가 같은 순서를 적는다. 이름은 이 문서가 붙였다.

| 단계 | 사용자가 하는 일 |
| --- | --- |
| 1 맡기기 | 어디에서 실행할지 고르고 첫 메시지를 보낸다 |
| 2 지켜보기 | 여러 작업 가운데 어느 것이 나를 기다리는지 안다 |
| 3 답하기 | 승인, 질문, 계획에 답한다 |
| 4 검토하기 | 변경을 읽고 고칠 곳을 에이전트에게 돌려준다 |
| 5 전달하기 | 검증, 커밋, push, PR, CI, 머지 |
| 6 정리하기 | 끝난 작업과 worktree 를 치운다 |
| 7 돌아오기 | 떠나 있던 동안 생긴 일을 본다 |

## 3. 경쟁 앱의 실제 흐름

### 3.1 맡기기

| 앱 | 순서 |
| --- | --- |
| Codex 앱 | New chat → 입력창 아래에서 Local, Worktree, Cloud 를 고른다 → Worktree 이면 시작 브랜치를 고른다 → 보낸다. worktree 는 `$CODEX_HOME/worktrees` 에 생기고, 고른 브랜치의 커밋 안 한 변경을 복사한다 |
| Claude Code Desktop | 입력창 아래 한 줄에 환경(Local, SSH, Cloud), 폴더, 권한 모드, 모델이 있다. worktree 격리는 보내기 전에 켠다 |
| Conductor | "Create from…" 에서 브랜치, GitHub 이슈, Linear 이슈, PR 을 고른다 → worktree 와 브랜치가 생긴다 → 설정 스크립트가 돈다 → 첫 대화가 브랜치 이름을 바꾼다 |
| Antigravity | 프롬프트를 보내면 창이 Local mode 와 New worktree mode 를 묻는다 |
| Orca | 저장소 옆의 + → 이름을 적는다. PR, Linear, Jira 주소를 붙이면 그 항목에 연결된다 → 창이 닫힌다. 생성은 뒤에서 진행 줄로 보이고, 실패하면 Retry 가 있다 |
| T3 Code | New thread → 필요하면 New worktree. 모델 선택기에서 모델을 여러 개 고르면 모델마다 스레드와 worktree 가 생긴다 |
| Cursor 3 | 에이전트를 local, worktree, cloud, SSH 에서 시작한다. 설정은 `.cursor/worktrees.json` 이다 |

공통점: 실행 위치는 입력창 옆에서 고른다. worktree 를 만드는 동안 사용자를 기다리게 하지 않는다.

### 3.2 지켜보기

| 앱 | 순서 |
| --- | --- |
| Codex 앱 | 사이드바는 프로젝트별이다. 읽지 않음 표시가 있다. "응답이 필요한 다음 대화"로 가는 단축키가 있다 (Windows `Ctrl+Alt+A`). 실행 중 보낸 메시지를 steer 로 할지 대기로 할지는 설정이다 |
| Claude Code Desktop | 사이드바를 상태, 프로젝트, 환경으로 거르고 묶는다. 대화 보기 수준이 세 개이다 (Normal, Thinking, Verbose, `Ctrl+O`). 실행 중 `Enter` 는 멈추지 않고 고칠 말을 보낸다 |
| Conductor | 사이드바를 상태로 묶는다 (backlog, in progress, in review, done). 계획 준비, 질문, 권한 요청은 아이콘이 다르다. 행에 마우스를 올리면 다음 행동이 보인다. 대기 메시지를 지우고, 바로 보내고, 고친다 |
| T3 Code | 사이드바 구역은 Pinned, Active, Settled, Snoozed 이다. Working 구역(beta)은 도는 스레드를 따로 모은다. 끝나거나 실패하거나 입력이 필요하면 Active 로 돌아온다 |
| Orca | 읽지 않은 worktree 는 굵게 보인다. 에이전트가 일을 멈추면 OS 알림과 소리가 난다. 위 막대의 종이 읽지 않은 것을 모은다. macOS Dock 에 수가 보인다 |
| Antigravity | 대화마다 Overview 패널이 하위 에이전트, 뒤에서 도는 작업, 목표, 산출물, 바뀐 파일을 모은다 |

공통점: 목록의 첫 기준은 "누구의 차례인가"이다. 프로젝트는 둘째 기준이다.

### 3.3 답하기

| 앱 | 순서 |
| --- | --- |
| Codex 앱 | 승인은 대화 안에 나온다. `Enter` 는 승인, `Esc` 는 거부이다. `/plan` 이 계획 모드를 켠다 |
| Claude Code Desktop | Manual 모드는 변경마다 diff 와 승인, 거부 단추를 보인다. 웹 동작은 Allow once, Always allow, Deny 이다 |
| Antigravity | 승인은 대화 안에 나온다. 범위가 셋이다: 이번만, 이 대화에서 이 도메인, 항상. 계획 카드에는 Proceed 가 있다 |
| Conductor | 계획은 승인하거나 의견을 준다. 권한 요청에서 도구를 영구 허용할 수 있다 |
| Orca | 모바일 앱의 "Needs you" 가 도구 승인, 접근 요청, 로그인 요청을 한곳에 모은다 |
| T3 Code | Supervised 모드에서 명령과 파일 변경을 묻는다. 하위 에이전트의 승인은 부모 대화에 나온다 |

공통점: 승인은 대화 안에 있다. 여러 작업의 요청을 한곳에 모으는 화면은 Orca 모바일에 있다. plan 26 의 A5 는 Codex 앱과
OmO Desktop 에도 있다고 적었다.

### 3.4 검토하기

| 앱 | 순서 |
| --- | --- |
| Codex 앱 | 검토 탭 → 범위를 고른다 (Unstaged, Staged, Commit, Branch, Last turn) → 줄의 + 로 메모를 단다 → 메시지를 보내면 메모가 함께 간다. 파일과 hunk 단위로 stage 와 revert 를 한다 |
| Claude Code Desktop | 대화의 `+12 -1` 을 누른다 → 파일별 diff → 줄의 + 로 메모를 달고 `Enter` → `Ctrl+Enter` 로 모두 보낸다. `/code-review` 의 결과 카드에 Walk through in changes, Fix this one, Apply fixes 가 있다 |
| Conductor | `⌘⇧D` → 파일을 "본 것"으로 표시한다. 그 파일이 바뀌면 "Ready for review" 로 돌아온다. 줄 메모는 입력창의 첨부가 된다. `⌘⌥J` 는 다음 메모로 간다 |
| Antigravity | "N files changed" 카드나 Overview 의 Review → 줄 메모 → 입력창 위에 "1 Comment" 칩이 생긴다 → 다음 메시지와 함께 간다. `Ctrl+L` 은 메모 없이 줄 범위를 인용한다 |
| T3 Code | Diff 패널의 출처 선택: 턴의 변경, 기준 브랜치와의 비교, 커밋. Code 탭에서 파일을 본 것으로 표시한다 |
| Orca | Annotate AI Diff: 바뀐 줄에 메모를 달아 에이전트에게 돌려준다 |

공통점: 줄 메모는 바로 가지 않는다. 다음 메시지에 붙는다. 본 파일 표시는 Conductor, T3 Code, Codex 의 PR 검토에 있다.

### 3.5 전달하기

| 앱 | 순서 |
| --- | --- |
| Conductor | `⌘⇧P` 로 PR 을 만든다 (에이전트가 설명을 쓴다) → Checks 탭이 git 상태, CI, 배포, 댓글과 리뷰, 할 일을 한 목록으로 보인다 → 남은 할 일과 실패한 검사는 머지를 막는 것으로 보인다 → 충돌은 에이전트에게 맡긴다 |
| Claude Code Desktop | Create PR → PR 이 Changes 카드에 CI 상태와 함께 보인다 → PR 제목 옆 메뉴에 Auto-fix (실패한 검사와 새 리뷰 댓글을 고친다) 와 Merge when ready 가 있다 |
| Codex 앱 | 앱에서 커밋, push, PR 을 한다. PR 검토에 Summary 와 Changes 탭, Mark as viewed 가 있다. Checks 의 Fix 는 실패한 검사를 대화에 붙이고, 사용자가 확인하면 보낸다 |
| T3 Code | Commit & push → Pull requests 가 검사, 댓글, 리뷰, 커밋을 보인다. Watch PR 은 2 분마다 읽고, 실패한 검사나 새 댓글이 있으면 에이전트를 깨운다 |
| Cursor 3 | diff 보기에서 stage, commit, PR 관리를 한다. 3.3 은 PR 검토 탭 (Reviews, Commits, Changes) 과 "여러 PR 로 나누기"를 넣었다 |
| Antigravity | Review 의 Commit 단추. 메시지를 비우면 만들어 준다. 옆 화살표에 Amend 와 Push 가 있다. PR 흐름은 문서에 없다 |
| Orca | 커밋, push, PR, 검사 기다리기를 한 화면에서 한다. 실패한 검사와 리뷰 댓글을 에이전트에게 돌려준다 |

공통점: 변경 뒤의 일을 한 곳에 순서대로 놓는다. 실패한 CI 에서 대화로 돌아가는 단추가 있다.

### 3.6 정리하기

| 앱 | 순서 |
| --- | --- |
| Codex 앱 | 보관하면 worktree 를 지운다. 지우기 전에 스냅샷을 남기고, 대화가 복원을 제안한다. 최근 15 개만 둔다 (설정). 고정했거나 도는 것은 지우지 않는다 |
| Claude Code Desktop | 행의 메뉴 → Archive. 커밋 안 한 변경이 있으면 "Archive anyway" 와 취소를 묻는다. PR 이 머지되거나 닫히면 자동 보관하는 설정이 있다 |
| Conductor | 보관 전에 보관 스크립트가 돈다. 커밋 안 한 변경은 커밋으로 남긴다. PR 이 머지되면 자동 보관하고 Unarchive 를 보인다. History 에서 대화까지 되살린다 |
| T3 Code | Settle (지금은 끝), Snooze (미루기), Archive. 7 일 동안 움직임이 없거나 연결된 PR 이 머지되면 자동 Settle. 새 메시지가 오면 다시 Active 이다 |
| Orca | 오른쪽 클릭 → archive, sleep, delete. 머지 안 한 브랜치는 남기고 "Review N Branches" 로 묻는다 |
| Cursor 3 | worktree 는 기계마다 25 개까지 둔다. 주기마다 오래된 것을 지운다 |

공통점: PR 머지가 정리의 신호이다. worktree 를 지울 때 커밋 안 한 변경을 잃지 않게 한다.

### 3.7 돌아오기

| 앱 | 순서 |
| --- | --- |
| Codex 앱 | 읽지 않음 표시. `Ctrl+Shift+U` 는 읽지 않음으로 되돌린다. `Shift+Esc` 는 모든 표시를 지운다. 보관한 대화는 설정에서 되살린다 |
| Conductor | 이미 고른 워크스페이스를 다시 누르면 가장 오래된 읽지 않은 대화로 간다 |
| Orca | 종 아이콘의 목록. 알림을 오른쪽 클릭하면 읽지 않음으로 되돌린다 |
| T3 Code | 스레드는 호스트에 있다. 모든 클라이언트가 같은 목록을 본다. 전화 앱으로 이어서 본다 |

## 4. Akra 의 지금과 끊기는 곳

| 단계 | 지금 (`31d5279e`) | 끊기는 곳 |
| --- | --- | --- |
| 1 맡기기 | 새 스레드 화면에서 폴더를 고르고 보낸다 | 실행 위치를 고를 수 없다 (격리 공간 API 는 #2775 가 만든다). 홈의 "최근 스레드"는 시간순이다. 나를 기다리는 것이 먼저 보이지 않는다 |
| 2 지켜보기 | 사이드바는 폴더별이다. 표시는 진행 중, 승인 대기, 실패, 읽지 않음이다. 현황판은 네 열이다 | 표시는 있고 이유는 없다. 폴더가 많으면 승인 대기를 찾으려고 목록을 훑는다. 다음 것으로 가는 명령이 없다 |
| 3 답하기 | 승인 카드는 열린 스레드의 입력창 위에 있다. `⌘⏎` 허용, `⌘⌫` 거부 | 다른 스레드의 승인은 그 스레드를 열어야 답한다. 현황판의 카드는 보기만 한다 |
| 4 검토하기 | 변경 패널: 파일 목록, 파일과 hunk 단위 stage 와 버리기 | 어디까지 읽었는지 남지 않는다. 턴이 변경을 바꾸면 처음부터 다시 읽는다. 대화의 변경 카드는 패널만 열고 파일은 고르지 않는다 |
| 5 전달하기 | 같은 패널의 아래에 커밋 목록, push 단추, PR 양식, PR 과 CI 상태가 있다. task 화면은 완료 기준과 증거를 보인다 | 다음에 할 일을 사용자가 세 구역(파일, 커밋, PR)을 읽어서 정한다. CI 가 실패하면 기록을 손으로 옮긴다. 증거 화면과 변경 패널이 이어지지 않는다 |
| 6 정리하기 | 스레드를 치우는 단추가 없다 (R10 "옮기지 않는 것"). 다른 client 가 보관한 스레드는 목록에서 뺀다 | 머지된 작업이 목록에 남는다 |
| 7 돌아오기 | 읽지 않음은 창의 메모리에 있다. 창이 뒤에 있으면 OS 알림이 온다 | 스레드를 열어도 어디부터 새 내용인지 보이지 않는다 |

Akra 가 이미 앞선 것은 바꾸지 않는다:

- main 이 동작 전에 git 상태를 다시 읽는다. push 와 PR 만들기는 대상을 보이는 확인 창을 거친다.
- 모델의 완료 보고와 호스트가 확인한 증거를 다른 사실로 둔다 (plan 24).
- 멈춘 이유를 kernel 이 안다. 경쟁 앱은 에이전트의 자기 보고를 읽는다.

## 5. 고칠 것

우선순위: **P1** 흐름이 끊기는 곳 · **P2** 흐름을 빠르게 하는 것 · **P3** 큰 기능. "mock" 열은 이 PR 의 블루프린트에
있는지이다.

### 5.1 맡기기

| ID | 제안 | 근거 앱 | 필요한 것 | mock | 우선 |
| --- | --- | --- | --- | --- | --- |
| F1-1 | 새 스레드 화면의 입력창 위 한 줄에서 폴더, 실행 위치 (worktree, 작업 폴더), 기준 브랜치를 고른다. 기준 브랜치는 worktree 일 때만 보인다 | Codex, Claude, T3, Antigravity | 격리 공간 API (#2775, A07) | ✓ | P1 |
| F1-2 | worktree 준비는 뒤에서 한다. 스레드는 바로 열린다. 준비는 대화의 한 줄이다. 실패하면 그 줄에 "다시 시도"가 있다 | Orca | 같음 | ✓ 성공 경로만 | P1 |
| F1-3 | 홈의 "최근 스레드"를 "이어서 하기"로 바꾼다. 순서는 내 차례, 읽을 것, 작업 중, 나머지이다. 줄마다 이유를 보인다 | Codex, Conductor, T3 | 주의 항목 목록 (plan 23, A02) | ✓ | P1 |

### 5.2 지켜보기

| ID | 제안 | 근거 앱 | 필요한 것 | mock | 우선 |
| --- | --- | --- | --- | --- | --- |
| F2-1 | 사이드바의 묶기를 바꾸는 단추: 프로젝트 / 상태. 상태 묶음은 내 차례, 읽을 것, 작업 중, 나머지이다. 빈 묶음은 그리지 않는다. 고정 구역은 두 보기에 모두 있다 | T3, Conductor, Claude | 주의 항목 목록. 그 전에는 지금의 표시 값 (`sidebar/threadStatus.ts`) | ✓ | P1 |
| F2-2 | 내 차례와 읽을 것의 줄은 브랜치 대신 이유 1 줄을 보인다. 예: "승인 필요: git push", "PR #412 의 CI 실패", "끝남 · 다음: 검토 0/3" | Conductor (다음 행동), Herdr | 원인의 `summary` (plan 23 §5) | ✓ | P1 |
| F2-3 | "다음 내 차례로 이동" 명령. 단축키 제안은 `⌘⇧A` (`Ctrl+Shift+A`) 이다. 끝에서는 처음으로 돌아간다 | Codex | 없음. 지금의 표시 값으로 된다 | ✓ | P1 |
| F2-4 | 대화 보기 수준 (보통, 자세히). 도구 호출을 접는 기본값을 바꾼다 | Claude | 없음 | – | P2 |

### 5.3 답하기

| ID | 제안 | 근거 앱 | 필요한 것 | mock | 우선 |
| --- | --- | --- | --- | --- | --- |
| F3-1 | 현황판의 "내 차례" 카드에서 스레드를 열지 않고 답한다: 승인은 허용과 거부, 실패는 "실패 기록 넘기기" | Orca 모바일, OmO | 주의함 (#2786, A02). 응답은 지금의 승인 계약을 쓴다 | ✓ | P1 |
| F3-2 | 엿보기: 카드에서 근거를 펼친다. 승인은 폴더와 이유, 실패는 마지막 답이다 | Herdr (plan 26 H4) | 없음 | ✓ | P2 |

### 5.4 검토하기

| ID | 제안 | 근거 앱 | 필요한 것 | mock | 우선 |
| --- | --- | --- | --- | --- | --- |
| F4-1 | 파일마다 "봄" 표시를 켜고 끈다. 그 파일의 변경이 바뀌면 표시를 지운다. 진행은 전달 띠의 "검토 2/3" 이다 | Conductor, T3, Codex | Desktop 만. 키는 스레드, 경로, 내용 지문이다 | ✓ | P1 |
| F4-2 | 대화의 변경 카드에서 파일을 누르면 변경 패널이 그 파일을 연다 | Claude, Antigravity | 없음 | ✓ | P1 |
| F4-3 | 줄 메모는 다음 메시지에 붙는다. 입력창 위에 칩으로 보인다 (R10 S6 의 계획 그대로) | Codex, Antigravity, Conductor, Claude | 없음 | ✓ (전부터 있음) | P1 |

### 5.5 전달하기

| ID | 제안 | 근거 앱 | 필요한 것 | mock | 우선 |
| --- | --- | --- | --- | --- | --- |
| F5-1 | 전달 띠: 변경 패널의 맨 위에 단계 줄을 둔다. 단계는 검토, 검증, 커밋, push, PR, CI, 머지이다. 끝난 단계는 ✓ 이다. 다음 단계 1 개를 강조하고 단추 1 개를 둔다. 위 막대의 변경 칩도 "다음: …" 을 보인다 | Conductor, Claude, Codex, T3 | 지금 있는 읽기: 작업 트리, 커밋 목록과 push 여부, PR 과 CI (`gitStatus.ts`, `gitBranch.ts`). 검증 단계는 task 증거 (A01) | ✓ | P1 |
| F5-2 | CI 가 실패하면 다음 행동은 "실패 기록 넘기기"이다. 실패한 검사의 이름과 기록의 끝부분을 그 스레드의 입력창에 채운다. 사용자가 보낸다 | Codex (Fix), Claude, Conductor, Orca | main 이 `gh` 로 실패 기록을 읽는다. 크기 상한을 둔다 | ✓ | P1 |
| F5-3 | PR 리뷰 댓글도 같은 방법으로 넘긴다 | Conductor, Orca, Claude | 업무 패널의 GitHub 읽기 | – | P2 |
| F5-4 | 검증 단계는 완료 기준이 있는 스레드에만 그린다. 다음 행동은 "검증 맡기기"이고, 증거가 없는 첫 기준을 입력창에 채운다 | Akra 고유 (plan 24) | `task_evidence` 조회 | ✓ | P2 |

### 5.6 정리하기

| ID | 제안 | 근거 앱 | 필요한 것 | mock | 우선 |
| --- | --- | --- | --- | --- | --- |
| F6-1 | PR 머지를 보면 그 스레드의 입력창 위에 "완료로 정리할까요?" 를 띄운다. 단추는 "완료로 정리"와 "나중에"이다. 스스로 정리하지 않는다 | Claude, Conductor, T3 | 개인 정리 저장 (A02 의 둘째 조각) | ✓ | P2 |
| F6-2 | 정리할 때 worktree 를 어떻게 하는지 말한다. 커밋 안 한 변경이 있으면 지우지 않는다 | Claude, Conductor, Codex | worktree 수명 (A07) | ✓ 문구만 | P2 |

### 5.7 돌아오기

| ID | 제안 | 근거 앱 | 필요한 것 | mock | 우선 |
| --- | --- | --- | --- | --- | --- |
| F7-1 | 읽지 않은 스레드를 열면 떠난 뒤의 첫 내용 앞에 "새 내용" 구분선을 그린다 | Orca, OmO (plan 26 R3) | 읽은 위치. 처음에는 창의 값이다 | ✓ | P1 |
| F7-2 | 스레드를 읽지 않음으로 되돌린다 | Codex, Orca | 같음 | – | P2 |

## 6. 결정 (제안)

- **UF-1 묶음의 이름:** plan 23 §2 의 이름을 쓴다 (내 차례, 읽을 것, 작업 중, 나머지). plan 26 H1 이 적은 이름
  (끝남, 일하는 중, 쉬는 중) 은 쓰지 않는다. 판정의 주인이 plan 23 이다.
- **UF-2 전달 띠는 막지 않는다:** 단계는 안내이다. 검토를 끝내지 않아도 push 를 할 수 있다. 막는 것은 저장소가
  enforce 를 켰을 때의 kernel 게이트뿐이다 (plan 24).
- **UF-3 다음 단계의 규칙:** 끝나지 않은 첫 단계가 다음이다. 실패한 CI 는 앞 단계가 남아도 먼저이다. 고치면 변경이
  바뀌고 앞 단계를 다시 한다. 규칙은 `blueprint/flow.ts` 의 `deliveryOf` 에 있다.
- **UF-4 넘기기는 채우기이다:** "실패 기록 넘기기"와 "검증 맡기기"는 그 스레드의 입력창을 채운다. 보내기는 사용자가
  누른다 (D-13). Claude 의 Auto-fix 와 T3 의 Watch PR 처럼 사람 없이 턴을 시작하는 동작은 Desktop 에 넣지 않는다.
  그 일은 kernel 의 병렬 전달과 이슈 자동화 (plan 28) 가 한다.
- **UF-5 "봄" 표시의 자리:** Desktop 의 값이다. kernel 에 두지 않는다. 다른 client 가 쓰지 않는 개인 표시이다.
- **UF-6 정리는 제안이다:** 머지를 봐도 스스로 정리하지 않는다. 자동 정리는 설정이 생긴 뒤에 다시 본다.
- **UF-7 현황판이 주의함의 화면이다:** 새 보기를 만들지 않는다. 현황판의 첫 열이 "내 차례"이고 거기에서 답한다.
  스레드가 아닌 항목 (task, PR, 저장소) 도 같은 열의 카드로 온다 (plan 23 §5).

## 7. 블루프린트에 넣은 것

`npm run blueprint` 로 본다. 모든 값은 mock 이다.

| 화면 | 바뀐 것 | 파일 |
| --- | --- | --- |
| 사이드바 | 묶기 단추 (프로젝트 / 상태), 주의 묶음, 이유 1 줄 | `blueprint/Sidebar.tsx` |
| 홈 | 기준 브랜치 선택, "이어서 하기" | `blueprint/Views.tsx` |
| 현황판 | 열 이름이 주의 묶음이다. "내 차례" 카드의 허용, 거부, 실패 기록 넘기기, 엿보기 | `blueprint/Views.tsx` |
| 변경 패널 | 전달 띠와 다음 행동, 파일의 "봄" 표시. 옛 단추 줄 (모두 스테이지, 커밋, PR 만들기) 은 띠가 대신한다 | `blueprint/RightPanel.tsx` |
| 위 막대 | 변경 칩의 "다음: …", PR 배지의 "머지됨" | `blueprint/BlueprintApp.tsx` |
| 대화 | "새 내용" 구분선, 변경 카드의 파일 누르기, worktree 준비 줄 | `blueprint/Conversation.tsx`, `blueprint/store.ts` |
| 입력창 | 넘긴 글 받기, 머지 뒤의 정리 제안 | `blueprint/Composer.tsx` |
| 명령과 단축키 | "다음 내 차례로 이동" (`⌘⇧A`), "사이드바 묶기 바꾸기" | `blueprint/Views.tsx` |
| 계산 | 주의 묶음과 이유, 전달 단계와 다음 행동 (순수 함수) | `blueprint/flow.ts`, `test/blueprintFlow.test.ts` |

mock 에서 한 번에 걸어 볼 수 있는 길:

1. 고정된 "사이드바에 스레드 상태 표시"를 연다. 위 막대가 "다음: 검토 0/3" 을 보인다.
2. 변경 칩을 누른다. 파일 세 개에 "봄" 을 켠다. 다음이 "검증 2/3" 으로 바뀐다.
3. "검증 맡기기"를 누른다. 입력창에 남은 기준이 채워진다. 보내면 mock 턴이 돌고 승인을 묻는다.
4. 현황판을 연다. "내 차례" 카드에서 허용을 누른다. 카드가 "작업 중"으로 간다.
5. "plan 19 문서 정리"를 연다. "새 내용" 구분선이 보인다. 파일 세 개에 "봄" 을 켜고 "GitHub 에서 열기"를 누른다.
   mock 은 머지된 것으로 표시한다. 입력창 위에 정리 제안이 뜬다.

## 8. 순서와 검증

| 조각 | 항목 | 먼저 있어야 하는 것 |
| --- | --- | --- |
| V1 전달 띠 | F5-1, F4-1, F4-2 | 없음. 지금 있는 git 과 PR 읽기로 한다. 검증 단계는 뺀다 |
| V2 실패 넘기기 | F5-2, F5-3 | V1 |
| V3 주의 묶음 | F2-1, F2-2, F2-3, F1-3, F3-1, F3-2, F7-1 | 주의함 (#2786). F2-3 과 F7-1 은 지금의 값으로 먼저 할 수 있다 |
| V4 맡기기 | F1-1, F1-2, F6-2 | 격리 공간 API (#2775) |
| V5 정리와 검증 단계 | F6-1, F7-2, F5-4 | 개인 정리 저장 (A02), task 증거 조회 (A01) |
| 나머지 | F2-4 | 조각마다 따로 계획한다 |

- V3 은 plan 26 의 U3 (주의 흐름) 과 같은 조각이다. 이 문서는 그 화면을 정한다. U4 – U6 은 바꾸지 않는다.
- 조각마다 PR 하나이다. 실제 앱의 화면 시험은 가짜 Gateway 와 숨긴 창으로 한다 (`npm run e2e`). 모델을 부르는
  요청을 보내지 않는다 (D-13).
- 블루프린트의 계산은 `desktop/test/blueprintFlow.test.ts` 가 확인한다: 단계의 순서, 실패한 CI 가 먼저인 것, 검증
  단계가 기준이 있을 때만 있는 것, 이유의 우선순위, 다음 내 차례의 순환.
- 블루프린트 화면은 2026-11-26 에 Windows 에서 확인했다. 빌드한 화면을 창 없는 브라우저로 열고 §7 의 길과 홈,
  단축키, 새 worktree 스레드를 눌렀다. 밝은 화면과 어두운 화면을 캡처로 보았다. macOS 는 확인하지 않았다.

## 9. 하지 않는 것, 근거 공백, 출처

하지 않는 것:

- 변경 범위 단추 (마지막 턴, staged, 브랜치). 2026-11-12 의 결정을 지킨다 (R10). "봄" 표시가 "무엇이 새로 바뀌었나"를
  답한다.
- 사람 없이 고치는 동작 (Auto-fix, Watch PR). UF-4.
- 한 프롬프트를 여러 모델에 보내기 (T3, Cursor `/best-of-n`), 창 나누기와 패널 끌어 놓기 (Claude, Antigravity),
  옆 대화 (`/btw`, `/side`), 예약 작업 (Codex, Claude). 흐름의 끊김이 아니다. 필요가 생기면 따로 계획한다.
- 앱 안의 머지 단추. 머지는 GitHub 나 kernel 의 전달이 한다. Desktop 은 "GitHub 에서 열기"만 둔다.

근거 공백:

- 어느 앱도 실행하지 않았다. 순서는 문서의 것이다. 문서와 실제 화면이 다를 수 있다.
- Cursor 의 Agents Window 는 문서에 창 안의 조작이 거의 없다. 사이드바의 상태 표시, 승인 화면, 대기 메시지는 확인하지
  못했다.
- Claude Code Desktop 문서의 뒷부분 (worktree 위치, 자동 보관, 알림) 은 요약으로 읽었다.
- Antigravity 의 "Inbox" 는 지금의 문서에 없다. codelab 은 옛 Agent Manager 를 설명한다.
- Orca 데스크톱의 승인 화면과 Conductor 의 머지 단추, 알림은 문서에 없다.
- F2-3 의 단축키가 다른 OS 단축키와 겹치는지 확인하지 않았다.

출처 (2026-11-26 에 읽음):

- Codex 앱: <https://learn.chatgpt.com/docs/app.md>, <https://learn.chatgpt.com/docs/code-review.md?surface=app>,
  <https://learn.chatgpt.com/docs/environments/git-worktrees.md>, <https://learn.chatgpt.com/docs/reference/commands.md>,
  <https://learn.chatgpt.com/docs/reference/settings.md>, <https://learn.chatgpt.com/docs/automations.md?surface=app>
- Claude Code Desktop: <https://code.claude.com/docs/en/desktop.md>, <https://code.claude.com/docs/en/desktop-quickstart.md>,
  <https://claude.com/blog/claude-code-desktop-redesign>
- Cursor: <https://cursor.com/docs/agent/agents-window>, <https://cursor.com/docs/configuration/worktrees>,
  <https://cursor.com/docs/agent/review>, <https://cursor.com/changelog/3-0>, <https://cursor.com/changelog/05-07-26>
- Conductor: <https://www.conductor.build/docs/workflow>, <https://www.conductor.build/docs/reference/checks>,
  <https://www.conductor.build/docs/reference/diff-viewer>, <https://www.conductor.build/docs/guides/review-and-merge>,
  <https://www.conductor.build/changelog>
- Antigravity: <https://antigravity.google/docs/getting-started>, <https://antigravity.google/docs/review-changes>,
  <https://antigravity.google/docs/artifact-review>, <https://antigravity.google/docs/side-panel>,
  <https://codelabs.developers.google.com/getting-started-google-antigravity>
- Orca: <https://github.com/stablyai/orca>, <https://orca.dev/docs>
- T3 Code: <https://github.com/pingdotgg/t3code>, <https://t3.codes/>
