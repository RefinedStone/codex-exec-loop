# Akra TUI UX Priority Audit

2026-07-31 현재 Akra TUI를 실제 Windows ConPTY에서 실행해 캡처하고, 최신 공식 자료를
기준으로 Codex CLI, Grok Build, OpenCode, Claude Code, Gemini CLI, Pi와 비교한 실행 우선순위
보고서다.

- [HTML report](index.html)
- 재현 화면: ready shell, inline command palette, diagnostics, shell commands help
- 기존 증거 재사용: real tmux PTY responsive composer, deterministic Parallel Operations board
- 결과: P0 4개, P1 5개, P2 3개의 reviewable backlog

## 핵심 결론

Akra의 inline main-buffer, host scrollback, typed projection, responsive composer 계약은 유지한다.
우선 구현할 세 가지는 semantic operator ribbon, contextual command palette, unified Work Center다.
경쟁 제품의 fullscreen chrome이나 자유로운 theme semantics를 그대로 이식하지 않는다.

## 캡처 범위

현재 캡처는 source-built `codex-exec-loop-native`를 Windows ConPTY `120x30`에서 실행하고 실제
Codex app-server에 연결했다. workspace와 user path는 익명화했다. PNG는 텍스트와 geometry를
비교하기 위한 normalized capture이며 semantic color의 정확한 권위는 repository snapshots와
기존 physical-terminal evidence다.

WSL은 이 조사 시점에 배포판 계정/mount 오류로 실행되지 않아 E1/E3 전체를 새로 캡처하지
못했다. 따라서 기존 real tmux PTY evidence를 명확히 구분해 사용했고, 이번 보고서는 구현 PR의
E1-E4 terminal approval을 대체하지 않는다.

## 공식 자료

- [OpenAI Codex developer commands](https://developers.openai.com/codex/cli/slash-commands)
- [SpaceXAI Grok Build](https://x.ai/cli), [Workflows](https://x.ai/news/workflows),
  [official source](https://github.com/xai-org/grok-build)
- [OpenCode TUI](https://opencode.ai/docs/tui/),
  [commands](https://opencode.ai/docs/commands/)
- [Claude Code interactive mode](https://code.claude.com/docs/en/interactive-mode),
  [statusline](https://code.claude.com/docs/en/statusline)
- [Gemini CLI commands](https://google-gemini.github.io/gemini-cli/docs/cli/commands.html)
- [Pi coding agent](https://github.com/badlogic/pi-mono/tree/main/packages/coding-agent)

공식 이미지의 pinned source와 SHA-256은
[`../tui-architecture/assets/README.md`](../tui-architecture/assets/README.md)에 있다.
