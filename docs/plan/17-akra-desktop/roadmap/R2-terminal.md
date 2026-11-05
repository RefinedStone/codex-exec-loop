# R2. 터미널

상태: 구현 완료 (Windows 실기 확인은 사용자 실사용) · 기준일: 2026-11-06 · 선행: P3 · 규모: 약 1주

## 목표

앱 안에서 Project 의 root 를 작업 폴더로 하는 터미널을 연다. 빌드, 테스트, git 명령을 앱을 떠나지 않고
실행한다.

## 범위

- 터미널 표시: xterm.js (WebGL 렌더러)
- 셸 실행: main 프로세스의 node-pty
- 탭이나 분할로 여러 터미널을 연다.
- 터미널의 작업 폴더는 현재 스레드의 cwd 또는 Project root 이다.

## 설계 메모

- 터미널 데이터는 양이 많다. renderer 와 main 사이에 별도 IPC 통로를 쓴다. 대화 메시지 버스와 섞지
  않는다.
- 에이전트의 명령 실행 출력(`item/commandExecution/outputDelta`)은 대화 화면에 표시한다. 사용자의
  터미널과는 별개이다.
- 셸은 사용자 로그인 셸을 쓴다. 에이전트 실행 환경의 보안 설정(D-11 의 upstream 설정)과는 다르다.
- Orca(MIT)의 터미널 구현(xterm + node-pty, 스크롤백 보존)을 참고할 수 있다. 차용하면 고지를 유지한다.

## 미결

- 앱을 다시 열 때 터미널 세션을 복원할지 정한다.
- 에이전트에게 터미널 출력을 넘기는 기능(선택 영역을 프롬프트로 보내기)을 넣을지 정한다.

## 진행 기록

### 2026-11-06: 터미널 탭 (PR 1)

- 의존성: `node-pty` 1.1.0 (main, N-API prebuild 가 Windows·macOS 용으로 들어 있어 Electron 용 재빌드가 없다. Linux 는
  설치할 때 소스로 빌드한다), `@xterm/xterm` 6.0.0, `@xterm/addon-fit` 0.11.0, `@xterm/addon-webgl` 0.19.0.
- main: `desktop/src/main/terminalService.ts`. 셸은 Windows 는 PowerShell 7(`pwsh.exe`)이 PATH 에 있으면 그것을,
  없으면 Windows PowerShell 을 쓴다. macOS·Linux 는 `$SHELL -l`. Windows 는 ConPTY 라 콘솔 창이 뜨지 않는다.
- IPC: 대화 메시지 버스와 다른 통로(`akra:terminal-*`)를 쓴다. 출력은 8ms 씩 모아 보낸다. 다른 창의 터미널에는
  쓰거나 닫지 못한다. 창이 닫히면 그 창의 셸을, 앱이 끝나면 모든 셸을 끝낸다 (Gateway 데몬과 다르다).
- 화면: 입력창 아래의 터미널 영역. "▸ 터미널" 또는 Ctrl+` 로 열고 닫는다. 탭으로 여러 개를 열고, 새 탭은 열린
  스레드의 cwd (없거나 지워졌으면 홈 폴더) 에서 연다. 스레드를 바꿔도 터미널은 그대로 있다. WebGL 로 그리고,
  쓸 수 없으면 DOM 렌더러로 그린다.
- 시험: TerminalService 단위 시험 (출력 묶음, 창 소유권, 창 닫기, 실패, 작업 폴더, 셸 선택). 실제 pty 와 화면
  시험(`test/e2e/terminal.mjs`)은 CI(Linux) 에서만 돈다. Windows 에서 실제 셸을 띄우면 사용자 화면에 창이 뜰 수
  있어서 로컬에서는 건너뛴다. 그래서 Windows 동작은 실사용으로 확인한다.
- 미결 그대로: 세션 복원, 터미널 출력을 에이전트에게 보내기, 분할 화면.
