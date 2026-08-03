# TUI Fullscreen 계층 아키텍처 및 시각 계약

[English](../../design/07-tui-layered-architecture-and-aesthetic-contract.md)

## 제품 의도

Akra native shell은 alternate-screen fullscreen 애플리케이션입니다. Transcript가 작업 공간이고
composer는 항상 보이며, tool 상세는 평소 한 줄로 조용히 보이다가 필요할 때 대화 안에서
펼쳐집니다. 이전 host scrollback 조합 경로는 제품 경로가 아닙니다.

## 소유권 흐름

```text
app-server event
  → conversation/core reducer
  → ConversationViewModel.messages
  → FullscreenConversationFrameProjection
  → FullscreenShellFrameModel + FullscreenInspectionFrameModel
  → pure Ratatui draw
  → FullscreenFrameRenderReceipt
  → stable compare-and-apply
```

- `ConversationViewModel.messages`가 유일한 순서 권한입니다.
- Assistant streaming은 `item_id`가 같은 row를 제자리에서 갱신합니다.
- Tool row는 도착한 시점에 같은 vector에 추가됩니다.
- 늦은 completion은 기존 assistant row만 갱신하며 tool을 앞뒤로 이동시키지 않습니다.
- `NativeTuiApp`은 shell, conversation, planning, runtime 네 private typed slice만 소유합니다.

## Transcript viewport

`TranscriptViewportUiState`가 absolute top row, page height, max scroll, follow-tail, 최신/확인 revision,
document identity, clickable card geometry를 소유합니다.

- PageUp·wheel up: 현재 읽는 위치를 고정합니다.
- Streaming append: 위치를 움직이지 않고 `new output`을 표시합니다.
- PageDown: tail에 닿으면 follow-tail을 다시 켭니다.
- Ctrl+Home / Ctrl+End: 처음 / 최신으로 이동합니다.
- Session이나 thread identity가 바뀌면 offset과 stale hit area를 함께 초기화합니다.

Terminal emulator의 scrollback은 대화 저장소가 아닙니다. Production에는 host history insertion,
transcript handoff ACK, viewport replay, newline fallback 분기가 없습니다.

## Tool/read/diff card

- Read/explore는 접힌 상태에서 의미 있는 label 하나만 표시합니다.
- Click 또는 Ctrl+E로 같은 digest의 card를 펼칩니다.
- 펼친 read/explore는 path, target, line range, 보존된 detail을 표시합니다.
- Patch는 접힌 상태에서 file summary를 보여 주고, 펼치면 hunk와 context를 표시합니다.
- 삭제 row는 red, 추가 row는 green 음영을 사용하며 gutter는 중립으로 유지합니다.
- `:activity`는 교차 turn inspector이며 tool 상세를 볼 수 있는 유일한 화면이 아닙니다.

## Frame 계약

- Terminal transaction은 draw 전에 owned `FullscreenShellFrameModel` 하나를 완성합니다.
- 활성 inspection은 `FullscreenInspectionFrameModel` variant가 소유합니다.
- Renderer는 `NativeTuiApp`, Core/application service, filesystem, network, clock, terminal I/O를
  다시 읽지 않고 Ratatui `Frame`만 변경합니다.
- Draw 뒤 terminal size와 resize epoch가 동일할 때만 `FullscreenFrameRenderReceipt`를 적용합니다.
- Receipt는 transcript viewport, list state, scroll, hit area 같은 UI feedback을 exact
  compare-and-apply합니다.

## Parallel stream

Parallel event는 `ParallelLiveStreamModel` 하나로 projection합니다. Geometry를 draw 전에 확정하고
`FullscreenAppendOnlyStream`으로 렌더링합니다. 짧으면 title을 보이고, 높이가 부족하면 title만
숨깁니다. Event row는 언제나 같은 app-owned viewport 안에서 순서를 유지합니다.

## 시각 문법

- Cyan/teal: product identity와 활성 affordance
- White/default: 주요 본문
- Muted gray: metadata와 비활성 hint
- Amber: 대기·주의·degraded
- Red: 실패와 삭제 diff
- Green: 성공과 추가 diff
- Magenta: 제한적인 identity accent

일반 대화 본문은 borderless 문서처럼 읽혀야 합니다. Border는 overlay, card, composer처럼 grouping이
실제로 도움이 되는 곳에만 사용합니다.

하단 status row는 낮은 명도의 surface를 사용하고, focused composer는 그보다 조금 밝은 surface와
완전한 둥근 frame으로 입력 소유권을 구분합니다. 이 frame은 기존 composer 높이 예산 안에서 open
rail을 대체하므로 resize 때 transcript를 밀거나 별도 dashboard panel을 만들지 않습니다.

## 반응형 계약

- 80 columns: 한 column, metadata 압축, composer 보존
- 120 columns: 기본 density와 full tool label
- 160 columns: 여백 확대, 불필요한 authority panel 추가 금지
- 짧은 높이: body가 먼저 줄고 composer와 필수 action row는 남음
- Resize race: 이전 receipt 폐기 후 새 wrapping과 hit area 계산

## 검증

1. 긴 transcript에서 composer가 보이는지 확인합니다.
2. Streaming 중 PageUp 위치와 `new output` badge를 확인합니다.
3. Ctrl+End가 tail follow를 복원하는지 확인합니다.
4. Read/explore와 patch card를 click 및 Ctrl+E로 각각 펼칩니다.
5. Semantic diff의 green/red 음영을 확인합니다.
6. 종료 후 alternate screen, mouse/focus/paste, cursor, raw mode가 복구되는지 확인합니다.

상세 방법은 `docs/validation/terminal-ui-testing-methodology.md`와
`docs/validation/tui-coverage-matrix.md`를 따릅니다.
