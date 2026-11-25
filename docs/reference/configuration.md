# Configuration Reference

[한국어](../ko/reference/configuration.md)

Akra has one deliberately small, secret-free configuration surface. It configures operator-facing defaults; it does not pass arbitrary Codex or app-server configuration through a repository.

## Files and precedence

| Scope | Path | Creation |
| --- | --- | --- |
| Global | AKRA_HOME/config.toml (default: ~/.akra/config.toml) | Created with canonical defaults on the first normal Akra start. |
| Project | Git-worktree/.akra/config.toml | Created only by akra config set --project. |

Every linked Git worktree has its own project file. A non-Git directory has no project scope and uses only global settings. --help and akra config path, list, get, or doctor never create the global file.

When a command explicitly targets a different workspace, Akra resolves that target's project-TOML and legacy Git layers. Its global, environment, and `-c/--config` inputs remain pinned to the process-start snapshot.

An effective leaf uses this fixed highest-to-lowest order:

    -c/--config > environment > project TOML > legacy repository Git config > global TOML > built-in

TOML tables merge by individual leaf. Removing a leaf with unset therefore exposes its value from the next lower layer rather than replacing an entire table. A missing schema_version means v1; writers always record schema_version = 1.

## Supported TOML

The first generated global file contains these canonical built-ins. Project files may contain only the rows marked yes.

| Group | Keys and built-ins | Project |
| --- | --- | --- |
| conversation | model = "gpt-5.6-sol"; reasoning_effort = "medium" | yes |
| tui | show_startup_visual = true; planning_worker_visibility = "normal" | yes |
| github | auto_discover_pull_request = true; review_poll_interval_secs = 60; push_remote = "origin"; pull_request_mode = "required" | yes |
| parallel | integration_branch = "prerelease"; evidence_mode = "observe" | yes |
| app_server | response_timeout_secs = 15; prompt_log = false | timeout only |
| subprocess | timeout_secs = 30 | yes |
| diagnostics | trace = "off"; spans = "none"; max_files = 7; max_file_bytes = 16777216; max_total_bytes = 67108864; tokio_console = false | no |
| admin | graphic_enabled = true; graphic_poll_interval_ms = 10000 | yes |

conversation.model = "default" and conversation.reasoning_effort = "default" explicitly request app-server defaults. They differ from unset, which resumes inheritance. Custom model IDs are accepted as bounded, whitespace-free strings. The picker preserves one temporarily as Configured: <id> rather than silently changing it to a catalog model. Sol recommends medium; Terra and Luna recommend max.

The validator rejects malformed TOML, unknown keys, wrong types, invalid values, unsupported schema versions, and project-only violations before normal startup. Enum values are lower-case: worker visibility normal|debug, PR mode required|auto|disabled, trace off|on|planning|full (or a valid tracing filter), and spans none|close|full. Existing trace aliases such as 1 and off remain accepted for environment compatibility.

## CLI

    akra config list [--global|--project]
    akra config get <key> [--global|--project]
    akra config set [--global|--project] <key> <value>
    akra config unset [--global|--project] <key>
    akra config path [--global|--project]
    akra config doctor
    akra [-c key=value ...] [command ...]

Without a scope, list and get display effective values with their origin. set and unset default to global scope. doctor is read-only: it reports both files, environment shadowing, legacy Git compatibility values, and path/security problems without repairing anything.

-c/--config may repeat and is process-only. For example:

    akra -c conversation.model=gpt-5.6-terra -c conversation.reasoning_effort=max

## Task delivery evidence

`parallel.evidence_mode` accepts exactly `observe` or `enforce`. The built-in `observe`
records delivery assessments without holding delivery solely for missing evidence. `enforce`
requires current host verification or an exact Human criterion waiver before publication.
`akra config set --project parallel.evidence_mode enforce` opts in for the canonical checkout.
Each repository's effective policy, including a configuration error, is pinned for the process;
service reconstruction and linked worker/integration worktrees do not reload it. Changes apply on
the next repository kernel/process start. An unreadable or malformed layer holds delivery instead
of falling back to `observe`. See [the delivery evidence contract](task-delivery-evidence.md),
including the limited meaning of a successful `git_diff_check_v1` recipe.

## Environment and legacy compatibility

Environment values remain above TOML until removed. The resolver records the winning variable in list, get, and doctor.

| Setting group | Accepted existing variables |
| --- | --- |
| TUI | AKRA_SHOW_STARTUP_VISUAL; AKRA_PLANNING_WORKER_VISIBILITY |
| GitHub | AKRA_GITHUB_POLL_INTERVAL_SECS; AKRA_GITHUB_PUSH_REMOTE; AKRA_GITHUB_PR_MODE |
| Parallel | AKRA_PARALLEL_INTEGRATION_BRANCH |
| App-server | AKRA_APP_SERVER_RESPONSE_TIMEOUT_SECS; AKRA_APP_SERVER_PROMPT_LOG |
| Subprocess | AKRA_SUBPROCESS_TIMEOUT_SECS |
| Diagnostics | AKRA_TRACE; AKRA_TRACE_SPANS; AKRA_TRACE_MAX_FILES; AKRA_TRACE_MAX_FILE_BYTES; AKRA_TRACE_MAX_TOTAL_BYTES; AKRA_TOKIO_CONSOLE |
| Admin | AKRA_ADMIN_GRAPHIC_ENABLED; AKRA_ADMIN_GRAPHIC_POLL_MS |

AKRA_GITHUB_PR, RUST_LOG, and AKRA_TRACE_FILE keep their existing specialized contracts and appear as diagnostic shadowing rather than TOML keys. The former `CODEX_EXEC_LOOP_*` names of these variables were removed (#2588). If one is still set, every Akra binary stops at startup and names the `AKRA_*` replacement; it is not ignored silently, so a restricted sandbox setting cannot fall back to the default. Legacy repository Git values remain compatible below project TOML: akra.githubPushRemote, akra.githubPrMode, and akra.parallelIntegrationBranch.

## Persistence, conversation, and safety

Global files are owner-only and their global directory must be current-user-owned and not group- or world-writable; project files are normal, reviewable worktree files. Both readers and writers reject symlinks, hard links/reparse points, non-regular files, unsafe global ownership/permissions, oversized files, and object changes during open. Writes use a path-hashed, OS-backed interprocess file lock below AKRA_HOME, reread under the lock, preserve comments with toml_edit, and atomically replace the target. The lock file may remain, but the operating system releases its exclusion when a writer exits unexpectedly.

Changing :model, :model default, or :think changes the live selection immediately, queues the next-new-thread global default, and, for an active thread, persists its own model/reasoning row. Core serializes rapid selections in request order; a failed global or thread write never rolls the live selection back and is shown separately. A new thread records the submitted options; reopening a saved Akra thread restores them. A pre-existing or external thread with no row deliberately sends None/None to app-server and keeps app-server defaults—later global changes do not rewrite it or another running Akra process.

Hidden planning and parallel workers retain their explicit model policy and do not inherit the interactive conversation default.

## Issue policy file

The issue convention and disclosure policy (#2416) live in a separate file, not in config.toml. `IssuePolicyService` reads and validates it and returns a typed `IssuePolicy`. No command or client uses it yet; the validators, masking, and publish path that consume it are proposed work (#2417 to #2419).

| Scope | Path |
| --- | --- |
| User | AKRA_HOME/issue-policy.toml (default: ~/.akra/issue-policy.toml) |
| Project | workspace/.akra/issue-policy.toml |

Each value resolves in this highest-to-lowest order: project file > user file > built-in. A table merges by individual value; `required_sections` is one value and is replaced as a whole. A missing schema_version means 1. Schema: [`schema/akra-issue-policy.schema.json`](../../schema/akra-issue-policy.schema.json). Example: [`schema/akra-issue-policy.example.toml`](../../schema/akra-issue-policy.example.toml).

| Key | Values | Built-in |
| --- | --- | --- |
| convention.template | relative path inside the repository, no `..` | none |
| convention.required_sections | list of section titles | empty |
| publish.spans.secret, name, path, log | remove, restorable_mask, user_confirm | secret = remove; others = restorable_mask |
| ai_handoff.spans.secret, name, path, log | remove, restorable_mask, user_confirm | secret = remove; others = restorable_mask |
| publish.required_validation | true only | true |
| vault.user_unlock | true only | true |
| code_validators (list of id, targets, required) | targets: publish, ai_handoff; required: true, false | empty; targets = publish; required = true |
| approved_code_validators.ID.command, sha256, timeout_seconds | user file only: absolute path, SHA-256 of the file, 1 to 300 | timeout_seconds = 10 |

`publish.spans` applies when text goes to the provider. `ai_handoff.spans` applies when text goes to an AI. The two tables are independent. `remove` discards the value, `restorable_mask` replaces it with a marker that the user can restore from the Vault, and `user_confirm` asks the user for each span.

The mandatory publish validation and the Vault user unlock are always on. No scope can turn them off: `publish.required_validation = false` or `vault.user_unlock = false` makes the file invalid. A file that is unreadable, malformed, has an unknown key, class, or action, an unsupported schema_version, or a template path outside the repository gives `IssuePolicyBlocked` with the scope of the bad file. The caller must then block publish and AI handoff for that project. There is no fallback to the built-in policy. Both files use the same read safety rules as config.toml.

A code validator is an executable that the user approved. `code_validators` selects a validator by ID; the list is one value, and a project list replaces the user list. Only the user file can set `approved_code_validators`: a project file with that table is invalid, so a repository cannot define a command. The validator gets the validated shared copy as JSON on stdin (`schemaVersion`, `target`, `title`, `body`) and an environment with only a fixed PATH. Exit code 0 passes, exit code 1 rejects (stdout is the message), and all other results count as "did not run". A copy of the approved file runs in an isolated temporary directory without network access and without access to user files. The isolation exists only on macOS (`/usr/bin/sandbox-exec`). On other platforms the validator does not run. A required validator that is not approved, is missing, has a different hash, exceeds the time limit, stops abnormally, or has no isolation blocks publish.

## Issue automation policy file

Issue automation policy lives in a separate `issue-automation.toml`. It declares collection selection, autonomy limits, risk rules, gates, budgets, and external feedback permissions. Collection, deterministic triage, and the issue coordinator consume the merged policy. Starts require current human or explicitly confirmed Gated policy authority and stage-specific permits. Admission enforces concurrent and UTC daily budgets; execution admission also enforces `budget.max_attempts`. GitHub status feedback and separately approved closing use the durable feedback journal. See [Gated behavior](current-product.md#gated-start-and-budgets), [integration and closing](current-product.md#issue-integration-and-external-closure), and [plan 28](../plan/28-akra-issue-automation.md#ia4-조각-상태).

| 범위 | 경로 |
| --- | --- |
| 사용자 | `$AKRA_HOME/issue-automation.toml` (기본 `~/.akra/issue-automation.toml`) |
| 저장소 | `workspace/.akra/issue-automation.toml` |

스키마는 [akra-issue-automation.schema.json](../../schema/akra-issue-automation.schema.json)이며, [사용자 예시](../../schema/akra-issue-automation.user.example.toml)와 [저장소 예시](../../schema/akra-issue-automation.project.example.toml)를 함께 제공합니다. 각 파일은 부분 설정입니다. `schema_version`을 생략하면 1이며, 나머지 값은 아래 병합 규칙을 따릅니다. 비어 있는 목록과 필드 생략은 서로 다른 뜻일 수 있습니다.

### 병합과 사용자 권한

일반 값은 개별 leaf마다 **저장소 > 사용자 > 내장** 순서로 정합니다. 배열은 특별한 규칙이 없는 한 하나의 값입니다. 아래 값은 저장소가 사용자 권한이나 보호 장치를 늘리거나 지우지 못하도록 별도로 병합합니다.

| 값 | 병합 규칙 |
| --- | --- |
| `authority.autonomy_cap` | 사용자 상한과 저장소 제한 중 낮은 수준. 요청한 `binding.github.mode`도 이 상한까지 낮춥니다. |
| `budget.*` | 사용자 최대값과 저장소 제한 중 작은 값. 저장소는 최대값을 늘릴 수 없습니다. |
| `feedback.comment/labels/transition/telegram` | 사용자의 허가와 저장소의 제한을 AND로 결합합니다. 사용자 허가의 기본값은 모두 `false`입니다. 저장소의 `true`만으로 권한을 얻지 못합니다. |
| `select.trusted_actors` | 사용자가 신뢰한 안정 ID 삼중항과 저장소 목록의 교집합. 저장소에서 생략하면 상속하고, `[]`이면 모두 제외합니다. |
| `select.comment_commands` | 사용자의 허가와 저장소의 제한을 AND로 결합합니다. 기본 `false`이며 저장소의 `true`만으로 댓글을 읽지 않습니다. [이슈 댓글 명령](current-product.md#issue-comment-commands)을 참고합니다. |
| `protection.paths` | 내장·사용자·저장소 경로 패턴의 합집합. `[]`으로 이전 보호를 지울 수 없습니다. |
| `human_required.rules/additional` | 내장·사용자·저장소 규칙의 합집합. 저장소는 사람 승인이 필요한 조건을 더할 수만 있습니다. |

기본 요청 수준은 `observe`, 기본 자율 상한은 `propose`이고 외부 이슈 쓰기 허가는 모두 꺼져 있습니다. 사용자 파일의 상한과 허가는 실제 접수·실행에서 강제하며, 파일만으로 자동 시작에 동의한 것으로 보지 않습니다.

런타임 Gated를 처음 선택할 때는 자동 시작에 동의하는 네이티브 확인이 필요합니다. 선택 없이 파일에만 `gated`나 `auto`를 적으면 유효 수준은 Propose 이하이며 원본 파일은 바꾸지 않습니다. 명시적인 Gated 선택 뒤에도 사용자·저장소 상한(공개 저장소에 설정된 Propose 상한 포함), 작성자 신뢰와 사람 필수 규칙이 우선합니다. 현재 런타임 선택은 Off/Observe/Propose/Gated이며 Auto는 선택 항목으로 제공하지 않습니다.

유효 수준이 Gated이고 해당 위험 등급의 `gates.gated.<risk>.start = "auto"`를 만족한 Ready 이슈만 자동으로 등록합니다. 전달·통합에는 사람 리뷰가 필요합니다. Gated 선택은 전역 병렬 pool을 켜지 않으므로, 별도 병렬 실행을 켜기 전에는 등록한 작업이 `ExecutionDisabled`로 대기합니다.

### 키·기본값·범위

| 키 | 허용값과 내장 기본값 |
| --- | --- |
| `schema_version` | `1`; 생략해도 1 |
| `binding.github.repository` | 선택 항목인 `owner/name`. 생략 시 후속 연결에서 origin을 사용 |
| `binding.github.mode` | `off / observe / propose / gated / auto`; 기본 `observe` |
| `authority.autonomy_cap` | 같은 5개 수준; 기본 `propose` |
| `select.mode` | `labels_any / all_open`; 기본 `labels_any` |
| `select.labels_any` | 정확한 라벨 이름의 OR; 기본 `["akra:auto", "akra:gated", "akra:manual"]`. `[]`은 선택 대상 없음 |
| `select.trusted_actors` | `{provider, server_id, actor_id}` 배열; 기본 `[]`. provider는 `github / jira` |
| `select.comment_commands` | boolean; 사용자 허가 기본 `false` |
| `classify.rule` | `class = low / normal / high`와 `labels_any`·`paths_any`의 순서 있는 규칙 배열; 기본 `[]`, 미일치는 `normal` |
| `gates.<mode>.<risk>.<gate>` | 아래 관문 표 참고. 모든 기본값은 `human` |
| `timeouts.start` | `1s`부터 `30d`까지; 기본 `24h` |
| `timeouts.remind_before` | `0s`부터 `30d`까지; 기본 `2h`. 유효 정책에서 start보다 작아야 하며, 0은 사전 알림 없음 |
| `budget.max_running` | 0부터 4,294,967,295까지 정수; 기본 2 |
| `budget.max_started_per_day` | 같은 범위; 기본 5 |
| `budget.max_attempts` | 같은 범위; 기본 3 |
| `feedback.comment/labels/transition` | boolean; 사용자 허가 기본 `false` |
| `feedback.telegram` | boolean; 기본 `false`. 결정 요청·중재자 결정·실패의 Telegram 알림. 알림은 권한이 아니므로 정책 판(SHA-256)에 넣지 않습니다 |
| `close.when` | `never / integration / provider_link`; 기본 `never` |
| `protection.paths` | 상대 경로 glob 배열; 아래 내장 보호 경로와 합집합 |
| `human_required.rules` | 아래 10개 고정 규칙; 내장값은 10개 전부 |
| `human_required.additional` | 추가 조건·관문 객체의 배열; 기본 `[]` |

`repository`의 owner는 영숫자로 시작·끝나는 1~39자 ASCII 영숫자·하이픈, name은 1~100자 ASCII 영숫자·밑줄·하이픈·점이며 `.`과 `..`는 허용하지 않습니다. 라벨·server ID·actor ID는 비어 있거나 앞뒤 공백·제어문자가 있으면 거절합니다. 내부 공백과 대소문자는 보존합니다. 신뢰 여부는 삼중항 전체의 정확한 일치로 정하며, GitHub의 `actor_id`에는 검증한 GraphQL Node ID를 넣습니다. `login` 키나 로그인 문자열 목록은 허용하지 않습니다. 예시의 Node ID는 실제 계정이 아닌 설명용 값입니다. Jira 식별자 형식을 담을 수 있다는 것은 Jira 수집·쓰기 어댑터가 구현됐다는 뜻이 아닙니다.

분류 규칙에는 `labels_any` 또는 `paths_any`의 조건이 최소 하나 있어야 하며, 두 조건과 각 목록의 항목은 OR입니다. 규칙 배열의 순서는 유지합니다. 시간은 정수와 `s / m / h / d`를 붙여 적습니다. 공백·부호·소수·선행 0은 거절하며, 0은 `remind_before`에서 `0s / 0m / 0h / 0d`로만 적을 수 있습니다. 유효 정책은 초 단위로 정규화하므로 `24h`와 `1d`는 같은 값입니다. JSON Schema는 각 부분 파일의 문법과 단위별 상한을 검사하고, 서비스는 병합 후 서로 다른 단위의 `remind_before < start` 관계를 검사합니다.

### 예산의 실행 기준

세 예산은 사람 시작과 정책 자동 시작에 모두 적용합니다. 필드 생략은 내장 기본값을 유지하며, 0은 새 시작을 막습니다.

| 키 | 계산과 강제 시점 |
| --- | --- |
| `budget.max_running` | 작업 공간 전체의 미종결 작업 예약 수를 최초 접수와 같은 트랜잭션에서 검사합니다. 큐 대기·실행 중·리뷰 중과 아직 task를 등록하지 않은 시작 예약도 포함합니다. |
| `budget.max_started_per_day` | UTC 날짜별 최초 접수를 작업마다 한 번 셉니다. 재접속·응답 유실 복구·재시작·날짜 변경 뒤 같은 작업의 복구는 다시 차감하지 않습니다. |
| `budget.max_attempts` | 이슈 작업의 구현 시도에 적용하며 보완 task도 포함합니다. 실행 한도를 먼저 설치한 뒤 배분을 허가합니다. 같은 논리 task의 질문에 답해 재개하는 것은 새 시도가 아닙니다. |

예산이 부족하면 미시작 이슈는 Ready에 남고 `budget_exhausted` 이유를 표시합니다. 예산 대기는 승인 fingerprint와 별도로 보관하므로 다른 작업의 예산 사용 때문에 기존 승인이 무효화되지 않습니다. 이미 시작한 작업이 시도 한도에 도달한 뒤에도 보완이 필요하면 `failed`로 전환하고 사람의 복구 결정을 요구합니다.

확인된 유효 수준이 Gated/Auto인 실행 중 커널은 클라이언트가 없어도 유지합니다. Observe/Propose는 기존 idle 종료를 유지합니다. 커널 유지 상태는 실행 허가가 아니며 접수·배분 때 현재 정책을 다시 검사합니다. 프로세스 재시작 뒤 저장된 선택만으로 커널을 무인 기동하지 않고, 다음 정상 연결 때 선택과 작업을 복구합니다.

### 관문과 사람 필수 규칙

`gates`의 mode는 `propose / gated / auto`, risk는 `low / normal / high`입니다. `off`와 `observe`에는 실행 관문을 설정하지 않습니다. 각 mode·risk·gate를 개별 leaf로 병합합니다.

| 관문 | 일반 허용값 | 추가 제한 |
| --- | --- | --- |
| `start` | `human / timeout_default / timeout_arbiter / auto` | `propose`에서는 `human` |
| `spec` | `human` | 다른 방식은 거절 |
| `plan` | `human / timeout_arbiter` | 자동 승인·기본값 마감은 거절 |
| `delivery` | `human / timeout_default / timeout_arbiter / auto` | `propose / gated`에서는 `human` |
| `merge` | `human / auto` | `propose / gated`에서는 `human` |

닫힘은 `gates.*.*.close`가 아닌 `close.when`으로 정합니다. 사람 필수 규칙은 `high_risk`, `held_or_manual`, `untrusted_actor`, `protected_path`, `large_diff`, `missing_acceptance`, `attempts_exhausted`, `disclosure_confirmation`, `budget_exhausted`, `arbiter_uncertain`입니다. 이름이 다른 임의 규칙을 `rules` 배열에 넣을 수 없습니다. 추가 조건은 `[[human_required.additional]]`로 적습니다.

```toml
[[human_required.additional]]
labels_any = ["needs-human"]
paths_any = ["src/payments/**"]
gates = ["plan", "merge"]
```

위 규칙은 라벨이나 경로 중 하나라도 맞으면 지정한 관문에 사람 승인을 요구한다는 선언입니다. 조건은 최소 하나, `gates`는 최소 한 항목이 있어야 합니다. 추가 규칙의 gate는 `start / spec / plan / delivery / merge / close`를 사용할 수 있습니다. 규칙의 이름이나 ID를 붙이는 별도 키는 없습니다.

내장 보호 경로는 `.github/workflows/**`, `scripts/release/**`, `.akra/issue-automation.toml`, `.akra/issue-policy.toml`, `**/AGENTS.md`, `**/.env`, `**/.env.*`, `**/credentials*`, `**/*.pem`, `**/*.key`입니다. glob은 저장소 상대 경로이며 `/`로 구분합니다. `**`는 전체 segment로만 사용하여 0개 이상의 경로 segment, `*`는 segment 안의 0자 이상, `?`는 segment 안의 한 글자를 뜻합니다. 빈·`.`·`..` segment, 앞뒤·중복 slash, 역슬래시, `:`, `[`, `]`, `{`, `}`, `!`, 제어문자와 전체 값의 앞뒤 공백은 거절합니다. 이 단계에서는 glob 문법을 검증하며 실제 diff와의 매칭은 후속 판정 작업입니다.

### 닫힘과 부분 설정의 검증

`close.when`은 `never`, `integration`, `provider_link` 중 하나입니다. 같은 파일에 `feedback.transition = false`와 `close.when = "integration"`을 함께 적으면 JSON Schema가 거절합니다. 프로젝트 파일이 `close.when = "integration"`만 지정하는 것은 허용합니다. 사용자 파일에서 `transition = true`를 상속할 수 있기 때문입니다. 모든 파일을 합친 유효 정책에서 transition이 꺼져 있고 integration 닫힘을 요청하면 서비스가 `policy_blocked`로 거절합니다. `provider_link`는 공급자의 연결 동작을 뜻하며 Akra의 직접 상태 전이 허가를 만들지 않습니다.

파일을 읽을 수 없거나, TOML 구문·키·enum·범위·조합이 잘못되면 해당 범위와 안전한 오류 코드가 있는 `policy_blocked`를 반환합니다. 잘못된 파일을 무시하고 내장 값으로 진행하지 않습니다. 정책 판은 정규화한 **유효 정책 내용**의 SHA-256이며, 주석·키 순서·표현만 달라져 유효 내용이 같으면 같습니다.

이 파일은 이슈 원문의 공개·마스킹 규칙을 담은 `issue-policy.toml`과 독립적입니다. 각 서비스는 자기 파일만 읽고 검증합니다. 신뢰 ID를 로그인 이름으로 자동 조회하거나, 정책을 읽으면서 공급자에 접근하는 동작은 없습니다.

## Deliberate exclusions and Codex difference

Akra excludes AKRA_HOME, credentials/tokens, allowlists, app-server process or shell environment, API-key forwarding, approval/reviewer/sandbox policy, and public-repository/autonomous-delivery opt-ins. Telegram keeps its private telegram.env; parallel agent profiles keep .akra/parallel-agent-profiles.json.

Codex supports broader user and trusted project configuration, profiles, system configuration, and project trust gating; see the official [Config basics](https://learn.chatgpt.com/docs/config-file/config-basic) and [Advanced config](https://learn.chatgpt.com/docs/config-file/config-advanced). Akra intentionally has no v1 profile or system layer and no project-trust registry. It uses one Git-worktree project file and a short allowlist so a repository cannot alter credentials, security policy, or process execution behavior.
