# Executive Summary

현재 reference repo는 본질적으로 로컬 `tmux`/CLI 기반 multi-agent orchestration shell이다. 핵심 실행 경로는 HTTP proxy나 shared gateway가 아니라 tmux pane 안에서 실행되는 `claude`/`codex`/`gemini`/`cursor` CLI이며, 그 위에 local state, team runtime, MCP stdio 서버, prompt/job persistence, HUD, notification gateway가 얹혀 있다.

- `강한 추정`: `CLIProxyAPIPlus`는 이 프로젝트의 중심 엔진이 아니다. 적어도 inspect한 repo 안에는 그 이름의 명시적 중심 dependency나 integration point가 보이지 않는다. 있다면 core data plane이 아니라 부품 또는 sidecar 성격일 가능성이 높다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/package.json`, `reference/ohmyclaude/oh-my-claudecode/src/cli/launch.ts`, `reference/ohmyclaude/oh-my-claudecode/src/team/model-contract.ts`, `reference/ohmyclaude/oh-my-claudecode/src/openclaw/index.ts`

- `확인됨`: Akra relevance는 “repo 전체를 가져오기”보다 “일부 architectural pattern만 차용하기”에 있다. Akra가 통째로 이 구조를 도입하면 TUI/session 중심성보다 local team-control-plane 성격이 강해질 가능성이 높다.  
Evidence: `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`, `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`, `docs/plan/28-terminal-agent-tmux-local-attach-gate-verdict.md`, `reference/ohmyclaude/oh-my-claudecode/src/team/runtime-v2.ts`

- `확인됨`: 가장 중요한 결론은 세 가지다.  
Evidence: `docs/plan/25-codex-assumption-to-capability-target-map.md`, `reference/ohmyclaude/oh-my-claudecode/src/hud/usage-api.ts`, `reference/ohmyclaude/oh-my-claudecode/src/team/runtime-v2.ts`

1. OMC의 중심은 proxy/gateway가 아니라 local terminal orchestration이다.
2. Anthropic subscription/OAuth를 third-party product core에 끌어올리면 정책 리스크가 커진다.
3. Akra가 가져와야 하는 것은 local attach/headless runner 경계, explicit state/control-plane, immutable routing snapshot이지, OMC 전체 product shell이 아니다.

# Akra Baseline

- `확인됨`: Akra는 Rust 기반 native-first TUI이고, 현재 중심 경로는 여전히 `codex app-server`이지만 capability seam을 분리하는 방향이 이미 문서와 코드에 반영돼 있다.  
Evidence: `src/adapter/inbound/tui/app/shell_entrypoint.rs`, `docs/plan/25-codex-assumption-to-capability-target-map.md`, `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`

- `확인됨`: Akra가 현재 중요하게 보는 seam은 `StartupProbe`, `InteractiveTurnRuntime`, optional `SessionCatalog`, `TerminalBridgeAttachment`다. Akra는 giant universal provider interface를 지양하고, launch/attach truth와 turn-runtime truth를 분리하려 한다.  
Evidence: `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`, `docs/plan/25-codex-assumption-to-capability-target-map.md`, `src/domain/recent_sessions.rs`, `src/domain/terminal_bridge_attachment.rs`

- `확인됨`: Akra의 현재 브리지 방향은 `tmux/local terminal bridge` 우선, `managed wrapper` fallback, `proxy/gateway` deferred다.  
Evidence: `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`, `docs/plan/24-terminal-agent-bridge-experiment-matrix.md`, `docs/plan/28-terminal-agent-tmux-local-attach-gate-verdict.md`

- `확인됨`: Akra는 main interactive session과 sub/task runtime이 서로 다른 backend를 가질 수 있도록 설계 방향을 잡고 있다. 특히 supersession/parallel mode는 별도 worktree와 queue/distributor lane을 이미 전제로 둔다.  
Evidence: `docs/supersession/current-contract.md`, `src/application/service/parallel_mode/turn.rs`

- `확인됨`: Akra는 이미 `codex app-server`와 `tmux local attach`를 대체 가능한 conversation port로 스위치할 수 있지만, planning worker는 아직 `codex app-server`에 남아 있다.  
Evidence: `src/adapter/outbound/terminal_bridge/mod.rs`, `src/adapter/inbound/tui/app/shell_entrypoint.rs`, `src/adapter/outbound/app_server/planning_worker.rs`

- `확인됨`: 이번 비교에서 Akra가 중요하게 보는 판단축은 다음과 같다.  
Evidence: `AKRA.md`, `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`, `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`

1. terminal/TUI 중심성이 유지되는가
2. session-oriented runtime truth가 유지되는가
3. main interactive session과 sub/task runtime을 서로 다르게 붙일 수 있는가
4. provider를 하나의 fake universal API로 위장하지 않는가
5. local bridge와 sidecar와 gateway를 명확히 구분하는가

# Deep Analysis

## Product identity

- `확인됨`: 이 프로젝트는 사용자에게 local multi-agent orchestration shell로 보인다. bare `omc`는 Claude launch wrapper이고, `omc team`은 local tmux worker orchestration이며, `omc interop`는 Claude/Codex split-pane interop surface다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/package.json`, `reference/ohmyclaude/oh-my-claudecode/src/cli/index.ts`, `reference/ohmyclaude/oh-my-claudecode/src/cli/launch.ts`

- `확인됨`: 실제 중심 기능은 terminal orchestrator + local management shell이다. universal provider proxy나 hosted management product가 아니다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/cli/launch.ts`, `reference/ohmyclaude/oh-my-claudecode/src/team/runtime-v2.ts`, `reference/ohmyclaude/oh-my-claudecode/src/team/tmux-session.ts`

## Runtime architecture

- `확인됨`: data plane은 tmux pane 안의 provider CLI다. `buildWorkerArgv`는 HTTP request가 아니라 executable argv를 만든다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/team/model-contract.ts`

- `확인됨`: control plane은 local state files, `team api`, monitor snapshots, heartbeats, prompt/job persistence다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/team/state-paths.ts`, `reference/ohmyclaude/oh-my-claudecode/src/team/api-interop.ts`, `reference/ohmyclaude/oh-my-claudecode/src/mcp/prompt-persistence.ts`, `reference/ohmyclaude/oh-my-claudecode/src/lib/job-state-db.ts`

- `확인됨`: local daemon/proxy가 핵심 전제는 아니다. detached child runtime은 있지만 shared inference daemon이나 universal proxy가 중심이 아니다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/cli/team.ts`, `reference/ohmyclaude/oh-my-claudecode/src/team/runtime-cli.ts`

## Process model

- `확인됨`: 뜨는 프로세스는 최소 다음 층으로 나뉜다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/cli/team.ts`, `reference/ohmyclaude/oh-my-claudecode/src/team/runtime-cli.ts`, `reference/ohmyclaude/oh-my-claudecode/src/mcp/standalone-server.ts`, `reference/ohmyclaude/oh-my-claudecode/src/mcp/team-server.ts`

1. foreground `omc` CLI
2. detached `runtime-cli` child
3. tmux pane 안의 provider workers
4. stdio MCP servers
5. optional OpenClaw HTTP/command gateway wake calls

- `확인됨`: background worker/watcher는 존재한다. 다만 central proxy process보다 `runtime-cli` poll loop, `monitorTeamV2`, heartbeat/status files, pane capture가 중심이다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/team/runtime-cli.ts`, `reference/ohmyclaude/oh-my-claudecode/src/team/runtime-v2.ts`

## CLIProxyAPIPlus integration point

- `강한 추정`: inspect한 repo 범위에서는 `CLIProxyAPIPlus` 문자열이나 명시적 integration point가 없다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/package.json`, `reference/ohmyclaude/oh-my-claudecode/src/cli/index.ts`, `reference/ohmyclaude/oh-my-claudecode/src/cli/launch.ts`, `reference/ohmyclaude/oh-my-claudecode/src/team/runtime-v2.ts`

- `확인됨`: 실제 중심 integration은 “외부 CLI 실행”이다. Claude/Codex/Gemini/Cursor는 local binary contract로 붙고, routing layer도 결국 어떤 CLI를 어느 pane에서 어떤 model/env로 띄울지 정한다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/team/model-contract.ts`, `reference/ohmyclaude/oh-my-claudecode/src/team/stage-router.ts`

- `불확실`: 사용자가 말한 `CLIProxyAPIPlus`가 OpenClaw 같은 sidecar/gateway를 가리키는지, 별도 비공개 컴포넌트를 가리키는지는 repo만으로는 확인되지 않는다.

## Auth/account model

- `확인됨`: worker runtime은 대체로 “이미 로그인된 local CLI 상태를 사용”하는 모델이다. `CLAUDE_CONFIG_DIR`, `ANTHROPIC_MODEL`, `ANTHROPIC_BASE_URL`, `OMC_CODEX_DEFAULT_MODEL` 같은 env가 worker로 전달된다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/team/model-contract.ts`, `reference/ohmyclaude/oh-my-claudecode/src/utils/config-dir.ts`

- `확인됨`: HUD는 Claude Code OAuth credentials를 읽고 refresh까지 수행한다. macOS는 Keychain, 그 외는 `~/.claude/.credentials.json` fallback을 사용하며, `platform.claude.com/v1/oauth/token` refresh와 `api.anthropic.com/api/oauth/usage` 조회를 수행한다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/hud/usage-api.ts`

- `확인됨`: Anthropic 공식 문서는 2026-04-24 기준으로, `Claude Code`의 OAuth authentication은 Free/Pro/Max/Team/Enterprise 구독 구매자용 ordinary use를 위한 것이고, third-party developers는 `Claude.ai login`을 제공하거나 Free/Pro/Max credentials를 사용자 대신 라우팅하면 안 된다고 명시한다.  
Evidence: https://code.claude.com/docs/en/legal-and-compliance, https://code.claude.com/docs/en/setup

- `강한 추정`: 따라서 OMC 코드에서 허용 가능한 범위는 “개인 로컬 HUD/observer가 자기 머신의 local credentials를 읽는 수준”이고, Akra product core나 Akra-managed sidecar가 이를 일반화해 third-party auth relay처럼 동작하는 순간 정책 리스크가 급격히 상승한다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/hud/usage-api.ts`, https://code.claude.com/docs/en/legal-and-compliance

- `확인됨`: OpenAI 쪽은 구조가 다르다. `codex app-server`는 `account/login/start`, `account/updated`, `account/rateLimits/read`, device-code login, 그리고 experimental externally-managed ChatGPT tokens까지 공식 auth surface로 노출한다. Akra가 Codex와 Claude를 같은 auth abstraction으로 다루면 안 되는 이유다.  
Evidence: https://developers.openai.com/codex/app-server#auth-endpoints, https://developers.openai.com/codex/app-server#3b-log-in-with-chatgpt-device-code-flow

## Session/runtime model

- `확인됨`: 이 프로젝트의 session 개념은 provider-universal session object가 아니다. 실제 identity는 tmux session name, pane id, team state root, worker name, task id, background job id의 조합이다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/team/state-paths.ts`, `reference/ohmyclaude/oh-my-claudecode/src/team/tmux-session.ts`, `reference/ohmyclaude/oh-my-claudecode/src/mcp/prompt-persistence.ts`

- `확인됨`: interactive session과 background task session은 구분된다. 팀 worker는 tmux pane interactive runtime이고, Codex/Gemini background jobs는 prompt/job persistence와 jobs DB를 통해 별도 lifecycle을 가진다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/team/runtime-v2.ts`, `reference/ohmyclaude/oh-my-claudecode/src/mcp/prompt-persistence.ts`, `reference/ohmyclaude/oh-my-claudecode/src/lib/job-state-db.ts`

- `확인됨`: attachment/reattach 개념은 분명하다. 다만 provider thread id가 아니라 local pane/session handle 중심이다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/team/tmux-session.ts`, `reference/ohmyclaude/oh-my-claudecode/src/team/state-paths.ts`

## Management/control plane

- `확인됨`: 설정 변경은 CLI config surface와 JSONC config 파일을 통해 이뤄진다. 팀 제어는 `omc team api`와 runtime state files가 담당한다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/cli/index.ts`, `reference/ohmyclaude/oh-my-claudecode/src/config/loader.ts`, `reference/ohmyclaude/oh-my-claudecode/src/team/api-interop.ts`

- `확인됨`: usage/quota/HUD/log/dashboard 성격의 surface가 존재한다. 하지만 이것이 프로젝트의 본질은 아니고, orchestration을 보조하는 local control-plane 기능이다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/hud/usage-api.ts`, `reference/ohmyclaude/oh-my-claudecode/src/mcp/job-management.ts`

## Protocol translation/routing layer

- `확인됨`: 이 repo는 OpenAI/Anthropic/Gemini/Codex 호환 API server를 노출하지 않는다. translation layer의 실체는 request/response JSON compatibility shim이 아니라 role-based provider selection, model selection, Claude fallback이다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/team/stage-router.ts`, `reference/ohmyclaude/oh-my-claudecode/src/team/model-contract.ts`

- `확인됨`: per-role routing snapshot은 team 생성 시점에 immutable하게 고정되며, primary provider binary가 없으면 Claude fallback으로 교체된다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/team/stage-router.ts`, `reference/ohmyclaude/oh-my-claudecode/src/team/runtime-v2.ts`

- `확인됨`: 이 계층은 Akra에 “일부”는 필요하지만, OMC 수준의 전체 `/team` role-routing/control-plane은 Akra core에는 과하다.  
Evidence: `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`, `reference/ohmyclaude/oh-my-claudecode/src/team/stage-router.ts`

## Operational complexity

- `확인됨`: 설치/실행/업데이트 경로는 단순하지 않다. tmux, provider CLI 설치, config dirs, worktrees, local state, MCP, detached runtime, optional notifications가 모두 얽힌다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/cli/tmux-utils.ts`, `reference/ohmyclaude/oh-my-claudecode/src/team/git-worktree.ts`, `reference/ohmyclaude/oh-my-claudecode/src/team/runtime-cli.ts`, `reference/ohmyclaude/oh-my-claudecode/src/cli/index.ts`

- `확인됨`: remote/shared deployment는 핵심 범위가 아니다. 공식 reference 문서도 remote MCP 연결은 supported라고 하면서, shared remote filesystem view나 general OMC cluster는 not implemented라고 선을 긋는다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/docs/REFERENCE.md`

- `강한 추정`: 이 전체 구조를 Akra core에 넣으면 auth 만료/갱신, multi-CLI drift, pane lifecycle, local state repair, worktree cleanup까지 같이 책임져야 해서 유지보수 폭발 가능성이 높다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/team/runtime-v2.ts`, `reference/ohmyclaude/oh-my-claudecode/src/hud/usage-api.ts`, `reference/ohmyclaude/oh-my-claudecode/src/team/git-worktree.ts`

## Borrowable patterns for Akra

- `확인됨`: local state root와 typed path builders는 차용 가치가 높다. Akra의 `SessionCatalog`/`TerminalBridgeAttachment` truth를 local handle 중심으로 유지하는 데 도움이 된다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/team/state-paths.ts`, `src/domain/recent_sessions.rs`, `src/domain/terminal_bridge_attachment.rs`

- `확인됨`: immutable routing snapshot은 차용 가치가 높다. provider runtime selection을 turn/team 시작 시점에 확정하고, 실행 중 config drift가 runtime semantics를 바꾸지 않게 하는 패턴이다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/team/stage-router.ts`, `reference/ohmyclaude/oh-my-claudecode/src/team/runtime-v2.ts`

- `확인됨`: detached runtime artifact convergence 패턴은 Akra sub/task runtime 쪽에 유용하다. detached child가 result artifact를 남기고, foreground shell이 status를 수렴하는 구조다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/cli/team.ts`, `reference/ohmyclaude/oh-my-claudecode/src/team/runtime-cli.ts`, `docs/supersession/current-contract.md`, `src/application/service/parallel_mode/turn.rs`

- `확인됨`: tmux pane liveness + status/heartbeat + pane capture를 조합한 monitor snapshot 패턴은 Akra의 local attach runtime에 일부 차용 가능하다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/team/runtime-v2.ts`, `src/adapter/outbound/terminal_bridge/mod.rs`

## Sidecar-only patterns

- `확인됨`: usage/quota observer는 sidecar-only가 맞다. Akra core가 provider credential refresh나 usage polling 책임을 져서는 안 된다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/hud/usage-api.ts`, https://code.claude.com/docs/en/legal-and-compliance

- `확인됨`: prompt persistence와 background job DB도 sidecar 또는 sub/task runtime support layer로만 두는 것이 맞다. main interactive TUI core에 들어가면 무게가 커진다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/mcp/prompt-persistence.ts`, `reference/ohmyclaude/oh-my-claudecode/src/lib/job-state-db.ts`

- `확인됨`: OpenClaw 같은 gateway wake/notification layer는 sidecar-only다. core runtime이 아니다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/openclaw/index.ts`

- `확인됨`: local MCP utility server는 sidecar-only다. Akra core runtime과 분리돼야 한다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/mcp/standalone-server.ts`, `reference/ohmyclaude/oh-my-claudecode/src/mcp/team-server.ts`

## Do-not-adopt patterns

- `확인됨`: Akra core에 universal AI gateway product 구조를 넣으면 안 된다. Akra의 현재 boundary 문서와 정면 충돌한다.  
Evidence: `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`, `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`, `docs/plan/28-terminal-agent-tmux-local-attach-gate-verdict.md`

- `확인됨`: subscription auth relay나 consumer credential reuse를 product backend처럼 다루면 안 된다. 특히 Claude 쪽은 공식 문서와 직접 충돌한다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/hud/usage-api.ts`, https://code.claude.com/docs/en/legal-and-compliance

- `확인됨`: Akra core를 OMC-style full local team shell로 재정의하면 안 된다. Akra는 session-oriented TUI가 중심이고, OMC는 local orchestration shell이 중심이다. 둘은 겹치지만 동일하지 않다.  
Evidence: `docs/supersession/current-contract.md`, `src/adapter/inbound/tui/app/shell_entrypoint.rs`, `reference/ohmyclaude/oh-my-claudecode/src/cli/index.ts`, `reference/ohmyclaude/oh-my-claudecode/src/team/runtime-v2.ts`

## Evidence

- Akra baseline  
`docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`  
`docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`  
`docs/plan/24-terminal-agent-bridge-experiment-matrix.md`  
`docs/plan/25-codex-assumption-to-capability-target-map.md`  
`docs/plan/28-terminal-agent-tmux-local-attach-gate-verdict.md`  
`docs/supersession/current-contract.md`  
`src/adapter/inbound/tui/app/shell_entrypoint.rs`  
`src/adapter/outbound/terminal_bridge/mod.rs`  
`src/domain/recent_sessions.rs`  
`src/domain/terminal_bridge_attachment.rs`  
`src/application/service/parallel_mode/turn.rs`  
`src/adapter/outbound/app_server/planning_worker.rs`

- Reference repo core  
`reference/ohmyclaude/oh-my-claudecode/package.json`  
`reference/ohmyclaude/oh-my-claudecode/src/cli/index.ts`  
`reference/ohmyclaude/oh-my-claudecode/src/cli/launch.ts`  
`reference/ohmyclaude/oh-my-claudecode/src/cli/team.ts`  
`reference/ohmyclaude/oh-my-claudecode/src/cli/tmux-utils.ts`  
`reference/ohmyclaude/oh-my-claudecode/src/team/runtime.ts`  
`reference/ohmyclaude/oh-my-claudecode/src/team/runtime-v2.ts`  
`reference/ohmyclaude/oh-my-claudecode/src/team/runtime-cli.ts`  
`reference/ohmyclaude/oh-my-claudecode/src/team/model-contract.ts`  
`reference/ohmyclaude/oh-my-claudecode/src/team/stage-router.ts`  
`reference/ohmyclaude/oh-my-claudecode/src/team/state-paths.ts`  
`reference/ohmyclaude/oh-my-claudecode/src/team/tmux-session.ts`  
`reference/ohmyclaude/oh-my-claudecode/src/team/git-worktree.ts`  
`reference/ohmyclaude/oh-my-claudecode/src/team/api-interop.ts`  
`reference/ohmyclaude/oh-my-claudecode/src/openclaw/index.ts`  
`reference/ohmyclaude/oh-my-claudecode/src/hud/usage-api.ts`  
`reference/ohmyclaude/oh-my-claudecode/src/mcp/standalone-server.ts`  
`reference/ohmyclaude/oh-my-claudecode/src/mcp/team-server.ts`  
`reference/ohmyclaude/oh-my-claudecode/src/mcp/prompt-persistence.ts`  
`reference/ohmyclaude/oh-my-claudecode/src/mcp/job-management.ts`  
`reference/ohmyclaude/oh-my-claudecode/src/lib/job-state-db.ts`  
`reference/ohmyclaude/oh-my-claudecode/docs/REFERENCE.md`

- Official docs  
https://code.claude.com/docs/en/legal-and-compliance  
https://code.claude.com/docs/en/setup  
https://developers.openai.com/codex/app-server#auth-endpoints  
https://developers.openai.com/codex/app-server#3b-log-in-with-chatgpt-device-code-flow

# Akra Mapping

## adopt in Akra core

- `확인됨`: `StartupProbe`, `InteractiveTurnRuntime`, optional `SessionCatalog`, `TerminalBridgeAttachment` 축을 유지하면서 local handle truth를 강화하는 것
- `확인됨`: attach-only, handle-based reattach, provider-backed catalog tier를 명시적으로 유지하는 것
- `확인됨`: provider runtime selection을 immutable launch snapshot으로 고정하는 패턴
- `확인됨`: detached sub/task runtime의 result artifact convergence 패턴 일부

## sidecar only

- `확인됨`: local headless CLI runner
- `확인됨`: prompt/job persistence
- `확인됨`: usage/quota observer
- `확인됨`: notification gateway
- `확인됨`: local MCP utility server

## do not adopt

- `확인됨`: universal AI gateway server product화
- `확인됨`: third-party subscription OAuth relay
- `확인됨`: Free/Pro/Max 또는 consumer credential pooling/routing
- `확인됨`: Akra core를 OMC-style full team shell로 바꾸는 구조

## research later

- `강한 추정`: sub/task queue에만 적용되는 minimal local headless CLI runner contract
- `강한 추정`: Codex 공식 auth surface를 sub/task runtime에서 어디까지 활용할지
- `불확실`: Claude local-only observer/sidecar의 허용 가능한 최소 범위와 distribution 방식

# Recommendation

`tmux/local terminal bridge 중심`이 가장 맞다.

- `확인됨`: 이것이 Akra의 현재 문서화된 primary path이고, 이미 capability seam도 그 방향으로 쪼개져 있다.  
Evidence: `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`, `docs/plan/28-terminal-agent-tmux-local-attach-gate-verdict.md`

- `확인됨`: reference repo에서 가장 가치 있는 부분도 proxy가 아니라 local pane/session/orchestration 패턴이다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/cli/launch.ts`, `reference/ohmyclaude/oh-my-claudecode/src/team/runtime-v2.ts`, `reference/ohmyclaude/oh-my-claudecode/src/team/tmux-session.ts`

- `확인됨`: `local headless CLI runner 중심`은 Akra main interactive session의 primary path로는 과하고, sub/task runtime 보조 수단으로만 검토하는 편이 맞다.  
Evidence: `docs/supersession/current-contract.md`, `src/application/service/parallel_mode/turn.rs`, `reference/ohmyclaude/oh-my-claudecode/src/mcp/prompt-persistence.ts`

- `확인됨`: `local sidecar daemon 추가`는 HUD/job persistence/notification 쪽에는 의미가 있을 수 있지만, Akra core를 그 구조 위에 재편하는 것은 권장되지 않는다.  
Evidence: `reference/ohmyclaude/oh-my-claudecode/src/hud/usage-api.ts`, `reference/ohmyclaude/oh-my-claudecode/src/openclaw/index.ts`

- `확인됨`: `full proxy/gateway 별도 제품화`는 현재 증거 기준으로 과하다. Akra 문서도 이를 deferred로 두고 있고, reference repo도 실제로 그 방향의 core product가 아니다.  
Evidence: `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`, `docs/plan/24-terminal-agent-bridge-experiment-matrix.md`, `reference/ohmyclaude/oh-my-claudecode/docs/REFERENCE.md`

# Open Questions

- `불확실`: 사용자가 말한 `CLIProxyAPIPlus`가 정확히 무엇을 뜻하는가  
중요성: 현재 repo에는 명시적 통합 흔적이 없어, “중심 엔진인가 sidecar인가”를 더 강하게 단정하기 어렵다.  
추가 확인 필요: upstream 설명, 별도 package 이름, 관련 문서 또는 다른 repo

- `강한 추정`: Akra sub/task queue용 Claude 경로에서 truly headless local runner가 필요한가, 아니면 tmux attach만으로 충분한가  
중요성: main/sub runtime 분리 설계와 sidecar 범위를 결정한다.  
추가 확인 필요: Akra 쪽 sub/task 실험 문서, local runner spike, terminal fidelity 비교 실험

- `확인됨`: Codex와 Claude는 공식 auth surface가 다르다  
중요성: provider runtime selection을 하나의 fake common auth/session model로 만들면 설계가 틀어진다.  
추가 확인 필요: OpenAI app-server auth/account surface의 실제 runtime coupling 범위, Anthropic Claude Code의 local-only 허용 범위 세부 문구
