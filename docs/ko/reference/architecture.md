# 런타임 아키텍처

[English](../../reference/architecture.md)

이 문서는 구현된 런타임의 canonical architecture와 authority reference입니다.

```text
adapter/inbound -> core or application -> domain
core -> application -> domain
application -> outbound ports -> adapter/outbound
composition -> concrete wiring
```

## 계층 소유권

| 계층 | 소유 | 소유하면 안 되는 것 |
| --- | --- | --- |
| `adapter/inbound` | 입력 mapping, rendering, local focus/editor/selection state | domain 정책, durable task truth, dispatch 정책 |
| `core` | app command/event/effect/completion 흐름, app state, projection, snapshot | TUI/HTTP/Telegram type, 구체 DB/Git/filesystem adapter |
| `application/service` | use-case orchestration, ordering gate, transaction, control-plane handle | widget, terminal event, transport DTO |
| `application/port` | application service가 요구하는 outbound contract | 구체 integration 상세 |
| `domain` | 순수 invariant, validation, decision, state transition | async runtime, IO, logging, UI, DB, filesystem, Git 호출 |
| `adapter/outbound` | app-server, DB, filesystem, Git, GitHub, Telegram integration | business policy |
| `composition` | dependency 생성과 concrete wiring | domain decision |

Mapping은 adapter에, policy는 domain 또는 application service에 둡니다. 실제 outbound boundary가
있을 때만 port를 추가합니다.

## Core runtime

`src/core`는 headless app runtime이며 application/domain layer를 대체하지 않습니다.

- `AppCommand` 또는 `CoreInput`: 사용자·lifecycle·tick·completion intent
- `Effect`: core 바깥에서 실행할 작업
- `Completion`: 같은 input queue로 돌아오는 effect 결과
- `AppEvent`: 외부에 유용한 전이
- `AppSnapshot`/projection: adapter가 읽는 view model

시작, session load, conversation 선택, turn 제출, stream reduction, 완료, post-turn 평가가 이 흐름을
사용합니다. Parallel mutation은 application 소유이며 `ParallelModeControlPlaneHandle`로 진입합니다.
Core는 projection을 복사할 수 있지만 두 번째 parallel runtime을 소유하면 안 됩니다.

Turn submission admission은 core가 소유하는 single-flight 권한입니다. TUI는 prompt intent 단계에서
editor를 지우거나 transcript history를 추가하지 않고, core가 accepted admission을 반환한 뒤에만
로컬 projection을 확정합니다. 활성 submission이 있으면 core는 명시적 rejection event를 내보내고
worker effect를 만들지 않으므로 adapter와 core가 가짜 `starting turn` 상태로 갈라지지 않습니다.

활성 turn steer도 같은 권한 경계를 사용합니다. Core는 정확히 일치하는 활성
submission/thread/turn identity만 승인하고, 해당 submission에 귀속된 correlation을 발급해 provider
worker 하나만 시작하며 stale completion을 버립니다. TUI에는 confirmation modal과 성공 시 변경되지
않은 draft만 지우기 위한 editor revision만 남습니다. 이미 승인된 steer는 turn terminal event만으로
무효화하지 않지만 conversation identity가 바뀌면 무효화합니다.

Parallel peek load는 core-correlated read입니다. Core가 요청 thread에 단조 증가 generation을
부여하고 최신 요청으로 이전 요청을 대체하며, 일치하지 않거나 중복된 completion은 TUI에 도달하기
전에 버립니다. TUI는 agent 선택과 loading/status/scroll 표현을 소유하고 다른 shell overlay가
대체하면 preview를 지웁니다. Peek 결과는 interactive conversation을 절대 교체하지 않습니다.

Session rename도 core-correlated single-flight 작업입니다. Provider의 성공 응답을 core가 수락하면
일치하는 catalog row, 불러온 conversation title, stream identity를 함께 갱신합니다. 충돌하는
catalog load와 같은 thread의 conversation load는 버리지 않고 rename 완료 뒤로 지연하므로 예전
read가 이전 title을 복원할 수 없습니다. TUI는 core가 수락한 projection과 stream event를 매핑할
뿐이며 rename editor draft, pending feedback, selected row만 소유합니다.

## 상태 권한

| 상태 | 권한 소유자 |
| --- | --- |
| cursor, modal, overlay, editor buffer, selected row | inbound adapter |
| session/conversation lifecycle, in-flight effect, stream reduction | core |
| parallel wake/effect ordering, stale-completion guard | application control-plane |
| task/direction/queue authority, lease, session record, delivery claim | SQLite-backed store |
| eligibility, capacity, retry, validation, stale-event decision | domain |

Invariant에 영향을 주거나 재시작 후에도 남아야 하는 상태는 TUI에만 둘 수 없습니다. Rendering이나
focus만을 위한 상태는 domain authority로 올리지 않습니다.

## Planning 경계

```text
inbound adapter
  -> application/service/planning
  -> application/port
  -> adapter/outbound/{db,filesystem,app_server}
```

Private SQLite store가 권한을 가집니다. Planning workspace file은 운영자가 편집하는 상세 문서,
prompt, staged draft, export, 복구 근거입니다. 승인 mutation은 revision-aware validation을 통과하고,
hidden worker output은 SQL이나 보호된 planning file을 직접 쓰지 않습니다.

`AKRA_HOME`은 신뢰할 수 있는 절대 경로여야 합니다. SQLite DB와 sidecar는 private regular
single-link file이어야 합니다. Repository incarnation marker는 재사용된 checkout 경로가 예전
저장소의 권한을 자동으로 채택하지 못하게 합니다.

## Parallel 경계

```text
TUI intent
  -> application control-plane handle
  -> domain decision
  -> durable store / effect runner
  -> projection
  -> core snapshot / TUI rendering
```

현재 control-plane은 mutex로 직렬화한 synchronous facade입니다. Application writer 하나, effect
accounting, stale completion drop, wake coalescing, durable backpressure, 단일 projection source를
제공합니다. 이 결정을 다시 검토하지 않고 mailbox actor, TUI/core의 raw parallel service owner,
두 번째 dispatch queue를 추가하지 않습니다.

Pool mutation은 repository-scoped OS lock도 획득합니다. 각 allocation은 추측할 수 없는 정확한
generation을 받고 lease, session, event, delivery, cleanup까지 전달됩니다. 지연 event는 mutation
전에 같은 generation인지 비교합니다. 모든 지원 플랫폼에서 SQLite가 권한을 가집니다.

Post-turn mutation은 continuation permit과 parallel epoch permit을 캡처합니다. 긴 작업은 제한된
commit section 밖에서 실행합니다. 무효화된 permit 결과는 진단에 남을 수 있지만 task authority를
변경하거나 delivery를 enqueue할 수 없습니다.

## 전달과 process 경계

Parallel acquisition은 push remote, credential 없는 canonical GitHub URL, repository/visibility,
integration branch, fetched base, source SHA, ordered reviewed range를 고정합니다. Remote 또는 review
drift는 해당 mutation 전에 차단합니다.

Host 소유 Git 작업은 다음을 지킵니다.

- 신뢰하는 native executable을 고정하고 상속된 Git 실행/route 제어를 제거합니다.
- hook, fsmonitor, signing, replacement object, lazy fetch, prompt, external diff를 끕니다.
- mutation 전에 effective repository/worktree config key 이름을 검사합니다.
- 실행 가능한 filter/driver, unsafe worktree redirect, alternate ref, external tool, 특수 경로,
  과도한 변경, unmerged entry, 숨겨진 dirty submodule state를 거부합니다.
- 격리된 credential-free network config와 별도로 검증한 API identity를 사용합니다.

Subprocess는 `src/subprocess.rs`를 사용합니다. Linux는 process-group과 제한된 `/proc` descendant
containment, Windows는 kill-on-close Job, 그 밖의 Unix는 process-group containment를 보장합니다.
같은 OS 사용자 프로세스의 악의적인 race는 filesystem-only threat boundary 밖입니다.

## TUI 소유권

TUI 변경은 state/reducer, controller/effect, projection/copy, theme/chrome, rendering/layout,
terminal-adapter 책임을 분리합니다. 시각 token은 `AkraTheme` 뒤에 두고 host scrollback과 live
viewport에 나뉘는 append-only row 사이에 panel chrome을 삽입하지 않습니다.

Planning worker 진단은 post-turn execution부터 screen model까지 domain
`PlanningWorkerPanelState`를 그대로 보관합니다. TUI presentation은 adapter 소유 status DTO나
왕복 mapper 없이 label과 content visibility만 파생합니다.

상세한 test-guarded 계약은 [TUI 계층 아키텍처](tui-contract.md)를 참고하세요.

## 금지 방향과 gate

- `domain`은 application, core, adapter, runtime, IO framework를 import하지 않습니다.
- `application`은 core, TUI, HTTP, Telegram, concrete outbound adapter를 import하지 않습니다.
- `core`는 inbound UI/transport type이나 concrete outbound adapter를 import하지 않습니다.
- TUI는 planning/parallel mutation의 core/application command gate를 우회하지 않습니다.
- Outbound adapter는 domain policy를 구현하지 않습니다.

```bash
. "$HOME/.cargo/env"
cargo test --test architecture_boundaries
cargo fmt --all -- --check
cargo test --locked
```

범위가 넓은 native/TUI 변경은 `bash scripts/check_native_pr.sh`를 실행합니다.
