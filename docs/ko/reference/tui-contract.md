# TUI 계층 아키텍처 및 시각 계약

[English](../../design/07-tui-layered-architecture-and-aesthetic-contract.md)

Native shell TUI는 작은 context에서도 안전하게 수정할 수 있어야 합니다. 문구 변경이 terminal
adapter 이해를 요구하거나, layout 변경이 제품 문구를 새로 만들거나, 색상 변경이 feature module에
Ratatui primitive를 흩뜨리지 않게 합니다.

## 계층

| 계층 | 소유 | 소유하면 안 되는 것 |
| --- | --- | --- |
| State/reducer | intent, mode transition, selection, editing state | widget, 시각 hierarchy, raw style |
| Controller/effect | service call, command dispatch, runtime side effect | status 문구, title, geometry |
| Projection/copy | view model, `Line`, label, status/key 문구 | `Frame`, `Layout`, terminal side effect, raw color |
| Theme/chrome | semantic style, brand token, panel frame, selection marker | feature state, controller behavior, 화면별 문구 |
| Rendering/layout | `Rect`, `Layout`, widget 배치 | 새로운 keybinding 주장, product copy, raw color/border policy |
| Terminal adapter | lifecycle, scrollback, viewport replay, host terminal effect | planning 의미, Akra 문구, overlay policy |
| Test/capture | rendering contract, snapshot delta, terminal evidence | 검토하지 않은 시각 계약 변경 |

문구는 projection/copy, action 가능 여부는 state/controller, overlay section은 view model 후
rendering, style은 `theme.rs`, geometry는 rendering/layout, scrollback/resize는 terminal adapter에서
시작합니다.

## Theme과 기반 규칙

- 기본 theme은 고정된 `Akra`이며 runtime theme switching은 현재 계약이 아닙니다.
- `AkraTheme`이 brand, accent, success, warning, danger, muted, shortcut, selected, panel, title,
  key, list marker의 semantic source입니다.
- Raw `Color::*`, `.bg(...)`, `Block::default().borders(...)`, list highlight symbol을 feature
  module에 두지 않습니다. 필요한 semantic helper를 `theme.rs`에 추가합니다.
- Shell은 terminal 전체 배경색에 의존하지 않고 읽을 수 있어야 합니다.

## Component 규칙

### Inline shell tail

- Border 없이 compact하게 유지합니다.
- Status ribbon, planning/queue summary, runtime notice, prompt, command hint 순서의 안정된
  hierarchy를 유지합니다.
- Terminal sync transaction 하나는 conversation shell의 core `AppSnapshot`과 render clock을
  `ConversationProjectionSample`로 한 번만 캡처합니다. Pre-history outer flow layout과 최종
  tail/live/cache projection은 같은 sample을 사용합니다. Handoff acknowledgement 뒤의
  UI-local state는 다시 읽을 수 있습니다.
- Supersession은 이 일관성 보장에 포함됩니다. Sample이 control-plane presentation projection과
  event-stream projection을 각각 한 번 소유하고, row plan과 draw는 같은 owned overlay view를
  사용합니다. 다른 overlay document는 owned screen model을 받을 때까지 별도 projection
  경계로 유지합니다. 자주 실행되는 prompt, pulse, scheduler 검사는 panel 전용 경량 sample을
  공유하며 transcript나 event-stream row를 복제하지 않습니다.
- Sample에서 만든 `ConversationScreenModel` 하나가 frame의 UI-local fact를 소유합니다. Tail,
  live transcript, cursor layout, frame cache는 같은 immutable projection을 사용하며 presentation
  helper가 `NativeTuiApp`, service, clock을 다시 읽지 않습니다.
- 장시간 운영에 필요한 밀도를 우선하고 marketing copy를 넣지 않습니다.
- 한국어와 wide-character prompt가 주변 layout 계약을 깨지 않아야 합니다.
- GitHub review setup은 draw와 draw 이후 size 검증이 모두 성공할 때까지
  `PendingFirstFrame`이어야 합니다. 실패하거나 resize race가 발생한 draw는 setup을 보내지 않고,
  첫 안정된 frame 전달만 `AppCommand::SetupGithubReviewPolling`을 정확히 한 번 보냅니다. Core는
  setup generation/workspace 승인과 poll cursor/single-flight를, composition은 Git, credential,
  discovery, service 구성과 exact-correlation registry를 소유합니다. Poll tick은
  `AppCommand::PollGithubReview`로 보내며 TUI에는 환경 파싱, poll timing, setup/poll 상태 표현만
  남습니다.

### 대화 Markdown과 diff 상세

- App-server의 agent text는 raw Markdown입니다. Transcript projection이 live delta, 완료 history,
  viewport replay에서 같은 규칙으로 표시 문법을 해석합니다.
- Fenced code의 delimiter와 info string은 transcript 내용이 아닌 parser 문법입니다. 서로 맞는
  opening/closing fence는 숨기고, 아직 닫히지 않은 streaming code의 본문은 유지하며,
  `AkraTheme`을 통해 code body만 스타일링합니다.
- Activity의 Diff 문서는 unified diff의 file header와 hunk range를 해석합니다. 각 code row는
  오른쪽 정렬된 line-number gutter 하나를 사용하며, 삭제는 old line, 추가와 context는 new line
  번호를 표시합니다.
- 추가, 삭제, gutter, metadata, hunk separator는 semantic theme style을 사용합니다. Wrap된
  continuation row는 source line 번호를 반복하지 않고 빈 gutter와 sign column을 유지합니다.
- Diff pagination은 old/new counter와 wrapped-line continuation을 포함한 semantic parser cursor를
  소유합니다. 이전/다음 page는 retained document를 다시 scan하거나 rendered text에서 번호를
  추측하지 않고 그 cursor를 복원합니다.
- Malformed, binary, combined, retention-truncated diff fragment는 muted metadata로 안전하게
  표시하며 control character가 실행 가능한 terminal escape로 전달되지 않게 합니다.

### Append-only stream

- Host scrollback과 live viewport로 row가 나뉠 수 있는 stream은 더 이상 titled panel이 아닙니다.
- Durable row와 live row 사이에 panel title을 삽입하지 않습니다. 전체 stream이 들어갈 때만 일반
  section title을 표시할 수 있습니다.
- Rendering surface를 `InlineTitledPanel`, `InlineScrolledPanel`, `InlineAppendOnlyStream` 중
  명시적으로 고릅니다.
- Parallel event stream은 generic titled helper가 아니라 named stream renderer와
  `InlineAppendOnlyStream`으로 진입합니다.
- Row retention, scroll offset, title visibility, live-tail chrome 변경은 정확한 redraw 전후를
  기록하는 frame-recorder regression을 추가합니다.

### Popup/inspection overlay

- `AkraTheme::panel_block`을 사용합니다.
- 필요한 section은 header, summary, primary content, status, keys 순서로 둡니다.
- 선택 row는 semantic selected style과 marker를 함께 사용해 색상만으로 표현하지 않습니다.
- Key footer는 `AkraTheme::key_line`을 사용하고 현재 state가 실제 처리하는 shortcut만 표시합니다.

### Title, warning, accessibility

- Title surface는 보통 `AkraTheme::title_line`을 사용합니다.
- Startup masthead는 conversation input을 가리지 않을 정도로 제한합니다.
- Warning/error는 운영 영향과 복구 방법을 명시하고 semantic warning/danger style을 사용합니다.
- 모든 interactive surface는 기존 keyboard path로 사용할 수 있어야 합니다.
- 긴 줄은 기존 wrap/clip 계약을 따르고 layout을 예측 불가능하게 확장하지 않습니다.
- Dense layout 변경은 narrow terminal과 한국어/wide-character를 함께 검증합니다.

## 문구와 anti-pattern

TUI 문구는 운영 도구처럼 직접적이고 상태 중심이어야 합니다. Raw 내부 ID, protocol 이름,
debug-only 세부 정보는 전용 inspection이 아닌 일반 상태에 노출하지 않습니다. 구현되지 않은 명령,
theme toggle, shortcut을 설명하지 않습니다.

피해야 할 변경:

- feature module에서 직접 색상·border·selection marker를 선택
- controller에서 operator-facing 문구 조립
- projection builder에서 `Frame`/`Layout` 사용
- rendering function에서 새로운 제품 문구 또는 keybinding 생성
- snapshot만 갱신하고 reducer/projection/frame sequence assertion을 생략
- interaction 요구 없이 한 overlay만 별도 시각 체계로 분기

## 수정 및 QA 순서

1. 요청에 맞는 가장 작은 계층 파일을 엽니다.
2. 문구 작업이면 rendering보다 projection부터 수정합니다.
3. 모든 시각 style과 marker는 `AkraTheme`을 사용합니다.
4. 새 module은 [개발 가이드](development.md)의 context budget을 따릅니다.
5. 시각/presentation 변경은 `bash scripts/check_tui_layering.sh`를 실행합니다.
6. 화면 계약을 증명하는 가장 작은 focused test 또는 snapshot을 추가합니다.

QA에서는 책임 계층, raw primitive 누출, 표시 shortcut의 실제 입력 경로, selection marker/style,
한국어와 narrow layout, 의도적인 snapshot 변경을 확인합니다. Terminal primitive 변경은
[검증 가이드](validation.md)를 따릅니다.
