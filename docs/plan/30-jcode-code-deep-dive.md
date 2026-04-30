# jcode 코드 딥다이브

이 문서는 `reference/jcode/jcode`를 경쟁 제품이자 참고 구현으로 분석하고, 우리 제품에 차용할 만한 부분을 선별한다. 판단 기준은 기능 수 경쟁이 아니라, 현재 repo의 **TUI + `codex app-server` 중심 제품 방향에 맞는가**다.

## 요약

jcode는 단순한 TUI 클라이언트가 아니라 자체 agent runtime 전체를 소유하는 큰 Rust 제품이다. 하나의 daemon server가 세션, provider, tool 실행, memory, swarm, side panel, gateway, mobile prototype, self-dev 흐름까지 관리하고, TUI client는 그 서버에 붙는 구조다.

우리에게 가장 가치 있는 부분은 다음이다.

- TUI 관측성: side panel, info widget, session picker preview, model/context/status rail, parallel work visibility.
- 성능 규율: startup profile mark, TUI render benchmark, adaptive performance tier, compile/test budget.
- 세션 연속성: 외부 transcript import/search/resume 아이디어를 app-server 경계 위에서 재해석.
- 병렬 작업 UX: worker heartbeat, stale 상태, completion report, file-touch notification, summary/full-context 분리.
- 품질 가드레일: code-size ratchet, dependency-boundary check, panic/unwrap budget, suite runner.

직접 차용하지 말아야 할 부분도 분명하다.

- multi-provider runtime 전체
- subscription/OAuth credential compatibility layer
- 자체 agent/tool runtime 전체
- full memory graph + embedding sidecar
- autonomous swarm spawning default
- browser bridge
- self-dev hot reload stack
- mobile/desktop/gateway 확장

핵심 결론은 이렇다. jcode는 **장기 실행 터미널 agent 작업을 빠르고, 재접속 가능하고, 관측 가능하고, 다중 세션 친화적으로 만드는 제품 감각**이 강하다. 하지만 아키텍처 모델은 우리에게 맞지 않는다. 우리는 jcode의 runtime을 복제하지 말고, app-server를 runtime authority로 유지하면서 operator-facing surface와 운영 규율만 선별적으로 가져와야 한다.

## 분석 대상 스냅샷

분석 대상은 `reference/jcode/jcode`다.

| 항목 | 관찰 |
| --- | --- |
| package | `jcode` v0.11.2, Rust 2024 |
| workspace | root crate + 다수의 `crates/jcode-*` helper crate |
| 제품 모델 | single shared server + attachable TUI clients |
| Rust 규모 | `src` 아래 Rust 파일 631개, 약 289k LOC |
| 큰 파일 | `src/server/client_lifecycle.rs`, `src/tui/ui.rs`, `src/provider/mod.rs`, `src/server/comm_control.rs`, `src/tui/app/remote/key_handling.rs` |
| test marker | `src`와 `crates`에서 Rust test marker 4500개 이상 |
| 추가 surface | iOS prototype, mobile core/simulator, WebSocket gateway, telemetry worker |

주요 확인 파일은 다음이다.

- `Cargo.toml`
- `README.md`
- `docs/SERVER_ARCHITECTURE.md`
- `docs/MODULAR_ARCHITECTURE_RFC.md`
- `docs/MEMORY_ARCHITECTURE.md`
- `docs/SWARM_ARCHITECTURE.md`
- `docs/COMPILE_PERFORMANCE_PLAN.md`
- `docs/CODE_QUALITY_AUDIT_2026-04-18.md`
- `crates/jcode-protocol/src/lib.rs`
- `src/server.rs`
- `src/server/client_lifecycle.rs`
- `src/server/swarm.rs`
- `src/server/comm_control.rs`
- `src/tool/mod.rs`
- `src/tui/info_widget.rs`
- `src/side_panel.rs`
- `src/memory.rs`
- `src/memory/model.rs`
- `src/memory_agent.rs`
- `src/memory_graph.rs`
- `src/import.rs`
- `src/tui/session_picker.rs`
- `src/provider/mod.rs`
- `src/provider/openai.rs`
- `src/provider/anthropic.rs`
- `src/gateway.rs`
- `ios/Sources/JCodeKit/*`

## 제품 아키텍처

jcode의 중심은 single-server, multi-client 구조다. 첫 `jcode` 실행은 detached `jcode serve` daemon을 띄우고 Unix socket에 연결한다. 이후 TUI client들은 같은 서버에 붙는다. 서버는 세션, provider 상태, background work, MCP pool, swarm 상태, event history, side panel, memory, debug control을 소유한다.

`crates/jcode-protocol/src/lib.rs`는 newline-delimited JSON over socket 형태의 protocol을 정의한다. `Request`는 message, cancel, background tool, soft interrupt, history sync, compacted history, reload, resume, transcript injection, shell input, model switching, subagent launch, feature toggle, swarm control까지 포함한다. `ServerEvent`는 text delta, tool lifecycle, generated image, usage/token, connection phase, memory activity, side panel snapshot, swarm status/proposal, reload progress, model change, error/done을 운반한다.

이 구조의 강점은 terminal client를 가볍고 재접속 가능하게 만든다는 점이다. session/runtime 상태가 하나의 daemon에 모이기 때문에, 다중 세션에서 incremental memory cost도 낮출 수 있다.

다만 우리 repo의 중심은 다르다.

- `docs/design/04-hexagonal-runtime-architecture.md`는 `adapter -> application -> domain` 방향을 명시한다.
- `src/adapter/outbound/app_server`가 외부 runtime boundary다.
- TUI는 provider/tool runtime을 직접 소유하지 않고 application facade를 통해 동작해야 한다.
- planning과 parallel mode는 operator-owned application/domain flow다.

따라서 jcode의 daemon architecture를 직접 채택하면 안 된다. 올바른 차용 방식은 **우리 app-server adapter와 TUI가 jcode 수준의 attachable control surface처럼 느껴지게 만드는 것**이다.

### 적용 판단

차용할 개념:

- reconnect-aware client state
- bounded history와 lazy transcript loading
- connection phase/status detail event
- session attach/preview UX
- first render/first input 기준의 성능 목표

차용하지 않을 것:

- 자체 provider server를 primary runtime으로 삼는 구조
- jcode protocol shape 전체
- hot reload/self-dev를 기본 제품 모델로 삼는 구조

## Workspace와 dependency 구조

jcode의 `Cargo.toml`은 monolith와 workspace의 중간 형태다. `jcode-embedding`, `jcode-pdf`, `jcode-provider-core`, `jcode-provider-metadata`, `jcode-tui-markdown`, `jcode-tui-mermaid`, `jcode-mobile-core`, `jcode-protocol` 등 여러 crate가 추출되어 있다.

하지만 `docs/MODULAR_ARCHITECTURE_RFC.md` 스스로도 현재 상태를 "modular monolith with a growing workspace shell"로 설명한다. 이 표현은 정확하다. 무거운 leaf dependency나 안정적인 value type 일부는 crate로 빠져 있지만, root crate는 여전히 CLI, server, session, provider, tool registry, TUI app state, memory, safety, ambient glue를 크게 소유한다.

우리 repo는 기능 범위는 작지만 boundary는 더 명확하다. `src/domain`, `src/application/service`, `src/application/port`, `src/adapter/*` 구조가 이미 있고, small-context readability를 설계 목표로 둔다. 따라서 jcode의 crate 수를 따라가는 것은 잘못된 목표다. 대신 **정말 compile/ownership 병목이 생긴 안정 seam만 추출한다**는 원칙을 가져오면 된다.

### 적용 판단

차용 후보:

- domain/application/adapter dependency boundary check
- TUI render helper나 stable value type이 커질 때만 별도 module/crate로 분리
- app-server protocol mapping은 outbound adapter 안에 고정
- 고변경 orchestration code가 stable type crate로 역류하지 못하게 막기

주의할 점:

- crate를 늘리는 것 자체를 구조 개선으로 착각하지 않는다.
- UI/product orchestration을 shared type crate로 끌어내지 않는다.

## TUI와 operator surface

jcode에서 우리에게 가장 차용 가치가 높은 부분은 TUI다. 단순 transcript renderer가 아니라, terminal을 operator cockpit으로 만든다.

주요 구현:

- `src/tui/info_widget.rs`: widget 종류, priority, preferred side, minimum height 정의.
- `src/side_panel.rs`, `src/tool/side_panel.rs`: session-scoped markdown page, managed page, linked file page, focus/delete/status.
- `crates/jcode-tui-markdown`: syntax highlight, copy target, lazy render, table/math/task-list, streaming context hook.
- `crates/jcode-tui-mermaid`: terminal diagram rendering, image protocol detection, cache, background render, OOM 방지 limit.
- `crates/jcode-tui-workspace`: compact workspace map.
- `src/tui/session_picker.rs`: grouped session list, preview, source filter, external transcript search.

가장 중요한 제품 아이디어는 "negative-space instrumentation"이다. conversation이 화면 전체를 채우지 않을 때 좌우 margin을 버리지 않고, runtime 상태를 표시한다. widget 종류도 실용적이다. diagram, workspace map, todo, context usage, usage limit, memory activity, model info, background task, git status, swarm status, ambient status 등이 priority에 따라 배치된다.

우리 TUI는 이미 conversation, diagnostics, sessions, queue inspection, planning controls, directions overlay를 갖고 있다. 다음 진화는 overlay를 계속 늘리는 것이 아니라, 자주 봐야 하는 상태를 작은 persistent rail로 옮기는 것이다.

### 적용 판단

즉시 후보:

- 오른쪽 rail: 현재 queue task, proposed task 수, skipped/blocked 요약, pause reason, resume action.
- 왼쪽 rail: app-server 상태, active session, diagnostics severity, model/context 정보가 있으면 표시.
- session overlay preview 강화: first prompt, summary, branch/cwd, modified time, source badge.
- planning artifact나 terminal capture를 보여주는 scoped side panel. 단, agent tool이 아니라 TUI/application state로 시작한다.

중기 후보:

- planning 출력이나 assistant 답변에서 diagram 수요가 생기면 mermaid rendering 검토.
- parallel lane state가 안정화된 뒤 workspace map 도입.

비추천:

- jcode markdown/mermaid renderer 전체 port.
- agent-writable side panel을 권한/영속성 모델 없이 도입.

## Tool runtime

jcode는 자체 agent tool runtime을 소유한다. `src/tool/mod.rs`에는 `Tool` trait가 있고, name, description, JSON schema, async execute를 정의한다. `ToolContext`는 session ID, message ID, tool call ID, working dir, stdin request channel, interrupt signal, execution mode를 담는다. `ToolOutput`은 text, title, metadata, image를 담는다.

registry 구현에서 참고할 만한 점은 많다.

- stateless base tool은 `OnceLock`에 캐시하고 session별로 `Arc` clone한다.
- clone마다 compaction manager를 새로 만들어 parallel subagent history corruption을 막는다.
- tool definition을 deterministic sort해서 prompt cache 안정성을 높인다.
- agent ecosystem별 tool alias를 제공한다.
- MCP tool registration을 background로 처리해 startup을 막지 않는다.
- tool output을 context budget 기준으로 제한한다.

tool inventory는 매우 넓다. read/write/edit/multiedit/patch/apply_patch, bash/bg, grep/glob/ls, agentgrep, browser, webfetch/websearch, LSP, todo, memory, side_panel, subagent, batch, conversation/session search, ambient permission, Gmail/schedule, selfdev, MCP, swarm communication까지 포함한다.

우리에게 이것은 runtime port 대상이 아니다. app-server가 tool/runtime authority여야 한다. 다만 app-server event를 TUI에 projection하는 방식, local direct command, output truncation policy에는 참고 가치가 있다.

### 적용 판단

차용 후보:

- capability/tool 표시를 deterministic ordering으로 유지.
- tool/shell output projection에 text/title/metadata/image 같은 normalized shape 도입.
- context나 UI를 망가뜨리지 않는 output truncation/summary policy.
- optional integration을 startup 이후 background로 discover/register.
- direct execution과 agent-turn execution을 UI에서 구분.

비추천:

- app-server와 경쟁하는 local tool runtime.
- application port/safety contract 없이 agent-writable local tool을 늘리는 것.

## Memory system

jcode의 memory system은 크고 진지하다. `MemoryEntry`는 category, tags, normalized search text, timestamps, access count, source, trust, strength, active/superseded state, reinforcement breadcrumb, embedding, confidence를 가진다. confidence는 category별 half-life로 decay된다. correction은 오래 유지하고, fact는 비교적 빨리 낡는 식이다.

`src/memory_graph.rs`는 memory/tag/cluster/edge/reverse edge/metadata를 저장한다. edge는 `HasTag`, `InCluster`, `RelatesTo`, `Supersedes`, `Contradicts`, `DerivedFrom`이다. weighted traversal과 top-K 선택도 들어 있다. `src/memory_agent.rs`는 background extraction, topic-change detection, periodic extraction, final extraction, maintenance를 담당한다.

구현된 core는 의미 있지만, 문서가 코드보다 앞서 있는 부분도 있다. graph cluster는 구조상 존재하지만, 문서의 hybrid graph memory 전체가 완성됐다고 보기는 어렵다. memory system은 embeddings, sidecar relevance, persistence, privacy, UI activity까지 제품 의무를 크게 늘린다.

우리 제품은 지금 full memory graph를 도입할 단계가 아니다. 더 적합한 차용은 operator-visible provenance다.

### 적용 판단

차용 후보:

- "이번 continuation이 어떤 accepted planning/context를 근거로 움직였는가"를 표시.
- planning-derived follow-up task에 provenance와 supersession marker를 남김.
- direction/queue task가 대체될 때 superseded relationship을 명시.
- memory activity UI 패턴은 참고하되, 자동 장기 memory는 보류.

보류:

- embeddings
- graph memory
- sidecar verifier
- automatic long-term user memory extraction

## Swarm과 parallel work

jcode의 swarm은 가장 야심찬 영역 중 하나다. 문서는 coordinator, worktree manager, agent lifecycle, communication channel, shared context, soft interrupt, file-touch notification, optimistic conflict handling, structured report를 설명한다. 코드도 `src/server/swarm.rs`, `src/server/comm_control.rs`, `src/tool/communicate.rs`에 상당 부분 구현되어 있다.

참고할 만한 구현 관점:

- worker 완료 전 completion report reminder를 주입한다.
- report는 normalize/truncate해서 전달한다.
- worker status update는 debounce한다.
- task progress는 heartbeat/stale/sweep interval을 가진다.
- coordinator-facing state에서 summary와 full context/read context를 구분한다.
- retry, reassign, replace, resume, wake, salvage 같은 assignment control이 있다.
- notification은 hard cancel보다 soft interrupt로 전달할 수 있다.

우리 repo에는 이미 `parallel_mode`, supersession, planning queue, worker lane 방향이 있다. jcode는 agent-autonomous communication에 가깝고, 우리는 operator-owned planning authority에 가깝다. 따라서 agent chat/swarm 자체보다 **병렬 작업 상태를 operator가 이해하는 방식**을 빌리는 게 맞다.

### 적용 판단

즉시 후보:

- parallel lane heartbeat와 stale 표시.
- background worker completion report contract.
- lane inspection에서 summary와 full context 분리.
- file-touch notification: "slot A가 slot B가 읽은 파일을 변경했다".
- 다수 worker 상태 update debounce.

비추천:

- autonomous agent spawning을 기본 UX로 삼는 것.
- agent-to-agent chat channel을 primary surface로 만드는 것.
- operator visibility 없는 optimistic conflict handling.

## Provider와 auth runtime

jcode의 provider layer는 매우 넓다. OpenAI/Codex, Anthropic/Claude, Gemini, Copilot, Azure, OpenRouter, Ollama/LM Studio, 여러 OpenAI-compatible preset을 다룬다. model list, route/cost metadata, reasoning effort, service tier, transport selection, native compaction, credential invalidation, provider-specific schema/tool translation도 포함한다.

기술적으로는 인상적이지만 우리에게는 위험하다. 일부 provider 경로는 subscription-backed CLI나 web backend 호환 동작에 기대는 것으로 보인다. Anthropic/OpenAI 쪽은 compatibility header, backend route, websocket/HTTP fallback, OAuth/token discovery가 상세하다. 문서상 credential discovery trust model은 신중하지만, 이 영역 전체가 고유지보수/정책 리스크를 만든다.

우리 repo의 제품 기준은 `codex app-server`다. jcode provider layer를 따라가면 제품 중심이 흐려지고 지원 부담이 크게 늘어난다.

### 적용 판단

차용 후보:

- auth/status diagnostics의 표현 방식.
- credential discovery transparency 원칙.
- machine-readable JSON status command.
- app-server가 노출하는 model/provider status를 TUI에 표시하는 방식.

비추천:

- reverse-engineered provider transport.
- subscription token import.
- multi-provider runtime을 근시일 목표로 삼는 것.

## Session continuity와 import

jcode는 session continuity에 많이 투자했다. `src/import.rs`는 Claude Code JSONL을 비롯해 Codex, Pi, OpenCode session import 흐름을 갖는다. session picker는 source filter, preview, transcript search, multi-resume/catch-up UX를 제공한다.

이 영역은 우리에게 적합하다. 우리 repo도 recent-session browser projection과 sessions overlay를 갖고 있다. jcode의 교훈은 외부 transcript를 "바로 실행 가능한 세션"으로 취급하는 것이 아니라, **검색 가능한 resume/context candidate**로 취급하는 것이다.

### 적용 판단

즉시 후보:

- session picker metadata 강화: first prompt, summary, branch, cwd, modified time, source, message count.
- app-server sessions와 local transcript index를 함께 검색.
- current app-server session과 imported/read-only transcript를 source badge로 구분.
- accepted planning이 있는 workspace를 resume할 때 planning state도 함께 표시.

주의:

- imported session은 deliberate conversion path 전까지 read-only로 둔다.
- provider/tool replay fidelity를 약속하지 않는다.

## Mobile, gateway, desktop

jcode는 WebSocket gateway와 iOS prototype도 갖고 있다. gateway는 paired device를 검증하고 remote WebSocket client를 서버 처리 경로에 bridge한다. iOS 쪽은 protocol mirror, connection, pairing, credential store를 갖는다. mobile core/simulator crate는 장기적으로 Rust core가 state/reducer/protocol을 소유하고 Swift가 shell이 되는 방향을 암시한다.

현재 우리 제품에는 우선순위가 낮다. 다만 원칙은 좋다. heavy work는 desktop/server process에 남기고, remote client는 thin paired state-synchronized surface로 둔다.

### 적용 판단

장기 후보:

- active app-server/TUI session을 모니터링하는 thin remote status client.
- mobile/web이 roadmap에 들어올 때 pairing/device registry.

비추천:

- 지금 native iOS shell 착수.
- desktop superapp 착수.
- TUI/app-server loop가 충분히 좋아지기 전 WebSocket gateway 착수.

## Performance와 운영성

jcode는 성능을 제품 기능처럼 다룬다. README는 time-to-first-frame, time-to-first-input, per-session PSS를 전면에 둔다. 코드에는 `src/startup_profile.rs`, `src/bin/tui_bench.rs`, compile-performance docs, compile-speed oriented release profile, `selfdev` profile이 있다.

참고할 만한 습관:

- startup profile marks와 timing report.
- synthetic session 기반 TUI render benchmark.
- SSH/WSL/Windows Terminal/load/memory를 고려한 adaptive performance tier.
- 최대 runtime speed보다 compile feedback을 우선한 release profile.
- build metadata 안정화로 cache invalidation 감소.
- dependency-boundary/code-size script.

이 부분은 우리에게 바로 적용 가능하다. TUI snapshot test와 shell flow 문서는 있지만, UI가 더 커지기 전에 성능 기준선을 잡아야 한다.

### 적용 판단

즉시 후보:

- app-server detection, diagnostics, session restore, first render, ready-to-submit timing mark.
- representative conversation/planning/parallel state를 쓰는 TUI benchmark.
- SSH/WSL/low-memory/slow-terminal용 reduced rendering policy.
- high-churn TUI/planning/parallel file에 code-size budget.

## 품질과 유지보수성

jcode는 test surface가 넓고 품질 문서도 진지하지만, 규모 부채가 분명하다. 자체 audit도 큰 파일, 긴 함수, 많은 unwrap/expect를 문제로 본다. 이 코드는 빠르게 제품을 확장하고 측정하는 능력이 강하지만, clean architecture의 모범은 아니다.

우리에게 중요한 교훈은 "넓은 surface를 따라가자"가 아니라 "넓어질 때 무너지지 않도록 guardrail을 먼저 깔자"다.

### 적용 판단

차용 후보:

- `domain`, `application`, `adapter` dependency-boundary check.
- 고위험 파일 file/function size ratchet.
- production code panic/unwrap budget report.
- startup, app-server parsing, stream reduction, session list mapping, TUI snapshots, planning, parallel-mode를 묶는 suite runner.

비추천:

- 경쟁 제품이 넓다는 이유로 우리 surface를 성급히 늘리는 것.

## 차용 매트릭스

| jcode 영역 | 우리 가치 | 적합도 | 판단 |
| --- | --- | --- | --- |
| startup profiling, TUI benchmark | 높음 | 높음 | 즉시 후보 |
| adaptive performance tier | 높음 | 높음 | 즉시 후보 |
| negative-space info widget | 높음 | 높음 | 즉시 후보 |
| session picker preview/search/source badge | 높음 | 높음 | 즉시 후보 |
| linked markdown side panel | 중상 | 높음 | scoped version |
| worker heartbeat/stale/completion report | 높음 | 높음 | parallel-mode에 적용 |
| file-touch notification | 높음 | 중상 | lane state 안정 후 설계 |
| tool output metadata/truncation | 중간 | 중상 | event projection에 참고 |
| memory activity UI | 중간 | 중간 | 표시 패턴만 차용 |
| full memory graph/embedding | 중간 | 낮음 | 보류 |
| mermaid renderer | 중간 | 중하 | 실수요 전 보류 |
| browser bridge | 중간 | 낮음 | 지금은 비추천 |
| full provider runtime | 낮음 | 낮음 | 비추천 |
| OAuth/subscription import | 위험 | 낮음 | 비추천 |
| autonomous swarm spawning | 중간 | 중하 | default로 비추천 |
| WebSocket/mobile gateway | 장기 가능 | 낮음 | 장기 후보 |
| self-dev hot reload | 낮음 | 낮음 | 비추천 |
| quality guardrails | 높음 | 높음 | 즉시 후보 |

## 권장 backlog

### 1. Measurement와 guardrail

보이는 UI를 더 키우기 전에 측정 기반을 깐다.

예상 산출물:

- diagnostics, app-server connect, session restore, first render, ready-to-submit timing mark.
- synthetic TUI render benchmark.
- code-size 및 dependency-boundary check.
- `docs/validation`에 performance baseline 기록.

### 2. Operator status rail

jcode info widget에서 아이디어를 빌리되, 우리 vocabulary에 맞춘다.

예상 산출물:

- queue/planning rail: now, next, candidates, blocked/skipped, pause reason, resume action.
- runtime rail: app-server status, active session, diagnostics severity, model/context 정보.
- narrow terminal degradation rule.
- snapshot coverage.

### 3. Session continuity upgrade

session picker를 단순 목록에서 resume/context browser로 강화한다.

예상 산출물:

- source badge와 richer metadata.
- selected session preview.
- app-server session과 optional local transcript index 검색.
- imported transcript read-only 처리.

### 4. Parallel lane observability

jcode swarm 자체가 아니라 worker-state discipline을 차용한다.

예상 산출물:

- heartbeat와 stale state.
- completion report contract.
- summary/full-context inspection 분리.
- file-touch/read notification.
- debounced status rendering.

## 명시적 비채택

다음은 제품 방향이 바뀌기 전까지 scope 밖에 둔다.

- 자체 multi-provider agent runtime 구현.
- 다른 CLI의 subscription OAuth credential import.
- autonomous swarm spawning을 기본 operator workflow로 삼는 것.
- explicit planning/context provenance가 성숙하기 전 full memory graph 도입.
- TUI/app-server path가 충분히 좋아지기 전 mobile, desktop, WebSocket gateway 착수.
- jcode renderer/tool/provider module 직접 복사.

## 최종 판단

jcode에서 가장 강한 부분은 장기 실행 terminal agent 작업을 빠르고, 재접속 가능하고, 관측 가능하고, 다중 세션 친화적으로 만든다는 점이다. 가장 약한 부분은 root crate가 너무 많은 runtime surface를 소유하고, 큰 파일과 넓은 provider/tool 범위로 유지보수 압력이 크다는 점이다.

우리의 최선은 선택적 차용이다.

1. measurement와 guardrail을 먼저 가져온다.
2. operator-facing visibility pattern을 가져온다.
3. session continuity와 parallel-work status 아이디어를 가져온다.
4. app-server를 runtime authority로 유지한다.
5. provider/auth/runtime 확장으로 이 repo를 두 번째 jcode로 만들지 않는다.
