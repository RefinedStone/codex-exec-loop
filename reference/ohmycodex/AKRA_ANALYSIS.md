# Akra Reference Analysis: oh-my-codex

This note records the April 24, 2026 analysis requested by `reference/ohmycodex/AKRA.md`.

The source reference checkout used for this analysis lived in the local ignored path
`reference/ohmycodex/oh-my-codex/` and is not part of the tracked repository state. File
references to the reference project below therefore describe the local review context, not
guaranteed paths in a clean clone.

## Status Legend

- `확인됨`: directly supported by inspected code or docs
- `강한 추정`: strongly implied by inspected code, but not fully proven from this repo alone
- `불확실`: needs extra code, runtime evidence, or upstream clarification

## Executive Summary

- `확인됨` `oh-my-codex`는 본질적으로 "Codex CLI 위의 workflow/orchestration product"입니다. README도 스스로를 Codex workflow layer로 규정하고, 실제 CLI entry도 `dist/cli/index.js`로 위임하는 얇은 런처이며, 실질 제품 표면은 TypeScript orchestration, tmux/team runtime, hooks, MCP, `.omx/` 상태계층, Rust sidecar에 분산돼 있습니다. 근거: `reference/ohmycodex/oh-my-codex/README.md`, `reference/ohmycodex/oh-my-codex/src/cli/omx.ts`, `reference/ohmycodex/oh-my-codex/src/cli/index.ts`, `reference/ohmycodex/oh-my-codex/src/team/runtime.ts`, `reference/ohmycodex/oh-my-codex/src/runtime/bridge.ts`
- `확인됨` Akra는 현재도 더 일관된 single-surface product입니다. startup diagnostics, session resume, planning, queue, automation, parallel supervision이 모두 하나의 inline shell contract에 묶여 있고, 아키텍처도 `adapter -> application -> domain`으로 더 분명합니다. 근거: `docs/design/01-current-product-state.md`, `docs/design/04-hexagonal-runtime-architecture.md`, `docs/supersession/current-contract.md`, `README.md`
- `확인됨` 반대로 `oh-my-codex`가 Akra보다 강한 지점은 운영 breadth와 team-runtime maturity입니다. `omx setup/update/uninstall/doctor/hud/team/api/explore/sparkshell/wiki`까지 연결된 운영 표면, 다층 fallback contract, `.omx` state compatibility, 대규모 test/docs density는 실제 위협입니다. 근거: `reference/ohmycodex/oh-my-codex/src/cli/index.ts`, `reference/ohmycodex/oh-my-codex/src/cli/team.ts`, `reference/ohmycodex/oh-my-codex/src/team/api-interop.ts`, `reference/ohmycodex/oh-my-codex/docs/codex-native-hooks.md`, `reference/ohmycodex/oh-my-codex/docs/STATE_MODEL.md`

가장 중요한 결론 3개:

1. `확인됨` Akra가 지켜야 할 차별화 축은 `inline shell 단일 surface + operator-owned planning authority + capability-first terminal runtime`입니다. `oh-my-codex`를 그대로 따라가면 제품 중심이 분산형 workflow shell로 이동합니다. 근거: `docs/design/01-current-product-state.md`, `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`, `reference/ohmycodex/oh-my-codex/README.md`
2. `확인됨` 즉시 차용 가치가 있는 것은 제품 전체가 아니라 `team api/state contract`, `readiness vs actual execution 분리`, `native/fallback matrix 문서화`, `운영용 보조 surface` 같은 패턴입니다. 근거: `reference/ohmycodex/oh-my-codex/src/team/api-interop.ts`, `reference/ohmycodex/oh-my-codex/docs/contracts/team-runtime-state-contract.md`, `reference/ohmycodex/oh-my-codex/docs/codex-native-hooks.md`
3. `확인됨` 양쪽 모두 현재 품질 기준선이 완전 녹색은 아닙니다. 2026-04-24 기준 fresh `cargo test -q`는 Akra에서 snapshot 1건 실패했고, fresh `npm test`는 `oh-my-codex`에서 Windows tmux-session 계열 회귀로 실패했습니다. 근거: `src/adapter/inbound/tui/app/shell_rendering_tests.rs`, `src/adapter/inbound/tui/app/snapshots/codex_exec_loop_native__adapter__inbound__tui__app__shell_rendering__tests__inline_main_buffer_viewport_replay_streaming.snap`, `reference/ohmycodex/oh-my-codex/src/team/__tests__/tmux-session.test.ts`

## Akra Baseline

- `확인됨` Akra의 현재 제품 정체성은 `codex app-server` 기반의 native Rust terminal client입니다. operator-facing command는 `akra`이고, inline shell이 유일한 frontend입니다. 근거: `README.md`, `docs/design/01-current-product-state.md`
- `확인됨` 현재 핵심 구조는 `adapter -> application -> domain`이고, planning authority/store, session runtime, TUI shell, outbound bridge를 포트 경계로 나누려는 방향이 명시돼 있습니다. 근거: `docs/design/04-hexagonal-runtime-architecture.md`, `src/application/port/outbound/planning_authority_port.rs`
- `확인됨` Akra가 이번 비교에서 중요하게 봐야 할 판단축은 운영경험/제품통합, terminal capability boundary, planning authority ownership, parallel supervision 유지보수성입니다. 근거: `docs/design/01-current-product-state.md`, `docs/plan/17-structure-and-architecture-debt-map.md`, `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`, `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`
- `확인됨` 현재 부채는 이미 자인돼 있습니다. shell presentation, planning controller, parallel mode service, large-file hotspots가 구조 부채 문서에 명시돼 있고, 실제 최대 hotspot도 `parallel_mode`와 planning/TUI 주변에 몰려 있습니다. 근거: `docs/plan/17-structure-and-architecture-debt-map.md`, `src/application/service/parallel_mode/mod.rs`, `src/adapter/inbound/tui/app/planning/controller.rs`

## 1. 제품 철학 비교

- `확인됨` `oh-my-codex`의 제품 철학은 "Codex를 대체하지 않고 더 강한 workflow/runtime layer를 얹는다"입니다. README의 기본 onboarding도 `omx --madmax --high`로 시작해 `$deep-interview -> $ralplan -> $team/$ralph`로 이어집니다. 근거: `reference/ohmycodex/oh-my-codex/README.md`
- `확인됨` 그래서 `oh-my-codex`는 prompt pack이 아니라 workflow product이고, 동시에 자체 runtime product이기도 합니다. 팀 실행, 상태 저장, hook routing, HUD, wiki, explore, sparkshell, team API interop까지 스스로 소유합니다. 근거: `reference/ohmycodex/oh-my-codex/src/cli/index.ts`, `reference/ohmycodex/oh-my-codex/src/team/runtime.ts`, `reference/ohmycodex/oh-my-codex/src/hud/index.ts`, `reference/ohmycodex/oh-my-codex/src/wiki/index.ts`
- `확인됨` Akra의 철학은 그보다 좁고 선명합니다. inline shell 하나에 continuity를 모으고, planning과 queue를 operator-owned 흐름으로 묶으며, terminal-agent 확장은 capability-first seam으로 제한합니다. 근거: `README.md`, `docs/design/01-current-product-state.md`, `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`, `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`
- `확인됨` 언어 선택은 2차 요소입니다. 더 중요한 차이는 `workflow shell 중심` 대 `native terminal client 중심`입니다. `oh-my-codex`는 TypeScript orchestration이 중심이고 Rust는 sidecar/helper 경향이 강하며, Akra는 Rust runtime/TUI가 중심입니다. 근거: `reference/ohmycodex/oh-my-codex/package.json`, `reference/ohmycodex/oh-my-codex/Cargo.toml`, `reference/ohmycodex/oh-my-codex/src/cli/index.ts`, `README.md`

직접 비교 결론:

- `확인됨` Akra가 더 나은 쪽은 제품 정체성의 집중도입니다.
- `확인됨` `oh-my-codex`가 더 나은 쪽은 workflow와 운영 표면의 밀도입니다.

## 2. 운영경험 / 제품통합 비교

- `확인됨` `oh-my-codex`는 운영 표면이 매우 넓습니다. `setup`, `update`, `uninstall`, `doctor`, `team`, `hud`, `state`, `explore`, `sparkshell`, `wiki`, `adapt`, `question`, `agents`, `session`까지 모두 CLI 1차 surface로 노출합니다. 근거: `reference/ohmycodex/oh-my-codex/src/cli/index.ts`
- `확인됨` 팀 실행 경험도 productized 되어 있습니다. `omx team status/resume/shutdown/api`는 JSON interop까지 제공하고, worktree/tmux/session/root state를 함께 다룹니다. 근거: `reference/ohmycodex/oh-my-codex/src/cli/team.ts`, `reference/ohmycodex/oh-my-codex/src/team/api-interop.ts`, `reference/ohmycodex/oh-my-codex/src/team/worktree.ts`, `reference/ohmycodex/oh-my-codex/src/team/tmux-session.ts`
- `확인됨` 상태 모델도 운영 친화적으로 세분화돼 있습니다. `.omx/state/<mode>-state.json`, session-scoped state, `skill-active-state.json`, team dispatch/mailbox/monitor snapshot, native hooks/fallback ownership split이 모두 문서화되어 있습니다. 근거: `reference/ohmycodex/oh-my-codex/docs/STATE_MODEL.md`, `reference/ohmycodex/oh-my-codex/docs/contracts/team-runtime-state-contract.md`, `reference/ohmycodex/oh-my-codex/docs/contracts/team-delivery-state-contract.md`, `reference/ohmycodex/oh-my-codex/docs/codex-native-hooks.md`
- `확인됨` Akra는 breadth 대신 coherence를 택합니다. diagnostics, sessions, automation, queue, planning, directions, supersession을 한 shell 안에 묶고, 외부 CLI는 `doctor/init/reset` 정도의 planning lifecycle 보조 surface로 제한합니다. 근거: `README.md`, `docs/supersession/current-contract.md`
- `확인됨` 운영경험 우선 비교축에서는 "단일 세션 연속성"은 Akra가 더 좋고, "운영 도구 총량/병렬 팀 런타임"은 `oh-my-codex`가 더 강합니다. Akra는 operator가 shell을 떠나지 않는 대신 기능 폭이 더 엄격하고, `oh-my-codex`는 runtime help가 풍부한 대신 제품 중심이 여러 surface로 퍼집니다. 근거: `docs/design/01-current-product-state.md`, `reference/ohmycodex/oh-my-codex/README.md`, `reference/ohmycodex/oh-my-codex/src/cli/index.ts`

직접 비교 결론:

- `확인됨` long-lived solo work의 일관성은 Akra 우위입니다.
- `확인됨` setup/doctor/team/admin breadth와 team runtime productization은 `oh-my-codex` 우위입니다.

## 3. 엔지니어링 구조 비교

- `확인됨` Akra의 구조는 더 명시적입니다. `adapter -> application -> domain` 규칙, outbound port, planning authority/store, terminal bridge research boundary가 문서와 코드에서 정렬돼 있습니다. 근거: `docs/design/04-hexagonal-runtime-architecture.md`, `src/application/port/outbound/planning_authority_port.rs`, `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`
- `확인됨` `oh-my-codex`는 docs/contract density가 높지만 구현 중심은 여전히 TypeScript 거대 모듈입니다. `src/team/runtime.ts` 4557 LOC, `src/cli/index.ts` 3493 LOC, `src/team/state.ts` 2100 LOC, `src/team/tmux-session.ts` 2000 LOC가 핵심 hot zone입니다. 근거: `reference/ohmycodex/oh-my-codex/src/team/runtime.ts`, `reference/ohmycodex/oh-my-codex/src/cli/index.ts`, `reference/ohmycodex/oh-my-codex/src/team/state.ts`, `reference/ohmycodex/oh-my-codex/src/team/tmux-session.ts`
- `확인됨` `oh-my-codex`의 Rust는 semantic sub-owner이지만 중심 product logic 전체를 대체하지는 않습니다. `omx-runtime`은 runtime command/event와 compatibility JSON를 담당하고, TS bridge가 이를 감쌉니다. 근거: `reference/ohmycodex/oh-my-codex/crates/omx-runtime/src/main.rs`, `reference/ohmycodex/oh-my-codex/crates/omx-runtime-core/src/lib.rs`, `reference/ohmycodex/oh-my-codex/src/runtime/bridge.ts`, `reference/ohmycodex/oh-my-codex/docs/contracts/rust-runtime-thin-adapter-contract.md`
- `확인됨` 반면 Akra는 계획적으로 repo-scoped planning authority를 outbound adapter 뒤로 밀어 넣고, tracked files를 review/export artifact로 낮추는 방향입니다. 근거: `docs/supersession/current-contract.md`, `src/adapter/outbound/db/sqlite_planning_authority_adapter.rs`
- `확인됨` 테스트/문서 밀도는 `oh-my-codex`가 더 높습니다. local checkout 기준 `src/**/__tests__` 파일 수는 276, `docs/contracts` 14개, `docs/qa` 29개, top-level release notes 30개입니다. 근거: `reference/ohmycodex/oh-my-codex/src`, `reference/ohmycodex/oh-my-codex/docs/contracts`, `reference/ohmycodex/oh-my-codex/docs/qa`
- `확인됨` 하지만 현재 test baseline은 양쪽 모두 완전 녹색이 아닙니다. Akra의 fresh `cargo test -q`는 `inline_main_buffer_viewport_replay_keeps_recent_transcript_while_streaming` snapshot mismatch 1건으로 실패했고, `oh-my-codex`의 fresh `npm test`는 Windows tmux-session 계열 회귀로 실패했습니다. 근거: `src/adapter/inbound/tui/app/shell_rendering_tests.rs`, `src/adapter/inbound/tui/app/snapshots/codex_exec_loop_native__adapter__inbound__tui__app__shell_rendering__tests__inline_main_buffer_viewport_replay_streaming.snap`, `reference/ohmycodex/oh-my-codex/src/team/__tests__/tmux-session.test.ts`

직접 비교 결론:

- `확인됨` 구조적 안전성과 future refactor governability는 Akra 우위입니다.
- `확인됨` 계약 문서화, 운영 회귀 테스트 범위, team runtime hardening coverage는 `oh-my-codex` 우위입니다.

## 4. 우리가 더 나은 지점

- `확인됨` Akra는 single-surface product coherence가 더 낫습니다. operator는 diagnostics, session resume, planning, queue, automation, supersession을 하나의 shell 안에서 이어서 다룹니다. 근거: `README.md`, `docs/design/01-current-product-state.md`, `docs/supersession/current-contract.md`
- `확인됨` Akra는 planning authority ownership이 더 선명합니다. accepted planning, queue state, runtime export, repo-scoped DB authority가 명시돼 있고, tracked planning files는 runtime truth가 아니라 review/export/import artifact로 낮아져 있습니다. 근거: `docs/supersession/current-contract.md`, `src/application/port/outbound/planning_authority_port.rs`, `src/adapter/outbound/db/sqlite_planning_authority_adapter.rs`
- `확인됨` Akra는 provider/runtime 문제를 capability로 쪼개서 생각합니다. `InteractiveTurnRuntime`, `StartupProbe`, `SessionCatalog`, `TerminalBridgeAttachment` 같은 seam naming은 향후 Claude/Codex 혼합에도 더 안전합니다. 근거: `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`
- `확인됨` Akra는 proxy/gateway 유혹을 더 잘 통제하고 있습니다. 문서가 tmux/local attach를 primary path로, managed wrapper를 fallback으로, proxy mediation을 defer로 명시합니다. 근거: `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`, `docs/plan/28-terminal-agent-tmux-local-attach-gate-verdict.md`
- `확인됨` 구조 부채도 더 정직하게 다루고 있습니다. hotspot order와 refactor order가 문서에 있고, "무엇이 왜 제품 비용인지"가 설명됩니다. 근거: `docs/plan/17-structure-and-architecture-debt-map.md`

## 5. 상대가 더 강한 지점

- `확인됨` `oh-my-codex`는 팀 런타임 maturity에서 앞섭니다. `omx team`은 tmux, worker pane, mailbox, dispatch, approvals, worktree, shutdown/recovery, CLI interop를 이미 한 제품 contract로 묶었습니다. 근거: `reference/ohmycodex/oh-my-codex/src/cli/team.ts`, `reference/ohmycodex/oh-my-codex/src/team/runtime.ts`, `reference/ohmycodex/oh-my-codex/src/team/api-interop.ts`
- `확인됨` 운영 보조 surface도 훨씬 많습니다. setup/update/uninstall/doctor/hud/wiki/explore/sparkshell/adapt까지 productized 되어 있어 "문제가 생겼을 때 어디서 본다"가 분명합니다. 근거: `reference/ohmycodex/oh-my-codex/src/cli/index.ts`
- `확인됨` native hooks와 fallback matrix를 명시적으로 관리합니다. 무엇이 native이고 무엇이 runtime fallback인지 문서와 구현이 연결돼 있습니다. 근거: `reference/ohmycodex/oh-my-codex/docs/codex-native-hooks.md`, `reference/ohmycodex/oh-my-codex/src/scripts/codex-native-hook.ts`
- `확인됨` 문서/테스트 투자도 위협입니다. 계약 문서와 QA 문서가 풍부하고, release note/history도 촘촘합니다. 근거: `reference/ohmycodex/oh-my-codex/docs/contracts`, `reference/ohmycodex/oh-my-codex/docs/qa`, `reference/ohmycodex/oh-my-codex/docs/release-notes-*.md`
- `확인됨` 따라서 위협은 "Akra가 workflow shell로 밀린다"가 아니라, "parallel supervision/admin/readiness/tooling에서 상대가 더 productized 되어 보인다"는 쪽입니다. 근거: `reference/ohmycodex/oh-my-codex/README.md`, `reference/ohmycodex/oh-my-codex/src/team/runtime.ts`, `docs/design/01-current-product-state.md`

## 6. 우리 로드맵에 바로 반영할 제안 5개

1. `확인됨` `cargo test -q`를 다시 녹색으로 되돌리는 일을 최우선 small slice로 두세요. 현재 실패는 shell rendering snapshot이 queue/planning projection drift를 노출하고 있어, single-surface coherence라는 핵심 가치에 직접 상처를 냅니다. 근거: `src/adapter/inbound/tui/app/shell_rendering_tests.rs`, `src/adapter/inbound/tui/app/snapshots/codex_exec_loop_native__adapter__inbound__tui__app__shell_rendering__tests__inline_main_buffer_viewport_replay_streaming.snap`
2. `확인됨` `parallel_mode` hotspot 분해를 당기세요. `oh-my-codex`의 가장 현실적인 경쟁력은 team runtime maturity인데, Akra는 바로 그 영역이 현재 1순위 구조 부채입니다. readiness, slots, distributor, recovery, snapshot 경계를 우선 분리해야 합니다. 근거: `docs/plan/17-structure-and-architecture-debt-map.md`, `src/application/service/parallel_mode/mod.rs`
3. `확인됨` startup readiness와 "실제 실행 smoke test"를 분리한 operator contract를 강화하세요. `oh-my-codex`의 `doctor` vs `exec` distinction은 실용적입니다. Akra도 install/readiness와 real runtime viability를 구분해 진단 UX를 강화할 가치가 큽니다. 근거: `reference/ohmycodex/oh-my-codex/README.md`, `reference/ohmycodex/oh-my-codex/src/cli/doctor.ts`, `README.md`
4. `확인됨` `team api/state contract` 수준의 bounded admin seam은 차용하되, core surface는 shell 안에 유지하세요. 즉, JSON parity/admin tooling은 sidecar or port 뒤에 두고, 제품 중심은 여전히 inline shell이어야 합니다. 근거: `reference/ohmycodex/oh-my-codex/src/team/api-interop.ts`, `reference/ohmycodex/oh-my-codex/docs/contracts/team-runtime-state-contract.md`, `docs/design/01-current-product-state.md`
5. `확인됨` tmux/local attach 중심 전략을 유지하되, attach/recovery/interrupt/approval truth를 product language로 더 노출하세요. `oh-my-codex`의 breadth를 따라갈 필요는 없지만, operator가 "지금 무엇이 가능한지"를 명확히 보는 경험은 더 보강해야 합니다. 근거: `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`, `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`, `docs/plan/28-terminal-agent-tmux-local-attach-gate-verdict.md`

## Recommendation

`tmux/local terminal bridge 중심`이 가장 맞습니다.

- `확인됨` Akra는 이미 그 방향으로 capability seam, transport matrix, readiness evidence를 쌓아 두었습니다. 근거: `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`, `docs/plan/22-terminal-agent-transport-and-attachment-matrix.md`, `docs/plan/27-terminal-agent-tmux-local-attach-readiness-evidence.md`
- `확인됨` `oh-my-codex`의 강점은 배울 만하지만, 제품 전체를 가져오면 Akra의 중심이 inline shell에서 다면적 workflow shell로 이동합니다. 근거: `reference/ohmycodex/oh-my-codex/src/cli/index.ts`, `reference/ohmycodex/oh-my-codex/README.md`
- `확인됨` `full proxy/gateway 별도 제품화`나 `.omx`식 sprawling runtime surface는 Akra 차별화를 약화시킬 가능성이 큽니다. 근거: `docs/design/01-current-product-state.md`, `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`

## Open Questions

- `불확실` Akra가 `team api` 수준의 bounded admin seam을 어디까지 가져갈지: CLI parity만 둘지, sidecar/MCP까지 둘지 추가 설계가 필요합니다. 근거: `reference/ohmycodex/oh-my-codex/src/team/api-interop.ts`, `docs/design/04-hexagonal-runtime-architecture.md`
- `확인됨` 현재 `cargo test -q` 실패가 순수 snapshot drift인지, planning fixture leakage인지 정리할 필요가 있습니다. 이것은 single-surface shell 신뢰도와 직결됩니다. 근거: `src/adapter/inbound/tui/app/shell_rendering_tests.rs`
- `확인됨` `oh-my-codex`의 Windows tmux/session regressions는 breadth의 비용을 보여 줍니다. Akra가 동일 영역을 확장할 때 cross-platform 지원 범위를 어디까지 product commitment로 잡을지 명확히 해야 합니다. 근거: `reference/ohmycodex/oh-my-codex/src/team/__tests__/tmux-session.test.ts`

## Evidence

- Akra baseline and roadmap:
  - `README.md`
  - `docs/design/01-current-product-state.md`
  - `docs/design/04-hexagonal-runtime-architecture.md`
  - `docs/plan/17-structure-and-architecture-debt-map.md`
  - `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`
  - `docs/plan/22-terminal-agent-transport-and-attachment-matrix.md`
  - `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`
  - `docs/plan/28-terminal-agent-tmux-local-attach-gate-verdict.md`
  - `docs/supersession/current-contract.md`
  - `src/application/port/outbound/planning_authority_port.rs`
  - `src/adapter/outbound/db/sqlite_planning_authority_adapter.rs`
  - `src/adapter/inbound/tui/app/shell_rendering_tests.rs`
  - `src/adapter/inbound/tui/app/snapshots/codex_exec_loop_native__adapter__inbound__tui__app__shell_rendering__tests__inline_main_buffer_viewport_replay_streaming.snap`
- Local reference checkout reviewed on April 24, 2026:
  - `reference/ohmycodex/oh-my-codex/README.md`
  - `reference/ohmycodex/oh-my-codex/package.json`
  - `reference/ohmycodex/oh-my-codex/Cargo.toml`
  - `reference/ohmycodex/oh-my-codex/src/cli/omx.ts`
  - `reference/ohmycodex/oh-my-codex/src/cli/index.ts`
  - `reference/ohmycodex/oh-my-codex/src/cli/team.ts`
  - `reference/ohmycodex/oh-my-codex/src/cli/explore.ts`
  - `reference/ohmycodex/oh-my-codex/src/cli/sparkshell.ts`
  - `reference/ohmycodex/oh-my-codex/src/cli/state.ts`
  - `reference/ohmycodex/oh-my-codex/src/team/runtime.ts`
  - `reference/ohmycodex/oh-my-codex/src/team/api-interop.ts`
  - `reference/ohmycodex/oh-my-codex/src/team/tmux-session.ts`
  - `reference/ohmycodex/oh-my-codex/src/team/worktree.ts`
  - `reference/ohmycodex/oh-my-codex/src/team/state.ts`
  - `reference/ohmycodex/oh-my-codex/src/team/state-root.ts`
  - `reference/ohmycodex/oh-my-codex/src/team/__tests__/tmux-session.test.ts`
  - `reference/ohmycodex/oh-my-codex/src/mcp/state-server.ts`
  - `reference/ohmycodex/oh-my-codex/src/mcp/state-paths.ts`
  - `reference/ohmycodex/oh-my-codex/src/modes/base.ts`
  - `reference/ohmycodex/oh-my-codex/src/runtime/bridge.ts`
  - `reference/ohmycodex/oh-my-codex/src/hud/index.ts`
  - `reference/ohmycodex/oh-my-codex/src/wiki/index.ts`
  - `reference/ohmycodex/oh-my-codex/crates/omx-runtime/src/main.rs`
  - `reference/ohmycodex/oh-my-codex/crates/omx-runtime-core/src/lib.rs`
  - `reference/ohmycodex/oh-my-codex/docs/STATE_MODEL.md`
  - `reference/ohmycodex/oh-my-codex/docs/codex-native-hooks.md`
  - `reference/ohmycodex/oh-my-codex/docs/contracts/team-runtime-state-contract.md`
  - `reference/ohmycodex/oh-my-codex/docs/contracts/team-delivery-state-contract.md`
  - `reference/ohmycodex/oh-my-codex/docs/contracts/rust-runtime-thin-adapter-contract.md`
  - `reference/ohmycodex/oh-my-codex/docs/wiki-feature.md`
