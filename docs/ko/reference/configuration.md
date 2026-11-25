# 설정 참조

[English](../../reference/configuration.md)

Akra의 설정 표면은 의도적으로 작고 secret-free입니다. 운영자에게 보이는 기본값만 설정하며 repository가 임의의 Codex 또는 app-server 설정을 전달하게 만들지 않습니다.

## 파일과 우선순위

| 범위 | 경로 | 생성 시점 |
| --- | --- | --- |
| 전역 | AKRA_HOME/config.toml (기본: ~/.akra/config.toml) | Akra를 정상 시작할 때 처음 한 번 canonical 기본값으로 생성합니다. |
| 프로젝트 | Git worktree/.akra/config.toml | akra config set --project만 생성합니다. |

Linked Git worktree마다 각자의 project 파일을 사용합니다. Git이 아닌 디렉터리에는 project 범위가 없고 전역 설정만 적용됩니다. --help, akra config path, list, get, doctor는 전역 파일을 만들지 않습니다.

명령이 다른 workspace를 명시하면 Akra는 그 대상의 project TOML과 호환 Git 계층을 다시 해석합니다. 전역·환경변수·`-c/--config` 입력은 프로세스 시작 시점의 snapshot을 그대로 유지합니다.

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
| parallel | integration_branch = "prerelease"; evidence_mode = "observe" | 예 |
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

## Task 전달 증거

`parallel.evidence_mode`는 정확히 `observe` 또는 `enforce`를 받습니다. 기본 `observe`는
전달 판정을 기록하고 증거 부족만으로 기존 전달을 막지 않습니다. `enforce`는 현재 호스트
검증이나 정확한 Human 기준 면제가 있어야 전달합니다. 대표 checkout에서
`akra config set --project parallel.evidence_mode enforce`로 설정합니다.
유효 정책과 설정 오류는 repository identity별로 프로세스 안에 고정합니다. 서비스 재생성이나
worker·integration worktree로의 이동으로 재해석하지 않으며, 변경은 다음 repository
kernel/process 시작에 반영합니다. 읽기·형식 오류를 `observe`로 바꾸지 않습니다.
`git_diff_check_v1` 성공의 제한을 포함한 [전달 증거 계약](../../reference/task-delivery-evidence.md)을 참고하세요.

## 환경변수와 호환성

환경변수는 제거할 때까지 TOML보다 우선합니다. resolver는 이긴 변수 이름을 list, get, doctor에 origin으로 남깁니다.

| 설정 그룹 | 계속 허용하는 기존 환경변수 |
| --- | --- |
| TUI | AKRA_SHOW_STARTUP_VISUAL; AKRA_PLANNING_WORKER_VISIBILITY |
| GitHub | AKRA_GITHUB_POLL_INTERVAL_SECS; AKRA_GITHUB_PUSH_REMOTE; AKRA_GITHUB_PR_MODE |
| Parallel | AKRA_PARALLEL_INTEGRATION_BRANCH |
| App-server | AKRA_APP_SERVER_RESPONSE_TIMEOUT_SECS; AKRA_APP_SERVER_PROMPT_LOG |
| Subprocess | AKRA_SUBPROCESS_TIMEOUT_SECS |
| Diagnostics | AKRA_TRACE; AKRA_TRACE_SPANS; AKRA_TRACE_MAX_FILES; AKRA_TRACE_MAX_FILE_BYTES; AKRA_TRACE_MAX_TOTAL_BYTES; AKRA_TOKIO_CONSOLE |
| Admin | AKRA_ADMIN_GRAPHIC_ENABLED; AKRA_ADMIN_GRAPHIC_POLL_MS |

AKRA_GITHUB_PR, RUST_LOG, AKRA_TRACE_FILE은 기존의 특수 계약을 유지하며 TOML 키가 아닌 diagnostic shadowing으로 표시됩니다. 이 변수들의 이전 `CODEX_EXEC_LOOP_*` 이름은 제거했습니다(#2588). 이전 이름이 남아 있으면 모든 Akra 바이너리가 시작을 멈추고 대신 쓸 `AKRA_*` 이름을 알려 줍니다. 조용히 무시하지 않으므로 권한을 낮춘 sandbox 설정이 기본값으로 돌아가지 않습니다. 호환 repository Git 값은 project TOML보다 낮게 유지합니다: akra.githubPushRemote, akra.githubPrMode, akra.parallelIntegrationBranch.

## 영속성, 대화, 보안

전역 파일은 owner-only이고 전역 디렉터리는 current user 소유이며 group/world writable이면 안 됩니다. project 파일은 review 가능한 일반 worktree 파일입니다. 두 범위 모두 symlink, hardlink/reparse point, regular file이 아닌 객체, 안전하지 않은 전역 소유권/권한, 크기 초과, open 중 객체 교체를 거부합니다. 쓰기는 AKRA_HOME 아래 path-hash OS-backed interprocess 파일 잠금에서 다시 읽고 toml_edit로 comment를 보존하며 atomic replace합니다. 잠금 파일은 남아도 writer가 비정상 종료하면 운영체제가 배타 잠금을 해제합니다.

:model, :model default, :think는 현재 메모리 선택을 즉시 바꾸고 다음 새 thread의 전역 기본값과 활성 thread의 model/reasoning 행을 저장 큐에 넣습니다. Core는 빠른 연속 선택도 요청 순서대로 직렬 저장합니다. 전역 또는 thread 저장이 실패해도 live 선택은 되돌리지 않고 각각의 실패를 표시합니다. 새 thread는 실제 제출 options를 저장하고 저장된 Akra thread는 열 때 복원합니다. 저장 행이 없는 기존/외부 thread는 app-server에 None/None을 보내 app-server 기본값을 유지하므로 나중의 전역 변경이 소급 적용되지 않습니다.

Hidden planning/parallel worker는 명시적 모델 정책을 유지하며 interactive conversation 기본값을 상속하지 않습니다.

## 이슈 정책 파일

이슈 규약과 공개 정책(#2416)은 config.toml이 아닌 별도 파일에 둡니다. `IssuePolicyService`가 파일을 읽고 검증한 뒤 타입이 있는 `IssuePolicy`를 돌려줍니다. 아직 이 값을 쓰는 명령이나 클라이언트는 없습니다. 이 값을 쓰는 검증기, 마스킹, 게시 경로는 제안 단계의 작업입니다(#2417 ~ #2419).

| Scope | 경로 |
| --- | --- |
| 사용자 | AKRA_HOME/issue-policy.toml (기본값: ~/.akra/issue-policy.toml) |
| 프로젝트 | workspace/.akra/issue-policy.toml |

값 하나마다 높은 순서에서 낮은 순서로 프로젝트 파일 > 사용자 파일 > 기본값을 적용합니다. table은 값 단위로 합칩니다. `required_sections`는 값 하나이므로 통째로 바뀝니다. schema_version이 없으면 1입니다. 스키마: [`schema/akra-issue-policy.schema.json`](../../../schema/akra-issue-policy.schema.json). 예제: [`schema/akra-issue-policy.example.toml`](../../../schema/akra-issue-policy.example.toml).

| Key | 값 | 기본값 |
| --- | --- | --- |
| convention.template | 저장소 안의 상대 경로, `..` 없음 | 없음 |
| convention.required_sections | 섹션 제목 목록 | 빈 목록 |
| publish.spans.secret, name, path, log | remove, restorable_mask, user_confirm | secret = remove, 나머지 = restorable_mask |
| ai_handoff.spans.secret, name, path, log | remove, restorable_mask, user_confirm | secret = remove, 나머지 = restorable_mask |
| publish.required_validation | true만 허용 | true |
| vault.user_unlock | true만 허용 | true |

`publish.spans`는 글을 공급자에 게시할 때 적용합니다. `ai_handoff.spans`는 글을 AI에 전달할 때 적용합니다. 두 table은 서로 독립입니다. `remove`는 값을 버리고, `restorable_mask`는 사용자가 Vault에서 복원할 수 있는 표식으로 바꾸고, `user_confirm`은 구간마다 사용자에게 묻습니다.

필수 게시 검증과 Vault 사용자 잠금 해제는 항상 켜져 있습니다. 어느 scope에서도 끌 수 없습니다. `publish.required_validation = false`나 `vault.user_unlock = false`가 있으면 그 파일은 잘못된 파일입니다. 파일을 읽을 수 없거나, 형식이 틀리거나, 모르는 key·종류·처리가 있거나, schema_version을 지원하지 않거나, template 경로가 저장소 밖이면 잘못된 파일의 scope와 함께 `IssuePolicyBlocked`를 돌려줍니다. 호출자는 그 프로젝트의 게시와 AI 전달을 막아야 합니다. 기본 정책으로 대신하지 않습니다. 두 파일은 config.toml과 같은 읽기 안전 규칙을 씁니다.

## 이슈 자동화 정책 연결

별도 `issue-automation.toml`은 수집 대상, 자율 수준 상한, 위험 규칙, 관문, 예산과 외부 피드백 권한을
선언합니다. 수집기·결정적 판정기·propose 조정기는 병합한 정책을 사용하며, 작업에는 현재 결정과 단계별
허가가 필요합니다. IA4-1 구현은 실행 admission에서 `budget.max_attempts`도 검사합니다.
외부 피드백 쓰기는 #2552의 후속 범위입니다. 설정의 전체 계약은
[정본](../../reference/configuration.md#issue-automation-policy-file), 구현 PR과 미병합 부모 체인의
상태는 [plan 28](../../plan/28-akra-issue-automation.md#ia4-조각-상태)을 따릅니다.

## 의도적인 제외와 Codex 차이

Akra TOML에는 AKRA_HOME, credential/token, allowlist, app-server process/shell environment, API-key forwarding, approval/reviewer/sandbox 정책, public-repository/autonomous-delivery opt-in을 넣지 않습니다. Telegram은 private telegram.env, parallel agent profile은 .akra/parallel-agent-profiles.json의 별도 계약을 유지합니다.

Codex는 더 넓은 user/trusted-project 설정, profile, system 설정, project trust gating을 지원합니다. 공식 [Config basics](https://learn.chatgpt.com/docs/config-file/config-basic)와 [Advanced config](https://learn.chatgpt.com/docs/config-file/config-advanced)를 참고하세요. Akra v1에는 profile/system 계층이나 project trust registry가 없습니다. Git worktree당 하나의 project 파일과 짧은 allowlist만 사용하므로 repository가 credential, 보안 정책, process 실행 동작을 바꿀 수 없습니다.
