# Executive Summary

- `[확인됨]` 현재 reference 프로젝트 CCS는 terminal attach orchestrator라기보다 multi-provider runtime manager, auth/account broker, local/remote CLIProxy control plane, dashboard, protocol translation을 묶은 운영 제품이다.
- 근거 파일: `reference/ccs/ccs/docs/project-overview-pdr.md`
- 심볼/키: `Product Overview`, `FR-003`, `FR-008`, `FR-009`, `AC-003`
- 설명: 제품 정의 자체가 multi-account, OAuth providers, remote CLIProxy, quota, dashboard, local proxy를 핵심 기능으로 둔다.
- 근거 파일: `reference/ccs/ccs/src/ccs.ts`
- 심볼/키: `registerTarget`, `startOpenAICompatProxy`, `ensureCliproxyService`
- 설명: 단일 CLI 엔트리에서 Claude/Droid/Codex target adapter, local proxy, browser/image/websearch runtime까지 함께 조합한다.

- `[확인됨]` CLIProxyAPIPlus는 CCS의 중심 추상화가 아니라 provider-specific backend/management dependency였고, 현재 로컬 코드 기준으로는 `plus` backend를 `original`로 강등하는 유지보수 상태다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/binary-manager.ts`
- 심볼/키: `resolveLocalBackend`, `getPlusBackendUnavailableMessage`
- 설명: CLIProxyAPIPlus upstream unavailable 상황을 전제로 runtime fallback을 구현해 두었다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/auth/oauth-handler.ts`
- 심볼/키: `requestPasteCallbackStart`, `buildManagementHeaders`
- 설명: Kiro/GitLab 등 provider-specific auth flow가 CLIProxy management API contract에 직접 매달린다.

- `[확인됨]` Akra relevance의 핵심은 “CCS 전체를 가져올지”가 아니라 “capability seam, startup/readiness, local daemon hygiene만 선택적으로 가져올지”다.
- 근거 파일: `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`
- 심볼/키: `InteractiveTurnRuntime`, `StartupProbe`, `SessionCatalog`, `TerminalBridgeAttachment`
- 설명: Akra는 이미 provider API가 아니라 capability seam 중심으로 방향을 굳혀 두었다.
- 근거 파일: `docs/plan/28-terminal-agent-tmux-local-attach-gate-verdict.md`
- 심볼/키: `tmux local attach stays the primary path`
- 설명: Akra의 1차 구현 경로는 proxy가 아니라 tmux local attach로 이미 판정되었다.

- `[확인됨]` 가장 중요한 결론 3개는 다음과 같다.
- `1.` Akra가 CCS 구조를 core에 흡수하면 terminal orchestrator가 아니라 proxy/gateway product 쪽으로 정체성이 이동한다.
- `2.` Akra가 빌릴 것은 multi-provider control plane이 아니라 capability wording, startup probe, optional catalog, local daemon/session hygiene다.
- `3.` 최종 추천은 tmux/local terminal bridge 중심 유지이며, 필요 시 local sidecar daemon을 얇게 추가하는 선이 상한이다.

# Akra Baseline

- `[확인됨]` Akra의 현재 구조는 `adapter -> application -> domain`과 small-context readability를 강하게 유지하는 hexagonal runtime이다.
- 근거 파일: `docs/design/04-hexagonal-runtime-architecture.md`
- 심볼/키: `Layer Ownership`, `Invariants`
- 설명: runtime feature는 service facade와 outbound port 뒤에 두고, protocol/filesystem detail은 adapter에 격리하는 방향을 명시한다.

- `[확인됨]` Akra는 이미 `codex app-server` 가정에서 capability-owned seam으로 이동 중이다.
- 근거 파일: `src/application/port/outbound/codex_app_server_port.rs`
- 심볼/키: `compatibility port while application services migrate to capability-owned seams`
- 설명: 기존 Codex-shaped port를 호환 계층으로 남기고 `StartupProbe`, `SessionCatalog`, `InteractiveTurnRuntime`로 분리하고 있다.
- 근거 파일: `src/application/service/conversation_service.rs`
- 심볼/키: `ConversationService`
- 설명: 대화 실행은 이미 `InteractiveTurnRuntimePort`에 의존한다.
- 근거 파일: `src/application/service/session_service.rs`
- 심볼/키: `SessionService`
- 설명: 세션 목록은 `SessionCatalogPort`로 분리되어 있다.
- 근거 파일: `src/application/service/startup_service.rs`
- 심볼/키: `StartupService`
- 설명: 시작 진단은 `StartupProbePort`를 통해 attachment profile과 access/readiness를 받는다.

- `[확인됨]` Akra의 현재 제약은 “모든 provider가 `codex app-server`처럼 행동한다”는 가정을 줄이되, TUI/session 중심성은 유지하는 것이다.
- 근거 파일: `docs/plan/25-codex-assumption-to-capability-target-map.md`
- 심볼/키: `Mapping Table`
- 설명: startup, session list, turn execution, approval, reconnect가 app-server assumption에 묶여 있음을 감사하고 capability target으로 재배치한다.

- `[확인됨]` Akra는 tmux/local attach를 primary path로 본다.
- 근거 파일: `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`
- 심볼/키: `Current Decision`
- 설명: pre-opened local terminal attach, especially tmux-oriented attachment를 concrete operator-ready path로 둔다.
- 근거 파일: `docs/plan/22-terminal-agent-transport-and-attachment-matrix.md`
- 심볼/키: `Candidate Summary`, `Working Conclusion`
- 설명: tmux local attach를 primary, managed wrapper를 fallback, proxy mediation을 deferred로 둔다.
- 근거 파일: `docs/plan/28-terminal-agent-tmux-local-attach-gate-verdict.md`
- 심볼/키: `Path Decision After Evidence`
- 설명: local path가 credible하므로 proxy mediation은 여전히 deferred라는 판정을 반복 확인한다.

- `[확인됨]` Akra가 이번 비교에서 중요하게 보는 판단축은 startup probe, interactive turn runtime, session catalog truth, interrupt/approval/recovery truth, main/sub runtime 분리다.
- 근거 파일: `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`
- 심볼/키: `Capability Summary`, `Session Contract Tiers`
- 설명: Akra가 universal provider session API를 만들지 않고 partial capability를 허용하는 기준을 명문화한다.
- 근거 파일: `src/domain/terminal_bridge_attachment.rs`
- 심볼/키: `TerminalBridgeAttachmentMode`, `TerminalBridgeRecoveryAnchor`
- 설명: launch/reattach/local attach/proxy-mediated와 recovery anchor vocabulary가 이미 domain에 있다.

# Deep Analysis

## Product identity

- `[확인됨]` CCS는 사용자가 보기에도 “Claude를 조금 도와주는 wrapper”보다 “다중 provider, 다중 account, dashboard, local/remote proxy를 관리하는 운영 도구”에 가깝다.
- 근거 파일: `reference/ccs/ccs/docs/project-overview-pdr.md`
- 심볼/키: `Tagline`, `Description`, `FR-003`, `FR-004A`, `FR-008`, `FR-009`, `FR-012`
- 설명: 제품 설명이 profile switching을 넘어서 OAuth providers, AI providers, remote CLIProxy, channels, analytics까지 포함한다.
- 근거 파일: `reference/ccs/ccs/src/ccs.ts`
- 심볼/키: `registerTarget(new ClaudeAdapter())`, `registerTarget(new DroidAdapter())`, `registerTarget(new CodexAdapter())`
- 설명: 엔트리포인트가 단일 target용 wrapper가 아니라 multi-target launcher임을 보여준다.

- `[강한 추정]` CCS의 실제 중심 기능은 Claude 실행 그 자체가 아니라 “Claude-compatible surface를 다른 provider와 runtime에 연결해 주는 mediation”이다.
- 근거 파일: `reference/ccs/ccs/src/proxy/server/messages-route.ts`
- 심볼/키: `handleProxyMessagesRequest`
- 설명: Anthropic `/v1/messages` 요청을 받아 OpenAI-compatible upstream으로 재전달한다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/service-manager.ts`
- 심볼/키: `ensureCliproxyService`
- 설명: dashboard와 auth/stats를 위해 persistent CLIProxy background instance를 전제로 한다.

## Runtime architecture

- `[확인됨]` CCS는 단일 프로세스가 아니라 여러 runtime shape를 동시에 가진다.
- 근거 파일: `reference/ccs/ccs/src/management/instance-manager.ts`
- 심볼/키: `ensureInstance`, `CLAUDE_CONFIG_DIR instance`
- 설명: profile별 isolated Claude instance directory를 만든다.
- 근거 파일: `reference/ccs/ccs/src/web-server/index.ts`
- 심볼/키: `startServer`
- 설명: dashboard HTTP server, WebSocket, auth/session middleware, local reverse proxy, watcher를 한 프로세스에 묶는다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/service-manager.ts`
- 심볼/키: `proxyProcess`, `tokenRefreshWorker`
- 설명: detached CLIProxy process와 token refresh worker를 별도로 관리한다.
- 근거 파일: `reference/ccs/ccs/src/proxy/proxy-daemon.ts`
- 심볼/키: `startOpenAICompatProxy`, `getOpenAICompatProxyStatus`
- 설명: OpenAI-compatible local proxy daemon을 또 하나 별도로 운영한다.

- `[확인됨]` data plane과 control plane이 섞여 있지만 둘 다 강하게 존재한다.
- 근거 파일: `reference/ccs/ccs/src/proxy/server/proxy-server.ts`
- 심볼/키: `/v1/messages`, `/v1/models`
- 설명: local proxy server는 명백한 data plane이다.
- 근거 파일: `reference/ccs/ccs/src/web-server/routes/index.ts`
- 심볼/키: `/api/cliproxy/*`, `/api/cliproxy-server`, `/api/auth`
- 설명: dashboard API는 auth/account/config/stats를 담당하는 control plane이다.

## Process model

- `[확인됨]` dashboard 기동은 CLIProxy service를 먼저 보장하고, 그 뒤 dashboard server를 띄우는 순서다.
- 근거 파일: `reference/ccs/ccs/src/commands/config-command.ts`
- 심볼/키: `handleConfigCommand`
- 설명: `ccs config` 실행 시 dashboard보다 먼저 `ensureCliproxyService()`가 호출된다.

- `[확인됨]` CLIProxy는 detached background process로 남고, 세션 lock과 refcount로 여러 CCS 세션이 공유한다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/service-manager.ts`
- 심볼/키: `spawn(... detached: true)`, `registerSession`
- 설명: CLIProxy를 백그라운드 서비스로 띄우고 detached로 유지한다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/session-tracker.ts`
- 심볼/키: `SessionLock`, `registerSession`, `unregisterSession`, `stopProxy`
- 설명: port별 lock file과 session refcount로 shared proxy lifecycle을 관리한다.

- `[확인됨]` OAuth는 별도 child process를 띄워 stdout/stderr를 파싱하는 방식이다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/auth/oauth-process.ts`
- 심볼/키: `executeOAuthProcess`, `handleProjectSelection`, `deviceCodeEvents`
- 설명: provider auth는 child process output parsing과 callback/device-code UI mediation으로 구성된다.

## CLIProxyAPIPlus integration point

- `[확인됨]` CCS는 CLIProxyAPIPlus를 embedded library로 쓰지 않고 외부 binary와 management HTTP API로 사용한다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/service-manager.ts`
- 심볼/키: `ensureCLIProxyBinary`, `spawn(binaryPath, proxyArgs, ...)`
- 설명: binary를 내려받아 외부 프로세스로 실행한다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/management-api-client.ts`
- 심볼/키: `ManagementApiClient`
- 설명: 별도 management API client를 통해 `/v0/management/*`와 routing endpoints를 호출한다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/proxy-target-resolver.ts`
- 심볼/키: `getProxyTarget`, `buildManagementHeaders`
- 설명: local/remote target, management secret, remote management key를 분기한다.

- `[확인됨]` provider별 Kiro/GHCP path는 CLIProxyAPIPlus-style contract를 기대했지만, 현재 local codebase는 그 backend를 신뢰할 수 없는 상태로 본다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/binary-manager.ts`
- 심볼/키: `CLIPROXY_PLUS_TRACKING_URL`, `resolveLocalBackend`
- 설명: Plus upstream repo 삭제를 전제로 original backend로 degrade하는 보호 로직이 있다.
- 근거 파일: `reference/ccs/ccs/docs/system-architecture/provider-flows.md`
- 심볼/키: `Supported Hardcoded Providers`
- 설명: 문서상 Kiro/GHCP가 CLIProxyAPIPlus에 묶여 있음을 드러낸다.

## Auth/account model

- `[확인됨]` 코드상 구현: CCS는 OAuth/device-code flow를 직접 시작하고, token file을 직접 읽고, account registry를 직접 관리한다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/auth/oauth-handler.ts`
- 심볼/키: `triggerOAuth`, `requestPasteCallbackStart`
- 설명: CCS가 browser/device-code/manual callback flow를 직접 orchestration한다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/auth/token-manager.ts`
- 심볼/키: `getProviderTokenDir`, `listProviderTokenSnapshots`, `isAuthenticated`
- 설명: token file을 직접 스캔해 auth 상태와 신규 account를 판별한다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/accounts/token-file-ops.ts`
- 심볼/키: `moveTokenToPaused`, `moveTokenFromPaused`, `deleteTokenFile`
- 설명: account pause/resume/delete를 token file 이동/삭제로 구현한다.
- 근거 파일: `reference/ccs/ccs/src/web-server/routes/cliproxy-auth-routes.ts`
- 심볼/키: `/accounts/:provider/:accountId/pause`, `/default`, `/delete`
- 설명: account CRUD와 auth flow 관리가 dashboard control plane API로 노출된다.

- `[확인됨]` 공식 문서상 허용 surface는 더 좁고 더 명시적이다.
- 공식 문서: `https://code.claude.com/docs/en/quickstart`
- 설명: Claude Code는 설치 후 `claude` 또는 `/login`으로 계정에 로그인하고, 자격증명은 시스템에 저장된다고 안내한다.
- 공식 문서: `https://code.claude.com/docs/en/iam`
- 설명: Claude Code는 `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_API_KEY`, `apiKeyHelper`, `CLAUDE_CODE_OAUTH_TOKEN` 같은 명시된 인증 surface만 문서화한다.
- 공식 문서: `https://developers.openai.com/codex/cli/reference#codex-login`
- 설명: Codex CLI는 `codex login`으로 ChatGPT account 또는 API key 인증을 수행한다고 문서화한다.
- 공식 문서: `https://developers.openai.com/codex/app-server#authentication-modes`
- 설명: Codex app-server는 `apikey`, `chatgpt`, experimental `chatgptAuthTokens`를 문서화하고, 외부 host app이 external token lifecycle을 책임질 때만 `chatgptAuthTokens`를 허용한다.
- 공식 문서: `https://developers.openai.com/codex/auth/ci-cd-auth`
- 설명: OpenAI는 API key가 기본 권장이고, ChatGPT-managed auth는 trusted private CI/CD에서만 advanced workflow로 다룬다고 분명히 적는다.

- `[강한 추정]` 코드와 공식 surface 사이 긴장은 “subscription credential을 제품 backend처럼 운영하려는 경향”에서 생긴다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/quota-fetcher-codex.ts`
- 심볼/키: `CODEX_API_BASE = https://chatgpt.com/backend-api`
- 설명: Codex quota는 public docs에 명시되지 않은 backend endpoint와 local auth file에 의존한다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/quota-fetcher-claude.ts`
- 심볼/키: `CLAUDE_OAUTH_USAGE_URL = https://api.anthropic.com/api/oauth/usage`
- 설명: Claude quota도 OAuth usage endpoint와 beta header에 강하게 결합되어 있다.
- 설명: 이런 계층은 local single-user utility에서는 돌아갈 수 있어도, Akra core가 책임질 public abstraction으로는 위험하다.

## Session/runtime model

- `[확인됨]` CCS의 session 개념은 terminal session보다 “proxy process session, auth session, provider account session, isolated Claude instance”의 혼합물이다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/session-tracker.ts`
- 심볼/키: `SessionLock`
- 설명: CLIProxy session은 port/PID/session IDs 기준의 shared service session이다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/auth/oauth-process.ts`
- 심볼/키: `sessionId`, `registerAuthSession`
- 설명: auth flow도 별도의 UI/session identity를 갖는다.
- 근거 파일: `reference/ccs/ccs/src/management/instance-manager.ts`
- 심볼/키: `ensureInstance`
- 설명: Claude 실행은 profile별 isolated config instance에 귀속된다.

- `[확인됨]` Akra가 중요하게 보는 “interactive main session vs sub/task session runtime 분리”와 CCS session model은 직접 대응하지 않는다.
- 근거 파일: `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`
- 심볼/키: `Session Contract Tiers`
- 설명: Akra는 attach-only/handle-based/provider-backed catalog tier를 구분하지만 CCS는 provider/account/proxy lifecycle이 더 전면에 있다.

## Management/control plane

- `[확인됨]` CCS는 완전한 management shell/control plane을 가진다.
- 근거 파일: `reference/ccs/ccs/src/web-server/routes/index.ts`
- 심볼/키: `apiRoutes.use('/cliproxy/auth'...)`, `'/cliproxy'`, `'/cliproxy-server'`
- 설명: auth/accounts/stats/sync/provider/server settings가 전부 API로 묶여 있다.
- 근거 파일: `reference/ccs/ccs/src/web-server/routes/cliproxy-stats-routes.ts`
- 심볼/키: `fetchCliproxyStats`, `fetchCliproxyModels`, `fetchClaudeQuota`, `fetchCodexQuota`
- 설명: status/models/error logs/quota/update/version check까지 control plane에서 다룬다.
- 근거 파일: `reference/ccs/ccs/src/web-server/routes/proxy-routes.ts`
- 심볼/키: `GET/PUT /api/cliproxy-server`, `/backend`, `/test`
- 설명: remote/local CLIProxy target과 backend choice를 dashboard에서 바꾼다.

- `[확인됨]` 이 control plane은 부수 기능이 아니라 제품 중심이다.
- 근거 파일: `reference/ccs/ccs/docs/project-overview-pdr.md`
- 심볼/키: `FR-005`, `FR-008`, `FR-009`
- 설명: dashboard, remote CLIProxy, quota management가 명시적 기능 요구사항이다.

## Protocol translation/routing layer

- `[확인됨]` CCS는 OpenAI/Anthropic compatibility shim을 실제 제품 기능으로 노출한다.
- 근거 파일: `reference/ccs/ccs/docs/openai-compatible-providers.md`
- 심볼/키: `What CCS Does`
- 설명: Anthropic `/v1/messages`를 OpenAI chat-completions로 번역한다고 문서화한다.
- 근거 파일: `reference/ccs/ccs/src/proxy/transformers/request-transformer.ts`
- 심볼/키: `ProxyRequestTransformer`
- 설명: Anthropic request payload를 OpenAI-compatible payload로 변환한다.
- 근거 파일: `reference/ccs/ccs/src/proxy/transformers/sse-stream-transformer.ts`
- 심볼/키: `ProxySseStreamTransformer`
- 설명: upstream JSON/SSE를 Anthropic-style response로 재구성한다.
- 근거 파일: `reference/ccs/ccs/src/proxy/request-router.ts`
- 심볼/키: `detectScenario`, `resolveProxyRequestRoute`
- 설명: model aliasing을 넘어서 scenario routing까지 수행한다.

- `[확인됨]` 이 계층은 Akra core에는 과하다.
- 근거 파일: `docs/plan/22-terminal-agent-transport-and-attachment-matrix.md`
- 심볼/키: `proxy or vibeProxy-style mediation`
- 설명: Akra 문서는 proxy mediation을 concrete local gap이 입증될 때까지 deferred로 둔다.

## Operational complexity

- `[확인됨]` CCS의 설치/실행/업데이트 경로는 여러 장애 지점을 갖는다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/binary-manager.ts`
- 심볼/키: `ensureCLIProxyBinary`, `checkForUpdates`, `resolveLocalBackend`
- 설명: backend별 binary install/update/version pin/fallback을 직접 관리한다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/service-manager.ts`
- 심볼/키: `withStartupLock`, `detectRunningProxy`, `waitForProxyHealthy`
- 설명: startup race와 health probe를 별도 복잡도로 다룬다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/remote-auth-fetcher.ts`
- 심볼/키: `allowSelfSigned`, `timeout`
- 설명: remote/shared deployment를 고려해 TLS/self-signed/timeout handling을 갖춘다.
- 근거 파일: `reference/ccs/ccs/src/commands/config-command.ts`
- 심볼/키: `shouldWarnAboutExposure`
- 설명: non-loopback bind host exposure를 경고할 정도로 네트워크 노출면을 관리한다.

- `[확인됨]` remote/shared deployment를 염두에 둔 구조가 이미 존재한다.
- 근거 파일: `reference/ccs/ccs/docs/project-overview-pdr.md`
- 심볼/키: `FR-008 Remote CLIProxy Support`
- 설명: remote CLIProxy support가 기능 요구사항으로 들어가 있다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/proxy-target-resolver.ts`
- 심볼/키: `ProxyTarget.isRemote`
- 설명: local/remote target 분기가 코드 레벨에서 1급 개념이다.

## Borrowable patterns for Akra

- `[확인됨]` Akra가 빌릴 수 있는 패턴은 capability가 아니라 운영 hygiene 쪽이다.
- 패턴 `1`: startup probe와 readiness를 실행 경로와 분리
- 근거 파일: `reference/ccs/ccs/src/cliproxy/service-manager.ts`
- 심볼/키: `detectRunningProxy`, `waitForProxyHealthy`
- 설명: launch 전 readiness truth를 별도로 확인하는 구조는 Akra startup probe 강화에 차용 가능하다.
- 패턴 `2`: local daemon/session lock hygiene
- 근거 파일: `reference/ccs/ccs/src/cliproxy/session-tracker.ts`
- 심볼/키: `SessionLock`, `proper-lockfile`
- 설명: optional sidecar를 둘 경우 PID/session/refcount lock pattern은 유용하다.
- 패턴 `3`: operator-facing exposure/capability copy
- 근거 파일: `reference/ccs/ccs/src/commands/config-command.ts`
- 심볼/키: `exposure warning copy`
- 설명: 로컬이 아닌 bind host, auth-disabled 상태, degraded mode를 truthfully 알리는 문구 패턴은 차용 가능하다.

- `[확인됨]` Akra core에 직접 맞는 것은 CCS 쪽보다 Akra가 이미 가진 seam language다.
- 근거 파일: `src/adapter/outbound/terminal_bridge/mod.rs`
- 심볼/키: `SessionCatalog::unsupported(SessionCatalogTier::HandleBasedReattach, ...)`
- 설명: Akra는 tmux local attach에서 optional catalog와 handle-based reattach를 이미 truthfully 모델링한다.

## Sidecar-only patterns

- `[확인됨]` 다음 패턴은 Akra에 필요해도 sidecar로만 허용하는 편이 맞다.
- 패턴 `1`: local diagnostics/status daemon
- 근거 파일: `reference/ccs/ccs/src/web-server/routes/cliproxy-stats-routes.ts`
- 심볼/키: `stats/quota/models/error logs routes`
- 설명: 이는 interactive terminal runtime primitive가 아니라 운영 관측 계층이다.
- 패턴 `2`: auth/account management UI
- 근거 파일: `reference/ccs/ccs/src/web-server/routes/cliproxy-auth-routes.ts`
- 심볼/키: `account CRUD`, `auth start/cancel/status`
- 설명: provider auth lifecycle 관리 plane은 terminal orchestrator core보다 sidecar에 맞다.
- 패턴 `3`: optional local proxy process
- 근거 파일: `reference/ccs/ccs/src/proxy/proxy-daemon.ts`
- 심볼/키: `startOpenAICompatProxy`
- 설명: translation proxy가 정말 필요해도 sidecar lifecycle로 격리해야 한다.

## Do-not-adopt patterns

- `[확인됨]` Akra core가 가져오면 안 되는 패턴은 다음과 같다.
- 패턴 `1`: universal AI gateway product화
- 근거 파일: `reference/ccs/ccs/src/proxy/server/proxy-server.ts`
- 심볼/키: `/v1/messages`, `/v1/models`
- 설명: Akra core가 이런 surface를 가지는 순간 TUI orchestrator보다 gateway 정체성이 강해진다.
- 패턴 `2`: multi-provider subscription auth brokerage
- 근거 파일: `reference/ccs/ccs/src/cliproxy/auth/oauth-handler.ts`
- 심볼/키: `triggerOAuth`, `manual callback replay`, `management OAuth callback`
- 설명: 제3자 제품이 subscription/OAuth lifecycle을 광범위하게 매개하는 구조는 정책/유지보수 리스크가 크다.
- 패턴 `3`: provider-private quota scraping
- 근거 파일: `reference/ccs/ccs/src/cliproxy/quota-fetcher-codex.ts`
- 심볼/키: `chatgpt.com/backend-api`
- 설명: private backend coupling은 Akra core abstraction에 넣을 수 없다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/quota-fetcher-claude.ts`
- 심볼/키: `api/oauth/usage`
- 설명: quota 관측은 sidecar 또는 research utility여야지 core contract가 되면 안 된다.
- 패턴 `4`: multi-target product sprawl
- 근거 파일: `reference/ccs/ccs/src/ccs.ts`
- 심볼/키: `ClaudeAdapter`, `DroidAdapter`, `CodexAdapter`
- 설명: Akra가 core에서 Claude/Codex/Droid를 동시에 first-class product target으로 키우면 현재의 session/TUI 중심 narrative가 약해진다.

## Evidence

- `[확인됨]` Akra baseline의 1차 근거는 planning/design 문서와 capability port 코드다.
- 근거 파일: `docs/plan/21-terminal-agent-bridge-research-and-capability-boundary.md`
- 심볼/키: `Current Decision`
- 설명: local attach first와 proxy deferred가 명시되어 있다.
- 근거 파일: `docs/plan/23-terminal-agent-capability-boundary-and-session-contract.md`
- 심볼/키: `Capability Summary`
- 설명: Akra가 원하는 seam vocabulary를 직접 정의한다.
- 근거 파일: `src/application/port/outbound/*.rs`
- 심볼/키: `StartupProbePort`, `SessionCatalogPort`, `InteractiveTurnRuntimePort`
- 설명: 실제 코드가 문서 방향을 따라 capability split을 구현한다.

- `[확인됨]` CCS 심층 분석의 1차 근거는 entrypoint, daemon launcher, auth storage, proxy translator, dashboard routes다.
- 근거 파일: `reference/ccs/ccs/src/ccs.ts`
- 심볼/키: `main`
- 설명: 제품 진입점에서 multi-target/runtime 구성을 확인할 수 있다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/service-manager.ts`
- 심볼/키: `ensureCliproxyService`
- 설명: background service 중심 구조를 확인할 수 있다.
- 근거 파일: `reference/ccs/ccs/src/cliproxy/auth/token-manager.ts`
- 심볼/키: `listProviderTokenSnapshots`
- 설명: auth file storage and discovery ownership을 확인할 수 있다.
- 근거 파일: `reference/ccs/ccs/src/proxy/transformers/request-transformer.ts`
- 심볼/키: `ProxyRequestTransformer`
- 설명: protocol translation layer의 실체를 확인할 수 있다.
- 근거 파일: `reference/ccs/ccs/src/web-server/routes/index.ts`
- 심볼/키: `apiRoutes`
- 설명: control plane surface의 범위를 확인할 수 있다.

# Akra Mapping

- `adopt in Akra core`
- `[확인됨]` capability wording의 정교화, explicit startup/readiness diagnostics, optional local daemon lifecycle truth, operator-facing degradation copy는 core 또는 core-adjacent adapter에 흡수 가능하다.
- 근거 파일: `src/adapter/outbound/terminal_bridge/mod.rs`
- 심볼/키: `load_startup_context`, `load_recent_sessions`, `runtime_control_truth`
- 설명: Akra는 이미 해당 seam을 갖고 있어 CCS식 hygiene만 선택 흡수하면 된다.

- `sidecar only`
- `[확인됨]` dashboard, auth lifecycle management UI, usage/quota observer, remote management client, translation proxy는 필요해도 sidecar여야 한다.
- 근거 파일: `reference/ccs/ccs/src/web-server/index.ts`
- 심볼/키: `startServer`
- 설명: 이 계층은 terminal runtime보다 운영 plane 성격이 강하다.

- `do not adopt`
- `[확인됨]` universal provider gateway, multi-account round-robin/failover product화, subscription OAuth mediation, provider-private quota scraping은 Akra core 금지 영역이다.
- 근거 파일: `reference/ccs/ccs/docs/project-overview-pdr.md`
- 심볼/키: `FR-008`, `FR-009`, `AC-003`
- 설명: 이 제품 방향을 Akra core에 넣는 순간 정체성이 바뀐다.

- `research later`
- `[확인됨]` tmux attach가 충분하지 않을 때의 managed local wrapper fallback, planning worker용 non-Codex runtime 보강, attach target discovery UX는 추가 연구 항목이다.
- 근거 파일: `docs/plan/22-terminal-agent-transport-and-attachment-matrix.md`
- 심볼/키: `managed local wrapper`
- 설명: fallback 후보는 이미 문서에 정의되어 있지만 아직 primary path는 아니다.

# Recommendation

- `[확인됨]` 최종 추천은 `tmux/local terminal bridge 중심`이다.
- 이유 `1`: Akra baseline이 이미 tmux local attach를 primary로 확정했고, capability seam도 그 방향에 맞춰 정리되어 있다.
- 근거 파일: `docs/plan/28-terminal-agent-tmux-local-attach-gate-verdict.md`
- 심볼/키: `tmux local attach stays the primary path`
- 설명: 현재 evidence gate가 이미 이 선택을 통과시켰다.
- 이유 `2`: CCS에서 core로 가져올 만한 것은 process/readiness hygiene뿐이며, 제품 중심 구조는 proxy/gateway 쪽이다.
- 근거 파일: `reference/ccs/ccs/src/proxy/server/proxy-server.ts`
- 심볼/키: `startOpenAICompatProxyServer`
- 설명: translation proxy는 terminal bridge와 별개의 제품 축이다.
- 이유 `3`: startup/readiness/catalog/diagnostics 강화가 필요하면 `local sidecar daemon 추가`까지는 허용 가능하지만, 그 경우에도 core는 attach-first orchestrator로 남겨야 한다.
- 근거 파일: `src/application/port/outbound/startup_probe_port.rs`
- 심볼/키: `StartupProbeContext`
- 설명: Akra는 sidecar 정보를 core seam으로 수용할 준비가 되어 있지만, gateway abstraction을 요구하지 않는다.

- `[확인됨]` 따라서 “Akra가 terminal orchestrator로 남는가, proxy/gateway로 변질되는가”라는 기준에서, CCS 전체 구조의 도입은 비권장이고 CCS 일부 운영 패턴만 제한적으로 차용하는 것이 맞다.

# Open Questions

- `[불확실]` Akra planning worker가 장기적으로도 `codex app-server` 의존을 유지할지, 아니면 Claude/terminal bridge를 worker runtime으로도 확장할지 추가 판단이 필요하다.
- 왜 중요한가: main interactive runtime과 sub/task runtime 분리 설계의 실제 소비처가 planning worker이기 때문이다.
- 추가로 볼 것: `src/application/service/planning/worker/*`, `src/adapter/outbound/app_server/planning_worker.*`

- `[불확실]` tmux attach UX에서 explicit pane handle만으로 충분한지, 작은 catalog/browser가 필요한지 추가 실험이 필요하다.
- 왜 중요한가: Akra가 optional `SessionCatalog`를 얼마나 풍부하게 제공할지 결정하기 때문이다.
- 추가로 볼 것: `src/adapter/outbound/terminal_bridge/mod.rs`, `docs/plan/27-terminal-agent-tmux-local-attach-readiness-evidence.md`

- `[불확실]` remote/shared deployment가 실제 product goal로 올라오는지 아직 근거가 부족하다.
- 왜 중요한가: 이 요구가 생기지 않으면 CCS류 remote management/proxy plane은 끝까지 core 밖에 남겨야 한다.
- 추가로 볼 것: Akra roadmap 문서, operator workflows, 실제 multi-host 운영 요구사항
