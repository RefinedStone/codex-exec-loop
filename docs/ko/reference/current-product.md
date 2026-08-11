# 현재 제품 및 운영 계약

[English](../../reference/current-product.md)

이 문서는 `prerelease`에 구현된 동작의 canonical reference입니다. 미래 작업은 제안 상태를 명시한
문서에 두며 이 문서에 섞지 않습니다.

## 제품 형태

- Akra는 공식 `codex app-server` interface 위에 만든 native-first Rust client입니다.
- Alternate-screen fullscreen TUI가 기본 화면입니다. 앱이 소유하는 transcript viewport 하나가
  대화 이력, streaming row, tool card, overlay, status, composer를 함께 담당합니다.
- Agent text는 app-server state에서 raw Markdown으로 유지하고 TUI projection 경계에서 렌더링합니다.
  fenced code delimiter와 language label은 대화 본문으로 표시하지 않습니다.
- 제출된 운영자 prompt는 `›` 표식 하나만 둔 낮은 명도의 full-width compact card로 표시합니다.
  별도의 `You:` label row를 쓰지 않고 줄바꿈 뒤에도 같은 surface를 유지하며, interaction chrome으로서
  선택되지 않습니다. `Auto Follow-up`처럼 기본값이 아닌 출처는 card 안의 compact inline badge로
  유지합니다.
- `src/core`가 headless app command, effect, completion, event, snapshot을 조정합니다.
- CLI, Admin, Telegram, automation adapter는 planning이나 parallel 정책을 별도로 구현하지 않고
  application service를 공유합니다.

## 운영 화면

| 화면 | 진입 | 구현된 동작 |
| --- | --- | --- |
| Conversation | 기본 | prompt 작성, turn streaming, approval 검토, 연속 작업 |
| Diagnostics | `Ctrl+d`, `:diag` | 시작 준비 상태와 blocker 확인 |
| Sessions | `Ctrl+o`, `:sessions` | 세션 검색·이름 변경·재개 또는 새 draft 시작 |
| Reviews | `:reviews` | 제한된 review center projection 확인 |
| Activity | `:activity [all\|diff\|output\|command\|patch\|…]`, `:act` | 보존된 progressive activity 카드 목록·선택 상세 확인; 줄 번호 unified diff를 표시하며 read/explore와 patch 상세는 실제 대화에서도 펼칠 수 있음 |
| Queue | `:queue`, `:q`, `akra queue` | 승인된 head, proposal, skip, receipt 확인 |
| Planning | `:planning`, `:planning-init` | planning 변경 staging·검증·승격 |
| Directions | `:directions` | direction과 queue-idle 지원 자료 관리 |
| Health | `:doctor`, `:planning doctor`, `akra doctor`, `akra status` | 작성 없이 planning 권한 상태 확인 |
| Configuration | `akra config` | 유효값과 origin 조회, 전역 또는 Git-worktree TOML 설정의 안전한 변경 |
| Parallel | `:parallel`, `:pa` | 자동화 활성화/갱신과 supervisor board 열기 |
| Parallel peek | `:peek` | 활성 병렬 agent 대화 확인 |

전역 키에는 종료 `Ctrl+q`, 새 draft `Ctrl+t`, 현재 inspection을 닫는 `Esc`/`Ctrl+c`가 있습니다.
Modal이 focus를 소유할 때는 전역 키보다 우선할 수 있으며, 표시되는 키 안내는 실제 controller
경로와 일치해야 합니다.

## Shell 명령 registry

원본 registry는 `src/adapter/inbound/tui/app/inline_shell_commands.rs`입니다.

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
:planning [doctor]
:doctor
:reset <queue|directions|all>
:new
:help
```

`:model`은 `모델 → 추론` 순서의 단계형 선택기를 엽니다. 실제 활성 provider만 표시하고,
선택한 모델이 지원하는 추론 수준과 그 추천값만 보여 줍니다. OpenAI 목록은 GPT-5.6 Sol,
Terra, Luna로 시작하며 GPT-5.6 행에는 `max`가 있고 `minimal`은 없습니다.

초기 interactive 기본값은 GPT-5.6 Sol과 `medium` effort입니다. `:model`, `:think`는 현재 선택을
즉시 바꾸고 다음 새 thread의 기본값을 전역 설정에 저장합니다. 활성 thread에는 선택한 model과
effort도 독립적으로 남깁니다. Akra 저장 options가 없는 기존 또는 외부 thread를 다시 열 때는 새 전역
값을 적용하지 않고 app-server에 두 필드를 unset으로 보냅니다. 계층 파일 계약과 CLI는
[설정 참조](configuration.md)를 참고하세요.

`:turns`는 단일 세션 auto-follow를 제어합니다. Parallel 자동화는 별도의 명시적 opt-in입니다.
`:stop`은 활성 app-server 세션을 중지하고 parallel epoch를 닫으며 두 continuation 경로를 모두
해제합니다. 이후 `:parallel`은 parallel continuation만 다시 활성화합니다.

## Turn과 approval 흐름

1. 시작 진단 중에도 입력을 작성할 수 있지만 제출은 준비 상태를 기다립니다.
2. Core는 한 번에 하나의 turn submission만 승인합니다. Accepted dispatch는 정확히 하나의 worker
   effect를 발행하고, TUI가 editor를 비우고 transcript history에 prompt를 추가하도록 허용합니다.
3. `Tab`으로 정확한 활성 turn에 전달할 내용을 확인할 수 있습니다. Core는 correlation된 steer
   worker 하나만 승인하고 provider 확인 전까지 draft를 유지하며 stale completion을 버립니다. 이후
   편집했거나 같은 문구를 다시 입력한 draft는 이전 확인 응답으로 지우지 않습니다.
4. Streaming assistant 출력은 canonical `item_id` row를 제자리에서 갱신합니다. Tool row는 도착
   시점에 추가되므로 늦은 최종 assistant event가 transcript 순서를 바꾸지 못합니다.
5. Typed activity, runtime notice, approval, warning, 펼칠 수 있는 tool card는 같은 fullscreen
   projection을 갱신합니다.
6. Post-turn 평가는 승인된 planning 상태에 따라 continuation을 진행·일시정지·종료합니다.

기본 app-server 실행 profile은 `approval=never`, `reviewer=none`,
`sandbox=danger-full-access`입니다. Main conversation, 재개 thread, hidden planning worker,
parallel worker의 thread/turn 경계가 모두 같은 profile을 사용합니다. Repository-local 설정이 Akra의
process 계약을 몰래 바꾸지 못하도록 project는 app-server 설정에서 계속 untrusted로 표시합니다.
Diagnostics에는 활성 profile이 항상 보이지만, 의도된 기본값 자체를 degraded warning으로 만들지는
않습니다.

Codex 탐색은 spawn 전에 absolute launcher, native interpreter, canonical package script를 계속
고정합니다. Ownership, ACL, repository boundary trust 실패는 기본적으로 startup을 막지 않고
warning으로 남지만 launcher shape, native format, absolute path, package target 검증은 유지합니다.
Mutation 권한이 있는 principal은 permissive하게 허용된 path를 spawn 전에 교체할 수 있습니다.
`AKRA_REQUIRE_TRUSTED_CODEX_EXECUTABLE=1`을 설정하면 이 노출을 차단하는 기존 strict fail-closed
trust boundary를 복원합니다. 값이 없거나 정확히 `0`이면 permissive 기본값을 유지하고, 그 외 값은
잘못된 설정으로 startup을 거부합니다.

제한된 배포는 `CODEX_EXEC_LOOP_APP_SERVER_APPROVAL_POLICY`,
`CODEX_EXEC_LOOP_APP_SERVER_APPROVALS_REVIEWER`,
`CODEX_EXEC_LOOP_APP_SERVER_SANDBOX_MODE`로 권한을 낮출 수 있습니다. Approval을 다시 켠 경우에만
상호작용 가능한 main conversation이 검토 가능한 명령 또는 제한된 추가 권한 요청에 답할 수
있습니다. `Y`는 한 번 승인하고 `N`/`Esc`는 거부하며 `Enter`는 아무 동작도 하지 않습니다. Timeout,
interrupt, disconnect, 잘못된 payload, 검토할 수 없는 file-change 요청, unattended worker는 모두
fail-closed입니다. 세션 전체 grant는 저장하지 않습니다.

## Planning 계약

- Planning은 `draft -> validate -> promote` 순서를 따릅니다.
- `PlanningTaskRepositoryPort` 뒤의 SQLite가 승인된 task, direction, queue, claim, runtime 권한입니다.
- `.codex-exec-loop/planning/`은 운영자 상세 문서, prompt, staged draft, export, 거부 근거를
  보관하며 task-authority DB가 아닙니다.
- Builtin `next-task`와 내부 continuation은 승인된 queue head만 실행합니다.
- Proposed task는 보이지만 승인 전에는 실행할 수 없습니다.
- Queue-idle 동작은 승인된 direction authority를 따릅니다.
- Admin/API intake는 검증된 `ready` task 하나를 만들며 `in_progress` task를 중단하지 않습니다.
- `akra planning-tool`은 automation caller의 구조화된 list/create/update 경계입니다.
- Hidden planning worker는 활성 app-server 실행 profile을 상속합니다. Developer 계약은 계속
  planning-only이며 host가 구조화 명령을 검증하고 적용합니다.

Git과 non-Git workspace 모두 `${AKRA_HOME:-~/.akra}/projects/<project>/runtime/planning-authority.db`
아래의 권한 저장소를 사용합니다. Git 저장소는 canonical Git common directory의 private incarnation
marker로 linked worktree를 공유하고 독립 clone을 분리합니다.

## Parallel 계약

- 인자 없는 `:parallel`/`:pa`가 enable-or-refresh 진입점이며 `:parallel on`은 없습니다.
- 처음 off에서 on으로 들어갈 때 readiness 확인, 보호된 3-slot pool reconcile, automation epoch
  생성, idle capacity만큼의 승인된 ready work dispatch를 수행합니다.
- 이미 활성화된 상태에서 다시 실행하면 readiness와 projection만 갱신합니다. Pool reset이나 두 번째
  epoch 생성은 하지 않습니다.
- `:parallel off`는 로컬 자동화를 중지하고 늦게 도착한 dispatch 결과를 무효화하지만 worktree는
  보호된 복구를 위해 남깁니다. Board를 닫는 것만으로 parallel mode가 꺼지지는 않습니다.
- Pool, lease, task, session, distributor mutation은 application과 durable cross-process gate를
  통과합니다. TUI state는 capacity, retry, dispatch 정책을 결정하지 않습니다.
- Worker는 활성 app-server 실행 profile을 상속합니다. 기본값은 approval prompt가 없는 unattended
  `danger-full-access`이고 변경을 commit하지 않습니다. 제한 profile에서 발생한 approval 요청은
  unattended connection이 계속 거부합니다.
  Host가 정확한 lease, worktree, branch, 고정 base, 변경 파일 제한, 최종 clean 상태를 확인한 뒤
  source commit을 만듭니다.
- 전달은 source push, PR 자동화/검토 확인, 정확히 검토된 범위의 integration branch 통합, remote
  확인, identity-checked cleanup 순서로 직렬 실행됩니다.
- Remote, repository, visibility, integration branch, base OID, source SHA, reviewed range는 전달
  전에 고정합니다. Drift가 생기면 mutable checkout 상태로 fallback하지 않고 차단합니다.
- 기본은 사람 review 필수입니다. Public repository 또는 autonomous delivery는 parent process의
  정확한 opt-in이 필요하며 repository config로 권한을 부여할 수 없습니다.

Board는 readiness, pool slot, active roster, 선택한 lifecycle, distributor head, queue 상태,
dispatch 보류 사유를 읽기 전용으로 보여줍니다.
Focused board도 같은 alternate-screen fullscreen frame transaction을 사용합니다. Parallel event는
앱이 소유하는 하나의 stream model로 렌더링하며 host history insertion이나 durable/live 분할을
사용하지 않습니다. 높이가 부족하면 title만 숨기고 event 순서와 row는 유지합니다.
`:peek`은 활성 agent 대화를 읽기 전용으로 미리 보여줍니다. Agent나 overlay를 바꾸면 늦게 도착한
결과가 최신 preview 또는 interactive conversation을 교체하지 못합니다.

## Admin 게임 프런트엔드

로컬 Admin 그래픽 dashboard는 같은 planning·parallel fact를 수동적으로 보여 주지만, harness
control은 실제 application command입니다. CSRF로 보호된 browser action은 하나의 persistent
`ParallelModeControlPlaneHandle`을 통해 automation epoch 활성화, 다음 accepted task 요청,
supervisor refresh, loop 비활성화를 수행합니다. HTTP adapter가 Git, GitHub, pool, distributor
adapter를 직접 호출하지 않습니다.

Post-merge PR 검증은 Admin/TUI refresh loop가 아니라 영속 application scheduler입니다. 저장소 로컬
`akra.prValidationMode`가 없으면 `observe`가 기본이므로 provider evidence와 정확한 Actions target
SHA는 기록하지만 자동 Queue admission은 차단합니다. `remediate`는 typed actionable finding만 기존
Planning Queue와 일반 worker lease/worktree 수명주기로 admission하고, `off`는 history를 삭제하지
않고 새 polling만 중지합니다. Admin Validation Rail은 Integrated, Verifying,
RemediationQueued, RemediationRunning, Verified, Blocked, Failed를 구분하고 rollout banner에서 실제
mode, Queue admission, Ruleset 별도 승인 경계를 표시합니다. 자세한 운영 절차는
[PR 검증 운영 전환 runbook](pr-validation-rollout.md)에 있습니다.

Rollout timing evidence는 별도 read path를 사용합니다. Application query service가 collector schema,
repository/base identity, generated time, evidence SHA, 48시간 freshness 경계를 검증한 뒤 actual과
historical Fast Gate 분포, CI/Post-Merge timing, 표본 범위, quota, typed warning을 projection합니다.
Repository authority는 redacted snapshot 최대 64개와 정확한 collection lease/CAS 상태를 보관합니다.
Dashboard bootstrap에는 최신 summary만 들어가고 evidence drawer가 cursor로 안정 정렬된 history를
최대 20개까지 lazy load합니다. SSE는 evidence revision invalidation만 전달합니다. Durable replay는
freshness를 다시 계산하며 source/store 장애가 발생해도 마지막 유효 read-only projection으로
degrade하고 parallel control plane은 중단하지 않습니다.

`/admin/akra`의 PixiJS 8 world는 같은 dashboard snapshot을 사용합니다. 검증된 frontend store,
semantic camera zoom, worker 이동, 가구 occlusion, scene selection은 presentation에만 속합니다.
상세 계약은 [Admin 게임 프런트엔드](admin-game-frontend.md)에 있습니다.

`akra admin --debug-harness`는 같은 dashboard 계약 뒤에서 application이 소유하는 비영속 Fake
scenario clock을 시작합니다. Normal delivery, blocked recovery, queue pressure를 실제 planning,
parallel, Git, GitHub 권위를 바꾸지 않고 재생하며 PR 검증 복구 시나리오 열 개도 제공합니다.

## 복구와 제한

- 잘못되거나 충돌한 planning update는 continuation을 멈추고 검토 근거를 보존합니다.
- Store-backed claim으로 재시작 후 공식 refresh와 distributor 복구가 가능하지만 operator 소유
  integration history를 hard reset하지 않습니다.
- Non-Git workspace는 planning authority를 사용하지만 Git worktree pool은 사용하지 않습니다.
- Planning 상세 작성은 수동이며 `llm-assisted` editor 경로는 비활성화되어 있습니다.
- Primitive-sensitive TUI 변경과 관련 restart/distributor/multi-worktree 흐름은 실제 terminal
  evidence가 필요합니다.

## 코드 진입점

- Core: `src/core/`
- TUI: `src/adapter/inbound/tui/`
- CLI: `src/adapter/inbound/cli.rs`
- Admin: `src/adapter/inbound/admin_api/`
- Telegram: `src/adapter/inbound/telegram_bot/`
- Planning: `src/application/service/planning/`, `src/domain/planning/`
- Parallel: `src/application/service/parallel_mode/`, `src/domain/parallel_mode/`
- App-server adapter: `src/adapter/outbound/app_server/`
- SQLite authority: `src/adapter/outbound/db/sqlite_planning_authority_adapter.rs`
