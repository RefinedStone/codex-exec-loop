# Akra Reference Analysis: vibeproxy

This note records the April 24, 2026 analysis requested by `reference/vibeproxy/AKRA.md`.

The source reference checkout used for this analysis lived in the local ignored path
`reference/vibeproxy/vibeproxy/` and is not part of the tracked repository state. File references to
the reference project below therefore describe the local review context, not guaranteed paths in a
clean clone.

## Status Legend

- `확인됨`: directly supported by inspected code or official docs
- `강한 추정`: strongly implied by inspected code, but not fully proven from this repo alone
- `불확실`: needs extra code, runtime evidence, or official clarification

## Executive Summary

- `확인됨` 현재 reference repo는 "macOS 메뉴바 UI가 붙은 로컬 프록시 제품"에 가깝습니다. 사용자가 붙는 주소는 `ThinkingProxy`가 여는 `localhost:8317`이고, 그 뒤에서 외부 `cli-proxy-api-plus`가 `8318`에서 돌아가며, 앱은 인증·설정·계정 풀링·핫리로드·대시보드 진입을 관리합니다. 근거: `vibeproxy/src/Sources/AppDelegate.swift`, `vibeproxy/src/Sources/ThinkingProxy.swift`, `vibeproxy/src/Sources/ServerManager.swift`, `vibeproxy/src/Sources/Resources/config.yaml`
- `확인됨` CLIProxyAPIPlus는 이 프로젝트의 "부품"이 아니라 사실상 중심 엔진입니다. Swift 앱은 관리 셸과 전면 보정 프록시이고, 핵심 provider auth, routing, backend API는 외부 바이너리에 실려 있습니다. 근거: `vibeproxy/src/Sources/ServerManager.swift`, `vibeproxy/.github/workflows/update-cliproxyapi.yml`, `vibeproxy/create-app-bundle.sh`
- `확인됨` Akra relevance 총평은 "코어 채용 부적합, 일부 운영 패턴만 제한적으로 차용 가능"입니다. Akra는 이미 `InteractiveTurnRuntime`, `StartupProbe`, `SessionCatalog`, `TerminalBridgeAttachment`로 capability seam을 정리했고, `tmux/local attach`를 1차 경로로 선택했습니다. 근거: `src/application/port/outbound/*.rs`, `src/adapter/outbound/terminal_bridge/mod.rs`, `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`, `docs/plan/27-terminal-agent-tmux-local-attach-readiness-evidence.md`, `docs/plan/28-terminal-agent-tmux-local-attach-gate-verdict.md`

## Akra Baseline

- `확인됨` Akra의 현재 구조는 `adapter -> application -> domain`이고, runtime seam을 capability 중심으로 쪼개는 방향입니다. 근거: `docs/design/04-hexagonal-runtime-architecture.md`, `src/application/port/outbound/interactive_turn_runtime_port.rs`, `src/application/port/outbound/startup_probe_port.rs`, `src/application/port/outbound/session_catalog_port.rs`
- `확인됨` Akra는 아직 `CodexAppServerPort` 호환 포트를 남겨 두었지만, 문서와 코드 둘 다 이를 임시 호환층으로 취급합니다. 근거: `src/application/port/outbound/codex_app_server_port.rs`, `docs/plan/25-codex-assumption-to-capability-target-map.md`
- `확인됨` Akra는 main interactive shell과 hidden planning 또는 subsession lane을 이미 분리하고 있습니다. queue-driven work는 별 worktree slot과 hidden planning refresh를 거칩니다. 근거: `docs/supersession/current-contract.md`, `src/adapter/outbound/app_server/planning_worker.rs`, `src/application/service/parallel_mode/turn.rs`, `src/application/service/parallel_mode/distributor.rs`
- `확인됨` terminal bridge의 현재 1차 판단은 `tmux/local attach`, 2차 fallback은 managed wrapper이며, proxy 또는 vibeProxy-style mediation은 명시적으로 defer입니다. 근거: `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`, `docs/plan/22-terminal-agent-transport-and-attachment-matrix.md`, `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`, `docs/plan/27-terminal-agent-tmux-local-attach-readiness-evidence.md`, `docs/plan/28-terminal-agent-tmux-local-attach-gate-verdict.md`, `src/domain/terminal_bridge_attachment.rs`, `src/adapter/outbound/terminal_bridge/mod.rs`

## Deep Analysis

### Product Identity

- `확인됨` 사용자에게 보이는 제품은 menu bar app이지만, 실제 중심 기능은 "개인 구독 자격 증명을 로컬 API 또는 proxy로 바꿔 외부 도구에 공급하는 것"입니다. 근거: `vibeproxy/FACTORY_SETUP.md`, `vibeproxy/src/Sources/AppDelegate.swift`, `vibeproxy/src/Sources/ThinkingProxy.swift`, `vibeproxy/src/Sources/ServerManager.swift`
- `확인됨` terminal orchestrator보다는 proxy 또는 gateway에 훨씬 가깝고, desktop wrapper와 management shell이 그 앞면을 담당합니다. 근거: `vibeproxy/src/Sources/main.swift`, `vibeproxy/src/Sources/AppDelegate.swift`, `vibeproxy/src/Sources/SettingsView.swift`

### Runtime Architecture

- `확인됨` control plane은 SwiftUI 또는 AppKit UI, 파일 감시, merged-config 생성, provider enable 또는 disable, auth mutation입니다. data plane은 인프로세스 `ThinkingProxy`와 외부 `cli-proxy-api-plus` 조합입니다. 근거: `vibeproxy/src/Sources/AppDelegate.swift`, `vibeproxy/src/Sources/ServerManager.swift`, `vibeproxy/src/Sources/ConfigComposer.swift`, `vibeproxy/src/Sources/ConfigInputFingerprint.swift`
- `확인됨` 최소 3종 프로세스가 뜹니다. 앱 프로세스, 장기 실행 backend `cli-proxy-api-plus`, provider별 short-lived auth subprocess입니다. `TunnelManager`는 존재하지만 호출 지점이 확인되지 않아 현행 shipped path는 아닙니다. 근거: `vibeproxy/src/Sources/ServerManager.swift`, `vibeproxy/src/Sources/TunnelManager.swift`

### CLIProxyAPIPlus Integration Point

- `확인됨` 외부 바이너리를 번들에 넣고 `-config`, `-claude-login`, `-codex-login` 같은 인자로 실행합니다. embedded library가 아니라 executable dependency입니다. 근거: `vibeproxy/src/Sources/ServerManager.swift`, `vibeproxy/create-app-bundle.sh`
- `확인됨` 앱은 backend management endpoint와 config 또는 auth dir를 직접 다룹니다. 단순 HTTP client가 아니라 supervisor이며 control plane입니다. 근거: `vibeproxy/src/Sources/AppDelegate.swift`, `vibeproxy/src/Sources/ServerManager.swift`, `vibeproxy/src/Sources/AuthStatus.swift`

### Auth And Account Model

- `확인됨` 앱은 `~/.cli-proxy-api/*.json`을 스캔하고, 계정 disable 또는 delete와 custom provider API key, Z.AI key 저장을 직접 관리합니다. 다중 계정 UI와 disable 플래그는 명시적입니다. 근거: `vibeproxy/src/Sources/AuthStatus.swift`, `vibeproxy/src/Sources/CustomProviderCredentialStore.swift`, `vibeproxy/src/Sources/ZAIAPIKeyStore.swift`, `vibeproxy/src/Sources/SettingsView.swift`
- `강한 추정` 실제 round-robin, failover, session-affinity 알고리즘은 opaque backend인 `cli-proxy-api-plus` 쪽 책임입니다. 현재 Swift 코드만으로는 세부 정책을 확정할 수 없습니다. 근거: `vibeproxy/src/Sources/SettingsView.swift`, `vibeproxy/src/Sources/ServerManager.swift`
- `확인됨` 구조 자체는 consumer subscription 자격을 product backend처럼 재사용하는 방향으로 읽힙니다. 특히 provider별 login subprocess와 로컬 API endpoint 조합이 그 성격을 드러냅니다. 근거: `vibeproxy/src/Sources/ServerManager.swift`, `vibeproxy/src/Sources/ThinkingProxy.swift`

### Session And Runtime Model

- `확인됨` Swift 코드에는 Akra 같은 terminal session, attach, recover truth가 없습니다. 여기서 핵심 상태는 auth account, provider enabled-state, merged config, HTTP request rewriting입니다. 근거: `vibeproxy/src/Sources/ThinkingProxy.swift`, `vibeproxy/src/Sources/ServerManager.swift`, `vibeproxy/src/Sources/SettingsView.swift`
- `강한 추정` provider-side sticky routing 또는 request session 개념이 있더라도 그것은 `cli-proxy-api-plus` 내부 책임입니다. 이 repo만으로 terminal-style session truth는 확인되지 않습니다.

### Management And Control Plane

- `확인됨` start 또는 stop, auth folder 열기, provider 토글, custom provider 추가, config error surface, backend dashboard open, auth file watcher, config fingerprint polling을 모두 가집니다. 부수 기능이 아니라 제품 중심 control plane입니다. 근거: `vibeproxy/src/Sources/SettingsView.swift`, `vibeproxy/src/Sources/AppDelegate.swift`, `vibeproxy/src/Sources/ConfigInputFingerprint.swift`, `vibeproxy/src/Sources/ServerManager.swift`

### Protocol And Translation Layer

- `확인됨` `ThinkingProxy`는 request body 수정, model alias rewrite, `-thinking-NUMBER`를 Anthropic body 또는 header로 변환, `cache_control` 제거, Amp path 또는 cookie 또는 location rewrite, Vercel AI Gateway reroute, `/api` retry까지 수행합니다. 이는 단순 pass-through가 아니라 compatibility shim입니다. 근거: `vibeproxy/src/Sources/ThinkingProxy.swift`, `vibeproxy/src/Sources/ModelAliasMapper.swift`
- `확인됨` 이 translation layer는 Akra에 그대로 가져오기엔 과합니다. Akra의 현재 문제는 terminal runtime capability이고, universal request compatibility layer가 아닙니다. 근거: `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`, `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`

### Operational Complexity

- `확인됨` 빌드와 배포는 Swift app, bundled binary, Sparkle, signing, auto-bump workflow 조합입니다. 런타임은 orphan kill, watcher 또는 poller, merged config rewrite, restart orchestration까지 포함합니다. Akra core에 넣기엔 과중합니다. 근거: `vibeproxy/create-app-bundle.sh`, `vibeproxy/src/Package.swift`, `vibeproxy/src/Sources/ServerManager.swift`, `vibeproxy/.github/workflows/update-cliproxyapi.yml`
- `확인됨` 기본 설정은 localhost-only에 가깝습니다. 다만 `TunnelManager`와 remote management 관련 흔적 때문에 원격 확장 가능성까지 완전히 배제된 구조는 아닙니다. 근거: `vibeproxy/src/Sources/Resources/config.yaml`, `vibeproxy/src/Sources/TunnelManager.swift`

## Policy Tension

- `확인됨` OpenAI Codex 공식 문서는 programmatic Codex CLI workflows에는 API key auth를 권장하고, Codex execution을 untrusted 또는 public environment에 노출하지 말라고 안내합니다. 근거: OpenAI Codex auth docs `https://developers.openai.com/codex/auth#sign-in-with-an-api-key`
- `확인됨` OpenAI 계정 정책은 계정 공유를 금지합니다. local single-user 사용은 shared product use보다 위험이 낮지만, subscription credential을 API-like backend로 재사용하는 구조는 정책 긴장이 있습니다. 근거: OpenAI Terms of Use `https://openai.com/policies/terms-of-use/`, OpenAI Help `https://help.openai.com/en/articles/10471989`
- `확인됨` Anthropic consumer terms는 계정 credential 공유 금지와, API 또는 명시 허용 외 자동 접근 금지를 더 직접적으로 적고 있습니다. local third-party proxy 구조는 특히 긴장이 큽니다. 근거: Anthropic Consumer Terms `https://www.anthropic.com/legal/consumer-terms`, Claude Code Pro or Max article `https://support.claude.com/en/articles/11145838-using-claude-code-with-your-pro-or-max-plan`
- `확인됨` 따라서 "코드상 이렇게 구현됨"과 "공식 surface상 안전하게 권장됨" 사이에는 분명한 간극이 있습니다. Akra가 이를 productized path로 가져가는 것은 정책 리스크가 큽니다.

## Akra Relevance

### Akra Core에 넣어도 되는 것

- capability-first seam naming
- `StartupProbe`, `InteractiveTurnRuntime`, optional `SessionCatalog`, `TerminalBridgeAttachment` 분리
- hidden planning worker와 main shell 분리 유지
- readiness gating과 explicit failure copy

### Akra 외부 sidecar로만 둘 것

- local sidecar lifecycle manager
- readiness poll과 ordered startup or shutdown
- config fingerprint 감시
- diagnostics 또는 dashboard surface

### 절대 가져오면 안 되는 것

- universal AI gateway 제품 구조
- OpenAI 또는 Anthropic 또는 Gemini 호환 API server 제품화
- multi-account pool 또는 round-robin product화
- consumer subscription OAuth relay
- browser or cookie or path compatibility cloaking
- session-oriented TUI보다 proxy server가 중심이 되는 구조

### 추가 연구가 필요한 것

- 공식 허용 surface만 쓰는 매우 좁은 local sidecar가 필요한지
- API key 또는 commercial terms 기반 공식 경로만으로 headless runner fallback을 만들 수 있는지
- opaque upstream 없이도 session truth를 보존할 최소 adapter가 무엇인지

## Recommendation

`tmux/local terminal bridge 중심`이 가장 맞습니다.

Akra는 이미 그 방향으로 capability seam과 evidence를 쌓았습니다. 반대로 reference repo는 terminal agent product라기보다 subscription-backed local proxy product입니다. `local headless CLI runner`는 fallback으로는 괜찮지만 1차 중심축으로 두면 hidden automation과 runtime abstraction이 다시 커집니다. `local sidecar daemon`은 필요해도 보조 수단이어야 하고, `full proxy or gateway 별도 제품화`는 Akra의 정체성을 바꿉니다.

## Open Questions

- `불확실` `cli-proxy-api-plus` 내부의 실제 session-affinity, failover, quota switching 알고리즘은 이 repo만으로 확인 불가합니다. 추가로 upstream source 또는 management API schema가 필요합니다.
- `강한 추정` Anthropic 또는 OpenAI consumer subscription을 third-party local proxy로 쓰는 행위는 정책 긴장이 크지만, "단일 개인의 로컬 개인 사용"에 대한 명시 허용 또는 금지 문구는 더 직접적인 지원 또는 법무 해석이 필요합니다.
- `확인됨` Akra에는 이미 `tmux/local attach` 증거와 gate verdict가 있으므로, 다음 실험은 proxy가 아니라 completion detection, approval handoff, recovery UX를 tmux path에서 다듬는 쪽이 더 가치가 큽니다. 근거: `docs/plan/27-terminal-agent-tmux-local-attach-readiness-evidence.md`, `docs/plan/28-terminal-agent-tmux-local-attach-gate-verdict.md`, `src/adapter/outbound/terminal_bridge/mod.rs`

## Evidence

- Akra baseline:
  - `docs/design/04-hexagonal-runtime-architecture.md`
  - `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`
  - `docs/plan/22-terminal-agent-transport-and-attachment-matrix.md`
  - `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`
  - `docs/plan/25-codex-assumption-to-capability-target-map.md`
  - `docs/plan/27-terminal-agent-tmux-local-attach-readiness-evidence.md`
  - `docs/plan/28-terminal-agent-tmux-local-attach-gate-verdict.md`
  - `docs/supersession/current-contract.md`
  - `src/application/port/outbound/interactive_turn_runtime_port.rs`
  - `src/application/port/outbound/startup_probe_port.rs`
  - `src/application/port/outbound/session_catalog_port.rs`
  - `src/application/port/outbound/codex_app_server_port.rs`
  - `src/adapter/outbound/app_server/planning_worker.rs`
  - `src/application/service/parallel_mode/turn.rs`
  - `src/application/service/parallel_mode/distributor.rs`
  - `src/adapter/outbound/terminal_bridge/mod.rs`
  - `src/domain/terminal_bridge_attachment.rs`
- Local reference checkout reviewed on April 24, 2026:
  - `reference/vibeproxy/vibeproxy/README.md`
  - `reference/vibeproxy/vibeproxy/FACTORY_SETUP.md`
  - `reference/vibeproxy/vibeproxy/src/Package.swift`
  - `reference/vibeproxy/vibeproxy/src/Sources/main.swift`
  - `reference/vibeproxy/vibeproxy/src/Sources/AppDelegate.swift`
  - `reference/vibeproxy/vibeproxy/src/Sources/ServerManager.swift`
  - `reference/vibeproxy/vibeproxy/src/Sources/ThinkingProxy.swift`
  - `reference/vibeproxy/vibeproxy/src/Sources/TunnelManager.swift`
  - `reference/vibeproxy/vibeproxy/src/Sources/AuthStatus.swift`
  - `reference/vibeproxy/vibeproxy/src/Sources/ProviderCatalog.swift`
  - `reference/vibeproxy/vibeproxy/src/Sources/ModelAliasMapper.swift`
  - `reference/vibeproxy/vibeproxy/src/Sources/ConfigComposer.swift`
  - `reference/vibeproxy/vibeproxy/src/Sources/ConfigInputFingerprint.swift`
  - `reference/vibeproxy/vibeproxy/src/Sources/NotificationNames.swift`
  - `reference/vibeproxy/vibeproxy/src/Sources/CustomProviderCredentialStore.swift`
  - `reference/vibeproxy/vibeproxy/src/Sources/ZAIAPIKeyStore.swift`
  - `reference/vibeproxy/vibeproxy/src/Sources/CustomProviders.swift`
  - `reference/vibeproxy/vibeproxy/src/Sources/SettingsView.swift`
  - `reference/vibeproxy/vibeproxy/src/Sources/Resources/config.yaml`
  - `reference/vibeproxy/vibeproxy/.github/workflows/update-cliproxyapi.yml`
  - `reference/vibeproxy/vibeproxy/create-app-bundle.sh`

