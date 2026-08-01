# Native TUI 검증

[English matrix](../../plan/12-platform-validation-matrix.md) ·
[Methodology](../../validation/terminal-ui-testing-methodology.md) ·
[Coverage](../../validation/tui-coverage-matrix.md)

현재 제품 frontend는 Ratatui/Crossterm alternate-screen fullscreen입니다. 검증 대상은 앱이 소유하는
transcript viewport, prompt 편집, overlay, resize, mouse capture, cursor, raw mode 및 terminal 복구입니다.

## 환경

- **E1**: Windows Terminal + WSL bash + fullscreen
- **E2**: Windows Terminal + PowerShell + fullscreen
- **E3**: tmux detached PTY + fullscreen
- **E4**: direct Linux terminal + fullscreen

Terminal mode, resize, cursor, wrapping, mouse, restore primitive를 변경하면 E1–E4를 모두 확인합니다.
순수 reducer/copy 변경은 Windows 계열 하나와 Linux 계열 하나로 줄일 수 있습니다.

## 자동 검증

1. Canonical transcript의 agent/tool/agent 순서와 늦은 completion 제자리 갱신
2. `TranscriptViewportUiState`의 PageUp/PageDown, follow-tail, unseen revision, identity reset
3. Ratatui `TestBackend`의 composer 보존, read/explore card, semantic diff
4. Stable resize-gated `FullscreenFrameRenderReceipt` compare-and-apply
5. Alternate-screen, focus, mouse, bracketed-paste, cursor 복구 escape sequence
6. Architecture guard의 renderer purity와 retired host delivery 재도입 차단

```bash
cargo fmt --all -- --check
cargo test --lib -- --test-threads=1
cargo test --test architecture_boundaries -- --test-threads=1
cargo clippy --all-targets --all-features -- -D warnings
bash scripts/check_native_pr.sh
```

## 수동 캡처

Primitive-sensitive 변경에서만 수동 캡처가 필수입니다.

1. 80×24 이하에서 fullscreen transcript와 composer를 함께 확인합니다.
2. Streaming 중 PageUp 후 보이는 row가 움직이지 않고 `new output`이 나타나는지 확인합니다.
3. Ctrl+End로 최신 row를 다시 따라가는지 확인합니다.
4. Read/explore card를 접은 상태와 click/Ctrl+E로 펼친 상태를 확인합니다.
5. Patch card의 green/red semantic diff를 확인합니다.
6. 종료 후 원래 shell 화면, cursor, input mode가 복구되는지 확인합니다.

기록 예시:

```bash
bash scripts/capture_native_validation.sh \
  --frontend fullscreen \
  --check-profile terminal-baseline \
  --terminal "Windows Terminal" \
  --result pass \
  --output-dir docs/validation
```

기록은 commit SHA, OS, terminal/version, shell, frontend, `TERM`, capture role, profile, checks,
result, notes를 포함합니다.
