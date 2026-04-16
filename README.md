# codex-exec-loop

`codex-exec-loop`는 `codex app-server` 위에서 동작하는 Rust 기반 TUI 클라이언트입니다.
목표는 Codex를 "한 번 실행하고 계속 이어서 쓰기" 좋게 만드는 것입니다. 새 드래프트 시작, 최근 세션 재개, 자동 후속 작업, 플래닝 파일 관리까지 한 터미널 흐름 안에서 처리합니다.

## 어떤 프로젝트인가

- 기본 프론트엔드는 inline shell입니다. 대화 기록은 터미널 scrollback에 쌓이고, 하단에는 현재 입력과 실시간 스트림만 남습니다.
- 시작하자마자 startup diagnostics를 돌려서 `codex` 실행 가능 여부, workspace 상태, app-server 초기화, 로그인 상태를 확인합니다.
- 최근 세션 브라우저가 있어서 예전 작업을 검색하고 바로 이어서 열 수 있습니다.
- auto follow-up 기능으로 "다음 작업 1개 더 진행", "문서 보강", "bugfix" 같은 후속 프롬프트를 자동으로 이어갈 수 있습니다.
- `.codex-exec-loop/planning/` 아래 planning 파일을 기준으로 queue-driven 작업 흐름을 만들 수 있습니다.
- 필요하면 특정 GitHub PR 상태를 폴링해서 셸 안에 함께 표시할 수 있습니다.

## 설치 전 준비

- `codex` CLI가 설치되어 있고 `PATH`에 잡혀 있어야 합니다.
- `codex login`이 끝나 있어야 합니다.
- 이 도구를 실행할 workspace 디렉터리가 있어야 합니다.
- 소스에서 빌드해 실행할 경우 Rust toolchain이 필요합니다.

## 설치와 실행

### 1. npm으로 설치

Codex CLI처럼 npm 전역 설치로 관리할 수 있습니다.
설치 후에는 현재 폴더에 실행 파일이 없어도 어느 workspace에서나 `akra`를 실행할 수 있습니다.

```bash
npm install -g @refinedstone/akra

cd /path/to/your/workspace
akra
```

업데이트와 제거도 npm으로 처리합니다.

```bash
npm update -g @refinedstone/akra
npm uninstall -g @refinedstone/akra
```

현재 npm 배포는 아래 플랫폼을 지원합니다.

- Linux `x64`
- macOS Apple Silicon `arm64`
- Windows `x64`

### 2. 소스에서 실행

가장 무난한 방법은 한 번 release 빌드를 한 뒤, 실제 작업할 workspace에서 바이너리를 실행하는 것입니다.

```bash
git clone https://github.com/RefinedStone/codex-exec-loop.git
cd /path/to/codex-exec-loop
. "$HOME/.cargo/env"
cargo build --release

cd /path/to/your/workspace
/path/to/codex-exec-loop/target/release/codex-exec-loop-native
```

어느 폴더에서나 `akra` 명령으로 실행하고 싶다면 source install도 가능합니다.
`~/.cargo/bin` 이 `PATH`에 잡혀 있다면 아래 한 번으로 끝납니다.

```bash
git clone https://github.com/RefinedStone/codex-exec-loop.git
cd /path/to/codex-exec-loop
. "$HOME/.cargo/env"
cargo install --path . --bin akra --locked

cd /path/to/your/workspace
akra
```

지금 이 저장소 자체를 대상으로 바로 써보고 싶다면 아래처럼 실행해도 됩니다.

```bash
cd /path/to/codex-exec-loop
. "$HOME/.cargo/env"
cargo run
```

현재 작업 디렉터리가 곧 workspace로 인식됩니다.

### 3. 패키징된 바이너리로 실행

배포된 native bundle을 받았다면 Rust 없이 바로 실행할 수 있습니다.
bundle 안에는 `akra` launcher도 함께 들어 있으므로, 압축을 푼 디렉터리를 `PATH`에 추가하면 어느 workspace에서나 `akra`로 실행할 수 있습니다.

태그 기반 자동 배포를 쓰는 경우, GitHub 저장소의 `Releases` 페이지에서 플랫폼별 asset을 바로 받을 수 있습니다.
아무 tag나 push하면 GitHub Actions가 해당 tag 이름으로 release를 만들고 asset을 업로드합니다.
`NPM_TOKEN` secret이 설정되어 있으면 같은 tag 버전으로 npm 패키지 `@refinedstone/akra`도 함께 publish 합니다.
npm 버전은 immutable 이므로, 이미 publish 된 버전은 같은 번호로 다시 덮어쓸 수 없습니다.
asset 파일명 안의 버전 문자열은 현재 `Cargo.toml` 패키지 버전을 그대로 사용합니다.

macOS/Linux:

```bash
export PATH="/path/to/codex-exec-loop-native-<version>-<target>:$PATH"
cd /path/to/your/workspace
akra
```

Windows PowerShell:

```powershell
$env:Path = "C:\path\to\codex-exec-loop-native-<version>-<target>;$env:Path"
Set-Location C:\path\to\workspace
akra
```

## 첫 실행 흐름

1. 프로그램이 열리면 startup diagnostics가 먼저 돌아갑니다.
2. diagnostics가 끝나기 전에도 프롬프트 입력은 가능합니다.
3. 준비가 끝난 뒤 `Enter`를 누르면 전송되고, turn 스트림이 셸 하단에 바로 보입니다.
4. 기존 세션을 이어가고 싶으면 `Ctrl+o` 또는 `:sessions`를 엽니다.
5. queue 기반으로 계속 일시키고 싶으면 `:planning`으로 planning을 먼저 준비한 뒤 auto follow-up을 켭니다.

startup이 막혔을 때는 `Ctrl+d` 또는 `:diag`로 상태를 확인하면 됩니다.

## 기본 사용법

### 전역 단축키

| 키 | 동작 |
| --- | --- |
| `Enter` | 현재 프롬프트 전송 |
| `Ctrl+j` | 줄바꿈 입력 |
| `Ctrl+u` | 입력창 전체 비우기 |
| `Ctrl+w` | 이전 단어 삭제 |
| `Ctrl+t` | 새 draft 열기 |
| `Ctrl+c` | 뒤로 가기 또는 현재 overlay 닫기 |
| `Ctrl+q` | 앱 종료 |
| `Ctrl+d` | diagnostics overlay 열기/닫기 |
| `Ctrl+o` | recent sessions overlay 열기/닫기 |
| `Ctrl+f` | automation controls overlay 열기/닫기 |
| `Ctrl+r` | startup checks 다시 실행 |

`Ctrl+c`는 일반적인 쉘 종료 키가 아니라 이 앱 안에서는 "취소/뒤로 가기"에 가깝습니다. 실제 종료는 `Ctrl+q`입니다.

### 셸 명령

| 명령 | 설명 |
| --- | --- |
| `:diag` | startup diagnostics 열기 |
| `:sessions` | 최근 세션 브라우저 열기 |
| `:auto` | automation controls 열기 |
| `:queue` | planning queue inspection 열기 |
| `:directions` | directions 유지보수 overlay 열기 |
| `:stop` | post-turn automation 끄기 |
| `:planning` | planning 초기화/편집 흐름 열기 |
| `:turns <1-50>` | 최대 auto follow-up 횟수 설정 |
| `:new` | 새 draft 열기 |
| `:help` | 사용 가능한 셸 명령 표시 |

지원 alias도 있습니다. 예를 들어 `:diagnostics`, `:session`, `:automation`, `:q`, `:planning-init`, `:auto-turns 10`도 동작합니다. `:directions`와 `:stop`은 단축 alias 없이 그대로 사용합니다.

### 프론트엔드 모드

- 프론트엔드는 이제 `inline` 한 가지입니다.
- 터미널 자체 scrollback이 기록 뷰 역할을 합니다.
- inspection overlay를 닫으면 곧바로 원래 입력 화면으로 돌아갑니다.

## recent sessions 사용법

세션 목록은 `Ctrl+o` 또는 `:sessions`로 엽니다.

| 키 | 동작 |
| --- | --- |
| `/` | 검색어 입력 시작 |
| `c` | 검색어와 필터 초기화 |
| `Tab` / `BackTab` | workspace 필터 전환 |
| `[` / `]` | 페이지 이동 |
| `PageUp` / `PageDown` | 페이지 이동 |
| `Up` / `Down` | 항목 이동 |
| `Home` / `End` | 첫 항목/마지막 항목으로 이동 |
| `g` / `G` | 첫 항목/마지막 항목으로 이동 |
| `Enter` | 선택한 세션 열기 |
| `n` | 새 draft 열기 |
| `r` | 목록 다시 불러오기 |
| `Ctrl+d` | diagnostics로 이동 |
| `Esc` / `Ctrl+c` | 닫기 |

검색은 공백 기준 토큰 매칭입니다. diagnostics가 통과하기 전에는 session list가 잠겨 있을 수 있습니다.

## auto follow-up 사용법

auto follow-up은 한 turn이 끝난 직후 planning queue를 기준으로 다음 작업을 자동으로 이어가는 기능입니다.

전역 단축키:

| 키 | 동작 |
| --- | --- |
| `Ctrl+a` | post-turn automation on/off |
| `Ctrl+f` | automation controls 열기/닫기 |
| `Ctrl+g` | stop keyword 편집 |
| `Ctrl+k` | stop keyword 규칙 on/off |
| `Ctrl+n` | "파일 변경 없음" 중지 규칙 on/off |
| `Ctrl+l` | 최대 auto turn 수 편집 |
| `Ctrl+b` | planner detail 표시 전환 |

현재 동작 규칙:

- 기본 stop keyword는 `AUTO_STOP`입니다.
- stop keyword 매칭은 대소문자를 구분하지 않고 token 단위로 처리됩니다.
- "파일 변경 없음" 규칙을 켜면, 완료된 turn에서 파일 변화가 없을 때 자동 후속을 멈춥니다.
- automation preview와 상태 패널은 항상 현재 planning queue 기준으로만 렌더링됩니다.

## planning 사용법

planning은 "다음 작업 queue를 파일로 관리하면서 계속 이어서 실행"하고 싶을 때 쓰는 기능입니다.

진입:

- `:planning`

planning이 만들어 두는 핵심 파일:

| 파일 | 역할 |
| --- | --- |
| `.codex-exec-loop/planning/directions.toml` | 작업 방향, detail doc 매핑, queue idle 정책 정의 |
| `.codex-exec-loop/planning/task-ledger.json` | 실제 작업 queue와 task 상태 |
| `.codex-exec-loop/planning/task-ledger.schema.json` | task-ledger 검증 스키마 |
| `.codex-exec-loop/planning/result-output.md` | 결과 출력 가이드 조각 |
| `.codex-exec-loop/planning/queue.snapshot.json` | runtime이 계산한 queue 스냅샷 |
| `.codex-exec-loop/planning/directions/<direction-id>.md` | direction별 상세 문서 |
| `.codex-exec-loop/planning/prompts/queue-idle-review.md` | queue idle review prompt |

기본 흐름:

1. `:planning`으로 진입합니다.
2. `simple mode` 또는 `detail mode`를 고릅니다.
3. 먼저 staged draft가 만들어집니다.
4. 검토 후 promote해야만 active planning 파일로 반영됩니다.

### planning 모드별 차이

- `simple mode`
  - 가장 빠른 시작 경로입니다.
  - generic direction 1개, 빈 task ledger, 기본 queue-idle review prompt를 staged draft로 만듭니다.
  - 기본 `queue_idle.policy = review_and_enqueue`로 시작하므로, 첫 답변 뒤 justified follow-up queue review가 바로 가능하게 설정됩니다.
  - `Enter` 또는 `Ctrl+P`로 promote합니다.
- `detail mode -> manual`
  - 셸 안의 draft editor로 들어가 planning 파일을 직접 편집합니다.
  - `llm-assisted` 항목은 UI에 보이지만 아직 지원되지 않습니다.

### `:directions` 유지보수

`directions.toml`은 계속 operator-owned 파일이지만, 셸 안에서 안전하게 관리할 수 있도록 `:directions` overlay가 추가되었습니다.

- `Enter`: staged directions editor 열기
- `d`: 빈 `detail_doc_path`가 있는 direction에 대해 `.codex-exec-loop/planning/directions/<direction-id>.md` 생성 흐름 열기
- `p`: queue-idle prompt markdown을 생성하거나 편집하기
- 변경은 모두 staged draft에서 이뤄지고, `Ctrl+P`로 promote하기 전까지 active planning에는 반영되지 않습니다.

`directions.toml`의 새 계약:

- 각 `[[directions]]`는 `detail_doc_path = ""` 또는 planning 하위 `.md` 경로를 가질 수 있습니다.
- `[queue_idle] policy`는 `stop` 또는 `review_and_enqueue`입니다.
- `review_and_enqueue`를 쓰면 `prompt_path`와 실제 markdown 파일이 필요합니다.

### planning 화면 단축키

초기 선택 화면:

| 키 | 동작 |
| --- | --- |
| `A` / `B` | simple/detail 선택 |
| `Up` / `Down` 또는 `j` / `k` | 선택 이동 |
| `Enter` | 선택 확정 |
| `Backspace` / `Left` | detail 하위 선택에서 뒤로 |
| `Esc` / `Ctrl+c` | 닫기 |

simple review 화면:

| 키 | 동작 |
| --- | --- |
| `Enter` / `Ctrl+P` | staged scaffold promote |
| `Ctrl+L` | max auto turns 수정 |
| `Ctrl+E` | staged draft 열어 직접 편집 |
| `Esc` / `Ctrl+c` | review 닫기 |

manual editor 화면:

| 키 | 동작 |
| --- | --- |
| `Tab` / `Shift+Tab` | 파일 전환 |
| `Arrow Keys` | 커서 이동 |
| `Enter` | 줄바꿈 |
| `Backspace` | 문자 삭제 |
| `Ctrl+w` | 이전 단어 삭제 |
| `Ctrl+S` | 저장 + 검증 |
| `Ctrl+P` | 저장 + active planning으로 promote |
| `Esc` / `Ctrl+c` | 닫기 |

`builtin next-task` 템플릿은 planning 상태를 사용합니다. queue head가 준비돼 있으면 그 작업을 이어가고, 바로 실행할 task가 없으면 `queue_idle.policy`를 따릅니다.

- `stop`: 현재 turn 이후 automation을 종료합니다.
- `review_and_enqueue`: hidden queue-manager planner가 directions, detail docs, accepted work list를 다시 보고 justified follow-up만 queue에 추가합니다.

현재 queue 상태를 빠르게 확인하려면 `:queue` 또는 `:q`를 사용하세요. 이 화면은 raw planning JSON 전체를 덤프하지 않고, 다음 실행 후보와 proposal, skipped 요약만 사람이 읽기 쉬운 형태로 보여줍니다. direction/queue-idle 정책을 관리하려면 `:directions`를 사용하세요.

## 환경 변수

자주 쓰는 환경 변수:

| 변수 | 설명 |
| --- | --- |
| `CODEX_EXEC_LOOP_GITHUB_PR=owner/repo#123` | 특정 PR 상태 폴링 |
| `CODEX_EXEC_LOOP_GITHUB_POLL_INTERVAL_SECS=60` | GitHub 폴링 간격 |

고급 app-server 정책 override가 필요하면 다음 변수도 사용할 수 있습니다.

- `CODEX_EXEC_LOOP_APP_SERVER_APPROVAL_POLICY`
- `CODEX_EXEC_LOOP_APP_SERVER_APPROVALS_REVIEWER`
- `CODEX_EXEC_LOOP_APP_SERVER_SANDBOX_MODE`

## 자주 겪는 문제

### `codex`를 찾지 못합니다

- `codex` CLI가 설치되어 있는지 확인하세요.
- `which codex` 또는 `command -v codex`로 경로를 확인하세요.

### startup diagnostics에서 account/app-server가 막힙니다

- `codex login` 상태를 다시 확인하세요.
- 앱 안에서 `Ctrl+r`로 startup checks를 다시 실행하세요.
- 상세 메시지는 `Ctrl+d` 또는 `:diag`에서 볼 수 있습니다.

### recent sessions가 비활성화되어 있습니다

- session browser는 startup diagnostics가 통과해야 열립니다.
- account 또는 initialize 상태가 막혀 있으면 먼저 그 문제를 해결해야 합니다.

### approval 요청은 보이는데 approve/deny를 못 합니다

- 현재 버전은 approval 상태 표시는 지원하지만, TUI 안에서 interactive approve/deny 동작은 아직 완전히 연결되어 있지 않습니다.

### planning의 `llm-assisted`가 선택은 되는데 진행이 안 됩니다

- 현재 UI에만 보이고 실제 기능은 비활성화되어 있습니다.
- 지금은 `detail mode -> manual`을 사용해야 합니다.

## 예시 파일

- [examples/initial_prompt.txt](./examples/initial_prompt.txt)

## 추가 문서

사용자 입장에서는 이 README만으로 시작해도 되지만, 더 자세한 현재 동작은 아래 문서를 보면 됩니다.

- [docs/design/01-current-product-state.md](./docs/design/01-current-product-state.md)
- [docs/design/02-tui-shell-flow.md](./docs/design/02-tui-shell-flow.md)
- [docs/design/06-planning-runtime-and-draft-editor.md](./docs/design/06-planning-runtime-and-draft-editor.md)
- [docs/design/04-hexagonal-runtime-architecture.md](./docs/design/04-hexagonal-runtime-architecture.md)
- [docs/plan/13-native-packaging-and-operator-runbook.md](./docs/plan/13-native-packaging-and-operator-runbook.md)
