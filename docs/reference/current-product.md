# Current Product and Operator Contract

[한국어](../ko/reference/current-product.md)

This document is the canonical shipped-behavior reference for `prerelease`. Future work belongs in
an explicitly proposed document, not here.

## Product Shape

- Akra is a native-first Rust client over official `codex app-server` interfaces.
- The alternate-screen fullscreen TUI is the primary surface. One app-owned transcript viewport
  owns conversation history, streaming rows, tool cards, overlays, status, and the composer.
- The startup logo occupies that transcript viewport; startup, draft editing, and active turns use
  one bottom-anchored status/composer geometry, so the first submission never moves the input box.
- Losing the controlling terminal terminates the native TUI through its normal shutdown boundary;
  a closed terminal or PTY must not leave the TUI or its app-server runtime resident.
- Agent text stays raw Markdown in app-server state and is rendered at the TUI projection boundary;
  fenced-code delimiters and language labels are not shown as conversation text.
- Submitted operator prompts render as compact full-width low-luminance cards with one `›` marker.
  They do not spend a separate row on a `You:` label, keep the same surface through wrapping, and
  remain non-selectable interaction chrome. A non-default origin such as `Auto Follow-up` remains
  visible as a compact inline badge inside the card.
- `src/core` coordinates headless app commands, effects, completions, events, and snapshots;
  composition-owned `NativeClientRuntime` gives the TUI one typed `CoreInput` dispatch boundary.
- CLI, Admin, Telegram, and automation adapters reuse application services instead of owning
  separate planning or parallel policy.
- Akra Desktop (`desktop/`, Electron) is a client of `akra-gateway`. It is a development build that
  runs from source; it is not part of the native archives or npm packages.

## Operator Surfaces

| Surface | Entry | Shipped behavior |
| --- | --- | --- |
| Conversation | default | draft prompts, stream a turn, review approvals, and continue work |
| Diagnostics | `Ctrl+d`, `:diag` | inspect startup readiness and blockers |
| Sessions | `Ctrl+o`, `:sessions` | search, rename, resume, or start a blank draft |
| Reviews | `:reviews` | inspect the bounded review center projection |
| Activity | `:activity [all\|diff\|output\|command\|patch\|…]`, `:act` | inspect a chronological typed activity timeline with outcome, exact elapsed time, foldable retained detail, and line-numbered unified diff; read/explore and patch detail is also expandable in the conversation |
| Queue | `:queue`, `:q`, `akra queue` | inspect accepted head, proposals, skip framing, and receipts |
| Planning | `:planning`, `:planning-init` | stage, validate, and promote planning changes |
| Directions | `:directions` | maintain directions and queue-idle supporting artifacts |
| Health | `:doctor`, `:planning doctor`, `akra doctor`, `akra status` | inspect planning authority without authoring |
| Configuration | `akra config` | inspect effective values and their origins; safely edit global or Git-worktree TOML settings |
| Parallel | `:parallel`, `:pa` | enable or refresh automation and open the focused Parallel Operations board |
| Parallel peek | `:peek` | inspect active parallel agent conversations |

Global keys include `Ctrl+q` to exit, `Ctrl+t` to start a blank draft, and `Esc`/`Ctrl+c` to close
the active inspection surface. A modal may override global keys while it owns focus; visible key
copy must match the current controller path.

## Kernel JSON CLI

`akra describe --json` prints the embedded kernel specification without starting a kernel.
`akra kernel <scope> <command> --field value --json` sends a command through the shared
application command port. `--workspace <directory>` selects another workspace.
Command names, scopes, and input fields come from the specification.

`akra kernel query <name> --json` reads kernel state. `akra kernel history --after <sequence>
--wait --json` waits for records. `akra kernel operation <op_id> --wait --json` waits for an
operation to finish. Output is one JSON line. Exit codes are 0 for acceptance, 2 for rejection,
and 1 for invalid input or an unavailable connection. Acceptance is not completion.

Mutations accept `--idempotency-key`. An omitted key is random and lasts for one invocation.
Callers that retry across invocations must supply the same key. Destructive commands require
`--confirm true`. When a command ends with an error, the output carries `idempotency_key`, and
`error.code` when the kernel answered with a JSON-RPC error. The caller sends the same command
again with that key; the kernel does not run it twice.

The Gateway path uses a transient attachment. A transient last client releases an idle kernel
without the durable client's ten-minute grace. With no Gateway, stored commands run under a
command-owned repository lock without starting app-server, schedulers, or prompt recovery.
Runtime commands are rejected with `kernel_unreachable` in three cases, and `message` names the
case: no Gateway runs, the Gateway runs but another process (a TUI) runs the kernel of that
repository, or the running Gateway is an older version than the CLI (it refuses the attach
request; restart the Gateway). In all three cases a stored command runs under the repository lock. The existing TUI-owned reset compatibility path remains until K6 and does not create
kernel audit records.

An idle kernel that only this CLI started stops when the command ends. A kernel with remaining
work or confirmed Gated/Auto automation stays up. The session list is not persisted between
kernel instances, so `load_sessions` helps while that kernel remains running. The automatic follow-up
settings are kept: the kernel records them (`auto_follow_configured`) and the next Gateway kernel
restores the last record.

Stored commands change stored planning data only, so they run without a running kernel. Today
they are `reset_planning`, the task commands (`create_task`, `update_task`, `delete_task`), and
the direction commands (`create_direction`, `update_direction`, `delete_direction`)
(`state: store` in the specification). The process that holds the repository lock runs them at once: the Gateway kernel,
or the command-only kernel when no Gateway runs or the running Gateway is an older version that
does not know the command. The answer of `create_task`, `update_task`, and `delete_task` comes
after the change: `result` (the task id and the new planning revision), `rejection`, or `failure`
(the cause, also in `command_completed.detail`); a failure exits with 1. `update_task` replaces
the fields of the task and `delete_task` needs `confirm: true`; both need
`expected_planning_revision`, and a different current revision rejects them with
`planning_revision_stale` and changes nothing. `planning_locked` rejects a command while another
change or parallel work holds the planning data. `akra kernel query planning` answers from the
store when no kernel runs, so a client can read the current revision. An object field goes as
JSON text: `akra kernel planning create_task --task '{"title":"..."}' --json`.

The direction commands follow the same rules. `create_direction` makes the id from the title.
`update_direction` replaces the whole direction (a missing summary becomes the title and a
missing state becomes `active`) and fails when no direction has that id. `delete_direction`
needs `confirm: true` and also deletes the tasks of the direction; its `result` has
`removed_task_count` and `default_retained`. The default direction is kept
(`default_retained: true`). A direction needs one success criterion or more.

The admin sends its task and direction changes as these commands. The direction forms
(`/admin/directions/upsert`, `/admin/directions/delete`, the same paths below `/admin/akra`)
and the JSON routes (`/api/planning/directions`, `/api/planning/directions/delete`) make
`create_direction`, `update_direction`, or `delete_direction` and answer like the task routes
that follow. A direction update or delete needs `expected_planning_revision`. A direction
request with an id is an update: it does not make a new direction with that id. The task forms (`/admin/tasks/upsert`,
`/admin/tasks/delete`, the same paths below `/admin/akra`) and the JSON routes
(`/api/planning/tasks`, `/api/planning/tasks/delete`) make `create_task`, `update_task`, or
`delete_task`, and the record of the command has `caller: akra-admin`. The admin reads the page
values from the store after the command ends. A task delete now needs
`expected_planning_revision`, the same as a task update; the delete form sends the value that the
tasks page showed. When the kernel rejects a change (`planning_revision_stale`,
`planning_locked`), the admin answers 409, not 500: the JSON routes answer
`{ "reason", "message" }`, and the forms answer the tasks page again with the message as its
notice and the current values. Another failure of these routes is a 500 that carries its cause
(`{ "message" }`, or the tasks page with the cause as its notice). Two cases are not recorded:
a folder that cannot have a repository kernel (a repository without a primary checkout, a
parallel slot worktree) changes tasks directly, and a repository whose kernel a TUI holds runs
the command without a record. The file and draft changes of the admin still run directly.

The planning reset of the other entrances also goes to the repository kernel. `akra reset`, the
admin reset (`/api/planning/reset`, `/admin/controls/reset`), and the Telegram `/reset` commands
send `reset_planning` through the same command port and wait for the end of that operation. The
record of the command names the entrance in `caller` (`akra-cli`, `akra-admin`, `akra-telegram`).
The text that each entrance shows did not change: the files that a reset rewrites and removes are
fixed for each target, and a failed reset shows the cause that the kernel recorded
(`command_completed.detail`, at most 300 characters). A folder that cannot have a repository
kernel (a repository without a primary checkout, a parallel slot worktree) still resets directly. The confirmation rule of the entrances did not change either: a queue
reset needs no confirmation, `directions` and `all` need one, and an unconfirmed request is refused
before it reaches the kernel. Under a TUI that runs the kernel, the reset still runs directly and
leaves no record (until K6).

The generated `akra-kernel` skill is installed independently of MCP server registration and
registered through `skills/extraRoots/set`. Its source is `schema/akra-kernel-spec.json`.
`node scripts/generate-kernel-skill.mjs` prints the generated document; `--check` verifies the
shipped asset.

## Shell Command Registry

The source registry is `src/adapter/inbound/tui/app/inline_shell_commands.rs`.

```text
:diag
:parallel [off]
:peek
:activity [all|diff|output|command|patch|mcp|plan|reason|agent|terminal|token|guardian|moderation|unknown]
:sessions
:reviews
:queue
:directions
:turns <positive|infinite|off>
:stop
:model [default]
:view [simple|medium|detail]
:language [english|korean]
:think <none|minimal|low|medium|high|xhigh|max|default>
:copy [selection|last]
:planning [doctor]
:doctor
:reset <queue|directions|all>
:new
:help
```

`:model` opens a staged `Model → Reasoning` picker. It shows the active provider without
inventing unavailable providers, then exposes only the selected model's reasoning choices and
marks its recommended level. The OpenAI catalog begins with GPT-5.6 Sol, Terra, and Luna; its
GPT-5.6 rows offer `max` and omit `minimal`.

The initial interactive defaults are GPT-5.6 Sol with `medium` effort. `:model` and `:think`
change the current selection immediately and persist the next-new-thread default in the global
configuration; an active thread also retains its selected model and effort independently. Reopening
an older or external thread without Akra's saved options deliberately leaves both fields unset for
the app-server rather than applying a newer global default. See the
[Configuration Reference](configuration.md) for the layered file contract and CLI.

`:turns` controls single-session auto-follow. Parallel automation is a separate explicit opt-in.
`:stop` terminates active app-server sessions, closes the active parallel epoch, and disarms both
continuation paths. A later `:parallel` re-arms only parallel continuation.

In the conversation, a left-button drag selects the exact rendered transcript cells, keeps their
background highlighted, and copies the resulting text through OSC 52 when the button is released.
The clipboard path supports direct terminals and tmux passthrough. `:copy selection` repeats the
last completed selection; `:copy last` copies the latest raw assistant answer. Akra owns mouse
reporting for the entire fullscreen lifecycle so wheel scrolling, cards, and transcript selection
behave consistently. There is no runtime or startup capture switch, and terminal-native drag
selection is not a supported Akra fullscreen interaction.

## Turn and Approval Flow

1. Input may be drafted before startup completes; submission waits until readiness permits it.
2. Core admits one turn submission at a time; an accepted dispatch issues exactly one worker effect
   and permits the TUI to clear the editor and append the prompt to transcript history.
3. `Tab` can confirm delivery into the exact active turn. Core admits one correlated steer worker,
   keeps the draft until provider acknowledgement, and ignores stale completions; later edits or
   identical retyped input are never cleared by an older acknowledgement.
4. Streaming assistant output updates its canonical `item_id` row in place. Tool rows append at
   arrival time, so a late final assistant event cannot reorder the transcript.
5. Typed activity, runtime notices, approvals, warnings, and expandable tool cards update the same
   fullscreen projection.
6. Post-turn evaluation advances, pauses, or stops continuation from accepted planning state.

The Activity surface joins progressive payloads with authoritative item-lifecycle records by exact
item identity. It exposes active, completed, failed, declined, interrupted, and observed outcomes
without deriving progress from prose. Exact active waits distinguish model response, subagent,
approval, task output, and retry. `Up`/`Down` selects a row; `Enter`, `e`, or a left click folds or
expands retained detail. App-server `Read`, `ListFiles`, and `Search` command actions project a
concise typed target header and reveal exact bounded targets only when expanded; raw command text is
not retained for these typed actions, and reads with no output still appear from lifecycle truth.
Completed rows stay visually quiet while active work remains dominant. Unified diffs show old/new
source line numbers and extend addition/deletion backgrounds across the code region while keeping
the gutter neutral.

The default app-server execution profile is `approval=never`, `reviewer=none`, and
`sandbox=danger-full-access`. Main conversations, resumed threads, hidden planning workers, and
parallel workers use that same profile at both thread and turn boundaries. The project is still
marked untrusted in app-server configuration so repository-local configuration cannot silently
replace Akra's process contract. Diagnostics always show the active profile, but the intentional
default is informational rather than a permanent degraded warning.

Codex discovery still pins an absolute launcher, native interpreter, and canonical package script
before spawn. Ownership, ACL, or repository-boundary trust failures are non-blocking by default and
remain visible as startup warnings; launcher shape, native format, absolute-path, and package-target
validation still apply. A principal with mutation rights can still replace a permissively accepted
path before spawn. Set `AKRA_REQUIRE_TRUSTED_CODEX_EXECUTABLE=1` to restore the strict fail-closed
trust boundary and block that exposure. An absent value or exact `0` keeps the permissive default,
while any other value rejects startup as invalid configuration.

Deployments can opt down with `AKRA_APP_SERVER_APPROVAL_POLICY`,
`AKRA_APP_SERVER_APPROVALS_REVIEWER`, and `AKRA_APP_SERVER_SANDBOX_MODE`. The former
`CODEX_EXEC_LOOP_APP_SERVER_*` names stop startup with a rename message. When approvals are enabled, only the interactive main
conversation can answer a reviewable command or bounded additional permission request. `Y`
accepts once; `N`/`Esc` declines; `Enter` is inert. Timeout, interrupt, disconnect, invalid
payloads, uninspectable file-change requests, and unattended workers fail closed. No session-wide
grant is cached.

## Planning Contract

- Planning follows `draft -> validate -> promote`.
- SQLite behind `PlanningTaskRepositoryPort` is the accepted task, direction, queue, claim, and
  runtime authority.
- `.akra/planning/` holds operator-authored details, prompts, staged drafts, exports, and
  rejected-write evidence. It is not the task-authority database. `.akra/runtime/` holds private
  locks, quarantine, and trace logs.
- The repository kernel lock owner moves a pre-#2588 `.codex-exec-loop/{planning,runtime,followups}`
  into `.akra/` with a directory rename before any planning read or write. It refuses to merge when
  both locations exist. Authority store schema 21 rewrites the stored `.codex-exec-loop/` path
  prefixes once, in the schema migration transaction.
- Builtin `next-task` and internal continuation execute only the accepted queue head.
- Proposed tasks remain visible but non-executable until accepted.
- Queue-idle behavior comes from accepted direction authority.
- Admin/API intake creates one validated `ready` task and never interrupts an `in_progress` task.
- `akra planning-tool` is the structured list/create/update boundary for automation callers.
- Tasks can carry user-owned acceptance criteria and a host-managed acceptance revision. The existing
  Kernel/admin mutation path applies planning revision CAS; worker task tools read criteria but cannot
  edit them. Legacy task documents remain readable. See [the acceptance contract](task-acceptance.md).
- Desktop can turn selected conversation text into an editable task draft. Explicit confirmation saves
  it as `awaiting_user`; opening or editing the draft does not start a model turn. The kernel panel's
  task editor also opens persisted tasks, including completed and cancelled tasks.
- The `list_tasks` kernel query reuses the existing planning-tool snapshot read and returns editable
  fields with the same planning revision. Updates preserve fields outside the editor and use that
  revision for compare-and-set. In-progress tasks remain protected by the host.
- A lost task-save reply is recovered through the original command key and operation record. Successful
  task mutations record only task ID and planning revision. Recovery is read-only; missing, expired,
  legacy or interrupted records leave the outcome unresolved and never create another task.
  Criteria storage does not execute validation or change the existing delivery gates.
- Desktop task details also accept an exact task ID and read stored execution, delivery and PR results.
  The `task_result` query reuses the full task snapshot and existing authority records. Immutable
  validation sources and explicit remediation correlations keep completed tasks linked after queue
  pruning; titles and branches are never join keys. Related tasks open separately. Task status,
  host verification, worker reports, PR merge evidence and unknown external-issue closure are distinct
  facts. Missing lineage, unavailable reads and changed repository connections remain explicit.
- `prepare_evidence` / `record_evidence` bind worker-reported results to the current acceptance scope,
  running lease, host-observed Git tree and a bounded private artifact. `task_evidence` and the
  optional first page in `planning` distinguish current, stale, waived and unavailable observations.
  A criterion waiver requires an exact native Desktop Human confirmation. These records do not
  attest independent test execution or change delivery permission. See [task evidence](task-evidence.md).
- A user-owned command criterion can select `git_diff_check_v1`. The Runtime-only
  `request_task_verification` command reserves an attempt with five scope/request fields; the host
  runs its fixed Git diff check in a background worker and publishes a separate execution receipt.
  `host_verification` keeps current host results apart from worker reports and retains stale or
  unavailable facts without exposing raw output or private execution authority. A successful diff
  check covers new whitespace errors and conflict markers, can pass an empty diff, and does not
  attest a build or functional tests. Restart recovery records interruption instead of rerunning.
  The receipt itself grants no completion, dispatch or delivery permission. See
  [host task verification](task-verification.md).
- Task delivery checks essential criteria before source push, PR creation and integration push,
  including ordinary Tasks without WorkOrder lineage. `parallel.evidence_mode = "observe"` is
  the default and records assessments; explicit `enforce` requires current host verification or
  an exact Human criterion waiver. Reported Pass and stale/missing host results do not qualify.
  The canonical repository's effective policy or configuration error is pinned until process
  restart. Frozen source, integration base/result tree and exact push OID remain bound to the
  assessment. Done, dispatch, PR human review, Spec approval and Auto authority are unchanged.
  See [delivery evidence](task-delivery-evidence.md).
- Hidden planning workers inherit the active app-server execution profile. Their developer
  contract remains planning-only, and the host validates and applies their structured commands.

Authority lives below `${AKRA_HOME:-~/.akra}/projects/<project>/runtime/planning-authority.db` for
both Git and non-Git workspaces. Git repositories use a private incarnation marker in the canonical
Git common directory so linked worktrees share one authority and independent clones do not.

## Issue Automation Execution

Jira Cloud 이슈 수집과 피드백은 명시적인 Cloud/project/account/저장소 바인딩을 사용한다.
작업 시작은 Propose와 Native Human 확인으로 제한하며, PR 조회 미지원은 Unknown으로 유지한다.
구성 예제, description 템플릿, 선택적인 숫자 ID 전이와 토큰 교체 계약은
[Jira Cloud 이슈 자동화](issue-jira-cloud.md)를 따른다.
Linear는 저장소 파일이 선택한 팀만 수집하고 같은 Propose·Native Human Start 상한을 쓴다.
`branchName`은 슬롯 브랜치의 힌트이며, 구성과 계약은 [Linear 이슈 자동화](issue-linear.md)를 따른다.

Implementation scope: IA4-1 ([#2551](https://github.com/RefinedStone/codex-exec-loop/issues/2551)),
on the [#2638 PR branch](https://github.com/RefinedStone/codex-exec-loop/pull/2638).
The implementation and its parent chain were integrated into prerelease on 2026-11-22;
[plan 28](../plan/28-akra-issue-automation.md#ia4-조각-상태) tracks the remaining slices.

The issue coordinator reads the existing work order's task lineage, worker sessions, and PR
validation facts through a neutral execution port. It projects queued, running, in-review, and
integrated work into the issue ledger. Local tracking runs separately from remote issue collection,
including while collection is off. Tracking an existing fact grants no new execution or delivery
permission; those actions retain current policy, source, gate, and control checks.
Accepted progress produces `issue_execution_progressed`; the first trusted integration produces
`issue_integrated`.

Execution, verification, and publication counts are separate. Stable execution and terminal receipt
identities prevent repeated observations and restarts from counting the same fact twice. Execution
counts advance for new execution receipts; only new `observe_again` or `queue_recovery` terminal
receipts advance verification counts.
`budget.max_attempts` applies to implementation attempts, including supplemental tasks, at durable
execution admission. An answer that resumes the same logical task consumes no additional
implementation attempt. When rework is still needed at the execution limit, the issue becomes
`failed` and requires human recovery. Publication attempts are counted by the durable feedback send journal, separately from these execution facts
(see [Issue Automation Feedback](#issue-automation-feedback)).

| Terminal classification | Execution response |
| --- | --- |
| `rework_needed` | Existing PR validation admits the supplemental task and work-order lineage atomically, within the work's execution limit. |
| `observe_again` | A durable recovery receipt registers a fresh validation successor for another observation. |
| `queue_recovery` | Recover the existing remediation queue using its original idempotency key and task correlation. |
| `contract_recovery`, `identity_conflict`, `pull_request_closed` | Open a human recovery decision; no automatic implementation retry. |

Recovery binds the original terminal record and observation revision. A repeated request reuses its
receipt and successor, preserving the original terminal and existing PR-validation FSM. A new
observation attempt has a distinct identity. Verification recovery does not consume an
implementation attempt.

An `awaiting_user` task opens one decision for the exact pending question. Its worktree and session
remain available for continuation. Replying requires a verified human confirmation and nonblank
shared answer text. The persisted continuation claim checks the question's task, session, thread,
worktree, lease generation, and current authorization before sending to that same thread. It never
falls back to a new thread. An uncertain send remains `unknown` and is not automatically resent.
Opening “my conversation” selects the confirmed current thread or a shared-copy draft; takeover
and sending a turn remain explicit actions. An ambiguous conversation identity remains unavailable.

A running issue worker can also ask a choice question with `akra kernel issue ask_issue_decision`
(IA8-1, [#2566](https://github.com/RefinedStone/codex-exec-loop/issues/2566)). The command takes the
worktree's current branch, a question, 2 to 32 options, and an optional recommendation. It takes no
issue number: the server finds the running lease of that branch, its thread, and the work order's
issue. The question and labels pass the same shared-copy policy as other decisions, and each option
has a fixed effect text. The command records the asking connection, opens an `agent_question`
decision card, and marks the task `awaiting_user` for that lease generation. The worker then ends its
turn; the existing preserve path keeps the thread and worktree. No second awaiting_user card opens for
that wait. Sending the same question again returns the same card. Each issue has at most one open
agent question; another question is rejected with `issue_question_limit` and its `next` steps. The
asking connection cannot answer its own question (`own_issue_question`), and every answer still needs a
verified human confirmation. The answer is sent once to the same thread through the continuation
claim above, as the question, the selected option, and the optional shared note. The generated
`akra-kernel` skill describes this usage.

Trusted integration is an immutable work fact. It fixes execution progress at `done`, separately
from the provider issue's open/closed state. Later `observe_again` recovery preserves that state and
the original integration identity while observing post-merge validation. It creates no new
implementation task; post-integration queue recovery is blocked. A previously exhausted `failed`
issue may become `done` when trusted integration arrives, while cancellation and the held flag are
preserved.

Desktop execution tracking and provider closing follow the contracts below. The combined
[flow validation](#issue-automation-flow-validation) (#2555) has passed its automated regressions;
live model validation has not been run.

### Gated Start and Budgets

IA5-1 ([#2556](https://github.com/RefinedStone/codex-exec-loop/issues/2556)) adds an explicit
runtime Gated choice. Its first selection requires native confirmation of automatic starts.
A file-only `gated` or `auto` request without that runtime choice is limited to Propose or lower;
the file itself is unchanged. Runtime Gated remains subject to user and repository caps (including
an existing Propose cap on a public repository), actor trust, protected paths, and human-required rules.

An eligible Ready issue is admitted automatically only when its effective level is Gated or Auto and its risk
class's Start gate is `auto`. This records policy authority, without fabricating a human decision.
A `human` Start gate still needs the current human decision; Gated delivery and integration also
retain human review.

| Budget | Enforcement |
| --- | --- |
| `max_running` (default 2) | Atomic admission counts all nonterminal work reservations in the workspace, including queued, running, in-review, and admitted work not yet submitted. Human and policy starts share the limit. |
| `max_started_per_day` (default 5) | Counts first work admissions within each UTC day. Reconnection, acknowledgement recovery, and restart do not charge that work again. |
| `max_attempts` (default 3) | Limits implementation attempts per issue work, including supplemental tasks, at execution admission. |

Repository settings can only lower the user limits. Omitted limits retain their built-in defaults;
zero prevents a new start. An unstarted issue blocked by a budget stays Ready with
`budget_exhausted`. Budget waiting is a separate projection, so another issue's budget use does not
change an existing approval's fingerprint.

Gated selection and issue Start do not enable the global parallel pool. Work may be registered
while it is off and remains `ExecutionDisabled` until parallel execution is explicitly enabled.
Execution limits are installed before dispatch permission is granted.

An already running kernel stays resident after its clients leave while its confirmed, validated
effective autonomy is Gated/Auto. Observe/Propose collection alone keeps the existing idle-unload
behavior. This lifecycle decision grants no execution permission; admission and dispatch still
validate current policy. Gateway 재시작 시에는 같은 authority 홈의 후보 등록부를 읽고,
기존 Git 신원과 저장소 잠금·미완료 변경 복구·현재 선택 및 정책을 확인한 뒤 client 없이
커널을 시작한다. 등록부는 실행 권한이나 병렬 opt-in을 대신하지 않는다.
자세한 순서·장애 격리·기존 설치의 등록 방법은
[Gateway의 이슈 자동화 재시작](issue-automation-restart.md)을 따른다.

### Auto Delivery

IA6-2 ([#2560](https://github.com/RefinedStone/codex-exec-loop/issues/2560)) adds a separately
confirmed runtime Auto choice. Current provider metadata must identify the same private or internal
source repository; public, unknown, or missing repository identity caps Auto at Propose. Existing
lower levels keep their own policy behavior. Policy-required human gates, protected changes, budgets,
held work, and actor trust continue to apply.

Auto delivery uses the existing distributor and binds its permit to the exact reviewed source range,
current Merge gate revision, and source repository. It requires enforced task evidence and at least
one essential Command criterion with `git_diff_check_v1`; missing or unsupported criteria hold
delivery. The issue body does not automatically become confirmed typed completion criteria.

Automatic integration additionally requires process opt-in
`AKRA_PARALLEL_AUTONOMOUS_DELIVERY=1`. Without that opt-in, the current PR must have human approval
for the exact source SHA. Required checks, clean mergeability, and frozen base/head identity remain
mandatory in both cases. Auto cannot skip an unavailable or disabled PR workflow. See the
[Auto delivery contract](issue-auto-delivery.md) for authority and compatibility details.

Selecting Auto does not enable the global parallel pool or grant Spec approval. Binding-wide
pause uses the separate runtime contract below. Gateway startup restores confirmed Gated/Auto
repositories through the shared recovery path described in [Gateway restart](issue-automation-restart.md).

### Binding Pause / Resume

`set_issue_binding_paused` is a Runtime command for one registered issue binding. Acceptance closes
new action admission and persists a pending intent; an absent receipt is not Pause completion.
Completion follows work authorization fencing, permit revocation, and draining admitted requests
through their response or Unknown settlement. A started worker turn is not claimed to be forcibly
stopped. Existing execution facts, human answers, and task-level holds remain available.

Resume requires Native confirmation bound to the exact workspace, command, binding run revision,
binding metadata, autonomy revision, and current policy. A changed basis keeps the binding paused.
Old-generation callbacks and permits cannot acquire the resumed generation. Other bindings and
ordinary work continue, including delivery past a paused queue head. Native and TUI execution use
their repository owner's shared admission; pending commands and immutable completion outbox records
are recovered before reopening execution. See [Binding Pause / Resume](issue-binding-pause.md) for
accepted/completed states, execution boundaries, recovery, and the limits of remote Unknown results.

## Issue Automation Feedback

IA4-2 ([#2552](https://github.com/RefinedStone/codex-exec-loop/issues/2552)) is implemented
in prerelease through PR #2640 and its #2638 parent chain.
It adds GitHub status comments and the existing `akra:running` label. Assignee changes and provider
closing remain outside this slice.

Only user-policy `feedback.comment` and `feedback.labels` permit these writes; both default to
false. A repository policy cannot turn them on. The status body is a deterministic projection of
saved work state and execution/verification counts, passed through the shared-copy masking policy
and secret detector. It accepts no raw agent message. The publication count stays in local feedback
metadata so posting a status does not itself trigger another status update.

The private issue SQLite database stores immutable intentions before dispatch. Only the successful
`Recorded -> Sent` claimant may invoke one remote mutation. Recovery abandons an unsent recorded
intent and only reads a sent or uncertain intent; it never resends it. The single status-comment
owner is fixed by the full issue identity, stable actor ID, comment ID, intent marker, and body hash.
A lost create response can bind ownership only after a complete, unique readback. Conflicting
acknowledgements or ownership changes remain unresolved. An unresolved feedback intention blocks
new feedback and direct title/body publication for the same remote issue, including workspace
aliases; an unresolved title/body publication also blocks feedback.

Before the first execution grant, enabled label feedback confirms an owned start marker. An existing
foreign marker is not adopted, and a missing repository label is not created. Control, selection,
classification, or human-required label conflicts block use of the marker. Confirmed owned labels
are removed after done, cancellation, or failure when current policy still permits writing. Off,
held, stale, unknown, and pending-intervention states prevent new writes; readback of an already
sent intention remains available independently.

Collection and feedback share the existing repository-wide, process-shared publication lease from provider read through
observation commit. Only the exact confirmed label event is excluded from semantic observation.
An uncertain label mutation defers observation without changing its stored basis or flags; new execution
grants wait until readback settles it. A later event from the same account remains visible, and current policy is checked before exclusion,
so a policy change cannot hide a newly meaningful hold or selection label. Comment timestamps are
not triage inputs.

Confirmed actions atomically enqueue `issue_feedback_posted` with identity metadata and no body.
The issue detail query exposes the confirmed comment ID, publication-attempt count, and
`feedback_unconfirmed` reason. Attempts count each durable send claim once, including an uncertain
outcome; repeated reconciliation, confirmation, and event acknowledgement do not increase it.
The count is independent of the implementation-attempt budget. Wire generation and fake-provider
tests require no real GitHub write or model turn.

IA7-4 ([#2565](https://github.com/RefinedStone/codex-exec-loop/issues/2565)) adds an optional
GitHub Projects (v2) board column as a selection condition and, only when the user policy enables
`feedback.board_field`, writes the execution phase to one single-select board field through the same
feedback journal. A body-write approval never authorizes a board write. An unreadable board,
including a login without the `read:project` scope, is `board_unavailable` and not a mismatch.
See [GitHub Projects 보드 연동](issue-projects-board.md).

## Issue Decision Telegram Notifications

IA8-3 ([#2568](https://github.com/RefinedStone/codex-exec-loop/issues/2568)) lets the existing
`akra telegram` runner announce issue decisions and take answers. Notifications are off by default.
Only user-policy `feedback.telegram` turns them on; a repository policy can only turn them off.
The value is not part of the policy version, so enabling it does not supersede open decisions.

- **Source.** After each poll the runner reads the repository kernel record from a durable cursor.
  It announces `issue_decision_requested` (only while the request is still open),
  `issue_decision_arbiter_finished` with `applied` or `waiting`, and `issue_execution_progressed`
  with state `failed`. The first enabled cycle, and cycles while the policy is off, start from the
  end of the record; old events are not replayed.
- **Content.** Identifiers and closed enum names only: decision ID, provider, repository and issue
  IDs, gate, kind, option IDs, arbiter outcome, terminal class, and attempt count. No issue title,
  body, question text, local path, or provider error text.
- **Once.** Each notification has an intent key (`decision_requested:<id>`, and so on) stored per
  bot stream and chat in the repository authority store before sending. A send that fails, or a
  process that stops during a send, leaves the intent `unknown`. It is not sent again. Delivery is
  at most once, not exactly once.
- **Answer.** `/decide <decision_id> <option>` from an allowed chat (and, in groups, an allowed
  user) sends the kernel command `answer_issue_decision` with `caller: akra-telegram`. The runner
  replies `answered`, `superseded`, `already_resolved`, a rejection reason, or `failed`.
- **Evidence.** A Telegram answer carries no Desktop confirmation, so the kernel records it as
  `by: agent`. It cannot pass a `human` gate; the reply is `human_confirmation_required` (Q-IA11).

## Issue Comment Commands

IA8-2 ([#2567](https://github.com/RefinedStone/codex-exec-loop/issues/2567)) reads `/akra hold`,
`/akra release`, `/akra retriage`, and `/akra approve` from GitHub issue comments. Comments are
not read by default. The user policy must set `select.comment_commands = true`; a repository
policy can only turn it off. Only comments by a GitHub user account in the user-file
`select.trusted_actors` list are considered.

A comment is a command only when its whole body is one of the four commands. Other comment text is
not interpreted or stored. An edited comment is never a command, because another account may have
edited it; post a new comment instead. Each processed command is recorded once in the private issue
SQLite ledger by the issue's stable identity and the comment ID. The comment body is not stored.

A running agent can post comments with the user's `gh` login, so a user-account comment is not
human-confirmation evidence:

| Command | Result |
| --- | --- |
| `hold` | Applied at the next collection cycle through the existing hold intervention, before triage and start. It revokes work permits and records `issue_held`. |
| `release`, `retriage` | Recorded as a request only. They could reopen execution, so a person must issue them again from Desktop. |
| `approve` | Recorded only. It never answers a decision and never passes a `human` gate. |

The hold uses the deterministic intervention command ID `issue-comment:<comment ID>`. If the process
stops after the hold but before the comment record, the next cycle reuses that intervention receipt
without a second change. A removed trusted actor's later comments are ignored. Comments from
another, independent account are also not human evidence in this slice.

## Issue Integration and External Closure

IA4-3 ([#2553](https://github.com/RefinedStone/codex-exec-loop/issues/2553)) keeps trusted
integration and the current provider state separate. The detail query exposes the immutable
integration record and the latest observed external state (`open`, `closed`, or `unknown`).
An old confirmed close receipt never overrides a later reopen or an unavailable observation.

| Policy | Behavior |
| --- | --- |
| `close.when = "never"` (default) | Keep the issue open after integration. Enabled status feedback still reports completion. |
| `close.when = "integration"` | Require user-policy `feedback.transition = true`, trusted integration without a pending human recovery, and a separate verified human Close decision. |
| `close.when = "provider_link"` | Carry a canonical issue reference through the immutable work-order lineage and delivery queue. Add a closing keyword only when the actual target repository default branch equals the PR base/integration branch. |

A Close decision binds the work order, current shared-content and policy hashes, authorization
epoch, gate revision, integration record, source SHA, and integrated SHA. A Start approval grants
no closing permission. Before consuming a Close approval, the coordinator refreshes the external
issue and its assessment. Changed or unavailable content, held/stale state, and pending
interventions prevent the mutation. Declining or leaving the decision unanswered preserves `done`
and leaves the provider issue open.

Closing uses the existing feedback journal and publication lease: persist the immutable intent,
claim `Sent` once, issue one mutation, then read the provider. A lost response never triggers a
second close request. Confirmation requires a persisted, matching mutation acknowledgement and the
same issue/repository/account with exactly one new close event after the observed basis. Without the
acknowledgement, a same-account human close is indistinguishable from the lost mutation, so the intent
stays unconfirmed while the query still reports the observed provider state. Reopen or intervening
events remain unconfirmed. This confirms the accepted operation and observed result, not exclusive
causation in a concurrent provider session. A confirmed
close is not automatically repeated after a human reopens the issue.

`issue_integrated` and `issue_closed` are distinct durable records. Confirmed feedback and a
provider-observed Open-to-Closed transition are marked by different origins; an observation does
not claim to know who closed the issue. Ordinary tasks without an issue work order retain their
existing PR body. `provider_link` does not guarantee provider closure: the distributor's existing
integration/PR lifecycle is unchanged, and external closure remains an observed fact. The
`prerelease`-to-`main` promotion is outside this workflow.

## Desktop Issue Execution Tracking

IA4-4 ([#2554](https://github.com/RefinedStone/codex-exec-loop/issues/2554)) adds execution
tracking to the existing issue detail. The list continues to use the server's running, review, and
done groups. The detail exposes a saved execution projection, server-generated state/recovery
summaries, and the existing conversation, issue, and PR links.

Implementation attempts use the current effective policy limit; a failed policy read leaves the
limit unknown. Verification attempts and feedback publication attempts are separate counts. The
executor's revoked or completed execution limit is not used as the display budget. A missing
execution record is distinct from an unavailable or mismatched record. A review PR URL is retained
only from the validated work-order/task/active-validation lineage; the UI does not assemble a URL
from an issue number or substitute another task's PR.

Trusted integration, current external issue state, and feedback confirmation remain independent.
The detail can show integration complete with an open external issue, a contract-recovery reason,
or an unconfirmed publication at the same time. Recovery reasons and next-action summaries come
from the server, while available intervention commands remain the existing server actions.
Opening the conversation rechecks its exact stored target through the existing navigation path.
The “다시 확인” button sends only the issue detail query: it does not wake collection, reconcile
execution, retry feedback, or start a model turn.

The recent timeline contains at most 20 real recorded events for the full stable issue identity.
It merges retained storage with recent runtime records, deduplicates by sequence, and preserves
sequence order even if wall-clock timestamps move backwards. The query asks for one extra matching
record to report that older retained records exist. An unavailable event store is explicit; missing
history is not reconstructed from projection revisions. This view adds no generic history paging.
Execution, integration, closure, and feedback events schedule fresh queries, using the existing
workspace, connection-generation, selection, and stale-response fences.

## Issue Automation Flow Validation

The IA4-5 ([#2555](https://github.com/RefinedStone/codex-exec-loop/issues/2555)) regression harness
connects real collection, decision, coordinator, feedback, and execution services to on-disk SQLite,
slot worktrees, and a local bare Git remote. Source/provider responses, worker completion, and the
verified-human capability are supplied by fixtures. All seven coupled-flow scenarios passed in
Linux; they do not establish a completed live workflow.

The production private publication lease is Unix-only; its non-Unix implementation rejects the
operation, so native Windows does not support this actual issue publication path. The coupled suite
is gated with `#[cfg(unix)]`. The seven coupled-flow cases passed in an isolated WSL Ubuntu Linux
clone, alongside the related SQLite and lifecycle regressions.

The scenarios reconstruct services against the same durable paths after registration failures or
lost acknowledgements. They check task identity across registration recovery, durable event receipts
after 10,001 synthetic events trigger production retention, and comment readback without resending
a lost create. They also cover current issue controls at slot and source-push boundaries. The run
found and fixed a registration-ACK-loss bug: execution observation now records the first recovered
`Submitted` fact in the same transaction as the task lineage, without backfilling existing lineage.
Service reconstruction does not simulate an OS process kill, and synthetic events do not start
model turns.

The [validation record](../validation/issue-automation-propose-flow.md) describes the exact real/fake
boundaries, test filters, and fields for observed results. Real model calls and test GitHub writes
remain unperformed. Live validation is planned on Linux with Desktop, Gateway, and the confirmation
helper sharing one user and `AKRA_HOME`, plus a GUI where the user can answer the real native
confirmation. WSLg GUI and GitHub/model authentication have not been checked. Separate user
permission and the product's native Start confirmation are required; the current
implementation-plus-planning path has no global one-turn guarantee.

## Parallel Contract

- Bare `:parallel`/`:pa` is enable-or-refresh; `:parallel on` is not implemented.
- The first off-to-on entry checks readiness, reconciles the guarded three-slot pool, opens an
  automation epoch, and dispatches accepted ready work up to idle capacity.
- Re-entering while enabled refreshes readiness and projection; it does not reset the pool or open
  a second epoch.
- `:parallel off` stops local automation and invalidates late dispatch results but leaves worktrees
  for guarded recovery. Closing the board does not disable parallel mode.
- Pool, lease, task, session, and distributor mutations are serialized through application and
  durable cross-process gates. TUI state never decides capacity, retry, or dispatch policy.
- The application control-plane's bounded unsettled-cleanup ledger is the sole cleanup-notice
  authority. The TUI captures its typed owned projection into the conversation screen model and
  renders it without a separate adapter or conversation ledger.
- Global cleanup-notice events invalidate the current frame for redraw only; exact retry settlement
  changes the control-plane projection.
- Workers inherit the active app-server execution profile; the default is unattended
  `danger-full-access` with no approval prompts. They leave edits uncommitted, and a restrictive
  profile still declines approval requests in unattended connections.
  The host validates the exact lease, worktree, branch, frozen base, changed-file bounds, and final
  cleanliness before creating a source commit.
- Delivery is serial: source push, PR automation/review verification, exact reviewed-range
  integration into the configured branch, remote verification, and identity-checked cleanup.
- Remote, repository, visibility, integration branch, base OID, source SHA, and reviewed commit
  range are frozen before delivery. Drift blocks instead of falling back to mutable checkout state.
- Human review is required by default. Public-repository or autonomous delivery requires an exact
  parent-process opt-in; repository configuration cannot grant either permission.

The focused Parallel Operations board uses the same alternate-screen fullscreen frame transaction.
It presents one priority blocker, accepted queue pressure, a lifecycle timeline,
three slot lanes, and a selected-lane delivery checklist. Pool slots, active roster, bounded session
detail, optional role profile, distributor head, queue state, and withheld-dispatch reason remain a
read-only projection; a missing join is shown as `DESYNC` or `unknown`, never invented progress.
Accepted dispatch work is visually separate from active leases.

Selection is keyed by slot/session identity and survives projection reorder on refresh. With an
empty task composer, `Tab`, `Shift+Tab`, and the arrow keys navigate the board and `Enter` inspects
the selected lane. `Ctrl+R` refreshes, `Ctrl+P` disables parallel mode, `:peek` opens the read-only
active-agent picker, and `Esc` closes the focused board. Once task input is non-empty, navigation,
printable keys, whitespace, and `Enter` belong to the composer so work can be submitted without
leaving Parallel Operations. The composer remains hidden and locked only while the supervisor
projection is loading; an existing draft is preserved until loading finishes. A genuine readiness
blocker remains the board's first actionable fact after loading and does not take task input away.
Closing preserves the draft while the enabled passive projection continues. The 80-column layout
stacks lanes before selected detail, while 120- and 160-column layouts show lifecycle, lanes, and
detail together; each ready focused layout reserves a composer tail.

Parallel event delivery keeps one generation-qualified canonical event window. The frame capture
turns it into one app-owned `ParallelLiveStreamModel`, finalizes geometry before drawing, and never
asks the renderer to rediscover ownership from text. Under height pressure the stream title may
collapse, but event order and rows remain in the same Ratatui viewport. There is no host frontier,
history insertion, durable/live split, or replay mode.

`:peek` opens a read-only active-agent conversation preview; switching agents or overlays prevents
late results from replacing the latest preview or the interactive conversation.

The local Admin graphic dashboard keeps those facts passive, but its harness controls are real
application commands. CSRF-protected browser actions can enable the automation epoch, request the
next accepted task, refresh the supervisor projection, or disable the loop through one persistent
`ParallelModeControlPlaneHandle`. The HTTP adapter drains typed background completions back into
that handle before reporting control state; it does not call Git, GitHub, pool, or distributor
adapters directly.

Post-merge PR validation is a durable application scheduler, not an Admin or TUI refresh loop. An
unset repository-local `akra.prValidationMode` defaults to `observe`: provider evidence and exact
Actions target SHAs are recorded, but automatic Queue admission is blocked. `remediate` admits only
typed actionable findings through the ordinary Planning Queue and worker lease/worktree lifecycle;
`off` stops new polling without deleting history. The Admin Validation Rail distinguishes
Integrated, Verifying, RemediationQueued, RemediationRunning, Verified, Blocked, and Failed, and its
rollout banner states the effective mode, Queue admission policy, and separate Ruleset approval
boundary. See [PR Validation Rollout](pr-validation-rollout.md).

Rollout timing evidence follows a separate read path. The application query service validates the
collector schema, repository/base identity, generated time, evidence SHA, and 48-hour freshness
boundary before projecting actual and historical Fast Gate distributions, CI/Post-Merge timing,
sample scope, quota, and typed warnings. The repository authority retains at most 64 redacted
snapshots and exact collection lease/CAS state. Dashboard bootstrap contains only the latest
summary; the evidence drawer lazily requests at most 20 stable, cursor-ordered history rows. SSE
contains only evidence revision invalidation. A durable replay recalculates freshness, and a source
or store outage degrades to the last valid read-only projection without stopping the parallel
control plane.

The `/admin/akra` game projection uses a PixiJS 8 world over the same dashboard snapshot. Its
validated frontend store, semantic camera zoom, worker movement, furniture occlusion, and scene
selection remain presentation-only; see [Admin Game Frontend](admin-game-frontend.md).

For UI/UX debugging, `akra admin --debug-harness` starts an application-owned, non-durable Fake
scenario clock behind the same dashboard contract. It supports normal delivery, blocked recovery,
queue pressure, and the ten PR-validation recovery scenarios without mutating real planning,
parallel, Git, or GitHub authority.

## Gateway Daemon

`akra-gateway` is a resident daemon that relays local clients (Akra Desktop and the kernel CLI) to a
Gateway-owned `codex app-server`. It is P1 of [the Akra Desktop plan](../plan/17-akra-desktop/README.md).
Akra Desktop (`desktop/`) runs from source as a development build; it has no packaged release yet.

- Endpoint: unix `$AKRA_HOME/run/gateway.sock` (directory `0700`, socket `0600`); Windows named pipe
  `\\.\pipe\akra-gateway-<user SID>` with an owner-only security descriptor, first-instance creation, and
  remote clients rejected. No TCP port is opened. Startup prints `akra-gateway listening on <endpoint>`.
- Upstream: the Gateway runs `codex app-server --listen unix://$AKRA_HOME/run/codex-app-server.sock`
  with the same pinned executable, `-c` security overrides, and scrubbed environment as the TUI. It
  restarts an exited upstream with 1 s to 30 s backoff and reports the count in `akra/gateway/info`.
  It records the upstream pid in `$AKRA_HOME/run/gateway-upstream.pid`; on unix a new Gateway stops an
  upstream that a crashed Gateway left behind (only when that process is still an `app-server` for the
  same socket) before it starts its own.
- Sessions: each client WebSocket gets its own `codex app-server proxy --sock …` child, so `initialize`,
  request ids, and thread subscriptions stay per connection. Frames pass through unchanged; only
  `akra/*` requests are answered by the Gateway (`akra/mcp/*` is the MCP hub and `akra/context/*` is the
  work context, see below;
  `akra/gateway/info` returns gateway and codex versions,
  start time, and upstream restarts). An upstream restart closes clients with WebSocket close `1012`;
  an unavailable upstream closes them with `1013`.
- Hosts (plan 25 S1a): a connection selects its upstream host with the WebSocket request path
  `/host/<id>`; any other path means `local`. Production registers `local`, the app-server above, and
  `cloud` only when `AKRA_DOTS_CLOUD` is `1`/`true` or the flag file `$AKRA_HOME/dots/cloud-enabled`
  exists; the Gateway reads this once at startup. Desktop's command "Dots 켜기 / 끄기" writes or
  removes that flag (`desktop/src/main/dotsSettings.ts`) and turns the sidebar's remote section on or
  off with it, so the user enables dots in one step. At startup Desktop also turns the section on when
  the flag exists and the user never chose a value for the section. The cloud host appears on the next Gateway start,
  without dropping local sessions.
  An unknown id is closed with `1008`. `akra/gateway/info` also returns `host` (the host of this
  connection) and `hosts` (id, codex version, start time, and restarts of each host). On a connection
  to a host other than `local`, `akra/kernel/*` is refused with `-32601`, `akra/context/*` answers
  with empty `resources` and does not read or write the ledger, `akra/mcp/*` is unchanged, and
  `AKRA_GATEWAY_RECORD` records nothing.
- Commit graph (plan 27 G1, G2): `akra/git/graph` with params `{ cwd, limit?, cursor? }` returns one page of
  the commit graph of the repository that contains `cwd`: `repository`, `stale`, `head`, `refs` (local
  branches, remote branches, and tags; first page only), `rows`, and `next`. A row carries the commit
  (`id`, `parents`, `authorName`, `authorTime`, `subject`) and its lane placement (`lane`, `lanesIn`,
  `lanesOut`, `lanesThrough`). The Gateway reads with `git for-each-ref` and
  `git log --date-order --branches --remotes --tags HEAD` through the hardened Git boundary and writes
  nothing. `limit` defaults to 300 and is capped at 1000. The Gateway keeps no state between pages: the
  client returns `next` as `cursor`, and a cursor from before a ref change answers `stale: true` with no
  rows. A cursor with more than 4,096 lanes is refused. A folder outside a repository answers
  `repository: false`. The methods exist only on `local`
  connections.
  Since plan 27 G2 a page also carries `refsDigest` (a digest of HEAD, the refs, and the worktrees) and,
  on the first page, `worktrees` (`path`, `head`, `branch` from `git worktree list`). `refs` includes
  backup refs (`refs/akra/backup/*`, kind `backup`), the log includes the commits they reach, and a row
  reached only from a backup ref has `backupOnly: true`. Nothing writes a backup ref yet (plan 27 G3).
  `akra/git/refs` with `{ cwd }` returns `{ repository, refsDigest }` without reading commits.
  `akra/git/commit` with `{ cwd, commit }` returns `{ commit }`: the full message, author, committer,
  parents, and the diff against the first parent (`git show --first-parent --no-ext-diff --no-textconv`),
  or `commit: null` for an unknown sha. `commit` must be a full sha (40 or 64 hexadecimal characters). A diff over
  2 MiB is not read; the answer has `diffTooLarge: true` and an empty `diff`.
  Since plan 27 G3 the Gateway also changes a repository, only through the operation engine
  (`src/application/service/git_operation.rs`). The operations are `reset` (move the checked-out
  branch and its worktree to a commit), `move_branch` (move a branch that no worktree has checked
  out), `checkout_branch`, and `checkout_commit` (detached). `akra/git/plan` with `{ cwd, operation }`
  returns a plan and changes nothing: `expectation` (the HEAD position and, for `move_branch` and
  `checkout_branch`, the commit of that branch), `before`, `after`, `impact` (`dropped`, `unreachable`, `gained` commits of a
  branch move), and `blockers`. `akra/git/execute` with `{ cwd, operation, expectation }` builds the
  plan again and runs it only when the expectation is equal and there is no blocker; otherwise it
  answers `{ status: "stale", plan }`. A run answers `done` or `failed` with the journal record.
  `akra/git/operations` returns the 50 newest records and `akra/git/undo` with `{ cwd, opId }` runs the
  inverse operation when the repository is still in the state that operation left, and answers
  `refused` otherwise. A `reset` or a checkout is undone only from the worktree that ran it; a
  `move_branch` from any worktree. Blockers: no repository or commit, an unfinished merge, rebase,
  cherry-pick, revert or bisect in this worktree, an unknown commit or branch, a detached HEAD or
  uncommitted tracked changes (for `reset`), a branch that a worktree has checked out or that an
  unfinished rebase or bisect of any worktree uses, a branch named `akra-agent/…`, a `.gitattributes`
  or `info/attributes` that sets a `filter`, and a repository Git setting that runs a program (a
  filter, `core.worktree`, `diff.external`, …; the check of `git_execution_guard`). Before a branch
  moves, the engine writes `refs/akra/backup/<branch>/<op-id>` at the old
  commit; after it, the engine deletes the backups of that kind that are both older than 30 days and
  beyond the newest 20 of their branch. The commands are `git update-ref`, `git reset --keep`, and
  `git switch`, with `submodule.recurse=false`. The Gateway's Git reads only the repository
  configuration; the commands also receive `core.autocrlf` and `core.eol` from the user and system
  configuration (repository, then user, then system), and no other user setting. The journal is
  `git-operations.jsonl` in the repository runtime directory under `AKRA_HOME` (newest 2,000 lines).
  The engine does not know about running turns.
- Cloud host (D-19): `/host/cloud` bridges to the OpenAI cloud app-server with the access token of
  `~/.codex/auth.json`. The bridge forwards only the public-protocol methods it allows
  (`initialize`, `initialized`, `thread/list`, `thread/read`, `thread/turns/list`,
  `thread/items/list`, `model/list` for reading, and `thread/resume`, `thread/unsubscribe`,
  `turn/start`, `turn/steer`, `turn/interrupt`, `thread/stop` for a conversation), plus error and
  result responses. It answers every other request with `-32601` and drops other notifications and
  binary frames. It never hand-writes a private method such as `turn/addUserMessage`. The bridge
  allows those write methods, but Desktop has no control that sends them: cloud threads are read-only
  in the UI. When the
  cloud rejects the login (HTTP 401 or 403) the connection closes with `1008` and reason
  `cloud login rejected`. The bridge pings the cloud every 10 seconds and ends the session after 60
  seconds of silence (`src/adapter/outbound/app_server/cloud_upstream.rs`).
- Dot list (D-19): while the cloud host is enabled, `akra/dots/list` answers `{ dots }` with the id,
  display name, status, kind, paused flag, and current work thread of each cloud agent. The Gateway
  reads them with `GET https://chatgpt.com/backend-api/tbo` through `curl -q --config -`, so the token
  stays out of argv, disk, and logs; it reads the first page only. When the cloud host is off the
  method is unknown (`-32601`). Desktop asks once per remote connection. Above the local folders,
  "Dots" holds the dots and, folded under "작업 대화",
  the work threads that an agent made (`threadSource` starts with `aeon`); "클라우드 · 읽기 전용" holds
  the cloud threads that the user made (`src/adapter/outbound/app_server/cloud_dots.rs`,
  `src/adapter/inbound/gateway/dots.rs`).
- The same cloud setting enables read-only access to personal ChatGPT history on remote connections.
  The main window requests `akra/chats/list` once for its remote session. Reconnecting or opening a
  dot window does not reload this list. Disconnecting clears the displayed titles and conversation
  because the next connection may use a different account. The "ChatGPT" section lets the user
  refresh the list manually.
  Selecting a chat calls `akra/chats/read` and displays text from the active conversation branch.
  During an account rate limit, a cached list or conversation carries `limited: true` and the
  optional `retryAfterSeconds`. Desktop identifies this as the last successful read. Without a
  cached value the Gateway returns `-32029` and, when known, `error.data.retryAfterSeconds`.
- Dot chat (D-19): `akra/dots/messages` with `{ roomId }` answers `{ messages }`, the latest 30
  messages of that message room, oldest first, each with id, time, author name, a from-dot flag, and
  text; deleted messages are left out. The room id must be letters, digits, `-`, or `_`. Desktop opens
  this chat when the user clicks a dot. The chat is read-only until the user clicks "보내기 허용" and
  confirms a warning; the permission holds for that dot until the app closes. Then a composer
  appears and `akra/dots/send` with `{ roomId, text }` posts the trimmed text (1 to 8000 characters) to
  the room with a fresh `request_id` and answers `{ sent: true }`. When the service answers HTTP 400 or
  more, the error names the status and the service's short reason. Desktop reads the room on open, on "새로 고침", and when the dot finishes a reply (live state, below).
  While the dot works and the window is visible, it reads again every 2 seconds, because the service
  does not announce a new message; a message the dot posts before its reply appears then.
  Only while the live channel is down, after a send it reads again every 2 seconds until the newest
  message is a new one from the dot, for at most 45 seconds. The answer also carries `dotReadAt`, the
  time the dot last read the room; the chat shows "읽음" under the user's newest message before it.
- Dot live state (D-19): `akra/dots/watch` with `{ roomId }` starts a watch for that connection and
  answers `{ watching: true }`; `akra/dots/unwatch` stops it; a connection watches at most 8 rooms and
  its watches end with it. The Gateway then sends `akra/dots/live` notifications with `roomId`,
  `connected`, `working`, `activityLabel`, and `usingComputer`. It reads them from the service: it
  gets a signed WebSocket address from `GET /celsius/ws/user`, sends `connect` (presence `background`)
  and `subscribe` to topic `calpico-room-live:<room>`, then holds `POST /messaging/rooms/{id}/live`
  (SSE, through curl) open; the service pushes `calpico-room-live-state` on the WebSocket while that
  stream is open. It reconnects after 5 to 60 seconds and reports `connected: false` in between. It reports
  `connected: true` only with a real room state: it sends nothing on its own when the channel comes
  up, and it skips a state whose threads are all still `attaching`, because that state does not know
  the turn status yet.
  Desktop watches the room of every dot. A room that goes from working to idle counts as a reply:
  Desktop reads the chat again, marks the dot "새 답" in the sidebar until the user opens it, and shows
  an OS notification without the message text when the user is not looking at that chat. While the
  dot works, the chat shows a typing mark and the header shows the service's activity label
  (`src/adapter/outbound/app_server/cloud_dots_live.rs`). A real send was verified once on 2026-11-14.
  The header shows "새로 고침" only while the live channel is down or after a failed read
  (`desktop/src/renderer/views/DotChat.tsx`, `desktop/src/renderer/views/dotSend.ts`).
- Dot side chat (D-19): the chat with a dot is not a main view. A click on the dot in the sidebar opens
  it in a resizable column at the right edge of the main window, beside the thread, home, or fleet
  view; a second click or the header's close button closes it. Below a window width of 1400 px the
  side chat and the right panel do not fit together, and the one opened last stays; when the window shrinks below that width with both
  open, the side chat closes
  (`desktop/src/renderer/views/DotDock.tsx`, `desktop/src/renderer/session/uiStore.ts`).
- Dot window (D-19): the side chat's "따로 뜬 창으로 옮기기" moves the chat to a 440 by 680 window that
  shows only that dot, and closes the side chat. The window's "본 창의 옆 칸으로 옮기기" moves it back
  to the main window that opened it and closes the window. The main process keeps one window per dot;
  a click on that dot in the sidebar focuses the window. The window loads the same renderer with
  `?dotWindow=<host>&dot=<id>`, opens its own remote Gateway connection, does not connect to `local`,
  and uses the main window's light or dark choice (stored in `localStorage`). The main process holds
  the send permission per dot while the app runs, so the side chat and the window ask once. The text
  in the composer moves with the chat in both directions. While a
  dot's window is open, that window shows the reply notification and the main window does not; the main window keeps
  its unread mark until the user sees the reply in the window
  (`desktop/src/renderer/views/DotWindow.tsx`, `desktop/src/main/index.ts`).
- Desktop remote hosts (plan 25, Desktop part of S2): Desktop reads `hosts` from `akra/gateway/info`. The
  command palette has one command for the remote section, "Dots 켜기 / 끄기" (above). The
  default is off, and turning it on shows a warning first. While it is on, each window opens one more
  Gateway connection per remote host on `/host/<id>` and the sidebar shows a section for that host,
  above the local work folders, with
  its threads. A remote thread opens read-only with `thread/read` and `thread/turns/list`. Desktop has no
  control that sends a turn, a resume, or an approval answer to a cloud thread; the user talks to a dot
  in the dot chat instead. The remote conversation
  does not use the kernel, changes, documents, or terminal panels. Remote state stays in memory. A connection that the Gateway closes with `1008` is not
  retried; the section shows a login message when the reason is `cloud login rejected`. The command and
  the section appear only when the Gateway registers a remote host
  (`desktop/src/main/gatewayConnection.ts`, `desktop/src/renderer/session/`).
- Desktop server requests (#2836): above the composer, Desktop shows a card for command and file-change
  approvals, extra-permission approvals (`item/permissions/requestApproval`; accept grants the requested
  profile for the turn or session, decline grants nothing), tool questions (`item/tool/requestUserInput`),
  and MCP elicitation (`mcpServer/elicitation/request`; form fields of string, number, integer, boolean,
  and enum types, or a URL to visit). A form with a field type Desktop cannot draw offers only decline and
  cancel. `serverRequest/resolved` removes a card that another connection answered. Any other server
  request is refused with `-32601`, and Desktop shows a notice that names the method.
- Desktop installed skills (#2383): the "스킬" panel lists installed skills, one row per runtime, host
  (`local` only), install scope, and install path. Codex rows come from `skills/list`; Claude Code rows
  come from `~/.claude/skills` and `<work folder>/.claude/skills` on disk (install state only). Only a
  real folder below `.claude/skills`, `.codex/skills`, or `.agents/skills` is writable. Plugin, system,
  and admin installs, linked folders, and other locations are read-only. A copy has no source until the
  user links one (public GitHub `owner/name`, sub-path, and a policy: stable release, branch, or pinned
  ref); Desktop never links by name or file date. The source ledger, snapshots, and the recovery journal
  are in `<userData>/skill-sources/`; nothing is written into the install folder for tracking.
  "지금 확인" resolves the ref to a commit without credentials and compares manifest hash v1 (sha256
  over every file of the skill folder, bytes as they are; rule in `desktop/src/main/skillManifest.ts`)
  of the skill sub-path only. A failed check keeps the last success and is never shown as latest.
  Apply and rollback run in the main process after a main-process confirmation: only the commit and
  hash of the last check, no merge, abort when local files changed after the preview, a snapshot of
  the previous files and ledger entry, and a journaled folder swap that is rolled back at the next
  start if it stopped. Downloaded files are never executed. Not shipped: `skills-lock.json` reuse,
  private repositories, scheduled checks, Claude Code plugin installs, and remote hosts
  (`desktop/src/main/skill*.ts`, `desktop/src/renderer/skills/`, `desktop/src/shared/skills.ts`).
- Kernel protocol (`akra/kernel/*`, plan 18 K3, protocol version 1 since the kernel spec of K4-1): a client sends `akra/kernel/attach`
  with a workspace and attaches to the kernel of the repository that holds it. The Gateway builds that
  kernel on first attach or after validating a registered automation restart candidate, and keeps
  it after clients leave while work or confirmed Gated/Auto automation remains, so auto-follow and
  parallel dispatch can continue. Worktrees of one repository
  share one kernel, which works in the primary checkout. An
  attach from an Akra parallel slot worktree, or to a repository without a primary checkout (a bare
  repository), fails. If another process (a TUI) runs the kernel of that
  repository, attach fails with `-32012`. After attach the client sends `akra/kernel/command` and
  `akra/kernel/query` and receives `akra/kernel/event` notifications; requests before attach fail
  with `-32011`, and a kernel that does not answer in time fails with `-32013` (a command may still
  run). No event arrives after the `akra/kernel/detach` answer. `akra/kernel/describe` answers with the
  kernel spec (protocol version 1): `methods`, `commands` (scope, summary, effect, idempotency, possible
  rejections, input and output JSON Schema), `queries`, `events`, `records`, `reasons`, `errors`, the meaning
  of the scope, effect, and idempotency values (`terms`), and the shared type definitions in `$defs`
  (JSON Schema 2020-12). The input schemas carry the rules the kernel checks (non-blank text, the
  length of `idempotency_key`). The state values of the parallel board (agent state, merge queue
  state, pull request validation state and phase) are closed lists in the spec. Fields have no description in the spec yet. The same value is checked
  in as `schema/akra-kernel-spec.json`. All reason codes are one list (`reasons`): command
  rejections and the reasons of `prompt_submission` events use the same names. Each reason and each
  error has `next`, an ordered list of steps a client can run as given: `wait_for { event }`,
  `wait { milliseconds }`, `query { name }`, `command { name }`, `method { name }`, `resend` (the same
  request; a command gets a new key), `ask_user`, and `stop`. A `wait_for` step has no condition, so
  the client matches the event to its prompt by `task_id`. A command rejection carries `reason`, `message` (the summary of the
  reason), and the `next` of that reason. A `prompt_submission` event with a reason carries the same
  `next`. When the kernel drops a queued prompt without a turn, the client gets `prompt_submission`
  with status `dropped` and reason `task_finished` (its task is already done or cancelled) or
  `rejected_again` (the one resend was refused again; the task stays queued). The `planning` and
  `parallel` queries and events carry the planning
  queue and the parallel state (pool, agents, merge queue, PR validation); `refresh_planning` reads the
  queue again. The `attention` query (plan 23 step 1, first slice) lists what a person must answer
  now. The kernel derives it on each read from the same snapshot and stores no copy. The causes are
  `approval_pending` (thread), `task_proposed` (task), and `pr_validation_blocked` or
  `pr_validation_failed` (pull request). Each item has `id` (`<subject kind>:<subject id>:<revision>`),
  `cause`, `subject`, `revision` (the approval id, the task `updated_at`, or the PR validation
  observation revision), `summary` (the source text), `updated_at` (only tasks have a time), and
  `next`. An item leaves the list when its source changes. There is no attention event, read state,
  or snooze yet; a client reads the query again after `thread_activity`, `planning`, or `parallel`.
  The `history` query (`{"type": "history", "after": <cursor>, "limit": <n>}`) pages
  through the kernel decision log in record order and answers with `records`, `next_after`, and
  `recorder` (the recorder state; `degraded: true` means this kernel lost records, so a restart
  restores less); send `next_after` as the next `after`. Each new record also arrives live as a `recorded` event with the
  same `sequence`; after `resync`, read `history` after the last sequence seen. Approval requests and
  their ends are records (`approval_requested`, `approval_resolved` with `accepted`, `declined`,
  `timed_out`, `withdrawn`, or `lost`); a restarted kernel records the approvals of the previous kernel
  as `lost` and cannot answer them. `akra/kernel/command` accepts an optional `idempotency_key`
  (1 to 128 characters) and answers a recorded command with
  `operation { op_id, state, next { query, after }, replayed }`: `op_id` is the sequence of the
  `command_received` record, `state` is `accepted` or `rejected` (the result of the admission), and
  the progress of the command is the log after `op_id`. The `operation` query
  (`{"type": "operation", "op_id": <id>}`) answers with the current `state` of that operation and its
  `records`: `accepted`, `running`, `done`, `rejected`, `failed`, `lost` (the kernel was built again
  while the operation was open), or `unknown` (no command record has that id, or it is older than the
  retention limit). The kernel computes the state from the records. The `command_received` record
  carries `caller`, the name the connection gave in `akra/kernel/attach` (`client_name`, at most 64
  characters; it is not authentication), so the log says which client sent a command and which
  client decided an approval. The kernel, not the inbound adapter, checks the `effect` of the spec: a
  `destructive` command without `confirm: true` fails with `-32010`, is not run, and is not recorded.
  The `command_received` record also
  carries the `completion` rule that applied when the kernel received the command; a command received
  by an older kernel has none and stays `accepted`. A `submit_prompt` that the kernel still prepares
  is `accepted`; it becomes `running` with its first later record. Each command in the spec has
  `completion`: `accepted` (it ends when the kernel accepts it), `records` (it ends with a later
  record: `submit_prompt`, `rename_session`, `steer_turn`, `stop_all`, `decide_approval`, and
  `reset_planning`), or `unreported` (no command of this version; it is the rule of commands that an
  older kernel received, and their state stays `accepted`). `rename_session`, `steer_turn`,
  `stop_all`, `decide_approval`, and `reset_planning` end with `command_completed` when the request
  of the command returns: `done` if it succeeded and `failed` if it did not. A failed request
  writes its cause into `command_completed.detail` as text for a person. `stop_all` ends with its
  first stop request; the kernel may send the stop again after a turn starts. A command that the
  kernel accepts but that does nothing (a rename while the session list loads, a stop while another
  stop is in flight, a reset while a planning operation runs) ends as `failed` at once. A
  `submit_prompt` ends with the `turn_ended` record of its turn, or with
  `command_completed` when no turn ran (failed preparation, hand-over to parallel slots, a turn
  submission that closed before a turn started), or with the settlement of its queued prompt. A
  queued prompt is `running`, not an end. The records of a `submit_prompt` carry its `op_id`
  (`thread_prompt_queued`, `thread_prompt_settled`, `turn_submitted`, `turn_started`, `turn_ended`).
  A restarted kernel records the turns of the previous kernel as `turn_ended` with `lost` and its
  other open operations as `command_completed` with `lost`; a queued prompt keeps its operation and
  the new kernel submits it again. A resend with the same key and the same command does not run again and gets
  the first receipt with `replayed`; the same key with another command fails with `-32014`. A refused
  command keeps its refusal for its key; send a new key to try again. Keys are shared by all clients
  of the repository kernel, so a client uses a random key for each command. Keys are remembered while
  their record is in the log (the latest 10,000 records), also across a kernel restart that wrote its
  records; a crash can lose the latest keys. A kernel without a log refuses a command with a key
  (`-32015`). A `submit_prompt` that the kernel refuses before preparation (`busy`,
  `preparation_active`, `unknown_thread`, `foreign_workspace`, `parallel_mode`, `parallel_mode_unknown`)
  now also answers with `rejection` and `state: rejected`. The session catalog lists the threads of
  every repository; `foreign_workspace` refuses a thread whose catalog `cwd` is outside the client's
  workspace and outside its parallel pool (`<name>-akra-worktrees/<hash>/akra-pool`), so no planning task of this kernel is made for it. `load_sessions` and `refresh_planning` are not recorded and
  have no receipt. While a client is attached the kernel refreshes the parallel supervisor snapshot, and the
  first attach runs one supervisor inspection that does not change the pool (it runs the push readiness
  check, like the TUI parallel view).
- A repository kernel with a log observes the git state of its workspace: the checked-out commit and
  branch. A separate thread reads it about every 30 seconds; when it differs from the last record, the
  kernel writes a `repository_changed` record (`head`, `branch`, `previous_head`, `previous_branch`).
  One record can cover several changes. The record does not say who made the change, so commits by an
  Akra turn, by the user, and by an agent that Akra did not open look the same. The record has no thread
  (`thread_id` is empty in `history`). A new kernel starts from the last recorded state, so a change made
  while no kernel ran is one record; the first observation in a log has no `previous_head`. The newest
  `repository_changed` record is kept when old records are trimmed. Other
  worktrees of the repository, other branches, and uncommitted file changes are not observed. The TUI
  runtime does not observe.
- A kernel with no attached client and no remaining work (turns, approvals, auto-follow, evaluation,
  planning work, parallel mode on, running parallel work, or confirmed effective Gated/Auto
  automation) for 10 minutes is unloaded, and its repository lock is free for a TUI. The next attach starts it again. Gateway shutdown stops the
  kernels before the upstream app-server.
- The kernel uses the Gateway upstream app-server through its own `codex app-server proxy`
  connection, so a client can `thread/resume` a thread that the kernel runs and receive its turn
  notifications live.
- One Gateway runs per user: a second instance exits with `another akra-gateway is already running`.
  Child processes are contained (Windows job objects) and end with the Gateway.
- `AKRA_GATEWAY_RECORD=1` records each session's text frames as JSONL under the user-private
  `$AKRA_HOME/recordings/`. Recordings contain real conversations and must not be committed.
- The TUI, Admin, and Telegram paths keep their own stdio app-server (plan P-08).

## MCP Hub

The MCP hub keeps one user-level set of MCP servers and their logins for every session and for Akra
itself ([plan 21](../plan/21-akra-mcp-hub.md)). Sessions do not load the tool schemas; they find and
call tools on demand.

- `akra mcp servers|add|remove|login|search|describe|call` prints JSON. `add <name> --url <https-url>`
  registers a streamable HTTP server (plain http only for localhost); `add <name> -- <command> [args...]`
  registers a stdio server. `call <server> <tool> [<json-object>|-]` reads the arguments inline or from
  stdin and exits with 1 when the tool result has `isError: true`.
- The command does not resolve repository configuration, so it runs in any directory.
- Engine: a hub-only `codex app-server` with `CODEX_HOME=$AKRA_HOME/mcp-hub`. When `akra-gateway` runs,
  `servers`, `search`, `describe`, and `call` go through its hub (`akra/mcp/statuses`,
  `akra/mcp/catalogs`, `akra/mcp/call`); otherwise the CLI uses its local backend. Codex owns the MCP transports, OAuth, and token storage and refresh. The
  registry is `$AKRA_HOME/mcp-hub/config.toml` in Codex `[mcp_servers.<name>]` form; extra Codex keys
  written by hand are kept.
- Write policy (#2830): `$AKRA_HOME/mcp-hub/write-policy.toml` lists write tools per server
  (`[servers.<name>] write = ["tool", ...]`; `"*"` marks every tool of the server, and `read = [...]`
  takes tools back out). Tools that the file does not list are reads and run as before; with no file,
  every tool is a read. A write call runs only after approval in Akra Desktop: the Gateway shows a card
  (`akra/mcp/writeApproval/requested`) in every local Desktop window, the first answer decides, and the
  call fails without running when the answer is decline, when no answer comes in 120 seconds, or when no
  Desktop is connected. Only a connection that sends `akra/mcp/writeApproval/subscribe` (Desktop does
  after `initialize`) is an approver, so the `akra mcp` connection that makes the call never counts. A late answer is ignored, and a retry asks again. Without a Gateway, `akra mcp
  call` refuses write tools. When the file cannot be read or parsed, no tool call runs.
- `call` opens an ephemeral thread with only the target server enabled. `servers`
  connects every enabled server. Requests that app-server sends to the hub (MCP elicitation and others)
  are declined.
- `search` uses server-specific tool catalogs in the private
  `$AKRA_HOME/mcp-hub/tool-catalog.sqlite` cache for 10 minutes. A fresh entry needs no app-server.
  Missing or expired entries refresh only that server. Full schemas are cached, but authentication
  and connection state are not; `servers`, `describe`, and `call` remain live.
  Configuration and configured credential-environment changes invalidate cache identity.
  Add, remove, login attempts, and explicit Gateway reload invalidate the generation.
  Idle shutdown and normal Gateway shutdown preserve catalogs. An in-flight old generation cannot
  restore invalidated entries. Failed discovery is not cached, including authentication failure.
  Cache read/write failures use live discovery; explicit invalidation failures are reported.
  Unannounced remote schema or account changes may remain visible in search until expiry.
- `login` prints the authorization URL on stderr and waits up to 10 minutes for the OAuth callback.
- The Gateway starts the resident engine on the first request, reuses one ephemeral thread per server,
  handles one request at a time, drops the engine after an error, and stops it after 10 idle minutes.
  `akra mcp` sends `hubHome` with every request; a Gateway that serves a different hub answers `-32040`.
  Requests without `hubHome` (Desktop) use the Gateway's own hub. The CLI
  falls back to its own engine only when it cannot connect, gets method-not-found or `-32040`, or the
  Gateway closes with 1013 before reading; any other failure after sending is an error, so a tool call
  never runs twice. `add`, `remove`, and `login` run in the CLI and then send `akra/mcp/reload`.
- When the registry has at least one server, the hub writes
  `$AKRA_HOME/mcp-hub/agent-skills/akra-mcp-hub/SKILL.md`, and every Akra app-server connection
  registers that folder with `skills/extraRoots/set` right after `initialize`. A failure only leaves
  a warning. Connections opened before the first server was added do not see the skill.
- `akra mcp skill install` writes the same skill to the user's Codex home
  (`$CODEX_HOME/skills/akra-mcp-hub/SKILL.md`, default `~/.codex`) and adds the full path of the `akra`
  executable. Codex sessions that Akra did not open read that folder, so they can use the hub too.
  `akra mcp skill status` and `remove` print the same JSON (`installed`, `path`); `remove` deletes only
  that file and the folder when it is empty. While the installed copy exists, Akra connections do not
  register the hub folder, so a session sees the skill once.

## Work Context

The Gateway records the external documents that a thread mentions, without a model call
([plan 22](../plan/22-akra-work-context.md)).

- `akra/context/thread { threadId }` reads the thread items that are new since the last call
  (`thread/items/list`, newest first, stops at the last read item), finds documents in user and agent
  messages, commands, and MCP tool arguments, stores them, and returns
  `{ resources: [{ kind, key, url, title, source, firstSeenAt }] }`. Command output and tool results are
  not read.
- Recognizers (`src/domain/work_context.rs`): Notion pages, Jira issues (`<site>/browse/KEY-123`), and
  GitHub pull requests and issues. A new service is one `ExternalResourceRecognizer`; the ledger, the
  Gateway, and Desktop handle only the kind name.
- Ledger: `$AKRA_HOME/work-context/ledger.sqlite` with documents, document-to-scope links (`thread`,
  `repository`, `task`, and `path` scopes), scan markers, the last checked version of each document, and
  the version each scope read or confirmed.
- Desktop reads the open thread's documents when a thread opens and when a turn ends, shows the count
  on the "문서" button, and lists them by kind in the "관련 문서" side panel.
- Changed documents: `akra/context/refresh { threadId }` also asks a version provider for the current
  version of each document whose last check is older than 5 minutes (at most 20 documents per call,
  stops after 3 failures) and returns the same list. Each resource has `changed` and
  `versionCheckedAt`. The first version a thread checks is its read version; a later different version
  sets `changed`. `akra/context/markRead { threadId, kind, key }` makes the checked version the read
  version. `akra/context/thread` asks no external service.
- Repository scopes (#2481 A03, first slice): `akra/context/attach { cwd, scope, url, source? }` links one
  recognized document to a scope in the repository of `cwd`. `scope` is `{ kind: "repository" }`,
  `{ kind: "task", taskId }`, or `{ kind: "path", path }`. The repository is the planning repository
  identity, so worktrees of one repository share scopes and a new repository at a reused folder does
  not. `path` is relative to the repository: `\` becomes `/`, `.` and empty parts are removed, and
  absolute paths or paths that leave the repository are refused. Symbolic links are not followed.
  `source` is `user` (default), `agent`, or `tool`. Attach asks the version provider like `refresh`
  and keeps the first confirmed version. `akra/context/list { cwd, scope }` returns the scope's
  documents from the ledger only. Every resource also has `confirmedVersion` (the read version for a
  thread) and `currentVersion` (the last checked version); `changed` compares them. The ledger keeps
  only the document identity, address, and versions, never document text.
- Version providers (`ExternalResourceVersionPort`): GitHub reads `updated_at` of the
  pull request or issue with `gh api --hostname github.com`, with `AKRA_GITHUB_TOKEN`, `GH_TOKEN`,
  `GITHUB_TOKEN`, or the stored `gh` login. Jira reads `updated` through a connected Desktop's
  existing Jira login. Credentials stay in Electron main; the renderer handles the reverse
  `akra/resourceVersion/current` request through a typed IPC call. Only local Gateway sessions can
  register a provider. A disconnect or registration replacement cancels pending requests.
  No connected provider means the document is skipped, without spending the failure budget.
  Notion uses the registered `notion` MCP server and its existing login. It fetches
  the page by ID with `notion-fetch` (or the connector's `fetch` alias), reads
  `page_last_edited_at`, and normalizes the timestamp to UTC. It shares the Gateway's
  resident hub backend. Tool errors, missing edit metadata, and invalid timestamps
  leave previous versions unchanged. Page content is not stored in the version ledger.
- Jira site identity includes the origin, non-default port, and context path. Root-site keys remain
  compatible. The first ledger open after this change schedules existing Jira threads for recollection.
  Old links retain a `legacy:` key until their exact URL is recollected; they cannot supply versions.
  This preserves references outside the 5,000-item recollection limit without assigning a potentially
  collapsed legacy identity to the wrong installation.
- Desktop calls `refresh` after the list is shown, marks changed documents with "바뀜", shows
  `문서 N · 바뀜 M` on the button, and calls `markRead` when the user opens a changed document.
- Limits: ephemeral threads and threads without turns have no items to read; the first read of a very
  long thread covers the newest 5,000 items.

## Recovery and Limits

- Invalid or conflicting planning updates pause continuation and preserve review evidence.
- Store-backed claims allow official refresh and distributor recovery after restart; recovery does
  not hard-reset operator-owned integration history.
- Non-Git workspaces use planning authority but not the full Git worktree pool.
- One Akra TUI runs per repository, including its linked worktrees. The TUI takes the repository
  kernel lock `$AKRA_HOME/projects/<namespace>/runtime/kernel.lock` before it opens the screen; a
  second TUI prints `another Akra kernel already runs for repository …` with the holder pid and exits.
  If the lock file cannot be opened at all, the TUI warns and starts without the lock.
- Planning detail authoring is manual; the `llm-assisted` editor path is disabled.
- Real-terminal evidence is still required for primitive-sensitive fullscreen TUI changes and affected
  restart/distributor/multi-worktree flows.

## Code Entry

- Core: `src/core/`
- TUI: `src/adapter/inbound/tui/`
- CLI: `src/adapter/inbound/cli.rs`
- Admin: `src/adapter/inbound/admin_api/`
- Telegram: `src/adapter/inbound/telegram_bot/`
- Gateway: `src/adapter/inbound/gateway/`, `src/adapter/outbound/app_server/gateway_upstream.rs`
- Work context: `src/domain/work_context.rs`, `src/application/service/work_context.rs`,
  `src/adapter/inbound/gateway/context.rs`, `desktop/src/renderer/work/threadDocuments.ts`
- MCP hub: `src/adapter/inbound/cli/mcp.rs`, `src/application/service/mcp_hub.rs`,
  `src/adapter/outbound/mcp_hub/`
- Desktop (in development, plan P2): `desktop/` — Electron shell that connects each window to `akra-gateway`;
  see `desktop/AGENTS.md`. The "Akra" side panel attaches to the repository kernel of the open thread's
  folder only while it is open, and shows threads with remaining work, auto-follow, parallel work, and the
  planning queue (`desktop/src/renderer/kernel/`). Kernel wire types and command effects are generated from
  `schema/akra-kernel-spec.json` with `npm run generate:kernel-protocol`. The shared kernel command sender
  adds a fresh UUID idempotency key to each mutation and leaves read commands keyless. It does not replay
  requests after disconnect. The panel currently sends only the read command `refresh_planning`;
  conversation and approval actions still use the app-server path.
  The browser panel can start a new local browser conversation. It verifies 16
  thread-scoped MCP tools before opening the conversation. The embedded view supports
  navigation, bounded accessibility snapshots and refs, input, same-origin frames,
  waits, console/network diagnostics, and JPEG captures up to 1280 pixels with duplicate
  suppression. It does not expose arbitrary JavaScript or CDP commands. Human input,
  hiding the panel, and changing the selected thread pause control. Resume is explicit.
  `browser_handoff` waits for that resume. Cold sessions and remote threads are not
  silently attached. A private `akra-gateway desktop-browser-bridge` process owns
  authenticated registrations and removes their secret sidecars on shutdown. The
  renderer receives a sidecar path, not its token. Screenshots are not saved by default.
  Browser diagnostics and actions live in `desktop/src/main/browserAgent/`; the thin
  MCP executable lives in `desktop/src/browser-mcp/`.
  Codex allows one writer process per thread. When another Codex process (for example the Codex app) holds the
  thread's writer lock, `thread/resume` fails with `already has an active writer`. Desktop then opens the thread
  read-only with `thread/read` and `thread/turns/list`, shows a banner, and disables the composer. The banner's
  "다시 연결" tries `thread/resume` again and reads the newest history when the lock is still held
  (`desktop/src/renderer/session/desktopSession.ts`).
  Conversation fork (#2625, first slice): the writer-lock banner shows "채팅 포크" next to "다시 연결", and the
  top bar of a normal local thread shows "대화 분기". Both send one official `thread/fork` through the existing
  Gateway path with `lastTurnId` pinned to the newest finished turn (completed, interrupted, or failed) read just
  before the request, `excludeTurns: true`, and `ephemeral: false`. A turn that is still running is not copied.
  The source gets only `thread/read` and `thread/turns/list`: no resume, turn, or unsubscribe. No prompt is sent
  after the fork. The fork opens only when the user is still on the source screen (navigation revision check);
  otherwise it is only added to the sidebar and the subscription that `thread/fork` made is ended. Repeated clicks
  share one request. When the connection drops during the request, or no response comes within 20 seconds
  (`FORK_RESPONSE_TIMEOUT_MS`), the result is unknown: Desktop does not retry, tells the user to look for a fork in
  the sidebar, and asks before a new fork. A success that comes after the 20 seconds only adds the fork to the
  sidebar; it does not change the screen. Desktop does not look up fork candidates with `thread/list`, because
  `thread/list` does not return `forkedFromId`. A fork is a conversation copy, not a Git worktree: it uses the same working folder,
  so two conversations that edit files at the same time can conflict. The fork shows this warning and an
  "원본 열기" button. Remote, temporary, sub-agent, dot, and browser conversations (browser tool calls in the loaded
  turns) are refused with a reason. Limits: the sidebar "분기" badge and the warning come from `forkedFromId` in
  the fork response; codex 0.172.0 `thread/list` returns `forkedFromId: null`, so they do not come back after a
  reconnect. The Gateway treats a successful `thread/fork` like `thread/start` for the storage-share conversation
  (`src/adapter/inbound/gateway/storage_context.rs`), so an opened fork can share storage evidence. After a fork
  that was only listed, the source must be opened again before it can share. Model, approval, sandbox, and MCP settings are those that app-server gives a normal fork; Desktop
  sends no overrides.
  Proposed, not implemented: fork from a selected older turn, fork into a separate Git worktree (#2481 A07),
  lineage after restart, remote/dot/browser forks, and fork candidate lookup after an unknown result.
- Set `AKRA_DESKTOP_DELEGATED_CHAT=1` before starting Desktop to enable its chat driver.
  The Tunnel launcher uses Local Agent runtime schema 23. When upgrading from schema 22,
  finish active work, reinstall the Local Agent in the launcher, then reconnect.
  The shared runtime status API checks the installed files against the manifest.
  A `ready` installation can still have a disconnected tunnel.
  In the work panel, open **ChatGPT 위임**, enter a text instruction, and select **텍스트 위임**.
  Open an item to see its status, the last recorded delivery and phase, and the result when it succeeds.
  **위임 취소** cancels an active item. **ChatGPT 로그인 열기** opens the dedicated login view.
  Desktop reports when the connected Gateway does not support delegation.
  Select a conversation to continue it under the matching ChatGPT account. The panel shows
  backend status and jobs waiting for your action. A retry creates a new job for an eligible item.
  `akra kernel delegation wait <id> --json` waits up to ten seconds for a change.
  Use `akra kernel delegation result <id> --json` to read the stored result.
- Repository delegation policy controls backend order, concurrency, daily limits and the deadline.
  The ledger records attempts and observations across restarts. An unknown delivery outcome
  requires attention before another send. The Codex backend uses an isolated read-only thread.
  Run `akra codex-delegation login` and sign in with ChatGPT to enable subscription
  access for delegated threads. Codex keeps this dedicated profile's session in the
  operating system credential store. Run the command as the user who runs the Gateway.
  For opt-in PR review, see [Configure defensive pull request review](pr-review-backend.md).
- Desktop basics (plan 26 U1, #2626): zoom is per area. The window page stays at 100%; the renderer applies
  CSS `zoom` to each body view (home, fleet, thread, remote), the sidebar, the top bar, each right-panel tab,
  the dot dock, the dot window, and the ChatGPT dock. The terminal multiplies its font size, and the browser
  panel's web page has its own factor (`browser-page`). The view menu and `Cmd/Ctrl` `+` `-` `0` change the
  focused area (the current body view when focus is elsewhere); `Cmd/Ctrl` + wheel and pinch change the area
  under the pointer. The sidebar and the top bar change only from the settings dialog. The factors are saved
  with the main window's size and position in `userData/window-state.json` (`zooms`); an old single `zoom`
  moves to the body views once. The macOS window buttons follow only the top bar factor. On macOS the main window has an inset title
  bar. A right-click menu gives the edit commands, spelling suggestions, and "링크 복사" in the app window and
  the browser panel. Each window keeps a navigation history of views and opened threads (top-bar buttons,
  `Cmd/Ctrl` `[` `]`, mouse back and forward buttons; a deleted thread is skipped). `Cmd/Ctrl+F` finds text
  in the user and agent messages of the open thread, turn by turn. Messages and code blocks have copy buttons.
  In an empty composer the arrow keys recall sent prompts (kept in the window's local storage). While the
  window is not focused, a finished or failed turn and a new approval request raise an OS notification that
  names the thread and opens it on click
  (`desktop/src/main/windowState.ts`, `desktop/src/main/contextMenuItems.ts`,
  `desktop/src/renderer/session/navHistory.ts`, `desktop/src/renderer/conversation/find.ts`,
  `desktop/src/renderer/session/attention.ts`).
- Desktop settings (plan 26 U2): the settings dialog opens from the app menu ("설정…", `Cmd/Ctrl+,`), the
  sidebar footer, and the command palette. It has a search field and these settings: theme, zoom per area, font size
  for the UI, the conversation, code and diffs, and the terminal (10–24 px); OS notifications (off, only when
  no app window has focus, always) and their sound; sleep prevention (off, only while a turn runs, always).
  main validates the values, saves them in `userData/app-settings.json`, and sends them to every window.
  Sleep prevention uses Electron `powerSaveBlocker` (`prevent-app-suspension`); each main window reports if it
  has a running turn. For dot replies only "off" applies; Desktop still decides by itself when a dot reply is
  worth a notification. The default mode is "only when no app window has focus"
  (`desktop/src/shared/settings.ts`, `desktop/src/main/appSettings.ts`,
  `desktop/src/renderer/views/SettingsDialog.tsx`).
  The "Notion" view of the "업무" side panel reads the `notion` server of the
  MCP hub through `akra/mcp/*`: recently viewed pages, search, open in the browser (https Notion hosts
  only), and hand-off that fills the new-thread screen's composer with the page (`desktop/src/renderer/work/`).
  The "Linear" view reads the latest 30 assigned, incomplete issues through Linear's
  GraphQL API. Main validates the personal key with `viewer` before saving encrypted
  credentials in `linear-token.bin`. Team/project IDs and the active-cycle filter
  operate on that cached list. Open and refresh fetch data; filters do not.
  Hand-off reads the description and ten newest comments, preserves the issue URL
  and suggested branch name, and fills a draft capped at 20,000 characters.
  It uses the visible local thread's folder or asks for one; cancellation preserves
  the draft. It does not create a thread, branch or turn. Linear issue URLs are also
  recognized as related documents, without a change-version provider. There are no
  issue mutations or OAuth flows (`desktop/src/main/linearService.ts`,
  `desktop/src/renderer/work/LinearPanel.tsx`, `src/domain/work_context.rs`).
  The "Projects" view reads one GitHub Project (`owner/number` or its address, kept in the renderer's local
  storage) with `gh api graphql` (`ProjectV2`, `read:project` scope, at most 500 items). It shows the open
  issues, pull requests, and drafts grouped by the Status field and counts the closed ones. The repository
  filter starts at the `origin` of the open thread's folder. Hand-off of an issue fills the composer with the
  shared copy of the issue title and body and a `Closes owner/name#N` request, and picks the first workspace
  folder whose `origin` is the issue's repository; without one, sending asks for a folder. Comments are not
  included. The shared copy comes from the Gateway method `akra/issue/handoffCopy` (params
  `{ owner, repository, number, cwd? }`, result `{ sharedCopy: { title, body } }`, `local` host only, #2428):
  the Gateway reads the issue with `gh`, finds known secret shapes (token prefixes, `Bearer` values, URL
  credentials, `key=value` secrets, PEM private keys), and applies the AI hand-off issue policy. The raw
  remote text does not reach the renderer. An unreadable policy or a span that needs user confirmation
  refuses the hand-off. Names and paths are not detected. With a project folder, the service reads the
  latest confirmed publication or creation approval from the canonical workspace journals; an edited,
  uncertain, or unreadable approved state refuses hand-off. Without a project approval history, or
  without a folder, manual reading uses the current applicable AI policy and grants no automatic
  execution authority. See [the approved-copy contract and evidence](../validation/issue-handoff-approved-copy.md)
  (`desktop/src/renderer/work/githubProjects.ts`, `desktop/src/renderer/work/GitHubProjectPanel.tsx`,
  `src/adapter/inbound/gateway/issue.rs`, `src/application/service/issue_handoff.rs`).
  The "변경" side panel lists the working-tree changes of the open thread's folder and can stage, unstage, discard,
  or open one file; the main process reads the file's state from git again before it acts, asks before a discard,
  and refuses conflicted files, files with a content filter (LFS), and discarding a folder
  (`desktop/src/main/gitStatus.ts`). Expanded tracked text files also show unstaged hunks with individual stage
  and discard actions. Discard asks for confirmation and preserves staged content. Main reads the diff again
  and rejects stale hunk IDs; unsupported files retain the whole-file actions (`desktop/src/main/gitHunks.ts`).
  Below the files it lists the commits that the branch added after its base
  (the PR's base branch, else the branch of `origin/HEAD`), marks the ones that are not pushed, and shows the
  branch's GitHub PR with its CI state through `gh` (`desktop/src/main/gitBranch.ts`). It can push the branch
  to the remote branch of the same name (never a force push) and create a PR with `gh`; the main process asks
  before both and shows the target (`desktop/src/main/gitPublish.ts`)
  The "그래프" side panel (plan 27 G1, G2) shows the commit graph of the open thread's folder, read-only. It reads
  `akra/git/graph` when the panel opens, when the folder changes, and on "새로 고침"; it draws only the
  visible rows and reads the next page 60 rows before the end. A row shows the lane lines, the ref labels,
  the subject, the author, the relative time, and the short sha. A remote branch on the same commit as the
  local branch of the same name is one label; the checked-out commit has a hollow node
  (`desktop/src/renderer/git/`).
  Since plan 27 G2: while the panel is open and the window is visible, it reads `akra/git/refs` every 3
  seconds and, when the digest differs, reads the same number of rows again and keeps the top visible
  commit in place. A click or the up and down arrow keys select a row; the lower half of the panel then
  shows that commit from `akra/git/commit` (message, author, parents, and the changed files in the shared
  diff view). The find box searches the loaded rows by subject, author, ref label, and sha prefix, dims
  the other rows, and "다음" reads up to 10 further pages of 1,000 rows when the loaded rows have no
  further match. The commit that a worktree has checked out carries a badge with the newest thread whose
  folder is in that worktree; the badge opens the thread. A backup ref has a dashed label, and a commit
  reached only from a backup ref is dim. The panel has no path or branch filter and no maximised layout.
  Since plan 27 G3 the detail of a selected commit has operation buttons: reset the checked-out branch
  to this commit, check out a local branch that is on this commit, check out the commit without a
  branch, and move another local branch here. A button reads a plan from `akra/git/plan` and shows it
  as text (the move, the commits that leave the branch, and the blockers); nothing changes until
  "실행". When the Gateway answers `stale`, the panel shows the new plan and does not run it. A
  finished operation shows one notice line with "되돌리기", and "작업 기록" lists the journal. "실행"
  and "되돌리기" are off while the open thread, or another thread whose folder is in the same
  worktree, has a turn in progress (`desktop/src/renderer/git/GitOperationViews.tsx`). The daemon also
  refuses (#2850): the Gateway remembers, per client connection, each thread's folder and whether its
  turn runs (from the app-server messages it relays), and a plan gets the `turn_running` blocker while
  a thread whose folder is in the target worktree has a turn in progress. So requests from the CLI or
  any other client are refused too. Turns that do not pass through a Gateway connection (the kernel's
  own app-server connection) and threads whose folder the Gateway has not seen are not counted; when
  a connection closes, the Gateway forgets what it saw on it. Operations wait in one queue per
  repository (all worktrees of a repository share it), so a long operation in one repository does
  not hold another. "되돌리기" of a reset or a checkout shows only in the worktree that ran it.
- Planning: `src/application/service/planning/`, `src/domain/planning/`
- Parallel: `src/application/service/parallel_mode/`, `src/domain/parallel_mode/`
- App-server adapter: `src/adapter/outbound/app_server/`
- SQLite authority: `src/adapter/outbound/db/sqlite_planning_authority_adapter.rs`
