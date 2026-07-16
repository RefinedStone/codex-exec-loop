"use strict";

const products = [
  {
    id: "opencode",
    name: "OpenCode",
    version: "v1.18.2",
    commit: "70b56a0a93d3",
    released: "2026-07-15",
    category: "reactive",
    thesis: "Server authority와 Solid/OpenTUI projection을 결합해 웹 UI와 유사한 반응성을 만든다. 다만 mutation 호출이 component 전반에 분산되어 ack/error transaction은 약하다.",
    flow: [
      ["INPUT", "OpenTUI event / keymap / mouse", "Kitty keyboard, mouse, dialog, route component가 사용자 intent를 받는다."],
      ["COMMAND", "Solid component → generated SDK", "component와 command가 Worker RPC 또는 HTTP/SSE SDK를 호출한다."],
      ["AUTHORITY", "Bun Worker / server Effect service", "session, message, file revert, permission의 authoritative mutation과 event를 소유한다."],
      ["PROJECTION", "16 ms batched Sync store", "live event와 hydration snapshot을 merge하고 visible message를 100개로 제한한다."],
      ["RENDER", "Solid JSX → OpenTUI", "provider tree, route, dialog, sidebar, diff, toast를 최대 60 fps renderer로 투영한다."],
    ],
    facts: [
      ["Framework", "SolidJS + @opentui/core + @opentui/solid + Effect"],
      ["State shape", "큰 Solid store와 event reducer. 단일 Elm/Redux mutation gate는 아니다."],
      ["Mouse", "renderer에서 활성화. message row mouse-up은 action dialog를 연다."],
      ["Extensions", "TS/JS plugins, skills, hooks, tools, 신규 TUI plugin API"],
      ["QA", "TUI test module 45개 / test-tree file 52개. sync/hydration test는 있으나 revert transaction E2E 공백이 보인다."],
    ],
    strength: "선언적 component composition과 서버 projection 경계 덕분에 복잡한 dialog/sidebar/diff UX를 빠르게 조합한다.",
    weakness: "직접 SDK mutation과 optimistic UI가 넓어 failure/stale ack가 화면에 원자적으로 반영되지 않을 수 있다.",
    akra: "row action menu와 compact undo dock만 차용하고, mutation은 exact identity + revision + authoritative ack 뒤에 반영한다.",
  },
  {
    id: "gjc",
    name: "GJC / Gajae-Code",
    version: "v0.11.0 beta",
    commit: "8132409c3f10",
    released: "2026-07-15",
    category: "custom",
    thesis: "Pi에서 fork한 message-agnostic custom TUI engine과 typed agent event controller를 분리한다. 강한 terminal QA를 갖지만 custom renderer와 큰 integration controller 비용도 직접 소유한다.",
    flow: [
      ["INPUT", "stdin → ProcessTerminal → StdinBuffer", "paste, escape sequence, IME와 key sequence를 terminal buffer가 정규화한다."],
      ["FOCUS", "TUI → focused Component", "editor mechanics와 mode semantics를 분리해 input controller로 보낸다."],
      ["AUTHORITY", "AgentSession / agent loop", "대화, tool 실행, workflow skill의 의미 상태를 소유한다."],
      ["PROJECTION", "typed AgentSessionEvent → EventController", "domain event를 incremental message/tool/status component로 바꾼다."],
      ["RENDER", "component lines → virtual viewport → diff", "next-tick coalescing, overlay composite, cursor marker, CSI 2026으로 출력한다."],
    ],
    facts: [
      ["Framework", "@gajae-code/tui: render(width) → string[] custom components"],
      ["Lineage", "badlogic/pi-mono fork 후 @gajae-code scope로 변경"],
      ["State shape", "agent event controller와 interactive-mode가 UI 조정을 담당"],
      ["Mouse", "앱 SGR click reporting은 끄고, managed tmux가 wheel scroll만 담당"],
      ["Extensions", "4 workflow skills, 4 role agents, bounded plugin families"],
      ["QA", "virtual terminal, golden, CJK/Jamo, resize storm, viewport, latency/perf gates"],
    ],
    strength: "terminal engine을 message 의미에서 분리하고, 시각 QA에 CJK·ANSI·fresh evidence를 명시한다.",
    weakness: "interactive-mode 약 3천 줄, renderer 약 2.6천 줄이다. beta 제품이 bespoke terminal engine의 장기 비용을 감당해야 한다.",
    akra: "virtual terminal과 visual QA contract는 채택하되 renderer fork나 giant controller는 만들지 않는다.",
  },
  {
    id: "codex",
    name: "OpenAI Codex",
    version: "rust-v0.144.4",
    commit: "8c68d4c87dc5",
    released: "2026-07-14",
    category: "native",
    thesis: "typed core/app-server command와 TUI event loop를 분리하고, semantic transcript를 resize 시 재투영한다. 순수 reducer는 아니며 거대한 AppEvent·ChatWidget을 correlation guard와 대규모 snapshot/VT 테스트로 통제한다.",
    flow: [
      ["INPUT", "Crossterm TuiEvent", "keyboard와 bracketed paste를 받고 mouse event는 현재 명시적으로 버린다."],
      ["COORDINATE", "App tokio::select! loop", "local AppEvent, active-thread event, TUI event, app-server event를 한 루프에서 합류한다."],
      ["AUTHORITY", "core / app-server AppCommand", "thread, turn, rollback, tool 실행의 의미 상태와 authoritative acknowledgement를 소유한다."],
      ["PROJECTION", "mutable App / ChatWidget", "typed event를 history cells, bottom pane, pending operation으로 투영한다."],
      ["RENDER", "Ratatui CustomTerminal", "draw request를 coalesce하고 semantic transcript에서 resize/reflow/scrollback을 재구성한다."],
    ],
    facts: [
      ["Framework", "Rust + Ratatui + Crossterm + Tokio"],
      ["State shape", "App와 ChatWidget mutable state. typed event/command boundary"],
      ["Undo", "thread/rollback ack 전에는 transcript를 trim하지 않는 pending operation"],
      ["Extensions", "skills, hooks, plugins. app-server plugin endpoint는 under development"],
      ["QA", "TUI 약 216k LOC, test attrs 약 3,009, insta snapshots 547, vt100/PTY suite"],
    ],
    strength: "rollback을 base thread ID와 pending guard로 묶고 authoritative success 뒤 scrollback을 rebuild한다.",
    weakness: "dispatcher와 ChatWidget이 매우 크고 local AppEvent channel은 unbounded다. Rust가 coordinator 비대를 막아주지는 않는다.",
    akra: "queue mutation의 operation ID, base thread/revision guard, ack 이후 projection rebuild를 직접 차용한다.",
  },
  {
    id: "claude",
    name: "Claude Code",
    version: "v2.1.210",
    commit: "b7784f2c63ed",
    released: "2026-07-14",
    category: "artifact",
    thesis: "공개 제품 소스는 없다. 공식 Linux artifact에서 React reconciler 19.2, Ink 계열 node, Yoga layout, Bun native executable 문자열은 확인했지만 현재 renderer의 active path인지는 검증할 수 없다. 공식 문서는 classic과 fullscreen이 동일 conversation을 유지한다고 설명한다.",
    flow: [
      ["INPUT", "Interactive surface", "공식 fullscreen 문서가 keyboard, dialog, mouse, transcript search/selection 동작을 설명한다. 내부 component framework는 비공개다."],
      ["SESSION", "durable transcript / agent process", "공식 문서는 session과 renderer, process liveness를 분리한다고 설명한다. 내부 store는 비공개다."],
      ["CHECKPOINT", "prompt checkpoint / rewind command", "code와 conversation restore 범위를 선택하지만 Bash·external changes는 추적하지 않는다."],
      ["PROJECTION", "semantic message rows", "공식 fullscreen 문서상 visible message virtualization과 auto-follow state를 가진다."],
      ["RENDER", "classic scrollback 또는 fullscreen alt-screen", "동일 conversation을 유지한 채 renderer를 전환하고 changed cells만 갱신한다."],
    ],
    facts: [
      ["Framework", "artifact markers: Bun + React reconciler 19.2 + Ink-like nodes + Yoga. active path unverified"],
      ["Source limit", "제품 구현·tests 비공개. exact reducer/store topology는 검증 불가"],
      ["Mouse", "fullscreen research preview에서 hover/click/selection/wheel 지원"],
      ["Extensions", "skills, agents, hooks, MCP, LSP, monitors, executable plugins"],
      ["QA evidence", "공개 test suite 대신 changelog와 renderer escape hatch만 확인 가능"],
    ],
    strength: "renderer를 durable conversation의 projection으로 취급해 classic/fullscreen 전환과 reattach를 가능하게 한다.",
    weakness: "fullscreen은 research preview다. ghost frame, resize queue, memory leak 수정이 changelog에 반복되며 내부 품질 구조는 감사할 수 없다.",
    akra: "semantic transcript와 visible viewport를 분리하되, 비공개 구현의 세부 구조는 모방 근거로 사용하지 않는다.",
  },
  {
    id: "pi",
    name: "Pi",
    version: "v0.80.7",
    commit: "818d67457cdd",
    released: "2026-07-14",
    category: "custom",
    thesis: "작은 component contract와 differential renderer 위에 강력한 Agent/AgentSession event boundary를 둔다. 확장성은 가장 넓지만 InteractiveMode와 AgentSession에 조정 책임이 집중된다.",
    flow: [
      ["MODEL", "pi-ai stream", "provider별 streaming event를 정규화한다."],
      ["AUTHORITY", "Agent", "transcript, model, tool, steering/follow-up queue의 의미 상태를 소유한다."],
      ["ORCHESTRATE", "AgentSession", "저장, compaction, retry, extension lifecycle과 순차 listener를 조정한다."],
      ["PROJECTION", "InteractiveMode", "agent event를 editor, overlay, tool view, queue, status component에 반영한다."],
      ["RENDER", "pi-tui differential renderer", "16 ms coalescing, changed-line tail update, synchronized output, IME cursor를 처리한다."],
    ],
    facts: [
      ["Request mapping", "사용자가 이름을 확신하지 못한 ‘py’를 coding-agent Pi로 해석"],
      ["Framework", "@mariozechner/pi-tui custom component/differential renderer"],
      ["State shape", "Agent authority + AgentSession orchestration + 6k LOC InteractiveMode"],
      ["Mouse", "SGR sequence 보존은 하지만 기본 tracking/click dispatch는 하지 않음"],
      ["Extensions", "tools, commands, shortcuts, flags, TUI components, renderers, lifecycle hooks"],
      ["QA", "@xterm/headless virtual terminal, 관련 test files 약 211개"],
    ],
    strength: "agent state와 event order가 명확하고 extension이 custom TUI component까지 도달한다.",
    weakness: "확장은 프로세스 전체 권한이며 기본 sandbox가 없다. InteractiveMode와 custom renderer 비용이 크다.",
    akra: "순서 보장 event와 xterm-headless testing을 채택하고 arbitrary in-process TUI extension은 거부한다.",
  },
  {
    id: "jcode",
    name: "jcode",
    version: "v0.47.0",
    commit: "f7f5898cf661",
    released: "2026-07-14",
    category: "native",
    thesis: "장기 실행 server가 session authority를 갖고 Ratatui client는 연결 가능한 projection이 된다. 고밀도 UX와 mouse를 제공하지만 giant mutable App, local/remote 중복, stale event 수동 보정이 남아 있다.",
    flow: [
      ["INPUT", "Crossterm / Ratatui client", "key, click, wheel, drag selection과 picker interaction을 처리한다."],
      ["TRANSPORT", "newline JSON / Unix socket", "TUI와 장기 실행 server 사이 command/event protocol을 제공한다."],
      ["AUTHORITY", "daemon session Agent", "provider, tools, persistence, rewind와 실행 상태를 소유한다."],
      ["PROJECTION", "remote server event → mutable App", "History와 stream event가 TUI fields를 직접 갱신한다."],
      ["RENDER", "Ratatui + custom render crates", "Markdown, Mermaid, image/LaTeX, side panel, info-widget priority를 조합한다."],
    ],
    facts: [
      ["Framework", "Rust + Ratatui 0.30 + Crossterm 0.29 + custom render crates"],
      ["State shape", "server authority는 강하지만 client App reducer와 local mode가 중복"],
      ["Mouse", "click, wheel, drag selection, picker가 일급 입력"],
      ["Extensions", "MCP stdio, hooks, skills, Claude plugin skill loading"],
      ["QA", "mouse, layout cache, frame flicker, glyph, image, picker 등 대규모 Rust tests"],
    ],
    strength: "session을 daemon에 두고 화면 정보에 priority, preferred side, minimum height를 부여한다.",
    weakness: "rewind 뒤 stale Done이 history를 되살릴 수 있어 stream state를 수동 clear한다. 공식 refactoring 문서도 App 분리를 미완료로 둔다.",
    akra: "공간 priority 모델과 server-authoritative mutation만 채택하고 local/remote 이중 구현과 custom scrollback은 피한다.",
  },
];

const rendererRows = [
  {
    id: "opencode", name: "OpenCode", category: "reactive",
    ui: "SolidJS + OpenTUI", uiNote: "Effect lifecycle / JSX providers",
    authority: "Bun Worker / server", frame: "OpenTUI renderer · 60 fps target", mouse: ["yes", "first-class"],
    qa: "45 modules / 52 tree files", extension: "plugins + TUI API",
  },
  {
    id: "gjc", name: "GJC", category: "custom",
    ui: "@gajae-code/tui", uiNote: "Pi-derived component lines",
    authority: "AgentSession", frame: "virtual viewport + line diff + CSI 2026", mouse: ["no", "app click off"],
    qa: "VT/golden/CJK/perf gates", extension: "bounded plugin families",
  },
  {
    id: "codex", name: "Codex", category: "native",
    ui: "Ratatui + Crossterm", uiNote: "CustomTerminal / semantic cells",
    authority: "core / app-server", frame: "coalesced draw · transcript reflow", mouse: ["no", "events dropped"],
    qa: "547 snaps + vt100/PTY", extension: "agent/runtime only",
  },
  {
    id: "claude", name: "Claude Code", category: "artifact",
    ui: "React/Ink/Yoga markers", uiNote: "artifact-observed · active path unverified",
    authority: "durable session", frame: "classic scrollback / virtualized fullscreen", mouse: ["yes", "fullscreen only"],
    qa: "public suite unavailable", extension: "plugins, not UI components",
  },
  {
    id: "pi", name: "Pi", category: "custom",
    ui: "@mariozechner/pi-tui", uiNote: "render(width) → lines",
    authority: "Agent + AgentSession", frame: "16 ms coalesce + changed-line tail", mouse: ["no", "SGR preserved"],
    qa: "xterm-headless · ~211 files", extension: "custom TUI components",
  },
  {
    id: "jcode", name: "jcode", category: "native",
    ui: "Ratatui + Crossterm", uiNote: "custom render/markdown crates",
    authority: "daemon session", frame: "Ratatui frame + custom scrollback", mouse: ["yes", "click/wheel/drag"],
    qa: "broad Rust UI test suite", extension: "hooks/skills, no UI SDK",
  },
];

const polishSystems = [
  ["01", "Semantic design tokens", "색을 직접 고르지 않고 permission, warning, diff, muted, selected 같은 의미를 component가 요청한다.", "OpenCode themes · Codex/Claude styles · AkraTheme"],
  ["02", "Stable component geometry", "overlay, input, status, list가 고정된 최소 높이와 viewport rule을 가져 streaming 중 layout shift를 제한한다.", "jcode info priority · OpenTUI/Yoga · Ratatui constraints"],
  ["03", "Incremental rendering", "changed line/cell만 출력하고 draw request를 coalesce해 terminal throughput과 flicker를 관리한다.", "Pi/GJC diff · Codex FrameRequester · Claude fullscreen"],
  ["04", "One interaction vocabulary", "keymap, command palette, dialog, row action, toast가 같은 action name과 focus contract를 사용한다.", "OpenCode keymap/dialog · Claude bindings · Pi keymap"],
  ["05", "Dense progressive disclosure", "기본 화면에는 now/next/status만 두고 tool detail, permission, history, task list는 overlay나 expandable row로 연다.", "OpenCode sidebar/dialog · Codex bottom pane · jcode side panel"],
  ["06", "Terminal-specific QA", "CJK width, IME, paste, resize, scrollback, stale rows, synchronized output을 실제 terminal model로 검증한다.", "GJC/Pi virtual terminal · Codex vt100/PTY · jcode render tests"],
];

const galleryItems = [
  {
    id: "opencode", name: "OpenCode", src: "assets/opencode-official-tui.png",
    alt: "OpenCode 공식 저장소의 terminal UI 스크린샷",
    caption: "OpenCode 공식 TUI 시각 자료. 저장소의 현재 v1.18.2 asset이며 presentation만 증명한다.",
    url: "https://github.com/anomalyco/opencode/blob/70b56a0a93d366889cae950379cc9d2537148fa2/packages/web/src/assets/lander/screenshot.png",
  },
  {
    id: "codex", name: "Codex", src: "assets/codex-official-tui.png",
    alt: "OpenAI Codex 공식 저장소의 terminal UI 스크린샷",
    caption: "Codex 공식 splash asset. plan/activity hierarchy와 semantic transcript 표현을 보여준다.",
    url: "https://github.com/openai/codex/blob/8c68d4c87dc54d38861f5114e920c3de2efa5876/.github/codex-cli-splash.png",
  },
  {
    id: "pi", name: "Pi", src: "assets/pi-official-tui.png",
    alt: "Pi 공식 문서의 interactive mode 스크린샷",
    caption: "Pi 공식 interactive-mode 자료. component tree, tool output, editor, status line의 밀도를 보여준다.",
    url: "https://github.com/earendil-works/pi/blob/818d67457cdd6b60bce6b121d16b23141c252dd8/packages/coding-agent/docs/images/interactive-mode.png",
  },
  {
    id: "jcode", name: "jcode", src: "assets/jcode-official-gallery.png",
    alt: "jcode 공식 golden test의 긴 transcript 화면",
    caption: "jcode v0.47.0 desktop golden fixture. 긴 transcript의 wrapping, density, bottom anchoring 검증 자료다.",
    url: "https://github.com/1jehuang/jcode/blob/f7f5898cf6614051dd791655b7a87544210c1bd1/tests/desktop-gallery-golden/gallery-long-transcript.png",
  },
  {
    id: "gjc", name: "GJC", src: "assets/gjc-official-hero.png",
    alt: "Gajae-Code 공식 저장소의 제품 소개 이미지",
    caption: "GJC v0.11.0 공식 promotional visual. TUI correctness나 production readiness의 근거로 사용하지 않는다.",
    url: "https://github.com/Yeachan-Heo/gajae-code/blob/8132409c3f10754fea5f3b0108a7bee979c43652/assets/hero.png",
  },
];

const openCodeFlow = [
  ["Message hover / mouse-up", "row에서 직접 destructive action을 실행하지 않고 <code>Message Actions</code> dialog를 연다."],
  ["Revert action", "dialog 첫 항목이 <code>sdk.client.session.revert(sessionID, messageID)</code>를 호출한다."],
  ["Optimistic composer", "원문을 composer에 즉시 넣고 dialog를 닫는다. server ack를 기다리지 않는다."],
  ["Server staged revert", "busy guard → snapshot restore/patch revert → diff → revert marker persist/event 순서다."],
  ["Sync projection", "event가 Solid store에 반영되고 redo dock/unrevert affordance가 노출된다."],
];

const akraFlow = [
  ["Intent only", "TUI는 <code>RemoveQueueItem { task_id, base_revision }</code> command만 보낸다."],
  ["Pending operation", "행은 유지하고 <code>operation_id</code>와 ack 대기를 표시한다. 중복 action은 막는다."],
  ["Authority decision", "application/store가 identity, revision, executable status를 검증해 accept/reject한다."],
  ["Revisioned snapshot", "accept event의 new revision과 correlation이 pending operation과 일치할 때만 projection을 바꾼다."],
  ["Undo dock", "성공 뒤에만 행을 제거하고 짧은 restore command를 제공한다. failure/stale이면 행을 유지한다."],
];

const undoRules = [
  ["01", "Action menu, not magic click", "row click은 선택 또는 menu open이다. 삭제 자체는 이름이 명확한 두 번째 action이어야 한다."],
  ["02", "Ack before projection", "composer, queue row, now/next copy는 authority success 전에는 확정 상태로 바꾸지 않는다."],
  ["03", "Identity + revision", "제목 문자열이 아니라 task ID, repository incarnation, base revision으로 stale mutation을 차단한다."],
  ["04", "Undo is a new command", "복구는 로컬 화면 되감기가 아니라 이전 결과를 참조하는 revisioned restore mutation이다."],
];

const debts = [
  {product:"OpenCode", id:"opencode", type:"verified", title:"큰 route와 분산 mutation surface", body:"session route 2,710 LOC, prompt 1,713 LOC, app 1,134 LOC이며 component가 SDK mutation을 직접 호출한다. 선언적 UI가 coordinator 복잡성을 제거하지 않았다.", url:"https://github.com/anomalyco/opencode/blob/70b56a0a93d366889cae950379cc9d2537148fa2/packages/tui/src/routes/session/index.tsx"},
  {product:"OpenCode", id:"opencode", type:"reported", title:"Undo가 server success 전에 composer를 바꾼다는 공식 audit", body:"공식 issue #36582는 undo/message revert를 포함한 fire-and-forget mutation과 error UX 부재를 지적한다. 본 조사에서 재현하지 않았으므로 reported다.", url:"https://github.com/anomalyco/opencode/issues/36582"},
  {product:"OpenCode", id:"opencode", type:"reported", title:"Queued message revert 후 surprise-send", body:"공식 issue #28843은 queued message를 revert해도 dequeue되지 않아 composer와 queue에 동시에 남는 현상을 보고한다.", url:"https://github.com/anomalyco/opencode/issues/28843"},
  {product:"OpenCode", id:"opencode", type:"reported", title:"100-message projection 밖에서 /undo 탐색 실패", body:"공식 issue #28257은 bounded visible history 밖의 마지막 user message를 /undo가 찾지 못한다고 보고한다.", url:"https://github.com/anomalyco/opencode/issues/28257"},
  {product:"GJC", id:"gjc", type:"documented", title:"Beta와 rough edges를 공식 명시", body:"v0.11.0 README는 프로젝트를 experimental beta로 분류한다. marketing visual의 production-ready 문구와 별개로 취급해야 한다.", url:"https://github.com/Yeachan-Heo/gajae-code/blob/8132409c3f10754fea5f3b0108a7bee979c43652/README.md#L23-L27"},
  {product:"GJC", id:"gjc", type:"verified", title:"Custom engine과 큰 integration class", body:"Pi fork 기반 renderer와 약 3천 줄 InteractiveMode를 동시에 유지한다. engine message-agnostic 설계가 composition 비용까지 제거하지는 않는다.", url:"https://github.com/Yeachan-Heo/gajae-code/blob/8132409c3f10754fea5f3b0108a7bee979c43652/packages/coding-agent/src/modes/interactive-mode.ts"},
  {product:"GJC", id:"gjc", type:"documented", title:"Compiled Bun binary native-addon loading limitation", body:"공식 개발 문서는 compiled binary가 @gajae-code/natives를 동적으로 찾지 못해 source dev:link 경로를 요구하는 제한을 설명한다.", url:"https://github.com/Yeachan-Heo/gajae-code/blob/8132409c3f10754fea5f3b0108a7bee979c43652/README.md#L304-L323"},
  {product:"Codex", id:"codex", type:"verified", title:"거대한 AppEvent / dispatcher / ChatWidget", body:"TUI는 순수 reducer가 아니다. mutable App/ChatWidget과 큰 dispatcher가 local, core, app-server events를 조정한다.", url:"https://github.com/openai/codex/blob/8c68d4c87dc54d38861f5114e920c3de2efa5876/codex-rs/tui/src/app/event_dispatch.rs"},
  {product:"Codex", id:"codex", type:"verified", title:"Unbounded local AppEvent channel", body:"App이 unbounded channel을 생성하고 clonable sender wrapper가 local coordination event를 전달한다. typed protocol과 강한 tests가 있어도 backpressure와 coordinator size는 남은 설계 비용이다.", url:"https://github.com/openai/codex/blob/8c68d4c87dc54d38861f5114e920c3de2efa5876/codex-rs/tui/src/app.rs#L782"},
  {product:"Codex", id:"codex", type:"documented", title:"Config-as-state 제거 TODO", body:"config persistence에는 config 전체를 state object처럼 다루는 구조를 제거하려는 TODO가 남아 있다.", url:"https://github.com/openai/codex/blob/8c68d4c87dc54d38861f5114e920c3de2efa5876/codex-rs/tui/src/app/config_persistence.rs#L772"},
  {product:"Claude Code", id:"claude", type:"verified", title:"구현과 test suite 비공개", body:"공식 repo는 README, changelog, plugins, release metadata를 제공하지만 제품 source와 tests는 없다. 내부 state topology의 품질은 외부에서 감사할 수 없다.", url:"https://github.com/anthropics/claude-code/tree/b7784f2c63ed4585c32bc20b94d3b64cf4fe6df3"},
  {product:"Claude Code", id:"claude", type:"documented", title:"Fullscreen은 research preview", body:"공식 문서는 alternate-screen renderer를 research preview로 분류하고 ConPTY stale fragment 대응 full-repaint escape hatch를 설명한다.", url:"https://code.claude.com/docs/en/fullscreen"},
  {product:"Claude Code", id:"claude", type:"documented", title:"Renderer와 session 회귀 수정이 계속됨", body:"v2.1.210 changelog에는 attach/resize queue, ghost frame, stale plan overwrite, renderer crash와 worktree isolation 관련 수정이 포함된다.", url:"https://github.com/anthropics/claude-code/blob/b7784f2c63ed4585c32bc20b94d3b64cf4fe6df3/CHANGELOG.md"},
  {product:"Pi", id:"pi", type:"verified", title:"6천 줄 InteractiveMode", body:"editor, overlay, streaming, tools, queue, retry, compaction, extension widgets, footer가 한 integration class에 집중된다.", url:"https://github.com/earendil-works/pi/blob/818d67457cdd6b60bce6b121d16b23141c252dd8/packages/coding-agent/src/modes/interactive/interactive-mode.ts#L340-L512"},
  {product:"Pi", id:"pi", type:"documented", title:"Full-trust extensions", body:"extensions는 프로세스 전체 권한을 가지며 Pi 자체 permission sandbox가 없다. 격리는 외부 container에 의존한다.", url:"https://github.com/earendil-works/pi/blob/818d67457cdd6b60bce6b121d16b23141c252dd8/packages/coding-agent/docs/extensions.md"},
  {product:"Pi", id:"pi", type:"documented", title:"TUI correctness bugs are normal", body:"공식 changelog는 CJK alignment, scrollback corruption, stale line, resize, image redraw, narrow terminal crash, Kitty race 수정을 기록한다.", url:"https://github.com/earendil-works/pi/blob/818d67457cdd6b60bce6b121d16b23141c252dd8/packages/tui/CHANGELOG.md"},
  {product:"jcode", id:"jcode", type:"verified", title:"Mutable App 분리는 아직 refactoring backlog", body:"공식 REFACTORING 문서는 App state, command parsing, remote-event reduction, rendering control 분리를 남은 작업으로 명시한다.", url:"https://github.com/1jehuang/jcode/blob/f7f5898cf6614051dd791655b7a87544210c1bd1/docs/REFACTORING.md"},
  {product:"jcode", id:"jcode", type:"verified", title:"Rewind 뒤 stale Done 보정", body:"truncated History 뒤 이전 turn Done이 도착해 내용을 되살리는 것을 막기 위해 client가 stream state를 수동 clear한다.", url:"https://github.com/1jehuang/jcode/blob/f7f5898cf6614051dd791655b7a87544210c1bd1/crates/jcode-tui/src/tui/app/remote/server_events.rs"},
  {product:"jcode", id:"jcode", type:"verified", title:"Local/remote rewind 중복", body:"server-authoritative remote path와 별도 local implementation이 함께 존재해 state transition과 cache cleanup을 두 번 유지한다.", url:"https://github.com/1jehuang/jcode/tree/f7f5898cf6614051dd791655b7a87544210c1bd1/crates/jcode-tui/src/tui/app"},
];

const priorities = [
  {badge:"P0-A", title:"Authority-ack projection gate", body:"queue/turn/session mutation은 operation ID, base revision, exact identity를 포함하고 success ack 뒤에만 TUI projection을 바꾼다.", proof:"reducer tests + stale/failure matrix"},
  {badge:"P0-B", title:"Pure ConversationScreenModel", body:"Core snapshot과 UI-only state를 받아 immutable copy/layout model을 만드는 순수 projection을 도입하고 runtime coordination을 제거한다.", proof:"snapshot parity + no-service boundary"},
  {badge:"P0-C", title:"Terminal delivery transaction", body:"semantic transcript, viewport, host scrollback, cursor, flush를 분리하고 resize/redraw/reattach 시 한 경로로 rebuild한다.", proof:"PTY frame recorder + CJK resize"},
  {badge:"P1", title:"Queue row actions + undo dock", body:"명확한 hit region의 row menu, pending state, success-only removal, short-lived restore affordance를 keyboard와 mouse에 동일 command로 연결한다.", proof:"mouse/key parity + ack/failure capture"},
  {badge:"P1", title:"Information priority contract", body:"now/next, activity, queue, warning, detail에 priority/min-height/truncation rule을 부여해 긴 copy가 핵심 상태를 밀어내지 못하게 한다.", proof:"80×24 / 120×40 / CJK captures"},
  {badge:"P2", title:"Extension boundary review", body:"skills/plugins는 agent workflow 확장으로 유지한다. TUI component plugin은 안정된 screen-model contract 이후 별도 판단한다.", proof:"architecture decision record"},
];

const sources = [
  {p:"OpenCode", topic:"release pin", type:"verified", label:"v1.18.2 release", url:"https://github.com/anomalyco/opencode/releases/tag/v1.18.2"},
  {p:"OpenCode", topic:"framework", type:"verified", label:"TUI package dependencies: OpenTUI, Solid, Effect", url:"https://github.com/anomalyco/opencode/blob/70b56a0a93d366889cae950379cc9d2537148fa2/packages/tui/package.json"},
  {p:"OpenCode", topic:"renderer", type:"verified", label:"createCliRenderer lifecycle, mouse, Kitty, 60 fps", url:"https://github.com/anomalyco/opencode/blob/70b56a0a93d366889cae950379cc9d2537148fa2/packages/tui/src/app.tsx#L186-L218"},
  {p:"OpenCode", topic:"topology", type:"verified", label:"TUI → generated SDK → Worker/HTTP server", url:"https://github.com/anomalyco/opencode/blob/70b56a0a93d366889cae950379cc9d2537148fa2/packages/opencode/src/cli/cmd/tui.ts#L210-L300"},
  {p:"OpenCode", topic:"projection", type:"verified", label:"16 ms event batching and SDK reconnect", url:"https://github.com/anomalyco/opencode/blob/70b56a0a93d366889cae950379cc9d2537148fa2/packages/tui/src/context/sdk.tsx#L48-L139"},
  {p:"OpenCode", topic:"projection", type:"verified", label:"Sync store hydration/live merge and visible window", url:"https://github.com/anomalyco/opencode/blob/70b56a0a93d366889cae950379cc9d2537148fa2/packages/tui/src/context/sync.tsx#L54-L145"},
  {p:"OpenCode", topic:"mouse undo", type:"verified", label:"Message row mouse-up opens action dialog", url:"https://github.com/anomalyco/opencode/blob/70b56a0a93d366889cae950379cc9d2537148fa2/packages/tui/src/routes/session/index.tsx#L1253-L1269"},
  {p:"OpenCode", topic:"mouse undo", type:"verified", label:"Message dialog revert/copy/fork actions", url:"https://github.com/anomalyco/opencode/blob/70b56a0a93d366889cae950379cc9d2537148fa2/packages/tui/src/routes/session/dialog-message.tsx#L10-L108"},
  {p:"OpenCode", topic:"undo authority", type:"verified", label:"SessionRevert server transaction", url:"https://github.com/anomalyco/opencode/blob/70b56a0a93d366889cae950379cc9d2537148fa2/packages/opencode/src/session/revert.ts#L38-L134"},
  {p:"OpenCode", topic:"plugins", type:"verified", label:"TUI plugin surface and v2 deprecation marker", url:"https://github.com/anomalyco/opencode/blob/70b56a0a93d366889cae950379cc9d2537148fa2/packages/plugin/src/tui.ts#L53-L119"},
  {p:"OpenCode", topic:"QA", type:"verified", label:"Pinned TUI test tree counted as 45 test modules / 52 files", url:"https://github.com/anomalyco/opencode/tree/70b56a0a93d366889cae950379cc9d2537148fa2/packages/tui/test"},
  {p:"OpenCode", topic:"bugs", type:"reported", label:"Fire-and-forget SDK mutation audit #36582", url:"https://github.com/anomalyco/opencode/issues/36582"},
  {p:"OpenCode", topic:"bugs", type:"reported", label:"Queued message revert duplication #28843", url:"https://github.com/anomalyco/opencode/issues/28843"},

  {p:"GJC", topic:"release pin", type:"verified", label:"v0.11.0 release", url:"https://github.com/Yeachan-Heo/gajae-code/releases/tag/v0.11.0"},
  {p:"GJC", topic:"status", type:"documented", label:"Experimental beta / rough edges notice", url:"https://github.com/Yeachan-Heo/gajae-code/blob/8132409c3f10754fea5f3b0108a7bee979c43652/README.md#L23-L27"},
  {p:"GJC", topic:"framework", type:"verified", label:"@gajae-code/tui component contract", url:"https://github.com/Yeachan-Heo/gajae-code/blob/8132409c3f10754fea5f3b0108a7bee979c43652/packages/tui/README.md#L1-L75"},
  {p:"GJC", topic:"lineage", type:"verified", label:"Pi fork and package rename changelog", url:"https://github.com/Yeachan-Heo/gajae-code/blob/8132409c3f10754fea5f3b0108a7bee979c43652/packages/tui/CHANGELOG.md#L968-L978"},
  {p:"GJC", topic:"architecture", type:"verified", label:"TUI runtime ownership and render pipeline", url:"https://github.com/Yeachan-Heo/gajae-code/blob/8132409c3f10754fea5f3b0108a7bee979c43652/docs/tui-runtime-internals.md#L5-L147"},
  {p:"GJC", topic:"mouse", type:"verified", label:"Terminal cleanup disables normal and SGR mouse reporting", url:"https://github.com/Yeachan-Heo/gajae-code/blob/8132409c3f10754fea5f3b0108a7bee979c43652/packages/tui/src/terminal.ts#L51-L55"},
  {p:"GJC", topic:"mouse", type:"documented", label:"Managed tmux wheel-scroll profile", url:"https://github.com/Yeachan-Heo/gajae-code/blob/8132409c3f10754fea5f3b0108a7bee979c43652/docs/environment-variables.md#L234-L276"},
  {p:"GJC", topic:"plugins", type:"verified", label:"Bounded GJC plugin families", url:"https://github.com/Yeachan-Heo/gajae-code/blob/8132409c3f10754fea5f3b0108a7bee979c43652/docs/gjc-plugins.md#L1-L82"},
  {p:"GJC", topic:"QA", type:"verified", label:"Visual QA contract: CJK, ANSI, fresh evidence", url:"https://github.com/Yeachan-Heo/gajae-code/blob/8132409c3f10754fea5f3b0108a7bee979c43652/docs/ui-design-visual-qa.md"},

  {p:"Codex", topic:"release pin", type:"verified", label:"rust-v0.144.4 release", url:"https://github.com/openai/codex/releases/tag/rust-v0.144.4"},
  {p:"Codex", topic:"framework", type:"verified", label:"Ratatui/Crossterm TUI dependencies", url:"https://github.com/openai/codex/blob/8c68d4c87dc54d38861f5114e920c3de2efa5876/codex-rs/tui/Cargo.toml"},
  {p:"Codex", topic:"event loop", type:"verified", label:"App event select loop", url:"https://github.com/openai/codex/blob/8c68d4c87dc54d38861f5114e920c3de2efa5876/codex-rs/tui/src/app.rs#L1172-L1214"},
  {p:"Codex", topic:"event channel", type:"verified", label:"Unbounded AppEvent channel creation", url:"https://github.com/openai/codex/blob/8c68d4c87dc54d38861f5114e920c3de2efa5876/codex-rs/tui/src/app.rs#L782"},
  {p:"Codex", topic:"event channel", type:"verified", label:"Clonable AppEvent sender wrapper", url:"https://github.com/openai/codex/blob/8c68d4c87dc54d38861f5114e920c3de2efa5876/codex-rs/tui/src/app_event_sender.rs#L23-L34"},
  {p:"Codex", topic:"rendering", type:"verified", label:"FrameRequester coalescing and 120 fps cap", url:"https://github.com/openai/codex/blob/8c68d4c87dc54d38861f5114e920c3de2efa5876/codex-rs/tui/src/tui/frame_requester.rs#L1-L127"},
  {p:"Codex", topic:"rendering", type:"verified", label:"Semantic transcript reflow on resize", url:"https://github.com/openai/codex/blob/8c68d4c87dc54d38861f5114e920c3de2efa5876/codex-rs/tui/src/transcript_reflow.rs#L1-L34"},
  {p:"Codex", topic:"mouse", type:"verified", label:"Mouse events explicitly ignored", url:"https://github.com/openai/codex/blob/8c68d4c87dc54d38861f5114e920c3de2efa5876/codex-rs/tui/src/tui/event_stream.rs#L173-L236"},
  {p:"Codex", topic:"rollback", type:"verified", label:"Backtrack pending guard and authoritative ack", url:"https://github.com/openai/codex/blob/8c68d4c87dc54d38861f5114e920c3de2efa5876/codex-rs/tui/src/app_backtrack.rs#L188-L240"},
  {p:"Codex", topic:"extensions", type:"documented", label:"Skills, hooks, and plugin app-server endpoints; plugins under development", url:"https://github.com/openai/codex/blob/8c68d4c87dc54d38861f5114e920c3de2efa5876/codex-rs/app-server/README.md#L210-L233"},

  {p:"Claude Code", topic:"release pin", type:"verified", label:"v2.1.210 release", url:"https://github.com/anthropics/claude-code/releases/tag/v2.1.210"},
  {p:"Claude Code", topic:"artifact", type:"verified", label:"npm package v2.1.210, native binary distribution", url:"https://registry.npmjs.org/@anthropic-ai/claude-code/2.1.210"},
  {p:"Claude Code", topic:"framework", type:"verified", label:"Artifact strings: React reconciler 19.2, Ink nodes, Yoga, Bun", url:"https://registry.npmjs.org/@anthropic-ai/claude-code-linux-x64/2.1.210"},
  {p:"Claude Code", topic:"rendering", type:"documented", label:"Classic and fullscreen renderer contract", url:"https://code.claude.com/docs/en/fullscreen"},
  {p:"Claude Code", topic:"undo", type:"documented", label:"Checkpoint and rewind behavior/limitations", url:"https://code.claude.com/docs/en/checkpointing"},
  {p:"Claude Code", topic:"interaction", type:"documented", label:"Interactive mode shortcuts and session controls", url:"https://code.claude.com/docs/en/interactive-mode"},
  {p:"Claude Code", topic:"plugins", type:"documented", label:"Plugin component reference", url:"https://code.claude.com/docs/en/plugins-reference"},
  {p:"Claude Code", topic:"bugs", type:"documented", label:"Official changelog renderer/session fixes", url:"https://github.com/anthropics/claude-code/blob/b7784f2c63ed4585c32bc20b94d3b64cf4fe6df3/CHANGELOG.md"},

  {p:"Pi", topic:"release pin", type:"verified", label:"v0.80.7 repository snapshot", url:"https://github.com/earendil-works/pi/tree/818d67457cdd6b60bce6b121d16b23141c252dd8"},
  {p:"Pi", topic:"authority", type:"verified", label:"Agent state ownership", url:"https://github.com/earendil-works/pi/blob/818d67457cdd6b60bce6b121d16b23141c252dd8/packages/agent/src/agent.ts#L60-L171"},
  {p:"Pi", topic:"queue", type:"verified", label:"Steering/follow-up consumption points", url:"https://github.com/earendil-works/pi/blob/818d67457cdd6b60bce6b121d16b23141c252dd8/packages/agent/src/agent-loop.ts#L95-L275"},
  {p:"Pi", topic:"framework", type:"verified", label:"TUI component interface", url:"https://github.com/earendil-works/pi/blob/818d67457cdd6b60bce6b121d16b23141c252dd8/packages/tui/src/tui.ts#L54-L84"},
  {p:"Pi", topic:"rendering", type:"verified", label:"Differential renderer and synchronized output", url:"https://github.com/earendil-works/pi/blob/818d67457cdd6b60bce6b121d16b23141c252dd8/packages/tui/src/tui.ts#L1250-L1580"},
  {p:"Pi", topic:"sessions", type:"verified", label:"Append-only session tree", url:"https://github.com/earendil-works/pi/blob/818d67457cdd6b60bce6b121d16b23141c252dd8/packages/coding-agent/src/core/session-manager.ts#L30-L51"},
  {p:"Pi", topic:"plugins", type:"verified", label:"Extension API and trust model", url:"https://github.com/earendil-works/pi/blob/818d67457cdd6b60bce6b121d16b23141c252dd8/packages/coding-agent/docs/extensions.md"},
  {p:"Pi", topic:"QA", type:"verified", label:"xterm-headless virtual terminal", url:"https://github.com/earendil-works/pi/blob/818d67457cdd6b60bce6b121d16b23141c252dd8/packages/tui/test/virtual-terminal.ts#L1-L185"},

  {p:"jcode", topic:"release pin", type:"verified", label:"v0.47.0 release", url:"https://github.com/1jehuang/jcode/releases/tag/v0.47.0"},
  {p:"jcode", topic:"topology", type:"verified", label:"Persistent server architecture", url:"https://github.com/1jehuang/jcode/blob/f7f5898cf6614051dd791655b7a87544210c1bd1/docs/SERVER_ARCHITECTURE.md"},
  {p:"jcode", topic:"protocol", type:"verified", label:"Newline JSON Unix socket protocol", url:"https://github.com/1jehuang/jcode/blob/f7f5898cf6614051dd791655b7a87544210c1bd1/crates/jcode-protocol/src/lib.rs"},
  {p:"jcode", topic:"layout", type:"verified", label:"Info widget priority and minimum height", url:"https://github.com/1jehuang/jcode/blob/f7f5898cf6614051dd791655b7a87544210c1bd1/crates/jcode-tui/src/tui/info_widget.rs"},
  {p:"jcode", topic:"debt", type:"verified", label:"Official refactoring plan", url:"https://github.com/1jehuang/jcode/blob/f7f5898cf6614051dd791655b7a87544210c1bd1/docs/REFACTORING.md"},
  {p:"jcode", topic:"rewind", type:"verified", label:"Rewind wire protocol", url:"https://github.com/1jehuang/jcode/blob/f7f5898cf6614051dd791655b7a87544210c1bd1/crates/jcode-protocol/src/wire.rs"},
  {p:"jcode", topic:"hooks", type:"documented", label:"Lifecycle hook contract", url:"https://github.com/1jehuang/jcode/blob/f7f5898cf6614051dd791655b7a87544210c1bd1/docs/HOOKS.md"},
  {p:"jcode", topic:"skills", type:"verified", label:"Claude plugin skill directory discovery", url:"https://github.com/1jehuang/jcode/blob/f7f5898cf6614051dd791655b7a87544210c1bd1/crates/jcode-base/src/skill.rs#L246-L247"},
  {p:"jcode", topic:"skills", type:"verified", label:"Skill loading across discovered roots", url:"https://github.com/1jehuang/jcode/blob/f7f5898cf6614051dd791655b7a87544210c1bd1/crates/jcode-base/src/skill.rs#L333-L343"},

  {p:"Akra", topic:"baseline", type:"verified", label:"Runtime architecture and state authority", url:"https://github.com/RefinedStone/codex-exec-loop/blob/1607f2cf3e43025c2371d5de9b1bbf1437b3fc78/docs/reference/architecture.md"},
  {p:"Akra", topic:"TUI contract", type:"verified", label:"Layered architecture and aesthetic contract", url:"https://github.com/RefinedStone/codex-exec-loop/blob/1607f2cf3e43025c2371d5de9b1bbf1437b3fc78/docs/design/07-tui-layered-architecture-and-aesthetic-contract.md"},
  {p:"Akra", topic:"projection", type:"verified", label:"ConversationViewModel source", url:"https://github.com/RefinedStone/codex-exec-loop/blob/1607f2cf3e43025c2371d5de9b1bbf1437b3fc78/src/adapter/inbound/tui/app/conversation_model/view_model.rs#L114"},
  {p:"Akra", topic:"composition", type:"verified", label:"NativeTuiApp aggregate", url:"https://github.com/RefinedStone/codex-exec-loop/blob/1607f2cf3e43025c2371d5de9b1bbf1437b3fc78/src/adapter/inbound/tui/app.rs#L323"},
  {p:"Akra", topic:"boundary debt", type:"verified", label:"Architecture boundary debt ratchets", url:"https://github.com/RefinedStone/codex-exec-loop/blob/1607f2cf3e43025c2371d5de9b1bbf1437b3fc78/tests/architecture_boundaries.rs"},
];

const evidenceLabel = (type) => `<span class="evidence ${type}">${type}</span>`;

function renderArchitecture(productId) {
  const product = products.find((item) => item.id === productId) ?? products[0];
  const stage = document.querySelector("#architectureStage");
  stage.innerHTML = `
    <div class="architecture-titlebar">
      <div>
        <h3>${product.name}</h3>
        <p>${product.thesis}</p>
      </div>
      <div class="version-lock">
        ${evidenceLabel("verified")}
        <strong>${product.version}</strong>
        <span>${product.commit} · ${product.released}</span>
      </div>
    </div>
    <div class="architecture-body">
      <div class="flow-diagram">
        <ol>
          ${product.flow.map(([label, title, detail]) => `
            <li><span>${label}</span><div><strong>${title}</strong><small>${detail}</small></div></li>
          `).join("")}
        </ol>
      </div>
      <div class="architecture-facts">
        <h4>TECHNICAL PROFILE</h4>
        <dl>
          ${product.facts.map(([label, value]) => `<div><dt>${label}</dt><dd>${value}</dd></div>`).join("")}
        </dl>
      </div>
    </div>
    <div class="architecture-verdict">
      <div><span>Strength</span><p>${product.strength}</p></div>
      <div><span>Critical debt</span><p>${product.weakness}</p></div>
      <div><span>Akra decision</span><p>${product.akra}</p></div>
    </div>
  `;
}

function setupArchitectureTabs() {
  const tabs = document.querySelector("#architectureTabs");
  products.forEach((product, index) => {
    const button = document.createElement("button");
    button.type = "button";
    button.role = "tab";
    button.id = `architecture-tab-${product.id}`;
    button.setAttribute("aria-controls", "architectureStage");
    button.setAttribute("aria-selected", String(index === 0));
    button.textContent = product.name;
    button.addEventListener("click", () => {
      tabs.querySelectorAll("button").forEach((item) => item.setAttribute("aria-selected", "false"));
      button.setAttribute("aria-selected", "true");
      document.querySelector("#architectureStage").setAttribute("aria-labelledby", button.id);
      renderArchitecture(product.id);
    });
    tabs.append(button);
  });
  document.querySelector("#architectureStage").setAttribute("aria-labelledby", "architecture-tab-opencode");
  renderArchitecture(products[0].id);
}

let matrixFilter = "all";

function renderMatrix() {
  const query = document.querySelector("#matrixSearch").value.trim().toLowerCase();
  const body = document.querySelector("#rendererMatrix");
  const filtered = rendererRows.filter((row) => {
    const matchesCategory = matrixFilter === "all" || row.category === matrixFilter;
    const haystack = Object.values(row).flat().join(" ").toLowerCase();
    return matchesCategory && haystack.includes(query);
  });
  body.innerHTML = filtered.map((row) => `
    <tr>
      <td>${row.name}<small>${products.find((product) => product.id === row.id).version}</small></td>
      <td><span class="matrix-chip">${row.ui}</span><small>${row.uiNote}</small></td>
      <td>${row.authority}</td>
      <td>${row.frame}</td>
      <td><span class="matrix-chip ${row.mouse[0]}">${row.mouse[0]}</span><small>${row.mouse[1]}</small></td>
      <td>${row.qa}</td>
      <td>${row.extension}</td>
    </tr>
  `).join("") || `<tr><td colspan="7"><div class="empty-state">검색 조건에 맞는 제품이 없습니다.</div></td></tr>`;
}

function setupMatrix() {
  document.querySelector("#matrixSearch").addEventListener("input", renderMatrix);
  document.querySelectorAll("#matrixFilters button").forEach((button) => {
    button.addEventListener("click", () => {
      matrixFilter = button.dataset.filter;
      document.querySelectorAll("#matrixFilters button").forEach((item) => {
        const active = item === button;
        item.classList.toggle("active", active);
        item.setAttribute("aria-pressed", String(active));
      });
      renderMatrix();
    });
  });
  renderMatrix();
}

function renderPolish() {
  document.querySelector("#polishGrid").innerHTML = polishSystems.map(([number, title, body, examples]) => `
    <article class="polish-item"><span>${number}</span><h3>${title}</h3><p>${body}</p><small>${examples}</small></article>
  `).join("");
}

function renderGallery(itemId) {
  const item = galleryItems.find((entry) => entry.id === itemId) ?? galleryItems[0];
  document.querySelector("#productGallery").innerHTML = `
    <img src="${item.src}" alt="${item.alt}" loading="eager">
    <figcaption><span>${item.caption}</span><a href="${item.url}" target="_blank" rel="noreferrer">pinned official asset ↗</a></figcaption>
  `;
}

function setupGallery() {
  const controls = document.querySelector("#galleryControls");
  galleryItems.forEach((item, index) => {
    const button = document.createElement("button");
    button.type = "button";
    button.role = "tab";
    button.id = `gallery-tab-${item.id}`;
    button.setAttribute("aria-controls", "productGallery");
    button.setAttribute("aria-selected", String(index === 0));
    button.textContent = item.name;
    button.addEventListener("click", () => {
      controls.querySelectorAll("button").forEach((control) => control.setAttribute("aria-selected", "false"));
      button.setAttribute("aria-selected", "true");
      document.querySelector("#productGallery").setAttribute("aria-labelledby", button.id);
      renderGallery(item.id);
    });
    controls.append(button);
  });
  document.querySelector("#productGallery").setAttribute("aria-labelledby", "gallery-tab-opencode");
  renderGallery(galleryItems[0].id);
}

function renderTransactions(target, items) {
  document.querySelector(target).innerHTML = items.map(([title, detail]) => `
    <li><div><strong>${title}</strong><p>${detail}</p></div></li>
  `).join("");
}

function setupUndoSimulator() {
  renderTransactions("#openCodeFlow", openCodeFlow);
  renderTransactions("#akraFlow", akraFlow);
  document.querySelector("#undoRules").innerHTML = undoRules.map(([number, title, body]) => `
    <article class="undo-rule"><span>${number}</span><h3>${title}</h3><p>${body}</p></article>
  `).join("");

  const row = document.querySelector("#queueRow");
  const actions = document.querySelector("#rowActions");
  const pending = document.querySelector("#pendingLine");
  const dock = document.querySelector("#undoDock");
  const status = document.querySelector("#simStatus");
  const revision = document.querySelector("#simRevision");
  const pendingOperation = document.querySelector("#pendingOperation");
  const undoDockState = document.querySelector("#undoDockState");
  const restoreButton = document.querySelector("#restoreQueueItem");
  const modeButtons = [...document.querySelectorAll("#simulationMode button")];
  let mode = "success";
  let busy = false;
  let currentRevision = 42;
  let operationSequence = 0;

  const nextOperationId = (kind) => `op_${kind}_${String(++operationSequence).padStart(4, "0")}`;

  const closeActions = () => {
    actions.hidden = true;
    row.setAttribute("aria-expanded", "false");
  };

  row.addEventListener("click", () => {
    if (busy) return;
    const opening = actions.hidden;
    actions.hidden = !opening;
    row.setAttribute("aria-expanded", String(opening));
    status.textContent = opening
      ? "선택은 상태를 바꾸지 않습니다. 명시적 command action을 선택하세요."
      : "action menu를 닫았습니다.";
  });

  document.querySelector("#cancelQueueMenu").addEventListener("click", () => {
    closeActions();
    row.focus();
  });

  modeButtons.forEach((button) => {
    button.addEventListener("click", () => {
      mode = button.dataset.mode;
      modeButtons.forEach((item) => {
        const active = item === button;
        item.classList.toggle("active", active);
        item.setAttribute("aria-pressed", String(active));
      });
      status.textContent = `다음 authority 응답을 ${mode}로 설정했습니다.`;
    });
  });

  document.querySelector("#removeQueueItem").addEventListener("click", () => {
    if (busy) return;
    const baseRevision = currentRevision;
    const operationId = nextOperationId("qrm");
    const responseMode = mode;
    busy = true;
    modeButtons.forEach((button) => { button.disabled = true; });
    closeActions();
    pending.hidden = false;
    pendingOperation.textContent = `authority ack 대기 · ${operationId}`;
    row.disabled = true;
    status.textContent = `TUI projection은 아직 유지됩니다. authority가 revision ${baseRevision}를 검증 중입니다.`;
    window.setTimeout(() => {
      busy = false;
      modeButtons.forEach((button) => { button.disabled = false; });
      pending.hidden = true;
      row.disabled = false;
      if (responseMode === "success") {
        currentRevision += 1;
        revision.textContent = `revision ${currentRevision}`;
        row.hidden = true;
        dock.hidden = false;
        undoDockState.textContent = `revision ${currentRevision} · task-user-207`;
        status.textContent = `${operationId} ack correlation이 일치했습니다. 이제 projection에서 행을 제거했습니다.`;
      } else if (responseMode === "stale") {
        status.textContent = "Stale revision을 거부했습니다. 행과 composer는 그대로 유지됩니다.";
      } else {
        status.textContent = "Authority mutation이 실패했습니다. 행을 유지하고 재시도 가능한 오류를 표시합니다.";
      }
    }, 650);
  });

  restoreButton.addEventListener("click", () => {
    if (busy) return;
    const baseRevision = currentRevision;
    const operationId = nextOperationId("qrs");
    const responseMode = mode;
    busy = true;
    modeButtons.forEach((button) => { button.disabled = true; });
    pending.hidden = false;
    pendingOperation.textContent = `authority ack 대기 · ${operationId}`;
    dock.hidden = true;
    restoreButton.disabled = true;
    status.textContent = `RestoreQueueItem { base_revision: ${baseRevision}, operation_id: ${operationId} } ack 대기 중입니다.`;
    window.setTimeout(() => {
      busy = false;
      modeButtons.forEach((button) => { button.disabled = false; });
      pending.hidden = true;
      restoreButton.disabled = false;
      if (responseMode === "success") {
        currentRevision += 1;
        revision.textContent = `revision ${currentRevision}`;
        row.hidden = false;
        status.textContent = `${operationId} restore ack 뒤에만 행을 다시 표시했습니다.`;
        row.focus();
      } else {
        dock.hidden = false;
        status.textContent = responseMode === "stale"
          ? "Restore stale revision을 거부했습니다. 제거된 projection을 유지합니다."
          : "Restore가 실패했습니다. undo dock을 유지해 다시 시도할 수 있습니다.";
      }
    }, 650);
  });
}

let activeDebtProduct = "all";

function renderDebts() {
  const showReported = document.querySelector("#showReported").checked;
  const filtered = debts.filter((item) => {
    return (activeDebtProduct === "all" || item.id === activeDebtProduct)
      && (showReported || item.type !== "reported");
  });
  document.querySelector("#debtList").innerHTML = filtered.map((item) => `
    <article class="debt-row">
      <div class="debt-product"><strong>${item.product}</strong><small>${products.find((product) => product.id === item.id)?.version ?? ""}</small></div>
      <div class="debt-copy"><h3>${item.title}</h3><p>${item.body}</p><a href="${item.url}" target="_blank" rel="noreferrer">official source ↗</a></div>
      ${evidenceLabel(item.type)}
    </article>
  `).join("") || `<div class="empty-state">선택한 조건의 부채 항목이 없습니다.</div>`;
}

function setupDebts() {
  const filters = document.querySelector("#debtFilters");
  [{id:"all", name:"전체"}, ...products.map(({id, name}) => ({id, name}))].forEach((product, index) => {
    const button = document.createElement("button");
    button.type = "button";
    button.role = "tab";
    button.id = `debt-tab-${product.id}`;
    button.setAttribute("aria-controls", "debtList");
    button.setAttribute("aria-selected", String(index === 0));
    button.textContent = product.name;
    button.addEventListener("click", () => {
      activeDebtProduct = product.id;
      filters.querySelectorAll("button").forEach((item) => item.setAttribute("aria-selected", String(item === button)));
      document.querySelector("#debtList").setAttribute("aria-labelledby", button.id);
      renderDebts();
    });
    filters.append(button);
  });
  document.querySelector("#debtList").setAttribute("aria-labelledby", "debt-tab-all");
  document.querySelector("#showReported").addEventListener("change", renderDebts);
  renderDebts();
}

function renderPriorities() {
  document.querySelector("#priorityStack").innerHTML = priorities.map((item) => `
    <article class="priority-row">
      <span class="priority-badge">${item.badge}</span>
      <h3>${item.title}</h3>
      <p>${item.body}</p>
      <span class="proof-target">proof<br>${item.proof}</span>
    </article>
  `).join("");
}

function renderSources() {
  const query = document.querySelector("#sourceSearch").value.trim().toLowerCase();
  const filtered = sources.filter((source) => Object.values(source).join(" ").toLowerCase().includes(query));
  document.querySelector("#sourceCount").textContent = `${filtered.length} / ${sources.length} sources`;
  document.querySelector("#sourceList").innerHTML = filtered.map((source) => `
    <article class="source-row">
      <strong>${source.p}</strong>
      <span class="source-topic">${source.topic}</span>
      <a href="${source.url}" target="_blank" rel="noreferrer">${source.label} ↗</a>
      ${evidenceLabel(source.type)}
    </article>
  `).join("") || `<div class="empty-state">검색 조건에 맞는 근거가 없습니다.</div>`;
}

function setupSources() {
  document.querySelector("#sourceSearch").addEventListener("input", renderSources);
  renderSources();
}

function setupNavigation() {
  const nav = document.querySelector("#sectionNav");
  const toggle = document.querySelector("#navToggle");
  toggle.addEventListener("click", () => {
    const open = !nav.classList.contains("open");
    nav.classList.toggle("open", open);
    toggle.setAttribute("aria-expanded", String(open));
    toggle.setAttribute("aria-label", open ? "목차 닫기" : "목차 열기");
  });

  nav.querySelectorAll("a").forEach((link) => {
    link.addEventListener("click", () => {
      nav.classList.remove("open");
      toggle.setAttribute("aria-expanded", "false");
      toggle.setAttribute("aria-label", "목차 열기");
    });
  });

  const links = new Map([...nav.querySelectorAll("a")].map((link) => [link.dataset.section, link]));
  const observer = new IntersectionObserver((entries) => {
    const visible = entries
      .filter((entry) => entry.isIntersecting)
      .sort((a, b) => b.intersectionRatio - a.intersectionRatio)[0];
    if (!visible) return;
    links.forEach((link) => link.classList.remove("active"));
    links.get(visible.target.id)?.classList.add("active");
  }, {rootMargin: "-18% 0px -62% 0px", threshold: [0, 0.12, 0.4]});
  document.querySelectorAll("[data-observe]").forEach((section) => observer.observe(section));
}

function setupRovingTablist(selector) {
  const tablist = document.querySelector(selector);
  const tabs = [...tablist.querySelectorAll('[role="tab"]')];
  const syncTabStops = () => tabs.forEach((tab) => {
    tab.tabIndex = tab.getAttribute("aria-selected") === "true" ? 0 : -1;
  });
  tablist.addEventListener("click", syncTabStops);
  tablist.addEventListener("keydown", (event) => {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const current = Math.max(0, tabs.indexOf(document.activeElement));
    const next = event.key === "Home"
      ? 0
      : event.key === "End"
        ? tabs.length - 1
        : (current + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length;
    tabs[next].focus();
    tabs[next].click();
  });
  syncTabStops();
}

function init() {
  setupNavigation();
  setupArchitectureTabs();
  setupMatrix();
  renderPolish();
  setupGallery();
  setupUndoSimulator();
  setupDebts();
  renderPriorities();
  setupSources();
  setupRovingTablist("#architectureTabs");
  setupRovingTablist("#galleryControls");
  setupRovingTablist("#debtFilters");

  const restoreDeepLink = () => {
    if (!window.location.hash) return;
    const target = document.querySelector(window.location.hash);
    if (!target) return;
    const scrollBehavior = document.documentElement.style.scrollBehavior;
    document.documentElement.style.scrollBehavior = "auto";
    target.scrollIntoView();
    window.requestAnimationFrame(() => {
      document.documentElement.style.scrollBehavior = scrollBehavior;
    });
  };
  const initialGalleryImage = document.querySelector("#productGallery img");
  if (initialGalleryImage?.complete) {
    window.setTimeout(restoreDeepLink, 0);
  } else {
    initialGalleryImage?.addEventListener("load", restoreDeepLink, {once: true});
  }
  window.addEventListener("load", () => window.setTimeout(restoreDeepLink, 0), {once: true});
  document.fonts?.ready.then(() => window.setTimeout(restoreDeepLink, 0));
}

init();
