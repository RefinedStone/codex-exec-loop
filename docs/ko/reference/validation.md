# 터미널 및 TUI 검증 가이드

영문 원본:

- [Platform Validation Matrix](../../plan/12-platform-validation-matrix.md)
- [Terminal UI Testing Methodology](../../validation/terminal-ui-testing-methodology.md)
- [TUI Coverage Matrix](../../validation/tui-coverage-matrix.md)
- [Validation Records](../../validation/README.md)

이 문서는 저장소 테스트가 읽는 반복 marker와 test-entrypoint inventory를 한국어로 다시 복제하지
않고, 실제 적용 방법을 통합 번역합니다. 영문 coverage matrix의 path inventory가 자동 검증의
canonical source입니다.

## 언제 적용하는가

Raw mode, terminal restore, inline history, host scrollback, viewport, resize, cursor, prompt editing,
overlay, live tail을 바꾸면 이 계약을 적용합니다. 기능 완성도 검증과 terminal 동작 검증은 구분합니다.

현재 제품 frontend는 inline main-buffer입니다. Counted `terminal-baseline` row만 기본 terminal
release gate에 포함됩니다. Alternate-screen 또는 branch-family evidence는 supplemental이며 필수 row를
대체하지 않습니다.

## 필수 baseline

| OS | Terminal | Shell | 우선순위 |
| --- | --- | --- | --- |
| macOS | Terminal.app | zsh | 필수 |
| macOS | iTerm2 | zsh | 필수 |
| Windows | Windows Terminal | PowerShell | 필수 |
| Windows | Windows Terminal | WSL bash | 필수 |
| Windows | Git Bash/동급 | bash | 선택 |
| Windows | JetBrains IDE terminal | WSL bash | 선택 |

각 필수 row에서 launch/exit/restore, 입력 편집과 cursor, inspection open/close, streaming과 buffered
input, resize와 scrollback, 영향받은 failure recovery를 확인합니다.

`phase1-operator-surface` profile은 status 문구, resume context, queue, automation,
planning/directions, `akra`/`:` lifecycle parity를 바꿀 때 사용합니다. Operator vocabulary와 next
action, resume 직후 planning/queue context, external/in-shell 상태 일치, raw ID 미노출을 추가로 봅니다.

`prompt-input-delay-pty` profile은 prompt echo, input buffering, PTY bridge, tmux/Zellij, integrated
terminal을 바꿀 때 사용합니다. Linux direct/tmux/Zellij, Windows Terminal PowerShell/WSL bash가 필수
row이며 startup-pending echo, cursor/editing, submit-to-stream, history restore, interrupt/exit를 확인합니다.

## 현재 stack과 environment class

Ratatui/Crossterm stack을 기본으로 유지합니다. 구조 추출은 별도 Decision Record가 trigger evidence를
증명할 때만 진행합니다.

- **E1**: Windows Terminal + WSL bash + inline
- **E2**: Windows Terminal + PowerShell + inline
- **E3**: tmux detached PTY + inline
- **E4**: direct Linux terminal + inline

Branch family:

- **B1**: `HostScrollback`
- **B2**: `ViewportReplay`
- **B3**: `StandardScrollRegion`
- **B4**: `NewlineFallback`

기본 `InlineHistoryRenderMode`는 `HostScrollback`이고 `ViewportReplay`는 명시적 override입니다.
기본 `HistoryInsertionMode`는 일반적으로 `StandardScrollRegion`, `WT_SESSION`에서는
`NewlineFallback`입니다. Override와 downgrade는 evidence에 정확히 기록합니다.

## 자동 테스트 계층

버그를 드러내는 가장 낮은 계층을 선택하되 redraw 순서에 의존하면 temporal evidence를 우선합니다.

1. `InlineFrameRecorder`: 매 draw의 screen, host scrollback, terminal history, app-side stream을 비교
2. Ratatui `TestBackend`: deterministic buffer와 screen 검사
3. `insta` snapshot: focused assertion 이후 안정된 전체 frame 고정
4. vt100 parser: ANSI, cursor, clear, wrap, scrollback primitive 검사

Projection test는 문구·순서·truncation·shortcut을, reducer/runtime test는 committed/live state와 reset,
terminal primitive test는 insertion/clear/wide character/cursor를, frame transaction test는 redraw
sequence와 duplicate/stale row를, scheduler test는 event coalescing과 draw request를 검증합니다.

Snapshot만으로 temporal bug를 증명하지 않습니다. Snapshot 근처에 그 bug class를 이름으로 설명하는
targeted assertion을 둡니다.

## 핵심 회귀 matrix

| 영역 | 최소 자동 증명 |
| --- | --- |
| Host scrollback | pending suffix insert, shifted insert, duplicate 없음 |
| Viewport replay | explicit-only, host insert 없음, 최근 transcript와 inline positioning 유지 |
| Resize | shrink/restore 후 stale row·duplicate tail 없음 |
| Clear/reset | pending history 제거, viewport reset, 새 header redraw |
| Session switch | 이전 transcript/deferred history 누출 없음 |
| Streaming | live delta 유지, final은 committed history로 이동 |
| Overlay | open/close 시 stale tail 제거와 정상 redraw |
| Parallel stream | 초기 status row 유지, host scrollback에 panel chrome 없음, titleless live tail |
| Fallback | standard/fallback insertion 모두 viewport state 유지 |

Host scrollback과 live viewport로 나뉘는 stream 사이에 panel title을 넣지 않습니다.
`InlineTitledPanel`, `InlineScrolledPanel`, `InlineAppendOnlyStream` typed surface를 사용하고 parallel
stream은 dedicated renderer로 진입합니다.

## Manual capture

Manual capture는 scrollback insertion, viewport mode, clear/restore, resize-dependent redraw, cursor
restore, escape sequence처럼 primitive-sensitive한 변경에만 필수입니다.

Artifact에는 baseline field(date, commit, OS, terminal/version, shell, frontend, `TERM`, capture role,
profile, checks, result, notes)와 필요한 경우 environment class, E/B family, effective render/insertion
mode, override 원인, scenario별 결과를 추가합니다.

최소 scenario:

1. committed history가 live viewport 위에 삽입됨
2. redraw 후 duplicate replay 없음
3. shrink/restore 후 stale row·duplicate tail 없음
4. clear/reset 후 깨끗한 header와 viewport
5. thread/session switch 후 history 누출 없음
6. 관련 시 parallel split stream 사이에 panel chrome 없음
7. 관련 시 fallback insertion의 viewport/cursor 복구

공통 primitive 또는 defaulting logic이 E1-E4에 영향을 주면 네 환경을 모두 캡처합니다. 한 branch
family에만 제한된 변경은 영향받은 first-class 환경과 대비되는 대표 경로 하나로 줄일 수 있지만,
reviewer가 unchanged automated proof와 primitive path를 확인해야 합니다. Supplemental evidence는
counted baseline 결손을 면제하지 않습니다.

## 명령

```bash
bash scripts/capture_native_validation.sh \
  --frontend inline \
  --check-profile terminal-baseline \
  --terminal "iTerm2 3.5" \
  --result pass \
  --output-dir docs/validation

bash scripts/summarize_native_validation.sh
bash scripts/summarize_native_validation.sh --fail-on-incomplete
bash scripts/check_tui_layering.sh
bash scripts/check_native_pr.sh
```

PowerShell은 `scripts/capture_native_validation.ps1`을 사용합니다. Plain summary는 현황 표시이고
`--fail-on-incomplete`만 명시적 gate입니다. 실제 기록은 `docs/validation/`에 둡니다.
