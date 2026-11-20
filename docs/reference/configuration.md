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
| parallel | integration_branch = "prerelease" | yes |
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

## Environment and legacy compatibility

Environment values remain above TOML until removed. The resolver records the winning variable in list, get, and doctor.

| Setting group | Accepted existing variables |
| --- | --- |
| TUI | CODEX_EXEC_LOOP_SHOW_STARTUP_VISUAL; legacy CODEX_EXEC_LOOP_SHOW_STARTUP_ASCII_ART; CODEX_EXEC_LOOP_PLANNING_WORKER_VISIBILITY; legacy CODEX_EXEC_LOOP_PLANNER_VISIBILITY |
| GitHub | CODEX_EXEC_LOOP_GITHUB_POLL_INTERVAL_SECS; AKRA_GITHUB_PUSH_REMOTE; AKRA_GITHUB_PR_MODE |
| Parallel | AKRA_PARALLEL_INTEGRATION_BRANCH |
| App-server | CODEX_EXEC_LOOP_APP_SERVER_RESPONSE_TIMEOUT_SECS; AKRA_APP_SERVER_PROMPT_LOG |
| Subprocess | CODEX_EXEC_LOOP_SUBPROCESS_TIMEOUT_SECS |
| Diagnostics | AKRA_TRACE; AKRA_TRACE_SPANS; AKRA_TRACE_MAX_FILES; AKRA_TRACE_MAX_FILE_BYTES; AKRA_TRACE_MAX_TOTAL_BYTES; AKRA_TOKIO_CONSOLE |
| Admin | AKRA_ADMIN_GRAPHIC_ENABLED; AKRA_ADMIN_GRAPHIC_POLL_MS |

CODEX_EXEC_LOOP_GITHUB_PR, RUST_LOG, and AKRA_TRACE_FILE keep their existing specialized contracts and appear as diagnostic shadowing rather than TOML keys. Legacy repository Git values remain compatible below project TOML: akra.githubPushRemote, akra.githubPrMode, and akra.parallelIntegrationBranch.

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

`publish.spans` applies when text goes to the provider. `ai_handoff.spans` applies when text goes to an AI. The two tables are independent. `remove` discards the value, `restorable_mask` replaces it with a marker that the user can restore from the Vault, and `user_confirm` asks the user for each span.

The mandatory publish validation and the Vault user unlock are always on. No scope can turn them off: `publish.required_validation = false` or `vault.user_unlock = false` makes the file invalid. A file that is unreadable, malformed, has an unknown key, class, or action, an unsupported schema_version, or a template path outside the repository gives `IssuePolicyBlocked` with the scope of the bad file. The caller must then block publish and AI handoff for that project. There is no fallback to the built-in policy. Both files use the same read safety rules as config.toml.

## Deliberate exclusions and Codex difference

Akra excludes AKRA_HOME, credentials/tokens, allowlists, app-server process or shell environment, API-key forwarding, approval/reviewer/sandbox policy, and public-repository/autonomous-delivery opt-ins. Telegram keeps its private telegram.env; parallel agent profiles keep .akra/parallel-agent-profiles.json.

Codex supports broader user and trusted project configuration, profiles, system configuration, and project trust gating; see the official [Config basics](https://learn.chatgpt.com/docs/config-file/config-basic) and [Advanced config](https://learn.chatgpt.com/docs/config-file/config-advanced). Akra intentionally has no v1 profile or system layer and no project-trust registry. It uses one Git-worktree project file and a short allowlist so a repository cannot alter credentials, security policy, or process execution behavior.
