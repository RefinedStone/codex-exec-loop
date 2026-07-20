# Grok Build TUI Rendering Architecture Deep Dive

이 문서는 Akra에서 관측된 다음 두 회귀를 기준으로 Grok Build의 현재 공개 소스를
재분석한 집중 보고서다.

1. 완료 응답의 위쪽 행이 사라지고 호스트 터미널에서 다시 스크롤할 수 없었다.
2. `settling planning queue` 동안 완료 응답이 좁은 live viewport에 갇혔다가 settlement
   종료 후에만 정상적으로 보였다.

기존 [Grok Build 종합 분석](./analysis.md)을 대체하지 않는다. 2026-07-21 현재 공개
HEAD를 다시 복제하여 TUI의 stream reduction, scrollback commit, viewport geometry,
queue authority, render scheduling 경로만 심층 비교한 후속 분석이다.

## Snapshot

| Field | Value |
| --- | --- |
| Grok Build source | `https://github.com/xai-org/grok-build` |
| Grok Build commit | `ba76b0a683fa52e4e60685017b85905451be17bc` (`Synced from monorepo`) |
| Grok Build `SOURCE_REV` | `ba69d70c2f7d70a130a323b2becdf137af784c7f` |
| Akra baseline | `48b59e2779f5f2a75c5fb307619212a3d3388e80` (`prerelease`) |
| Audit date | 2026-07-21 (Asia/Seoul) |
| Method | Fresh shallow clone, whole-repository static graph, direct source tracing, targeted Rust tests |
| Scope | First-party source; `third_party/` excluded |

전체 스캔은 first-party 파일 2,692개를 code 2,233, config 250, documentation 199,
script 9, data 1로 분류했다. Cargo workspace에는 package 79개가 있다. 이 규모는 Grok
Build 역시 작거나 본질적으로 단순한 reference implementation이 아님을 보여 준다.
병합 graph는 29,883 nodes / 47,593 edges이며 import 관계 3,830개가 deterministic import
map과 일치한다. 단 한 건의 type-aware 보정은 `postinstall.js`가 존재하지 않는 `file:`
node가 아니라 `package.json`의 실제 `config:` node를 가리키도록 복원한 것이다. 최종
검증은 dangling reference, duplicate ID, layer 누락이 0건이었고, 연결이 없는 독립
symbol/document node 284개만 비차단 경고로 남겼다.

## Executive Verdict

두 Akra 증상의 직접 원인은 모델 출력 품질이나 모델 지능 저하가 아니다. 이미 생성된
완료 응답을 어느 저장소가 언제 소유하고, 어떤 터미널 primitive로 durable history에
commit하는지가 잘못된 것이다.

| Symptom | Direct cause | Structural cause |
| --- | --- | --- |
| 위쪽 행 소실, 스크롤 불가 | 기본 scroll-region 삽입이 일부 터미널에서 native scrollback을 만들지 않음 | Ratatui viewport 이동과 host scrollback durability를 같은 것으로 간주한 capability 가정 |
| settlement 동안 좁은 응답 | 완료 응답의 viewport handoff release를 planning settlement 완료까지 지연 | transcript delivery lifecycle과 post-turn planning lifecycle의 결합 |
| planning editor 결과가 stream 중 소실 | 모든 conversation runtime event가 planning UI revision을 무조건 supersede | 서로 다른 event domain을 하나의 coarse revision으로 무효화 |

Grok Build도 같은 종류의 viewport 버그를 겪었다. 현재 소스 주석에는 tall streaming
viewport 때문에 prompt가 화면 위에 고립된 `"input snaps to the top"` 버그와, 저장된
높이를 실제 높이로 오인해 dropdown이 화면 밖에 그려진 `"empty dropdown over a full
screen"` 버그가 명시되어 있다. 차이는 “Grok에는 버그가 없다”가 아니다. Grok의 현재
critical path에는 다음 불변식이 코드 구조로 더 직접 표현되어 있다는 점이다.

- finalized block은 한 번만 native scrollback에 commit한다.
- commit 여부와 post-commit viewport 높이를 같은 projection으로 계산한다.
- viewport를 commit 전에 post-commit 높이로 맞춘 뒤 commit하고 live region을 그린다.
- grow/shrink 판단은 설정값이 아니라 실제 `viewport_area.height`를 기준으로 한다.
- full TUI는 scrollback 최소 5행, queue 최대 3행의 명시적 layout budget을 가진다.
- writer acknowledgement 전에는 다음 frame을 in-flight로 만들지 않는다.

Akra가 Grok 전체 구조를 복제할 이유는 없다. 필요한 것은 위 불변식을 Akra의
Codex-first inline shell에 맞게 좁게 도입하는 것이다.

## Akra Failure Reconstruction

### 1. Host scrollback이 실제로 생성되지 않은 경로

Akra inline shell에는 서로 다른 두 물리 표면이 있다.

```text
ConversationViewModel transcript
  -> projected committed lines
  -> HistoryFlushState pending suffix
  -> HistoryInsertionAdapter
  -> host terminal native scrollback

ConversationViewModel live handoff
  -> InlineConversationFrameProjection
  -> Ratatui Viewport::Inline(16)
  -> active screen
```

`HistoryFlushState`는 이미 전송한 transcript snapshot과 새 suffix, wrapped row 수,
visible row accounting을 보관한다. 그러나 2026-07-20 이전 기본
`HistoryInsertionMode`는 `StandardScrollRegion`이었다. Scroll region 안에서 화면
행을 이동하는 동작은 terminal emulator에 따라 화면만 이동시키고 native history에는
행을 추가하지 않을 수 있다.

그 결과 앱 관점에서는 history insertion이 성공하고 live viewport에서 이전 행을
제거했지만, 사용자가 의존하는 host scrollback에는 그 행이 없었다. 사용자가 본 현상은
“위쪽 렌더링이 날아갔다”였지만 더 정확한 장애는 다음과 같다.

```text
app transcript:       존재
Ratatui active frame: 제거됨
native scrollback:    없음
user recovery path:   없음
```

`22f755db fix(tui): preserve host scrollback`은 기본을 `Automatic`으로 바꾸고, 일반
conversation에서는 `NewlineFallback`, parallel renderer에서는
`StandardScrollRegion`으로 resolve한다. 또한 VT100에서 180행 응답의 first/middle/last
marker가 모두 scrollback에 남는 회귀 테스트를 추가했다.

이 수정은 직접 증상을 해결하지만 `StandardScrollRegion`을 쓰는 parallel branch는
계속 terminal primitive 증거가 필요한 잔여 위험이다.

분석 시점에는 `history_insertion.rs`의 머리말과 canonical terminal validation 표가
여전히 과거의 “일반 기본은 scroll region, Windows Terminal만 newline fallback” 정책을
설명하고 있었다. 이 보고서 변경과 함께 두 설명을 현재 `Automatic` resolution에 맞춰
수정했다. Runtime fix와 검증 문서가 어긋나면 다음 변경이 과거 capability 가정을
복원할 수 있기 때문이다.

### 2. 완료 응답과 planning settlement가 결합된 경로

Akra live viewport는 두 render mode 모두 `Viewport::Inline(16)`으로 고정된다. 완료된
agent message는 viewport handoff가 release되기 전까지 live projection에 남는다.

수정 전 상태 전이는 다음과 같았다.

```text
Streaming
  -> Turn completed
  -> begin_post_turn_settlement()
       completed answer remains in live handoff
  -> "settling planning queue"
       answer + status + prompt compete inside 16 rows
  -> complete_post_turn_settlement()
       begin_viewport_transcript_handoff_release()
  -> host scrollback flush
```

따라서 80행 완료 응답도 settlement 동안 16행 viewport에 갇혔다. status와 prompt가
같은 영역을 사용하므로 사용자는 stream 영역 자체가 갑자기 매우 좁아진 것으로 보게
된다. Planning evaluation이 끝나면 handoff가 풀리고 응답이 host scrollback으로
commit되어 “나중에 정상화”된다.

`8e6eacf4 fix(tui): flush replies before settlement`은
`begin_viewport_transcript_handoff_release()`를 settlement 완료가 아니라 시작으로
옮겼다. 이제 상태 전이는 다음과 같다.

```text
Streaming
  -> Turn completed
  -> begin_post_turn_settlement()
       completed answer release starts immediately
  -> next successful draw commits answer to host scrollback exactly once
  -> settlement status alone remains live
  -> complete_post_turn_settlement()
```

새 회귀 테스트는 settlement가 계속 표시되는 동안 80행 응답의 first/middle/last
marker가 host scrollback에 이미 존재함을 검증한다.

### 3. Test oracle가 버그를 정상 동작으로 고정한 경로

수정 전 테스트 이름은
`completed_agent_handoff_stays_visible_until_settlement_then_flushes_once`였고, 실제로
settlement 동안 final marker가 active screen에 1번 보이고 host scrollback에는 0번
보이는 것을 요구했다. 즉 테스트가 사용자 불편을 막지 못한 정도가 아니라 잘못된
lifecycle 결합을 정답으로 고정했다.

수정 후 테스트는 `completed_agent_handoff_flushes_at_settlement_and_only_once`로 바뀌고,
settlement screen에는 final marker 0개, host scrollback에는 1개를 요구한다.

이 사례의 교훈은 snapshot 수보다 invariant의 방향이 중요하다는 것이다. 잘못된
oracle을 많이 실행하면 잘못된 동작이 더 안정적으로 유지된다.

### 4. Coarse supersession의 관련 증거

`48b59e27 fix(tui): preserve planning completion on stream` 이전에는 모든
`ConversationRuntimeEvent`가 `planning_ui_intent_revision`을 증가시켰다. 관련 없는
runtime notice도 in-flight planning editor completion을 stale로 만들어 버릴 수 있었다.

현재는 다음 event만 planning intent를 supersede한다.

- prompt admission
- approval submission
- correlated planning file change를 포함한 terminal stream event

일반 runtime notice와 무관한 stream fact는 supersede하지 않는다. 이는 render 버그와
별개 수정이지만, Akra의 구조적 문제를 같은 방향으로 보여 준다. 서로 독립적인 lifecycle
및 event domain을 하나의 broad aggregate와 revision에 결합하면, 한 기능의 진행 상태가
다른 기능의 결과를 지우거나 지연시킨다.

## Why This Recurred

### Multiple partial authorities

현재 terminal-visible 결과는 한 객체가 단독으로 소유하지 않는다.

| Layer/state | Owns |
| --- | --- |
| `ConversationViewModel` | committed messages, live handoff, settlement state |
| `HistoryFlushState` | scrollback diff baseline, pending suffix, rendered row accounting |
| `TerminalViewportState` | last geometry, cursor, back-buffer trust, insert mode |
| Ratatui `Terminal` | active viewport and frame buffer |
| Terminal emulator | actual native scrollback |

이 분리는 그 자체로 잘못이 아니다. 현재 terminal transaction에는
`HistoryFlushResult::history_committed()`와 handoff acknowledgement가 이미 있다. 문제는
그 물리 write barrier보다 앞선 “turn terminal 이후 어떤 응답이 commit-eligible인가”가
별도 handoff boolean과 settlement 시점에 의해 결정됐다는 점이다. Receipt는 eligible로
넘어온 행만 확인할 수 있으므로, 각 부분 상태는 내부적으로 일관됐지만 전체 사용자
결과는 일관되지 않았다.

### Independent lifecycles were serialized

완료 transcript delivery와 post-turn planning evaluation은 서로 다른 lifecycle이다.
Planning settlement 동안 manual input을 막아야 한다는 정책은 타당할 수 있지만, 그것이
완료 응답의 durable-history 전환까지 막아야 한다는 결론은 나오지 않는다.

```text
turn lifecycle:     streaming -> terminal -> transcript committed
planning lifecycle:                   evaluating -> settled
input admission:                       blocked    -> open
```

세 축은 상관관계가 있지만 동일한 상태 전이는 아니다.

### Terminal capability was inferred from an API call

`append_lines`, scroll region, cursor 이동이 오류 없이 끝났다는 사실은 host terminal
history가 보존됐다는 증거가 아니다. Active screen과 native scrollback은 별도 관측
대상이어야 한다. `TestBackend` screen assertion만으로 이 차이를 검출할 수 없다.

### State-heavy integration increases local-fix pressure

`NativeTuiApp`은 스스로 “intentionally state-heavy”라고 설명하며 conversation,
planning, parallel mode, overlays, runtime channels를 한 aggregate에서 통합한다. 최근
`9683cf54`가 per-frame full `AppSnapshot` projection을 좁혔고, `48b59e27`가 broad
revision invalidation을 좁힌 사실은 이 경계가 실제로 높은 인지 부하와 collateral
invalidation을 만들었음을 보여 준다.

## Grok Build Architecture

### Runtime topology

```text
xai-grok-pager-bin (composition root)
  ├─ xai-grok-pager             AppView, Action/Effect, full TUI
  ├─ xai-grok-pager-minimal     native-scrollback renderer
  ├─ xai-grok-pager-render      frame diff, writer queue/ack
  └─ xai-grok-shell             session actor, agent runtime, queue authority
       ├─ xai-grok-tools
       └─ xai-grok-workspace
```

Whole-repository import topology와 file role을 함께 적용한 layer 분포는 다음과 같다.
각 file-level node는 정확히 한 layer에만 배정됐으며 총 2,779개다.

| Layer | File-level nodes |
| --- | ---: |
| 터미널 사용자 경험 | 536 |
| 테스트 및 검증 | 535 |
| 에이전트 및 세션 런타임 | 403 |
| 도구 및 외부 통합 | 283 |
| 워크스페이스 및 플랫폼 서비스 | 267 |
| 빌드·설정 및 릴리스 | 268 |
| 문서 및 에이전트 지침 | 200 |
| 공용 계약 및 스키마 | 190 |
| 모델 및 컨텍스트 지능 | 97 |

Pager application code는 `Action -> state mutation + Vec<Effect>`를 synchronous,
testable dispatch로 정의하고, effect만 async task로 분리한다. ACP stream은
`AcpUpdateTracker`가 `SessionUpdate`를 `ScrollbackState` mutation으로 줄인다.
Tracker는 UI와 network를 소유하지 않고 stream ordering, message/thinking/tool entry,
orphan update, waiting reason을 보관하는 stateful reducer다.

### Full TUI path

Full TUI는 terminal native history가 아니라 앱 소유 `ScrollbackState`를 화면 안에서
render한다.

```text
ACP update
  -> AcpUpdateTracker
  -> ScrollbackState
  -> AgentViewLayout::compute
  -> screen-owned scrollback pane
```

Layout은 scrollback에 `Constraint::Min(5)`를 부여한다. Queue pane은
`MAX_QUEUE_HEIGHT = 3`이며 실제 desired height도 1~3행으로 clamp한다. 이 budget은
planning/queue/status가 등장해도 history가 0행으로 굶는 것을 막는다.

### Minimal/native-scrollback path

Minimal mode는 finalized block을 `Terminal::insert_before`로 한 번만 native
scrollback에 넣고, 작은 live region에는 running tail, status, overlay, prompt만 둔다.
한 frame의 순서는 코드 주석에 load-bearing contract로 기록되어 있다.

```text
0. synchronized update + autoresize
1. welcome/plan commit 준비
2. viewport를 post-commit target height로 조정
3. finalized blocks를 native scrollback에 commit
4. live region redraw
```

핵심은 2번이 3번보다 먼저라는 점이다. Tall streaming tail의 현재 높이가 아니라
commit 후 남을 tail의 높이를 먼저 계산한다. 그래야 `insert_before`가 최종 높이의
viewport를 완료 block 바로 아래에 재배치한다.

`sync_viewport`는 commit 예정 여부에 따라 resize primitive도 분리한다.

- commit 예정: 높이만 먼저 바꾸고 clear하지 않는다. 곧 실행될 `insert_before`가
  clear, scroll, reposition을 소유한다.
- commit 없음: `set_viewport_height`가 grow 시 덮일 committed rows를 native history로
  올리고, shrink 시 stale overlay rows를 지운다.

`set_viewport_height`는 저장된 `Viewport::Inline(height)`가 아니라 실제
`viewport_area.height`로 grow/shrink를 판단한다. Minimal commit path가
`set_viewport_area`를 직접 호출할 수 있어 두 값이 일시적으로 다를 수 있기 때문이다.

### Render backpressure and acknowledgement

Grok은 frame byte write를 dedicated OS thread에 넘긴다. Presenter는 writer sequence를
기록하고 `WriterEvent::Written(sequence)` acknowledgement가 오기 전까지 새 frame을
in-flight로 만들지 않는다. 그 사이의 presentation request는 dirty flag로 coalesce한다.
따라서 PTY backpressure가 Tokio event loop를 막거나, 아직 기록되지 않은 frame 위에
무제한 새 frame이 쌓이는 것을 줄인다.

### Prompt queue authority

Queue는 shell session actor의 serialized mailbox가 authoritative하다. Version, owner,
position, running prompt id를 포함한 `x.ai/queue/changed` broadcast가 attached client의
truth signal이다. Pager는 server queue mirror와 local non-shared queue를 합쳐 그린다.

이 경계는 planning/queue 진행 상태가 renderer의 transcript durability를 소유하지
않게 한다. 다만 Grok도 optimistic queue IDs, local queue, shared mirror, send-now
confirmation state를 함께 관리하므로 이 영역은 단순하지 않다.

### Architecture guards

Minimal crate에는 source guard가 있어 다음 legacy helper 호출을 금지한다.

- `resize_purge_rerender`
- `emit_to_scrollback`
- `resize_viewport_height`

이유는 terminal이 이미 소유한 committed history를 앱이 다시 emit하면 duplicate되거나
clear sequence에 의해 지워질 수 있기 때문이다. Critical invariant를 문서에만 두지
않고 compile-time test로 막는 예다.

## Direct Comparison

| Concern | Akra at `48b59e27` | Grok Build at `ba76b0a6` | Decision |
| --- | --- | --- | --- |
| Finalized output | handoff flag + history diff; recent fix releases at settlement start | explicit committed frontier, print-once `insert_before` | Grok의 explicit commit phase를 좁게 차용 |
| Host history | terminal strategy selected by mode; normal default newline fallback | minimal mode terminal-native history; full mode app-owned pane | Surface ownership을 mode contract로 고정 |
| Viewport height | both inline modes fixed 16 rows | minimal dynamic post-commit height; full history min 5 | fixed height 자체보다 commit/status budget을 명시 |
| Geometry truth | adapter cache + Ratatui terminal geometry | actual `viewport_area.height` is grow/shrink truth | 실제 geometry 단일 진실 소스 채택 |
| Settlement | transcript release와 분리되도록 최근 수정 | queue/session actor state와 scrollback commit 분리 | 독립 lifecycle 유지 |
| Frame writes | synchronous adapter transaction | writer thread + sequence acknowledgement + coalescing | backpressure가 재현될 때만 도입 검토 |
| Stream reducer | Core stream + TUI conversation projection | `AcpUpdateTracker -> ScrollbackState` | TUI-facing terminal reducer 경계를 더 명확히 |
| Regression proof | frame recorder, TestBackend, VT100, manual matrix contract | unit/differential plus PTY harness | PTY temporal assertions를 release gate로 승격 |
| Complexity | state-heavy TUI aggregate, deep feature modules | 79 packages, large App/Agent state, IoC seam | Grok 전체 구조 복제 금지 |

## Intelligence vs Architecture

이 문제를 “모델의 지능 저하”만으로 설명하는 것은 정확하지 않다. 그러나 에이전트의
분석 과정에도 명확한 실수 포인트가 있었다.

1. 화면 캡처를 처음에는 layout 문제로 좁게 읽고, marker가 app transcript, active
   frame, native scrollback 중 어디에 존재하는지 끝까지 추적하지 않았다.
2. `settlement 동안 응답을 viewport에 유지한다`는 기존 테스트를 사용자 요구보다
   강한 정답으로 받아들였다.
3. Ratatui screen 이동과 terminal emulator native scrollback 생성을 같은 성공으로
   취급했다.
4. Planning settlement와 transcript commit을 독립 lifecycle로 다시 그리기 전에
   가까운 함수의 시점만 수정하려는 local-fix bias가 있었다.
5. 첫 회귀 뒤 즉시 PTY/VT100 temporal trace를 필수 증거로 삼지 않아 같은 영역을
   여러 번 수정했다.

이는 “모델이 갑자기 멍청해졌다”기보다 잘못된 local oracle과 높은 cross-layer 인지
부하 아래에서 발생한 추론 실패다. 더 좋은 모델도 잘못된 invariant를 충실히 따르면
같이 실패한다. 반대로 현재 모델도 아래의 관측 순서와 architecture guard가 있으면
재발 가능성을 크게 낮출 수 있다.

## Recommended Akra Target

### P0 — invariant를 shipped contract로 고정

```text
Streaming
  -> TurnTerminal
       finalized transcript becomes commit-eligible immediately
  -> CommitPending
       one terminal transaction writes it exactly once
  -> Settling
       status-only; cannot own finalized transcript
  -> Idle
```

- `TurnTerminal -> CommitPending`은 planning evaluation 결과와 무관해야 한다.
- `Settling`은 manual input admission을 닫을 수 있지만 completed answer를 live tail에
  보관할 수 없다.
- 기존 `HistoryFlushResult`를 exact conversation/turn/handoff generation과 연결하고,
  conversation handoff acknowledge는 그 correlated success에만 반응해야 한다.

### P1 — terminal transaction의 한 소유자

- history suffix selection, physical insertion, geometry observation, back-buffer invalidation,
  commit acknowledgement 순서를 하나의 terminal transaction contract로 유지한다.
- 실제 viewport area를 geometry truth로 사용한다.
- renderer나 planning controller가 host history replay/clear를 직접 호출하지 못하게
  architecture test를 추가한다.
- `HistoryInsertionMode`의 capability는 startup에 한 번 선택하고 transaction 중
  추측하거나 바꾸지 않는다.

새 application port는 필요하지 않다. 이것은 TUI adapter 내부 terminal boundary다.
기존 `HistoryFlushState`와 `InlineTerminalState`를 전면 재작성하기보다
commit-eligibility phase와 기존 receipt의 correlation을 명시하는 최소 추출이 적절하다.

### P1 — typed supersession domains

`planning_ui_intent_revision`처럼 넓은 revision을 event 수신 자체로 증가시키지 않는다.
다음처럼 domain-specific correlation을 사용한다.

- transcript commit generation
- planning editor intent generation
- viewport geometry epoch
- conversation identity/turn correlation

한 domain의 event가 다른 generation을 supersede하려면 typed evidence가 있어야 한다.
현재 `changed_planning_file_paths` 기반 좁은 supersession이 그 방향의 좋은 예다.

### P1 — temporal regression matrix

다음 검증은 한 frame의 최종 snapshot만 보지 말고 모든 draw transaction을 기록해야
한다.

| Scenario | Required assertion |
| --- | --- |
| long final + settlement | first/middle/last가 settlement 첫 draw 후 host scrollback에 존재 |
| exactly once | settlement 완료, idle redraw 뒤에도 marker count = 1 |
| resize during commit | history monotonic, prompt 위치 정상, stale row 없음 |
| overlay grow/shrink | committed history 보존, overlay close 뒤 blank band 없음 |
| focus reacquire | full repaint하되 history replay 없음 |
| thread switch | 이전 pending handoff/history suffix가 새 thread로 누출되지 않음 |
| parallel insertion branch | first-class terminal별 native scrollback 보존 증거 |
| unrelated stream fact | planning completion을 supersede하지 않음 |

### P2 — layout budget 명시

UX 관점에서도 async status가 등장할 때 이미 읽던 완료 콘텐츠가 jump하거나 clipped되면
안 된다. Full-screen 계열 surface에는 Grok처럼 history minimum과 queue/status maximum을
명시한다. Akra inline surface에서는 fixed 16을 즉시 동적으로 바꿀 필요는 없지만,
완료 history, live stream, settlement status의 row ownership을 명시해야 한다.

## Verification

### Reproduced

```text
cargo test -p xai-ratatui-inline
  unit:         49 passed
  differential: 2 passed
  doctest:       6 passed, 4 ignored

cargo test --locked settlement_flushes_long_completed_answer_to_host_scrollback
  1 passed

cargo test --locked vt100_host_scrollback_preserves_long_single_completion_beyond_screen_cap
  1 passed

cargo test --locked planning_ui_supersession_requires_a_correlated_planning_file_change
  1 passed
```

### Not reproduced

`cargo test -p xai-grok-pager-minimal --lib`은
`xai-grok-tools-api` build script에서 중단됐다. 저장소의 `bin/protoc`이 DotSlash를
요구하지만 분석 환경에 `dotslash`가 없고 PATH에도 `protoc`이 없었다. 분석을 위해
환경에 새 전역 도구를 설치하지 않았다.

Minimal PTY directory의 실제 test file 22개는 모두 `#[ignore]`다. 소스에는
`minimal_commits_response_to_scrollback`,
`minimal_resize_preserves_committed_scrollback`,
`minimal_committed_content_survives_overlay_grow` 등 정확한 회귀 시나리오가 존재하지만,
default test invocation이 이를 실행한다는 주장은 할 수 없다.

## Evidence Ledger

### Grok Build

| ID | Source | Supports |
| --- | --- | --- |
| G1 | [`xai-grok-pager/src/app/mod.rs`](https://github.com/xai-org/grok-build/blob/ba76b0a683fa52e4e60685017b85905451be17bc/crates/codegen/xai-grok-pager/src/app/mod.rs#L1) | Action/Effect/TaskResult and event-loop split |
| G2 | [`xai-grok-pager/src/acp/tracker.rs`](https://github.com/xai-org/grok-build/blob/ba76b0a683fa52e4e60685017b85905451be17bc/crates/codegen/xai-grok-pager/src/acp/tracker.rs#L209) | stream-to-scrollback reducer ownership |
| G3 | [`views/agent.rs`](https://github.com/xai-org/grok-build/blob/ba76b0a683fa52e4e60685017b85905451be17bc/crates/codegen/xai-grok-pager/src/views/agent.rs#L203) | scrollback minimum layout budget |
| G4 | [`views/queue_pane.rs`](https://github.com/xai-org/grok-build/blob/ba76b0a683fa52e4e60685017b85905451be17bc/crates/codegen/xai-grok-pager/src/views/queue_pane.rs#L373) | queue maximum height and presentation-only ownership |
| G5 | [`xai-grok-pager-minimal/src/lib.rs`](https://github.com/xai-org/grok-build/blob/ba76b0a683fa52e4e60685017b85905451be17bc/crates/codegen/xai-grok-pager-minimal/src/lib.rs#L47) | post-commit sizing and commit order |
| G6 | [`minimal/live.rs`](https://github.com/xai-org/grok-build/blob/ba76b0a683fa52e4e60685017b85905451be17bc/crates/codegen/xai-grok-pager-minimal/src/live.rs#L701) | remaining-tail height and prior snaps-to-top bug |
| G7 | [`minimal/overlay.rs`](https://github.com/xai-org/grok-build/blob/ba76b0a683fa52e4e60685017b85905451be17bc/crates/codegen/xai-grok-pager-minimal/src/overlay.rs#L134) | commit/no-commit viewport resize branches |
| G8 | [`xai-ratatui-inline/src/terminal.rs`](https://github.com/xai-org/grok-build/blob/ba76b0a683fa52e4e60685017b85905451be17bc/crates/codegen/xai-ratatui-inline/src/terminal.rs#L847) | actual viewport height and grow preservation |
| G9 | [`minimal/guard.rs`](https://github.com/xai-org/grok-build/blob/ba76b0a683fa52e4e60685017b85905451be17bc/crates/codegen/xai-grok-pager-minimal/src/guard.rs#L1) | forbidden history replay/resize helpers |
| G10 | [`render/draw.rs`](https://github.com/xai-org/grok-build/blob/ba76b0a683fa52e4e60685017b85905451be17bc/crates/codegen/xai-grok-pager-render/src/render/draw.rs#L53) | writer sequences and dedicated writer thread |
| G11 | [`app/event_loop.rs`](https://github.com/xai-org/grok-build/blob/ba76b0a683fa52e4e60685017b85905451be17bc/crates/codegen/xai-grok-pager/src/app/event_loop.rs#L316) | one-frame-in-flight presenter and coalescing |
| G12 | [`shell/session/commands.rs`](https://github.com/xai-org/grok-build/blob/ba76b0a683fa52e4e60685017b85905451be17bc/crates/codegen/xai-grok-shell/src/session/commands.rs#L530) | authoritative serialized prompt queue |
| G13 | [`minimal_commits_response_to_scrollback.rs`](https://github.com/xai-org/grok-build/blob/ba76b0a683fa52e4e60685017b85905451be17bc/crates/codegen/xai-grok-pager/tests/pty_e2e/minimal/minimal_commits_response_to_scrollback.rs#L5) | native scrollback PTY scenario and ignored status |
| G14 | [`minimal_resize_preserves_committed_scrollback.rs`](https://github.com/xai-org/grok-build/blob/ba76b0a683fa52e4e60685017b85905451be17bc/crates/codegen/xai-grok-pager/tests/pty_e2e/minimal/minimal_resize_preserves_committed_scrollback.rs#L5) | resize preservation PTY scenario and ignored status |

### Akra

| ID | Source | Supports |
| --- | --- | --- |
| A1 | [`app.rs`](../../../src/adapter/inbound/tui/app.rs#L41) | fixed inline viewport and state-heavy aggregate |
| A2 | [`inline_terminal_adapter.rs`](../../../src/adapter/inbound/tui/app/inline_terminal_adapter.rs#L33) | dual surfaces and fixed `Viewport::Inline(16)` |
| A3 | [`history_flush.rs`](../../../src/adapter/inbound/tui/app/inline_terminal_adapter/history_flush.rs#L11) | transcript/host-history reconciliation state |
| A4 | [`history_insertion.rs`](../../../src/adapter/inbound/tui/app/history_insertion.rs#L18) | insertion modes and current automatic resolution |
| A5 | [`view_model.rs`](../../../src/adapter/inbound/tui/app/conversation_model/view_model.rs#L769) | settlement start releases transcript handoff |
| A6 | [`messages.rs`](../../../src/adapter/inbound/tui/app/conversation_model/view_model/messages.rs#L452) | handoff release and physical flush acknowledgement |
| A7 | [`conversation_runtime.rs`](../../../src/adapter/inbound/tui/app/conversation_runtime.rs#L69) | typed planning UI supersession criteria |
| A8 | `22f755db` | host scrollback preservation fix and VT100 long-response regression |
| A9 | `8e6eacf4` | immediate reply release at settlement start |
| A10 | `48b59e27` | correlated planning completion supersession |

## Evidence Limits

- Public Grok repository는 monorepo sync snapshot이다. Shallow clone이므로 비공개 monorepo
  history와 각 viewport 버그의 원래 수정 commit은 확인할 수 없다.
- Grok authenticated model turn, 실제 제품 binary, terminal별 manual matrix는 실행하지
  않았다.
- Whole-repository graph의 large community는 deterministic batch로 분할됐고 high-degree
  neighbor 일부는 batch prompt 크기 제한으로 truncate됐다. Critical TUI 결론은 이
  graph summary가 아니라 위 직접 source trace로 재검증했다.
- UI/UX cross-check는 async content가 기존 콘텐츠를 밀거나 clip하지 않고 wait status를
  별도 feedback으로 보여야 한다는 일반 원칙만 사용했다. Web/mobile 지침을 terminal
  correctness 근거로 사용하지 않았다.
