# Orca에서 Akra로의 갭 매트릭스

[English](../../../competitive/orca/gap-matrix.md)

이 매트릭스는 `6013055491943336660e12e5dec93c9ece4575bb`의 Orca v1.4.137과
`6274a7fc9703f85e4fb6247541dc0d4ed6c5fb9b`의 Akra 1.3.5를 비교한다. 전체 점수가 아니라
제품 기준 decision source다. 증거와 한계는 [evidence.md](evidence.md)에 있다.

## 상대 매트릭스

| 역량 | Orca v1.4.137 | Akra `6274a7fc` | 결정 | 우선순위 | 증거 |
| --- | --- | --- | --- | --- | --- |
| 제품 권한 | desktop ADE가 agent CLI, worktree, terminal, editor/browser, source control 조정 | 공식 `codex app-server` 기반 Codex-first application 및 delivery authority | Akra 명제 유지, operator pattern만 도입 | invariant | [topology](evidence.md#product-release-and-topology), [Akra](evidence.md#akra-baseline-evidence) |
| Worktree aggregate | worktree가 branch, agent, terminal, file, diff, PR, lineage를 결합한 top-level task workspace | accepted task, lease, official session, source, delivery record가 authority, slot은 resource | task authority를 대체하지 않고 portfolio 노출 | P1 | [model](evidence.md#worktree-discovery-and-ownership) |
| Repository inventory | managed, external, legacy worktree, Git-authoritative refresh 및 import visibility | board는 generated three-slot pool과 integration state만 join | repository-wide read model 추가 | P1 | [runtime observation](evidence.md#audit-environment-and-privacy), [Akra](evidence.md#akra-baseline-evidence) |
| Base 선택 | local/remote branch 또는 commit, offline fallback 포함 refresh | fetched configured integration branch/OID가 automation baseline | automation에는 strict remote proof 유지, arbitrary base는 read-only display 허용 | invariant/P1 | [creation](evidence.md#creation-bootstrap-and-metadata) |
| 생성 및 collision UX | dynamic path/branch creation, bounded suffixing, branch reuse, PR start point, sparse mode | fixed generated sibling path, detached baseline slot, generated agent branch | fixed pool을 대체하지 않고 미래 manual action을 위해 explicit conflict에서 학습 | shipped/contingent | [creation](evidence.md#creation-bootstrap-and-metadata), [Akra](evidence.md#akra-baseline-evidence) |
| 소유권 | managed/external/unknown 및 instance metadata와 provenance | generated pool path, branch prefix, exact lease generation, canonical/link-free identity | product ownership과 authority generation을 모두 projection | P1 | [ownership](evidence.md#worktree-discovery-and-ownership), [cleanup](evidence.md#akra-baseline-evidence) |
| Bootstrap | worktree `orca.yaml`의 visible terminal setup, sparse/shared path | worker는 pre-provisioned slot에서 실행, host가 Git execution config audit 및 commit 소유 | unattended authority에서 implicit repository-controlled setup 거부 | invariant | [bootstrap](evidence.md#creation-bootstrap-and-metadata) |
| 상태 표시 | sidebar 및 branch, agent, terminal, diff, PR, comment, lineage, external state | TUI board가 readiness, pool, roster, selected lifecycle, queue, delivery boundary 표시 | repository portfolio detail 추가, task/delivery truth는 primary 유지 | P1 | [analysis](analysis.md#worktree-lifecycle), [Akra](evidence.md#akra-baseline-evidence) |
| Worktree jump 및 resume | worktree-scoped tab/layout/session, CLI selector, local/WSL/SSH host scope | session browser는 current workspace context에서 공식 thread resume, general worktree jump 없음 | workspace/thread authority가 explicit해진 뒤에만 추가 | P2 | [continuity](evidence.md#terminal-session-and-remote-continuity) |
| 병렬 capacity | dynamic human-created worktree, experimental coordinator concurrency | durable dispatch 및 capacity decision이 있는 fixed three-slot pool | bounded unattended capacity로 차별화 | shipped | [automation](evidence.md#automation-permissions-and-telemetry), [Akra](evidence.md#akra-baseline-evidence) |
| 격리 | separate branch/directory, Manual/agent override가 바꾸지 않으면 새 agent launch에 high-autonomy argument 미리 입력 | separate slot 및 workspace-write worker, unattended approval decline, scrubbed environment, audited Git boundary | worktree-as-security-sandbox 거부 | invariant | [permissions](evidence.md#automation-permissions-and-telemetry) |
| Dirty, lock, nested state | lock parse, normal dirty/untracked removal 차단, nested/unsafe target 거부 | reset/cleanup은 dirty, untracked, pending, non-integrated, identity-mismatched state 차단, exact post-integration cleanup만 ignored build output purge 가능 | Akra fail-closed check 보존, compact reason vocabulary 도입 | shipped/P1 UX | [Orca removal](evidence.md#removal-and-recovery), [Akra](evidence.md#akra-baseline-evidence) |
| Destructive ordering | target 재해석, lock/dirty 확인, scoped PTY/watcher close 시도 후 remove, close failure는 best effort | pool cleanup은 lease/path/branch/integration 입증, arbitrary PTY는 소유하지 않음 | preflight ordering 사용, Akra에 explicit fail/continue policy 선택 | contingent | [removal](evidence.md#removal-and-recovery) |
| Branch 보존 | `branch -d`가 moved/unpublished history 보존, 별도 이후 preserved-branch action은 expected-`HEAD` CAS 가능, forced worktree removal 자체에는 적용하지 않음 | local/remote cleanup이 frozen identity 비교하고 moved branch 보존 | action boundary는 다르지만 동일 safety principle, Akra proof 유지 | shipped | [Orca removal](evidence.md#removal-and-recovery), [Akra](evidence.md#akra-baseline-evidence) |
| External mutation 및 orphan recovery | import/reconcile, stale registration, orphan provenance, Windows partial deletion | conservative pool reconciliation이 unproved path 보존하고 split brain 차단 | external projection 먼저 추가, auto-import/delete 금지 | P1 | [recovery](evidence.md#removal-and-recovery) |
| Base drift | post-create base reconciliation 및 on-demand behind probe, unknown은 non-blocking이고 SSH probe는 unknown | 새로 진행된 remote baseline은 mutation 차단, frozen target 및 exact range가 delivery gate | automation authority는 Akra가 앞섬 | shipped | [Orca drift](evidence.md#base-and-drift-reconciliation), [Akra](evidence.md#akra-baseline-evidence) |
| Commit/source freeze | interactive worktree 및 provider state | host-owned bounded commit과 exact source SHA 및 1-128 linear commit range | 차별화 | shipped | [Akra](evidence.md#akra-baseline-evidence) |
| PR 및 review | 강력한 interactive commit/push/draft PR/check/review/comment/merge path | default approval/check/clean/matching-head gate 및 serialized integration | deep-link UX 도입, Akra authority 보존 | shipped plus existing delivery work | [Orca PR](evidence.md#git-hosted-review-and-delivery), [Akra](evidence.md#akra-baseline-evidence) |
| Cleanup 완료 | safe local branch handling이 있는 workspace/archive lifecycle | remote integration verification, PR close, source compare-and-delete, slot baseline cleanup | terminal delivery proof로 차별화 | shipped | [Orca removal](evidence.md#removal-and-recovery), [Akra](evidence.md#akra-baseline-evidence) |
| Remote workspace | first-class SSH worktree, relay PTY/file/diff, non-authoritative offline fallback | local fixed pool, Admin/Telegram은 remote workspace가 아닌 control surface | generic SSH IDE lane을 시작하지 않고 product demand가 있을 때만 future node contract 재사용 | reject/current | [remote continuity](evidence.md#terminal-session-and-remote-continuity) |
| 성능 | 비교 가능한 packaged process-tree sample 없음 | 비교 가능한 versioned process-tree/worktree sample 없음 | 승자 불명, 기존 evidence contract 수정 | existing P0 | [limits](evidence.md#audit-limits), [existing owner](../jcode/gap-matrix.md#p0-native-performance-evidence-contract) |
| 릴리스 품질 | 광범위한 worktree-focused static test, 이 감사에서는 미실행 | 폭넓은 Rust test 및 architecture gate, exact release validation은 portfolio 작업으로 남음 | 모든 claim을 exact source 및 raw validation에 결합 | existing P0 | [quality](evidence.md#quality-evidence), [existing owner](../jcode/gap-matrix.md#p0-frozen-source-validation-evidence) |

## 포트폴리오 규칙

Orca는 Akra에 하나의 실제 새 product gap을 제공한다. repository-wide worktree fleet visibility다.
다른 교훈은 병렬 backlog 이름을 만들지 않고 기존 작업을 수정한다.

| Orca 교훈 | 소유 중인 Akra 항목 | 수정만 수행 |
| --- | --- | --- |
| worktree/terminal/diff/PR status를 하나의 drilldown으로 유지 | [Protocol-Native Live Execution Rail](../jcode/gap-matrix.md#p0-protocol-native-live-execution-rail) 및 [Authoritative Delivery Outcomes](../jcode/gap-matrix.md#p1-authoritative-delivery-outcomes) | worktree ownership 및 path/branch link field 추가, runtime activity와 delivery authority를 병합하지 않음 |
| worktree-scoped agent/session lineage | [Official Session Search And Provenance](../jcode/gap-matrix.md#p2-official-session-search-and-provenance) | explicit workspace identity와 missing/moved workspace state 포함 |
| scheduled 및 event-driven launch | [Automation Intake Core](../agent-canvas/gap-matrix.md#p1-automation-intake-core) | idempotent typed intake 및 lease acquisition 유지, Orca-specific automation lane을 만들지 않음 |
| create/list/switch/remove 성능 | [Native Performance Evidence Contract](../jcode/gap-matrix.md#p0-native-performance-evidence-contract) | exact worktree-count, cold/warm Git state, disk, process-tree, raw sample 추가 |
| 빠른 release에도 exact stable tag 사용 | [Frozen Source Validation Evidence](../jcode/gap-matrix.md#p0-frozen-source-validation-evidence) | worktree lifecycle test 및 capture를 release SHA에 결합 |
| command-addressable worktree selector | [Contextual Command Discovery](../opencode/gap-matrix.md#3-contextual-command-discovery) | Akra 기존 registry를 통해 availability 및 authority reason 노출, 두 번째 command system을 추가하지 않음 |

새 P0는 없다. Orca는 기존 live rail, recovery, automation, performance, validation, delivery owner를
대체하지 않는다. Akra의 현재 three-slot automation은 새 inventory 없이도 올바르므로 새 inventory는
P1에서 시작한다.

## 도입

### 1. P1 권한 있는 워크트리 포트폴리오 읽기 모델

현재 repository의 모든 worktree를 위한 read-only application model을 추가한다. TUI-only `git` call로
존재해서는 안 된다.

**소유 경계**

- repository/worktree identity, observation generation, field-group freshness를 위한 domain DTO;
- authoritative Git inventory 및 bounded status inspection용 outbound port;
- porcelain parsing 및 platform path normalization용 Git adapter;
- Git inventory를 기존 parallel authority의 current managed slot 및 integration identity에만 join하는
  애플리케이션 서비스;
- typed TUI projection. Session/activity 및 delivery owner는 나중에 optional typed link를 제공할 수 있다.
  이 slice는 그 state를 rebuild하거나 persist하지 않는다.

**최소 필드**

- raw path를 remote DTO에 노출하지 않고 canonical common Git directory에서 도출한 repository identity;
- Git worktree identity는 tagged result다. `resolved`는 canonical Git-reported path와 resolved
  per-worktree Git directory를 결합한다. `unresolved`는 bounded Git-reported spelling, 가능한 경우
  common-directory admin-entry evidence, explicit failure reason만 유지한다. display spelling을 조용히
  identity로 승격하지 않는다;
- refresh 전에 시작한 response가 더 새로운 snapshot을 대체하지 않도록 monotonic observation generation과
  관찰 시각;
- 별도의 `registration_authority`, `status_freshness`, `managed_binding` field. live Git inventory,
  timed-out status inspection, exact SQLite lease-generation match는 하나의 boolean이 아니다;
- 소유권: `primary`, `managed_slot`, `managed_integration`, `observed_external`, `unknown_legacy`.
  일반 등록만으로 사람 소유자를 입증하지 않는다;
- Git registration, branch 또는 detached state, `HEAD`, lock 및 lock reason;
- staged, unstaged, untracked, ignored, pending-operation, submodule summary. inspection이 bounded이거나
  실패하면 explicit unknown value 사용;
- 가능한 경우 matching Akra slot과 exact lease generation 또는 integration identity. Task, activity,
  official session, source, PR, delivery detail은 기존 application projection이 소유하는 optional link로
  남는다;
- arbitrary terminal 또는 prompt content 없음.

**초기 상한**

- refresh마다 inventory record 최대 512개 및 NUL-delimited Git output 8 MiB;
- record마다 encoded path data 최대 32 KiB;
- refresh마다 status inspection worktree 최대 64개, selected 및 Akra-managed row 우선, in-flight command
  최대 8개;
- inspected worktree마다 최대 1 MiB 또는 status record 50,000개, refresh 하나에 retained status output
  16 MiB;
- status command마다 2초, aggregate refresh 10초;
- explicit `truncated`, `not_inspected`, `timed_out`, `failed` state. partial/truncated result는 유용한
  orientation이지만 future action의 authoritative proof가 아니다.

**안전 계약**

- Git은 registration authority이고 SQLite는 lease/task/delivery authority로 남는다;
- canonical, link-free identity 없는 path match는 Akra ownership을 부여하지 않는다;
- unresolved missing/prunable row는 Git common-directory admin evidence에서 표시할 수 있지만 start,
  resume, import, reset, prune, deletion을 authorize할 수 없다;
- list와 refresh는 read-only이며 provision, reset, import, prune, kill, delete하지 않는다;
- optional lock을 끈 기존 pinned 및 scrubbed host-Git boundary를 사용한다. inspection은 bounded
  list/status/identity command만 실행할 수 있고 repository hook, filter, tool, credential helper,
  prompt를 실행할 수 없다;
- unavailable 또는 truncated Git inventory를 live registration authority로 나타낼 수 없다;
- bounded inspection failure는 row를 drop하지 않고 row별로 visible하게 남는다;
- 외부에서 제거된 worktree는 다음 complete authoritative refresh에서 사라진다. 이 slice는 arbitrary
  external path의 tombstone을 저장하지 않는다. missing managed slot은 exact SQLite/pool owner가 missing으로
  계속 projection할 때만 visible하게 남을 수 있다;
- 기본적으로 private absolute path를 remote/Telegram projection에 넣지 않는다. Admin과 TUI는 기존
  authenticated/local boundary 아래에서 local display path를 보여 줄 수 있다.

**증명**

- 기본 체크아웃, 분리 슬롯, 통합, 일반 브랜치, 외부 워크트리, 잠긴 워크트리, 줄 바꿈/공백 경로,
  별도 Git 디렉터리, 심볼릭 링크/정션 별칭, 오래된 등록 픽스처;
- integration test가 Akra 밖에서 external worktree를 만들고 제거하여 process restart, tombstone creation,
  pool mutation 없이 refresh가 이를 추가한 뒤 제거함을 입증;
- missing managed slot은 exact authority-backed blocked row로 남지만 arbitrary missing external path는
  그렇지 않음;
- limit, timeout, concurrency, partial-result, stale-generation, cancellation test가 모든 initial bound를
  실행;
- generation replacement test가 stale row를 새 slot lease에 bind할 수 없음을 입증;
- before/after capture가 list와 refresh 전반에서 ref, worktree index, repository/worktree config,
  SQLite pool authority가 byte-identical함을 입증;
- architecture gate가 inbound adapter가 Git adapter가 아닌 service projection에 의존함을 입증.

### 2. P1 워크트리 포트폴리오 TUI

read model이 green이 된 뒤 기존 command registry에서 하나의 compact searchable overlay를 추가한다.
file browser가 되지 않고 네 가지 질문에 답해야 한다.

1. 어떤 worktree가 존재하는가?
2. 어느 것이 단순 관찰 대상이고 어느 것이 Akra 소유인가?
3. 현재 출시된 managed lifecycle state 중 어느 것이 각 managed worktree와 연결돼 있는가?
4. worktree가 stale, dirty, locked, blocked이거나 manual inspection에만 안전한 이유는 무엇인가?

default row는 branch/detached identity, ownership, dirty/lock marker, pool projection에서 이미 사용할 수 있는
coarse managed lifecycle을 담아야 한다. selected detail은 local display path, exact `HEAD`, lease generation,
status breakdown, recovery guidance를 보여 줄 수 있다. Optional activity, session, source, PR, delivery link는
기존 owning projection이 제공할 때만 render하고, 아니면 UI가 unavailable이라고 알린다. current overlay/list
및 parallel detail composition을 재사용하고 general tree widget framework를 만들지 않는다.

첫 slice는 inspect-only다. 이미 출시된 parallel detail을 열거나 local path를 copy/show하거나 safe command를
print할 수 있다. 공식 session, active app-server workspace, shell cwd의 선택 또는 변경은 thread context와
runtime ownership을 바꿀 수 있으므로 별도 P2로 남는다.

**증명**

- managed, external, locked, dirty, stale row가 섞인 narrow/wide snapshot;
- keyboard search/selection 및 exact command-registry availability test;
- 외부에서 생성한 worktree가 refresh에 나타나는 real-terminal capture;
- overlay를 open, refresh, search, close하는 동안 ref, index, Git config, pool authority가 byte-identical함.

### 3. P1 Admin 워크트리 포트폴리오 투영

application read model이 안정화된 뒤 같은 service projection에서 별도 read-only Admin DTO와 endpoint를
노출한다. handler는 authentication 및 presentation만 소유한다.

- 모든 field를 allowlist하고 raw absolute path 대신 bounded local label을 기본 반환한다;
- JSON에서 registration, status, managed-binding freshness를 별도로 유지한다;
- truncated 및 unavailable inventory를 명시적으로 나타낸다;
- optional session/delivery link는 owning projection이 출시한 뒤에만 노출한다;
- valid-session, missing/invalid capability, origin, path-redaction, truncation, no-mutation API test를
  추가한다;
- import, reset, prune, terminal, deletion action을 추가하지 않는다.

### 4. P2 워크트리 범위 세션 이동

Orca식 session database를 만들지 말고 기존 session provenance owner를 확장한다. 이 slice는 먼저 새
ownership class가 아닌 interactive-target capability를 추가한다.

- `primary`는 interactive start/resume에 eligible하다;
- `managed_slot`은 exact parallel lease가 계속 소유하고 이 action으로 진입할 수 없다;
- `managed_integration`, `unknown_legacy`, unresolved, missing, remote row는 절대 eligible하지 않다;
- `observed_external`은 local operator가 full current repository/worktree identity를 명시적으로 선택할 때까지
  inspect-only다. 해당 action은 observation generation에 bind된 one-request, process-local capability를
  mint한다. deletion, setup, automation, lease, durable ownership은 부여하지 않는다;
- 사용 직전에 service가 identity resolution을 다시 실행하고 변경된 generation, path, Git directory,
  repository, registration state를 거부한다.

start와 resume은 서로 다른 authority를 갖는다.

- 선택한 current worktree cwd를 기존 start request에 전달한 뒤 applied cwd를 다시 검증해 새 official
  thread를 시작한다;
- linked thread의 official persisted cwd를 기존 resume path로 load하고 그 cwd가 link/reparse escape 없이
  current inventory identity에 canonical하게 map되도록 요구해 resume한다. selected row가 thread의 persisted
  cwd를 override하지 않는다;
- 아무것도 시작하지 않고 ineligible workspace를 inspect한다.

current conversation을 조용히 `chdir`하거나 live worker의 target을 바꾸거나 다른 checkout에서 만든 thread를
reinterpret하지 않는다. worktree path는 context이지 thread authority가 아니다.

**증명**

- source 및 target workspace가 서로 다른 official thread ID와 transcript history를 유지한다;
- managed slot/integration 및 unresolved/unknown row는 interactive-target capability를 mint할 수 없다;
- external capability는 single-use, generation-bound이며 Git, pool, delivery mutation을 authorize할 수 없다;
- moved/deleted selected worktree는 start를 차단하고 missing/mismatched persisted thread cwd는 history를
  inspectable하게 유지하면서 resume을 차단한다;
- approval 및 interrupt request는 exact official thread/turn에 bind된 상태로 유지되고 다른 worktree의
  runtime을 resolve할 수 없다.

### 5. 조건부 수동 폐기 계약

read model과 함께 delete/import/archive button을 추가하지 않는다. 이후 operator use case가 Akra-owned
manual retirement를 정당화하면 다음 순서를 갖는 별도 reviewed slice를 사용한다.

```text
refresh authoritative Git inventory
-> prove canonical target and explicit Akra/manual ownership
-> reject primary, nested, linked/aliased, locked, dirty, pending, or unknown state
-> capture expected branch HEAD and confirm the exact consequence
-> stop only resources proven to belong to that worktree
-> ask Git to remove it
-> delete a branch only with safe ancestry or expected-HEAD CAS
-> preserve moved/unpublished history
-> reconcile and persist a terminal outcome
```

path가 generated처럼 보인다는 이유로 orphan을 recursively delete하지 않는다. 검증된 Git backlink,
common admin entry, application provenance, canonical path identity, bounded recovery action이 모두 필요하다.
Windows partial removal과 Git registration prune은 서로 다른 outcome이어야 한다. 이는 이 분석에서 수용한
implementation work가 아니라 미래 safety contract다.

## 거부

### 보안 샌드박스로서의 워크트리

cwd가 워크트리라는 이유만으로 승인/샌드박스 우회 플래그를 전달하지 않는다. Akra의 주 세션 승인
투영, 무인 거부 정책, 자식 환경 필터링, 유효 Git 설정 감사, 고정된 실행 파일, 고정되고 격리된 전달
컨텍스트를 보존한다. 워크트리는 자격 증명, 프로세스, 네트워크, 훅, 필터, 공유 Git 상태를 가두지
않는다.

### 저장소 제어 무인 초기 구성

worker가 slot을 소유하기 전에 `orca.yaml`식 setup, hook, package script, arbitrary command를 자동 실행하지
않는다. Akra에 나중에 bootstrap이 필요하면 explicit operator policy, immutable command identity, audited
environment, bounded time/output, visible provenance, failure 시 authority mutation 금지가 필요하다. 현재
fixed reusable pool은 이 필요 대부분을 피한다.

### 자유 형식 또는 클라이언트 메타데이터 권한

comment, label, color, pane name, terminal output, agent self-report는 operator context를 개선할 수 있지만
task completion, lease ownership, review, integration, cleanup proof가 될 수 없다. typed DB authority와
official/runtime evidence를 유지한다.

### 기본 자동화로서의 동적 용량

관찰한 모든 developer worktree를 automation lane으로 바꾸지 않는다. fixed capacity는 resource demand,
lease generation, cleanup, serialized delivery를 audit 가능하게 한다. external worktree는 명시적 미래 import
contract가 owner와 target을 입증할 때까지 read-only로 남는다.

### 인터페이스 및 공급자 경쟁

이 분석에 대한 대응으로 Orca의 Electron IDE, generic browser/editor, mobile control, multi-agent launcher,
computer-use surface, SSH relay를 복사하는 것을 거부한다. Akra의 TUI, app-server runtime, Admin, CLI,
Telegram, automation은 계속 application service를 공유해야 한다. Remote node 작업은 별도로 정당화되며
read-only가 우선이다.

### 대화형 Git 상태를 전달 권한으로 사용

고정된 소스/대상 식별 정보, 검토/검사 관문, 분리 통합, 원격 검증, 비교 후 삭제 정리를 “현재
워크트리를 푸시할 수 있어 보인다” 또는 공급자 기본 병합 동작으로 대체하지 않는다. Orca의 PR UX는
링크와 진단에 영감을 줄 수 있지만 Akra 분배기가 최종 권한으로 남는다.

## 차별화

### 동적 워크스페이스 UX와 제한된 자동화 권한

Orca는 arbitrary agent를 fan out하고 살아 있는 workspace 사이를 이동하려는 developer에게 최고일 수 있다.
Akra는 accepted Codex task가 bounded capacity를 사용하고 reviewed, identity-proven remote outcome에 도달하기를
원하는 operator에게 최고여야 한다. portfolio view는 이 구분을 지우지 않고 UX gap을 연결한다.

### 고정된 검토 결과

모든 managed-worktree detail에서 더 강한 Akra 증거를 읽기 쉽게 만든다.

- 정확한 임대 세대 및 현재 수명 주기 소유자;
- 고정된 통합 기준점, 소스 브랜치, 소스 SHA/범위;
- 공식 완료와 호스트 커밋 준비 상태의 구분;
- 푸시, PR, 승인/검사/병합 가능성, 통합, 원격 검증, 정리 결과;
- 검토 기본값을 우회했을 때 명시적인 고위험 자율 정책 출처.

Orca는 하나의 cohesive lifecycle이 가치 있는 이유를 보여 준다. Akra는 client metadata가 아닌 durable
authority를 projection하여 그 lifecycle에 더 큰 책임성을 부여할 수 있다.

### 보수적인 조정

Orca의 orphan recovery가 더 넓지만 automation에는 Akra의 default가 올바르다. path 또는 generation을
입증할 수 없으면 보존하고 block한다. portfolio는 이 conservative result를 쉽게 이해하게 해야 한다.
recovery guidance는 operator feature이지 automatic deletion이 안전하다는 증거가 아니다.

### Codex 네이티브 세션 컨텍스트

Worktree navigation은 terminal scrollback이나 CLI resume token을 conversation source of truth로 취급하지
않고 official thread 및 turn identity를 유지해야 한다. Akra는 Orca 수준 workspace navigation과
protocol-native provenance를 결합할 수 있다.

## 비교 실험

이 실험은 disposable repository와 raw artifact를 사용한다. read-only inventory slice를 차단하지 않으며
별도 product priority를 만들지 않는다.

### 워크트리 수명 주기 픽스처

두 제품에 대해 exact version과 전후 Git state를 capture한다.

- 첫 clean worktree 생성;
- simultaneous worktree 3개와 branch/path collision;
- 추적됨, 추적되지 않음, 무시됨, 잠김, 중첩됨, 작업 대기 상태;
- 외부 추가/제거, 오래된 등록, 이동한 브랜치;
- creation/removal phase 사이의 app termination;
- 기준점 전진, 리베이스 충돌, 통합, 최종 정리;
- platform이 지원하는 경우 path의 space 및 newline;
- Windows file-handle partial deletion 및 WSL path spelling.

각 transition은 `git worktree list --porcelain`, branch ref, `status --porcelain=v2`, expected 및 actual
`HEAD`, product metadata, process tree, terminal outcome을 기록한다. synthetic content만 사용하고 absolute
user path는 redact한다.

### 워크트리 성능 프로필

기존 performance evidence contract를 다음과 같이 수정한다.

- worktree 1, 8, 32, 128개에서 cold/warm repository discovery;
- 외부 추가부터 표시까지의 지연;
- 생성부터 에이전트 터미널 준비까지의 지연;
- 대화형 화면 전환 지연;
- clean 및 blocked removal latency;
- incremental disk size 및 full desktop/daemon/agent process-tree RSS/CPU.

OS/filesystem, repository object count 및 status, Git version, terminal geometry, authentication, agent start
policy, run count, raw sample, exact source/artifact identity를 기록한다. 해당 sample이 생길 때까지
“Orca feels faster”와 “native must be faster”는 모두 unverified다.

## 권장 순서

이는 Orca에서 도출한 작업에만 적용되는 dependency order다. 이 P1/P2 item을 기존 portfolio P0보다
앞으로 옮기지 않으며 file과 owner가 겹치지 않을 때만 lane을 시작한다.

1. explicit bound, fixture, mutation 금지를 갖춘 P1 authoritative read model을 출시한다.
2. P1 TUI portfolio 및 real-terminal external-change capture를 출시한다.
3. authentication, redaction, no-mutation test를 갖춘 P1 Admin read DTO/API를 추가한다.
4. 각 existing owning projection이 반영된 뒤에만 optional activity, session, source, PR, delivery link를
   추가한다. 그동안 unavailable은 valid value로 남는다.
5. operator가 portfolio를 빈번한 navigation surface로 사용할 때만 existing session provenance를 P2
   worktree-scoped start/resume으로 수정한다.
6. 기존 P0 owner 아래 shared lifecycle 및 performance experiment를 실행한다.
7. observed use가 필요를 보여 주고 별도 safety contract를 review할 수 있을 때만 manual
   import/archive/removal을 고려한다.

모든 implementation row는 하나의 branch/worktree/PR이며 `prerelease`를 통해 반영하고 integration checkout과
disposable worktree를 clean하게 남긴다.

## 성공 감사

다음 조건을 충족하면 이 분석을 성공적으로 반영한 것이다.

- Akra가 Git 또는 pool을 변경하지 않고 explicit bound까지 registered worktree를 나열하고 truncation을
  조용히 생략하지 않고 non-authoritative로 표시한다;
- 모든 row가 path 또는 하나의 boolean에서 추측하지 않고 registration authority, status freshness,
  managed binding, ownership을 각각 선언한다;
- external add/remove 및 locked/dirty state가 restart 없이 reconcile된다. removed external row는 invented
  tombstone 없이 사라지고 exact authority-backed missing managed row는 blocked 및 visible 상태로 남는다;
- managed row는 exact lease generation에 link되고 기존 owning projection이 제공하는 task, official
  session, source, PR, delivery field만 보여 준다. unavailable은 explicit하다;
- TUI와 Admin은 하나의 application projection을 사용하고 inbound adapter에는 worktree policy가 없다;
- external 또는 ambiguous path가 암묵적으로 lane 또는 deletion target이 되지 않는다;
- fixed slot capacity, reviewed frozen delivery, conservative cleanup이 바뀌지 않는다;
- permission 및 sandbox bypass가 worktree presence에서 도출되지 않는다;
- 모든 performance claim이 공유 raw process-tree 및 Git-state evidence contract를 포함한다;
- 미래 destructive action은 success를 보고하기 전에 target, ownership, dirtiness, lock, expected branch
  `HEAD`, runtime-resource scope, terminal reconciliation을 입증한다.
