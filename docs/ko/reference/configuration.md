# 설정 참조

[English](../../reference/configuration.md)

Akra의 설정 표면은 의도적으로 작고 secret-free입니다. 운영자에게 보이는 기본값만 설정하며 repository가 임의의 Codex 또는 app-server 설정을 전달하게 만들지 않습니다.

## 파일과 우선순위

| 범위 | 경로 | 생성 시점 |
| --- | --- | --- |
| 전역 | AKRA_HOME/config.toml (기본: ~/.akra/config.toml) | Akra를 정상 시작할 때 처음 한 번 canonical 기본값으로 생성합니다. |
| 프로젝트 | Git worktree/.akra/config.toml | akra config set --project만 생성합니다. |

Linked Git worktree마다 각자의 project 파일을 사용합니다. Git이 아닌 디렉터리에는 project 범위가 없고 전역 설정만 적용됩니다. --help, akra config path, list, get, doctor는 전역 파일을 만들지 않습니다.

유효 leaf의 우선순위는 높은 쪽부터 아래처럼 고정됩니다.

    -c/--config > 환경변수 > 프로젝트 TOML > 호환 repository Git config > 전역 TOML > 내장값

TOML table은 통째로 바꾸지 않고 leaf별로 병합합니다. unset은 해당 leaf를 다음 하위 계층으로 되돌릴 뿐 table 전체를 지우지 않습니다. schema_version이 없으면 v1로 해석하고 writer는 항상 schema_version = 1을 기록합니다.

## 허용 TOML

첫 전역 파일에는 아래 canonical 내장값이 기록됩니다. 마지막 열이 예인 항목만 project 파일에 둘 수 있습니다.

| 그룹 | 키와 내장값 | 프로젝트 |
| --- | --- | --- |
| conversation | model = "gpt-5.6-sol"; reasoning_effort = "medium" | 예 |
| tui | show_startup_visual = true; planning_worker_visibility = "normal" | 예 |
| github | auto_discover_pull_request = true; review_poll_interval_secs = 60; push_remote = "origin"; pull_request_mode = "required" | 예 |
| parallel | integration_branch = "prerelease" | 예 |
| app_server | response_timeout_secs = 15; prompt_log = false | timeout만 |
| subprocess | timeout_secs = 30 | 예 |
| diagnostics | trace = "off"; spans = "none"; max_files = 7; max_file_bytes = 16777216; max_total_bytes = 67108864; tokio_console = false | 아니요 |
| admin | graphic_enabled = true; graphic_poll_interval_ms = 10000 | 예 |

conversation.model = "default", conversation.reasoning_effort = "default"는 app-server 기본값을 명시적으로 요청합니다. 상속을 다시 받는 unset과 다릅니다. custom model ID는 길이가 제한되고 공백이 없는 문자열로 허용됩니다. 선택기는 이를 catalog 모델로 바꾸지 않고 임시 Configured: <id> 항목으로 보존합니다. Sol의 추천 effort는 medium, Terra와 Luna는 max입니다.

문법 오류, 알 수 없는 키, 타입/값 오류, 지원하지 않는 schema, project 금지 키는 정상 시작 전에 거부합니다. enum은 소문자이며 worker visibility는 normal|debug, PR mode는 required|auto|disabled, trace는 off|on|planning|full(또는 유효한 tracing filter), spans는 none|close|full입니다. 기존 환경변수 호환을 위해 trace의 1, off 같은 별칭도 계속 허용합니다.

## CLI

    akra config list [--global|--project]
    akra config get <key> [--global|--project]
    akra config set [--global|--project] <key> <value>
    akra config unset [--global|--project] <key>
    akra config path [--global|--project]
    akra config doctor
    akra [-c key=value ...] [command ...]

범위를 생략한 list와 get은 유효값과 origin을 표시합니다. set, unset의 기본 범위는 전역입니다. doctor는 두 파일, 환경변수 shadowing, 호환 Git config, 경로/보안 오류를 고치지 않고 진단만 합니다.

-c/--config은 여러 번 쓸 수 있고 그 프로세스에만 적용됩니다.

    akra -c conversation.model=gpt-5.6-terra -c conversation.reasoning_effort=max

## 환경변수와 호환성

환경변수는 제거할 때까지 TOML보다 우선합니다. resolver는 이긴 변수 이름을 list, get, doctor에 origin으로 남깁니다.

| 설정 그룹 | 계속 허용하는 기존 환경변수 |
| --- | --- |
| TUI | CODEX_EXEC_LOOP_SHOW_STARTUP_VISUAL; legacy CODEX_EXEC_LOOP_SHOW_STARTUP_ASCII_ART; CODEX_EXEC_LOOP_PLANNING_WORKER_VISIBILITY; legacy CODEX_EXEC_LOOP_PLANNER_VISIBILITY |
| GitHub | CODEX_EXEC_LOOP_GITHUB_POLL_INTERVAL_SECS; AKRA_GITHUB_PUSH_REMOTE; AKRA_GITHUB_PR_MODE |
| Parallel | AKRA_PARALLEL_INTEGRATION_BRANCH |
| App-server | CODEX_EXEC_LOOP_APP_SERVER_RESPONSE_TIMEOUT_SECS; AKRA_APP_SERVER_PROMPT_LOG |
| Subprocess | CODEX_EXEC_LOOP_SUBPROCESS_TIMEOUT_SECS |
| Diagnostics | AKRA_TRACE; AKRA_TRACE_SPANS; AKRA_TRACE_MAX_FILES; AKRA_TRACE_MAX_FILE_BYTES; AKRA_TRACE_MAX_TOTAL_BYTES; AKRA_TOKIO_CONSOLE |
| Admin | AKRA_ADMIN_GRAPHIC_ENABLED; AKRA_ADMIN_GRAPHIC_POLL_MS |

CODEX_EXEC_LOOP_GITHUB_PR, RUST_LOG, AKRA_TRACE_FILE은 기존의 특수 계약을 유지하며 TOML 키가 아닌 diagnostic shadowing으로 표시됩니다. 호환 repository Git 값은 project TOML보다 낮게 유지합니다: akra.githubPushRemote, akra.githubPrMode, akra.parallelIntegrationBranch.

## 영속성, 대화, 보안

전역 파일은 owner-only이고 전역 디렉터리는 current user 소유이며 group/world writable이면 안 됩니다. project 파일은 review 가능한 일반 worktree 파일입니다. 두 범위 모두 symlink, hardlink/reparse point, regular file이 아닌 객체, 안전하지 않은 전역 소유권/권한, 크기 초과, open 중 객체 교체를 거부합니다. 쓰기는 AKRA_HOME 아래 path-hash interprocess lock에서 다시 읽고 toml_edit로 comment를 보존하며 atomic replace합니다.

:model, :model default, :think는 현재 메모리 선택을 즉시 바꾸고 다음 새 thread의 전역 기본값과 활성 thread의 model/reasoning 행을 저장 큐에 넣습니다. Core는 빠른 연속 선택도 요청 순서대로 직렬 저장합니다. 전역 또는 thread 저장이 실패해도 live 선택은 되돌리지 않고 각각의 실패를 표시합니다. 새 thread는 실제 제출 options를 저장하고 저장된 Akra thread는 열 때 복원합니다. 저장 행이 없는 기존/외부 thread는 app-server에 None/None을 보내 app-server 기본값을 유지하므로 나중의 전역 변경이 소급 적용되지 않습니다.

Hidden planning/parallel worker는 명시적 모델 정책을 유지하며 interactive conversation 기본값을 상속하지 않습니다.

## 의도적인 제외와 Codex 차이

Akra TOML에는 AKRA_HOME, credential/token, allowlist, app-server process/shell environment, API-key forwarding, approval/reviewer/sandbox 정책, public-repository/autonomous-delivery opt-in을 넣지 않습니다. Telegram은 private telegram.env, parallel agent profile은 .akra/parallel-agent-profiles.json의 별도 계약을 유지합니다.

Codex는 더 넓은 user/trusted-project 설정, profile, system 설정, project trust gating을 지원합니다. 공식 [Config basics](https://learn.chatgpt.com/docs/config-file/config-basic)와 [Advanced config](https://learn.chatgpt.com/docs/config-file/config-advanced)를 참고하세요. Akra v1에는 profile/system 계층이나 project trust registry가 없습니다. Git worktree당 하나의 project 파일과 짧은 allowlist만 사용하므로 repository가 credential, 보안 정책, process 실행 동작을 바꿀 수 없습니다.
