# Repository-Wide Test Contract Taxonomy

## 목적

이 문서는 `TEST-00`의 산출물이다. 기준 문서는
[repository-wide-rebuild-roadmap.md](./repository-wide-rebuild-roadmap.md)이며, 목적은
repo 전체 테스트를 구현 파일 구조가 아니라 계층 계약으로 분류하는 것이다.

테스트는 다음 질문에 답해야 한다.

- 이 변경이 domain decision을 바꾸는가?
- application command/event ordering을 바꾸는가?
- inbound adapter mapping 또는 rendering contract를 바꾸는가?
- outbound store/recovery boundary를 바꾸는가?
- operator journey가 실제로 달라지는가?

새 테스트는 이 중 하나의 primary contract를 선택한다. 한 테스트가 모든 계층을 동시에
보호하려고 하면 실패 원인이 흐려지고 worker가 다음 slice의 범위를 안전하게 잡기 어렵다.

## Contract Layer

| Layer | 보호하는 계약 | 기본 위치 | 검증 기준 | 금지 패턴 |
| --- | --- | --- | --- | --- |
| Domain Decision | I/O 없는 invariant, eligibility, classification, projection decision | `src/domain/**/tests.rs` | `cargo test domain::<context>` | repository, filesystem, TUI type import |
| Application Ordering | command/event fan-in, single-writer ordering, service orchestration, effect id/epoch stale drop | `src/application/service/**/tests/*` | context별 `cargo test application::service::<context>` 또는 focused test name | adapter copy, terminal key, HTML/form string 의존 |
| Outbound Store/Recovery | SQLite authority, runtime projection, filesystem workspace, git/GitHub recovery boundary | `src/adapter/outbound/**/tests.rs` | adapter-focused cargo test, temp dir/DB 기반 recovery assertion | service policy를 adapter에서 재판단 |
| Inbound Mapping | CLI/Telegram/admin/TUI command spelling을 typed request로 매핑 | `src/adapter/inbound/**/tests.rs`, parser module tests | focused parser/route tests | transport string을 domain enum처럼 승격 |
| TUI Runtime | shell input routing, background message routing, overlay focus, scheduler tick | `src/adapter/inbound/tui/app/shell_runtime/tests/*` | `cargo test shell_runtime` 또는 flow/input/scheduler focused test | background thread가 UI 또는 durable store 직접 mutate |
| TUI Rendering | projection-to-widget copy, viewport, modal layout, visible operator contract | `src/adapter/inbound/tui/app/shell_rendering*_tests.rs`, snapshots | `cargo test shell_rendering` and reviewed snapshot delta | business rule을 renderer에서 계산 |
| Source-Level Guard | cross-surface import 금지, parser reuse, route-pair wiring 등 runtime fixture가 과한 구조 계약 | 해당 adapter/application test module | stable symbol assertion + 문서 anchor | line number, formatting, broad implementation trivia 고정 |
| Journey Validation | 실제 operator flow, release/restart/blocking evidence | `docs/validation/*`, release notes, scripts output | linked PR/commit/capture evidence | 현재 구현 설명을 backlog처럼 중복 |

## 배치 규칙

- domain rule을 바꾸면 먼저 domain test를 추가한다. application test는 repository load/save 순서와
  domain decision 호출 결과만 검증한다.
- application runtime이나 orchestration을 바꾸면 fake port/repository로 command/event ordering을
  검증한다. thread, real GitHub, terminal rendering은 기본 fixture가 아니다.
- inbound adapter를 바꾸면 parser 또는 request mapper test를 먼저 추가한다. handler test는
  mapper 결과가 facade/service로 전달되는지만 검증한다.
- outbound adapter를 바꾸면 temp dir, temp DB, fake git/GitHub port로 recovery contract를 검증한다.
  application policy를 adapter test에 복사하지 않는다.
- TUI key/input 변경은 `shell_runtime` 또는 작은 controller/parser test에 둔다. copy/layout 변경은
  rendering contract test나 snapshot에 둔다.
- source-level guard는 구조 위반을 빠르게 막아야 할 때만 쓴다. 예: admin HTML/JSON route pair가
  같은 request DTO를 쓰는지, TUI hint path와 execution path가 같은 parser를 쓰는지.

## Major Context Matrix

| Context | Primary contract | 현재 anchor | 보강할 때 우선 실행 |
| --- | --- | --- | --- |
| Parallel control-plane | blocked slot, idle capacity, wake coalescing, stale epoch, worker completion event path | `src/domain/parallel_mode/tests.rs`, `src/application/service/parallel_mode/tests/*`, `shell_runtime/tests/flows.rs` | `cargo test domain::parallel_mode`, `cargo test parallel_mode`, focused shell runtime flow |
| Planning domain/application | queue ordering, proposal promotion, repair eligibility, task mutation invariant, worker command extraction | `src/domain/planning/queue/tests.rs`, `src/application/service/planning/**/tests.rs` | `cargo test domain::planning`, `cargo test application::service::planning` focused module |
| Planning authoring/admin | draft validation, close risk, direction/task/draft mutation request mapping | `planning_draft_editor_ui/tests.rs`, `admin_api/tests.rs`, planning controller source guards | `cargo test planning_draft_editor_ui`, `cargo test adapter::inbound::admin_api` |
| TUI conversation/automation | stream lifecycle, post-turn automation fan-in, queued auto prompt provenance, prompt lock | `conversation_model_tests.rs`, `shell_runtime/tests/*`, `tui-conversation-automation-split-plan.md` anchors | `cargo test conversation_runtime`, `cargo test shell_runtime` |
| TUI rendering | transcript, overlays, planning editor, queue, supersession timeline visibility | `shell_rendering_tests.rs`, `shell_rendering_contract_tests.rs`, snapshots | `cargo test shell_rendering` and reviewed snapshot output |
| Inbound command surface | CLI/admin/Telegram/TUI command spelling to shared request/command vocabulary | `cli.rs` tests, `telegram_bot/tests.rs`, `admin_api/tests.rs`, TUI shell parser tests | focused parser tests, `cargo test adapter::inbound` when broad |
| Store/runtime state | durable authority, runtime projection, process-lifetime store, workspace artifacts, mirror recovery | `sqlite_planning_authority_adapter/tests.rs`, filesystem adapter tests, parallel runtime tests | `cargo test adapter::outbound::db::sqlite_planning_authority_adapter`, focused runtime recovery test |
| App-server and external ports | protocol parsing, stream reduction, review polling, worker launch boundary | `app_server/protocol/contract_tests.rs`, `review_poller/tests.rs`, port fake tests | focused outbound adapter test |

## Verification Ladder

Use the narrowest level that covers the changed contract, then widen only when the change crosses
boundaries.

| Change type | Required verification |
| --- | --- |
| 문서만 변경 | `git diff --check` and link/path search |
| Pure domain decision | focused domain test; add application focused test only if service mapping changed |
| Application service ordering | focused application service test; run context-wide filter when command/event vocabulary changed |
| Outbound DB/filesystem/git adapter | adapter focused test with temp storage; recovery path assertion |
| Inbound parser/mapper | focused parser/route test; source-level guard if route pair wiring is the contract |
| TUI key routing/runtime | focused shell runtime/controller/parser test; rendering test only if visible copy/layout changed |
| TUI rendering/snapshot | rendering contract test; snapshot delta review; broad TUI script for major layout PRs |
| Broad native/TUI PR | `bash scripts/check_native_pr.sh` when practical, otherwise explicit cargo test/fmt/clippy matrix |
| Lint-sensitive Rust change | `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings` |

## Source-Level Guard 기준

Source-level guard는 runtime scenario를 만들면 테스트가 과하게 크거나 느려지는 구조 계약에만 쓴다.

허용되는 예:

- 같은 parser helper가 command execution과 inline hint path에 사용되는지 검증
- admin HTML route와 JSON route가 같은 `PlanningAdmin*Request`를 생성하는지 검증
- TUI editor keymap이 `PlanningAdmin*`나 `PlanningControl*` vocabulary를 import하지 않는지 검증
- read-only dashboard가 mutation command를 직접 호출하지 않는지 검증

금지되는 예:

- exact line number, formatter output, comment text를 contract로 고정
- function body 전체 문자열을 fixture처럼 비교
- behavior test로 쉽게 검증 가능한 domain decision을 source text로 검증

Source-level guard를 추가하면 문서의 regression anchor 목록에도 이름을 남긴다.

## Fake와 Runtime State 규칙

- fake port는 테스트 module 또는 `test_helpers`에 둔다. production path에서 접근 가능한 global
  singleton으로 승격하지 않는다.
- process-lifetime runtime state는 각 harness가 명시적으로 초기화한다. 이전 test의 wake, epoch,
  in-flight flag가 다음 test에 남으면 contract 실패다.
- durable state test는 temp DB/temp dir을 사용하고, recovery assertion은 재로딩 또는 새 adapter
  instance로 확인한다.
- Noop repository global map은 `#[cfg(test)]` fake 전용이다. application runtime store 또는
  production repository로 일반화하지 않는다.
- TUI rendering test는 real terminal state를 요구하지 않아야 한다. ratatui/vt100 fixture나
  rendering DTO를 사용한다.

## Worker 시작 Checklist

1. Roadmap slice가 바꾸는 primary contract layer를 하나 고른다.
2. 같은 layer의 기존 anchor를 찾아 test name과 파일 위치를 PR 설명에 적는다.
3. 새 regression이 필요하면 가장 좁은 layer에 추가한다.
4. cross-layer 변경이면 domain/application/adapter test를 각각 작게 둔다. 하나의 giant journey
   test로 모든 것을 덮지 않는다.
5. 문서 anchor가 있는 slice는 `new/docs` 문서의 완료 근거에 test name을 추가한다.
6. PR verification에는 실제 실행한 명령만 적는다.

## 완료 기준

`TEST-00`은 테스트 파일 이동이나 snapshot 갱신을 요구하지 않는다. 완료 기준은 다음과 같다.

- 계층별 테스트 분류와 금지 패턴이 문서화되어 있다.
- major context별 현재 anchor와 우선 실행 명령이 추적 가능하다.
- source-level guard와 fake/runtime state 사용 기준이 명확하다.
- roadmap에서 이후 slice가 이 taxonomy를 verification matrix로 참조할 수 있다.
