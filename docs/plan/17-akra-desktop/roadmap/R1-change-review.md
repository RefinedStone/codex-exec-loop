# R1. 변경 검토 (diff)

상태: 완료 · 기준일: 2026-11-06 · 선행: P3 · 규모: 1~2주

## 목표

에이전트가 만든 파일 변경을 앱 안에서 검토한다. 승인 전에 내용을 보고, 턴이 끝난 뒤에 전체 변경을 본다.

## 범위

- 턴 단위 변경 보기: `turn/diff/updated` 알림의 diff 를 표시한다.
- 파일 변경 아이템: `item/fileChange/*` 의 변경 내용을 파일별로 표시한다.
- 파일 변경 승인(`item/fileChange/requestApproval`) 대화상자에 diff 를 함께 표시한다.
- 작업 트리 상태: 스레드 cwd 의 git 상태(변경 파일 목록)를 main 에서 조회해서 표시한다.

## 설계 메모

- diff 표시는 Monaco diff 편집기 또는 가벼운 diff 표시 라이브러리 중에서 고른다. Orca 는 Monaco 를 쓴다.
- git 조회는 main 의 서비스로 둔다. renderer 는 메시지 버스로 요청한다.
- app-server 에 git 관련 기능이 있으면 그것을 먼저 쓴다 (공식 인터페이스 우선).
- 되돌리기 기능은 공개 프로토콜에서 지원하는 범위만 쓴다.

## 미결

- 줄 단위 코멘트를 에이전트에게 다시 보내는 기능(Claude 데스크톱 앱에 있음)을 넣을지 정한다.
- worktree 를 스레드마다 자동으로 만들지 정한다 (하네스의 worktree 슬롯과 관계, R5).

## 진행 기록

### 2026-11-06: diff 표시 (PR 1/2)

- 표시 방식: Monaco 대신 직접 만든 가벼운 unified diff 표시를 쓴다. 의존성을 더하지 않는다. 문법 색과
  좌우 비교는 넣지 않았다. 필요하면 나중에 Monaco 를 검토한다.
- `fileChange` 아이템의 diff 형식 (codex 0.159, 실제 스레드 30개의 변경 1,437개 조회): add/delete 는 파일
  내용 그대로, update 는 `@@` hunk 부터 시작한다 (`---`/`+++` 머리 없음).
- 파일 변경 아이템: 파일마다 접힌 줄로 보여 준다 (종류, 경로, +/- 줄 수). 펼쳤을 때만 줄을 그린다. 1,000줄이
  넘으면 나머지는 버튼으로 연다.
- 턴 요약: 턴 끝에 "이 턴의 변경 · 파일 N개 +a -d" 를 둔다. `turn/diff/updated` (git 형식) 를 받았으면 그것을,
  기록에서 연 이전 턴은 적용된(`completed`) `fileChange` 아이템의 변경을 보여 준다. `turn/diff/updated` 는
  기록에 남지 않는다.
- `item/fileChange/patchUpdated` 로 아이템의 변경 내용을 바꾼다.
- 파일 변경 승인: 요청에 변경 내용이 없으므로 같은 `itemId` 의 `fileChange` 아이템에서 diff 를 찾아 펼쳐 보여
  준다.
- 확인: vitest (diff 해석, 턴 diff, patchUpdated), 실제 스레드에서 펼치기 55ms. 파일 변경 승인 화면은 턴이
  필요해서 실기 확인을 하지 않았다 (D-13).

### 2026-11-06: 작업 트리 상태 (PR 2/2)

- 공식 `gitDiffToRemote` 를 먼저 시험했지만 쓰지 않는다. untracked 파일까지 한 번에 diff 로 만든다. 최근 스레드
  폴더 5개 중 2개에 untracked 파일이 1.3 GB, 3.9 GB 있었고, 그 폴더에서는 응답이 접속 한도(256 MiB)를 넘어
  Gateway 세션 전체가 끊겼다. 다른 열린 스레드의 구독도 함께 끊긴다.
- 그래서 설계 메모대로 main 의 서비스로 둔다 (`desktop/src/main/gitStatus.ts`). renderer 는 IPC
  (`gitStatus`, `gitFileDiff`) 로 요청한다. 모두 읽기 명령이고 셸 없이 실행한다 (Windows 에서 콘솔 창을 띄우지
  않는다).
- 목록: `git status --porcelain=v1 -z --branch --untracked-files=normal` 과 `git diff HEAD --numstat -z`.
  untracked 폴더는 펼치지 않는다. 목록 출력은 4 MB, 명령은 15초까지이다.
- 파일 diff: 펼칠 때 하나씩 읽는다. 추적 중인 파일은 `git diff HEAD -- <경로>`, 새 파일은 내용이다. 1 MB 까지만
  읽고, NUL 바이트가 있으면 바이너리로 표시한다. 경로는 작업 폴더 안의 상대 경로만 받는다.
- 패널은 펼쳤을 때만 읽고, 펼친 동안 턴이 끝나면 다시 읽는다.
- Windows 에서 콘솔 없이 실행한 git 이 `git-lfs` 같은 콘솔 프로그램을 띄우면 그때마다 터미널 창이 뜬다. 그래서
  읽기 명령에는 LFS 필터, fsmonitor, textconv 를 끈다. 대신 시각만 바뀐 LFS 파일은 "수정" 으로 보일 수 있다.
- 경로는 저장소 루트 기준이다. 스레드 cwd 가 하위 폴더여도 루트에서 파일을 찾는다.

### 2026-11-06: 파일 변경 승인 실기 확인

- 사용자 허락을 받고 짧은 턴 4회로 확인했다. 빈 연습 폴더에서 read-only 스레드를 열고 파일 하나를 만들어 달라고 했다.
  앱 입력창에서 보낸 턴의 승인 창에 diff 가 나왔다 (`note.txt`, 추가 1줄). 앱에서 거부하자 다른 접속의 요청도
  닫혔다. 파일은 만들어지지 않았고 스레드는 보관했다.
- 한계: 진행 중인 `fileChange` 아이템은 실시간 `item/started` 로만 온다. `thread/turns/list` 와 `thread/read` 에는
  없고, 승인 요청에도 변경 내용이 없다. 그래서 요청이 온 뒤에 스레드를 연 창(다른 창, 다시 연결한 창)은 diff 를
  받을 수 없다. 그 창에는 요청을 처음 받은 창에서 확인하라고 안내한다.
