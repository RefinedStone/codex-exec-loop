# Akra TUI UX Competitive Refresh

[Open the interactive HTML report](index.html).

This report refreshes the 2026-07-31 priority audit against Akra commit
<code>a36aeb4d9f9ede56fef8d6eedd1e3da3432ee044</code>. It verifies that the previous top three
slices shipped, recaptures the current Akra TUI at 120×30 and 80×24, reproduces a startup-blocked
state, and compares locally installed Codex and Grok Build command surfaces without submitting a
prompt.

## Decision

The next three reviewable slices are:

1. operator-grade startup failure cards with <code>Cause -&gt; Impact -&gt; Fix -&gt; Verify</code>
2. truthful <code>OFF</code> and empty-state projections across Work Center and Activity
3. a bounded post-turn return recap built from existing turn, approval, and delivery authority

Akra should retain its inline main-buffer and host-scrollback contract. Codex's quiet hierarchy and
bounded recent-terminal summary are useful patterns. Grok Build's workflow-first phase, agent, and
progress hierarchy is useful, but its full-screen viewport ownership is not compatible with Akra's
chosen terminal contract.

## Reproduced Evidence

- Akra: real Windows ConPTY, real Codex app-server, 120×30 and 80×24
- blocked startup: deliberately unsafe Codex executable path, captured through the real startup gate
- Codex: <code>codex-cli 0.146.0-alpha.9.2</code>, Windows ConPTY 120×30
- Grok Build: <code>grok 0.2.114 (0c78503879) [stable]</code>, Windows ConPTY 120×30
- no task prompt was submitted in any capture
- workspace paths in rendered evidence were replaced with <code>&lt;workspace&gt;</code>

Capture metadata and raw UTF-8 terminal text are in [assets](assets/). Screenshots demonstrate
presentation only; they do not establish model quality, performance, durability, or release support.

## Source-Level Findings

- <code>work_center.rs</code> calculates <code>AGENTS OFF</code> and
  <code>DELIVERY OFF</code>, but their summary/detail can still reuse readiness-waiting projection
  text
- the task detail always formats a thread field, even when the draft has no thread id
- <code>activity.rs</code> emits the folded-selected-row instruction when both the document and card
  list are empty
- failed startup state is repeated as summary, warning, and check content; the renderer preserves
  raw payload but does not yet provide a recovery-oriented semantic hierarchy

## Official Product Sources

- [OpenAI Codex slash commands](https://developers.openai.com/codex/cli/slash-commands)
- [OpenAI Codex repository](https://github.com/openai/codex)
- [Grok Build overview](https://docs.x.ai/build/overview)
- [Grok Build modes and commands](https://docs.x.ai/build/modes-and-commands)
- [Grok Build keyboard shortcuts](https://docs.x.ai/build/keyboard-shortcuts)
- [Grok Workflows](https://x.ai/news/workflows)
- [Grok Build repository](https://github.com/xai-org/grok-build)
