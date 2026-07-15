# jcode에서 Akra로의 갭 매트릭스

[English](../../../competitive/jcode/gap-matrix.md)

이 문서는 [jcode v0.43.0 분석](analysis.md)을 Akra 결정으로 전환한다. 증거와 한계는
[evidence.md](evidence.md)에 있다. `Ahead`는 고정된 스냅샷의 해당 차원에서 더 강한 증거가 있다는
의미이지 전체 제품 점수가 아니다.

## 상대 매트릭스

| 차원 | jcode v0.43.0 | Akra `66333152` | 결론 | 결과 | 증거 |
| --- | --- | --- | --- | --- | --- |
| 제품 명제 | 폭넓은 고숙련 harness: speed, session, customization, memory, swarm 주장 | Codex-first client와 durable planning/delivery, 그러나 end-to-end promise가 여러 문서에 분산 | 명확성은 jcode가 앞섬 | Codex에서 reviewed integration까지의 promise를 밝히고 입증 | [product](evidence.md#product-and-release), [Akra](evidence.md#akra-baseline-evidence) |
| 런타임 권한 | daemon, provider, tool, session, memory, swarm 소유 | runtime은 공식 `codex app-server`에 위임, operator workflow 소유 | 의도적으로 다름, security 승자 불명 | 더 좁은 owned boundary를 유지하고 native protocol value를 더 많이 노출 | [runtime](evidence.md#runtime-architecture), [Akra](evidence.md#akra-baseline-evidence) |
| Startup/input 증거 | public table과 executable PTY/PSS script, 현재 수치는 v0.43에 고정되지 않음 | immediate-frame scheduler test 및 terminal capture, repeatable benchmark 없음 | discipline은 jcode가 앞서며 performance 승자 불명 | speed claim 전에 Akra evidence contract 구축 | [performance](evidence.md#performance) |
| TUI 정보 계층 | prioritized negative-space widget, side panel, diagram, workspace 및 swarm instrumentation | 강력한 inline shell, overlay, parallel board, host scrollback, coarse completed-item activity line, started/delta/diff/plan/token detail 누락 | density는 jcode, host-history contract는 Akra가 앞섬 | generic widget framework가 아닌 bounded protocol-native execution rail 확장 | [TUI](evidence.md#tui-and-visual-system), [Akra](evidence.md#akra-baseline-evidence) |
| Terminal 철학 | custom scrollback 및 풍부한 internal viewport, smooth-scroll 한계 인정 | inline rendering이 host scrollback을 보존하고 terminal 전반의 insertion/restore test | 다름 | Akra host scrollback 보존, priority와 drilldown pattern만 차용 | [TUI](evidence.md#tui-and-visual-system), [Akra](evidence.md#akra-baseline-evidence) |
| Live turn 제어 | soft interrupt, communication, server-owned session control | active turn input은 대기, protocol schema에는 `turn/steer`가 있으나 port에는 없음 | 현재 interaction breadth는 jcode가 앞섬 | CAS-safe app-server turn steering 추가 | [runtime](evidence.md#runtime-architecture), [Akra gaps](evidence.md#akra-baseline-evidence) |
| Session 경제성 | shared daemon이 server-owned state 재사용, reconnect/reload가 핵심 | official session catalog와 resumed Codex thread가 있는 하나의 TUI, parallel worker는 process 추가 | topology가 다르고 cost 승자 불명 | complete process tree 측정 및 attach/resume projection 개선 | [runtime](evidence.md#runtime-architecture), [limits](evidence.md#audit-limits) |
| Active-turn 연속성 | daemon-owned session runtime은 설계상 client-independent, live survival 미재현 | official thread history는 resume하지만 connection loss/drop이 child를 종료하고 reconnect는 active-turn reattachment 검증 없이 새 child 시작 | continuity contract는 jcode가 앞섬 | 두 번째 daemon 없이 active-turn exit, failure, restart reconciliation 정의 | [runtime](evidence.md#runtime-architecture), [Akra](evidence.md#akra-baseline-evidence) |
| Planning 권한 | legacy coordinator plan과 ready, blocked, failed, ownership, artifact, confidence state가 있는 live/migrating DAG | accepted SQLite planning authority, staged draft, queue, repair, supersession | operator ownership은 Akra, graph vocabulary는 jcode가 풍부 | authority model 유지, 유용한 곳에 explicit dependency/failure evidence 도입 | [swarm](evidence.md#swarm-and-planning), [Akra](evidence.md#akra-baseline-evidence) |
| 병렬 조정 | recursive spawn, message, activity age, report, change alert, optional worktree, deep mode는 member 1,000명 허용 | fixed pool, lease, session detail, control plane, supervisor, distributor | 다름, jcode live scale/cost 미검증 | 누락 activity/evidence field와 explicit resource budget 추가, same-checkout coordination은 도입하지 않음 | [swarm](evidence.md#swarm-and-planning), [limits](evidence.md#audit-limits) |
| 격리 | optimistic no-lock same-repo work 지원, 문서는 optional worktree role 설명하나 automated lifecycle 미검증 | lane마다 worktree/branch/reviewable slice 하나 | 검증된 default isolation은 Akra가 앞섬 | isolation과 hotspot ownership을 product capability로 표시 | [swarm](evidence.md#swarm-and-planning), [Akra](evidence.md#akra-baseline-evidence) |
| 전달 | 문서는 integration을 worktree manager에 할당하나 automated worktree/delivery lifecycle 미검증 | default commit/push/PR/reviewed-range integration 및 cleanup, explicit parent high-risk opt-in은 review/check gate와 PR automation도 생략할 수 있음 | 검증된 default delivery contract는 Akra가 앞서나 autonomous bypass는 review evidence 약화 | delivery evidence, PR/gate provenance, critical review response를 first-class TUI/Admin state로 승격 | [swarm](evidence.md#swarm-and-planning), [Akra](evidence.md#akra-baseline-evidence) |
| 장기 memory | automatic extraction/retrieval, embedding, tool, session search, full hybrid graph는 일부 계획 상태 | durable planning, official session, direction/task provenance, semantic memory 없음 | breadth는 jcode가 앞섬 | embedding 전에 누락 provenance link 추가 | [memory](evidence.md#memory-providers-and-extensibility) |
| Provider 폭 | 다수 OAuth, API, local, compatible, runtime adapter | 공식 runtime을 통한 Codex만 지원 | 선택 폭은 jcode가 앞섬 | 여기서 경쟁하지 않음, Codex specialization이 wedge | [providers](evidence.md#memory-providers-and-extensibility), [Akra](evidence.md#akra-baseline-evidence) |
| 확장성 | tool, MCP, hook, provider profile, side panel, self-development | Codex capability와 application port 및 fixed operator surface | user customization은 jcode가 앞섬 | app-server authority를 존중하는 capability/status discovery만 추가 | [providers](evidence.md#memory-providers-and-extensibility) |
| Browser Admin | 감사한 source에 검증된 product web Admin 없음 | Axum/Askama read-only parallel dashboard, JSON API, review center, task, metric, game diorama | web observability는 Akra가 앞서나 full control plane은 아님 | lifecycle metric 및 guarded application-service action 추가 | [remote](evidence.md#desktop-and-remote-control), [Akra](evidence.md#akra-baseline-evidence) |
| Desktop | 큰 custom Rust prototype, public architecture는 여전히 proposed | desktop client 없음 | investment는 jcode가 앞서나 maturity 불확실 | TUI/Admin core loop가 측정 가능하게 우수해질 때까지 전환하지 않음 | [desktop](evidence.md#desktop-and-remote-control) |
| 원격 제어 | gateway/mobile direction, daemon/debug interface, hook | Admin, Telegram, GitHub delivery/review, CLI/JSON tool | 검증된 현재 operation은 Akra가 앞섬 | 동일 structured lifecycle 및 guarded command 중심으로 통합 | [remote](evidence.md#desktop-and-remote-control), [Akra](evidence.md#akra-baseline-evidence) |
| 안전 표면 | 폭넓은 provider/tool/auth surface, permission UI, pre-tool hook, design-stage ambient safety | official runtime, bounded interactive approval, unattended decline, identity-gated GitHub write | 구조적 차이, 비교 safety 미검증 | fail-closed policy와 delivery audit을 표시, superiority 주장 전 shared threat model 추가 | [providers/hooks](evidence.md#memory-providers-and-extensibility), [Akra](evidence.md#akra-baseline-evidence) |
| 품질 instrumentation | 광범위한 test, benchmark, ratchet, cross-platform job | 강력한 Rust test, native/TUI layering 및 terminal validation, performance budgeting은 적음 | breadth는 jcode가 앞섬 | measurable performance 및 maintainability ratchet을 선택적으로 추가 | [quality](evidence.md#quality-and-maintainability) |
| Quality-ratchet 상태 | clean v0.43 tag에서 check-in된 ratchet 네 개가 local failure | current native gate는 광범위, 이 감사에서는 모든 CI 재실행하지 않음 | jcode 약점, Akra 상태 미비교 | configured job이 아닌 green artifact를 proof로 취급 | [guard output](../../../competitive/jcode/guard-results.txt), [quality](evidence.md#quality-and-maintainability) |

## 도입

### 1. Versioned 계약으로서의 성능

jcode가 게시한 수치가 아니라 측정 규율을 도입한다.

Akra에는 다음 metric을 분리해야 한다.

- process spawn부터 first meaningful shell frame까지;
- first frame부터 visible prompt echo까지;
- process spawn부터 app-server startup 뒤 ready-to-submit까지;
- submit부터 first protocol event 및 first assistant delta까지;
- full process tree의 idle, streaming, parallel-pool PSS;
- fixed synthetic stream에서 p50/p95 frame work 및 event backlog.

Raw sample, version, auth state, warm/cold state, terminal geometry, process tree, environment stamp는
필수다. raw artifact 없는 rendered summary는 evidence가 아니다.

### 2. Priority 기반 TUI Instrumentation

scarce terminal space에 explicit priority와 minimum useful size가 있다는 jcode 규칙을 도입한다.
Akra fact에 다음과 같이 mapping한다.

1. active approval 또는 failure
2. 실시간 명령/패치/계획 활동
3. context pressure 및 active model/effort
4. current planning task 및 continuation reason
5. parallel lane status 및 selected lane detail
6. diagnostic 및 secondary hint

wide terminal에는 rail을 보여 줄 수 있다. narrow terminal은 낮은 priority fact를 한 status line 또는 기존
detail overlay로 collapse해야 한다. dashboard shape를 보존하려 empty box를 render하지 않는다.

### 3. 풍부한 병렬 Activity

lane이 healthy한지 operator가 판단하는 데 도움이 되는 field를 도입한다.

- lifecycle state age와 last activity age를 서로 다른 value로 표시;
- 현재 app-server item/tool 분류;
- task/role label;
- bounded validation 및 completion evidence;
- failure reason 및 blocked dependency;
- report-to owner 및 integration state;
- assignment time에 선언된 file/hotspot ownership.

이는 새 agent messaging system이 아니라 기존 parallel session detail 및 runtime event contract에 속한다.

### 4. Nonblocking 조정

worker가 start, wait, report하거나 official session truth를 refresh하거나 delivery를 기다리는 동안 coordinator와
TUI를 계속 사용할 수 있어야 한다. inbound handler를 worker completion에 block하지 말고 기존 control-plane
effect와 event projection을 확장한다.

### 5. 결과 중심 Guardrail

Akra에서 measurable risk가 있는 곳에 budget을 도입한다.

- 시작/입력/PSS 회귀 예산;
- TUI 프레임/이벤트 적체 예산;
- current agent guide와 일치하는 LLM-facing TUI file-size ratchet;
- production panic 및 swallowed-boundary-error report;
- `domain`, `application`, `core`, adapter의 dependency direction check.

gate는 release commit에서 통과해야 한다. stale baseline은 false confidence를 만들기 때문에 baseline이
없는 것보다 나쁘다.

## 거부

<a id="multi-provider-runtime"></a>
### 다중 공급자 런타임

Akra에 provider transport, subscription OAuth import, model catalog, provider-specific tool translation을
추가하지 않는다. 공식 runtime을 중복하고 credential attack surface를 넓히며 Codex-first position을
지울 것이다.

<a id="local-agent-tool-runtime"></a>
### 로컬 에이전트 도구 런타임

file, shell, browser, LSP, MCP, memory tool을 위한 parallel registry를 만들지 않는다. 유용한 app-server
event와 capability를 port를 통해 projection한다. GitHub delivery, planning persistence, Telegram처럼 실제
Akra-owned boundary에만 outbound port를 추가한다.

### 동일 Checkout Swarm

file-read notification과 agent-to-agent conflict repair를 위해 worktree isolation을 포기하지 않는다.
notification은 유용한 warning이지 isolation이 아니다. Akra의 one-lane/one-worktree 규칙은 reviewed
integration에 더 강한 default다.

### 조정 권한으로서의 Agent Messaging

DM이나 broadcast channel을 task truth의 source로 만들지 않는다. accepted planning, lease, runtime event,
completion evidence, integration state가 계속 authoritative하며 operator가 inspect할 수 있어야 한다.

### 완전한 Semantic Memory Graph

official-session search와 planning provenance에 측정된 failure가 생기기 전에는 embedding 또는 automatic
personal memory를 추가하지 않는다. memory system에는 privacy control, correction, expiry, conflict
resolution, packaging, resource budget, visible provenance가 필요하다.

### Custom Scrollback 교체

host terminal history를 in-app transcript viewport로 교체하지 않는다. inline scrollback contract는 Akra의
차별점이다. rich live state는 bounded 상태로 유지되어야 하며 permanent chrome으로 replay돼서는 안 된다.

### Desktop 및 Mobile 확장

TUI에 live protocol detail이 없고 Admin에 실제 operational metric이 없는 동안 native desktop 또는 mobile
shell을 시작하지 않는다. Admin은 runtime duplication이 훨씬 적은 cross-device operator surface를 이미
제공한다.

<a id="self-development-hot-reload"></a>
### 자체 개발 핫 리로드

agent가 Akra 자체를 rewrite하고 hot-reload하는 것을 primary product workflow로 최적화하지 않는다. normal
worktree, validation, review, release, rollback을 사용한다. developer iteration speed는 두 번째 runtime
lifecycle이 아니라 module boundary와 build profile로 개선해야 한다.

## 차별화

### Codex 프로토콜 선도

Akra는 유용한 공식 app-server feature를 가장 먼저 정확하게 노출하는 wrapper여야 한다. 현재 schema에는
제품이 projection하지 않는 behavior가 이미 있다.

즉시 대상:

- `turn/steer`와 `expectedTurnId` compare-and-set semantic;
- `item/started` lifecycle;
- 명령 출력 delta;
- 파일 패치 갱신;
- bounded summary 및 selected full-diff inspection을 갖춘 aggregated turn diff update;
- plan update;
- 토큰/컨텍스트 사용량 갱신.

공식 Codex improvement마다 복리로 쌓이므로 generic provider choice보다 강한 차별점이다.

### 단순 완료가 아닌 전달

worker가 summary를 emit했다고 끝난 것이 아니다. Akra는 default path를 보여 주고 enforce해야 한다.

```text
assigned -> running -> reported -> commit ready -> source range frozen
-> configured local validation clear or not-required -> source published -> PR open
-> trusted-CI validation clear or not-required
-> frozen-head approval + CLEAN + required checks clear -> integration applied
-> integration ref pushed and remotely verified -> integrated
-> PR closed -> source/slot cleaned
```

explicit high-risk autonomous path는 reviewed state처럼 가장하지 말고 branch를 노출해야 한다.

```text
commit ready -> source range frozen -> local validation clear or not-required
-> source published -> PR open -> trusted-CI validation clear or not-required
-> review/check gates skipped (parent policy recorded) -> integration applied
-> integration ref pushed and remotely verified -> integrated -> PR closed -> source/slot cleaned

commit ready -> source range frozen -> local validation clear or not-required
-> source published -> trusted-CI validation clear or not-required
-> PR and review/check gates skipped (parent policy recorded) -> integration applied
-> integration ref pushed and remotely verified -> integrated -> source/slot cleaned
```

Autonomous policy는 validation policy를 절대 우회하지 않는다. required trusted-CI check가 PR context 없이
실행될 수 없으면 Direct mode는 계속 blocked다.

모든 state에는 owner, timestamp, failure reason, drilldown이 있어야 한다. 여기서 Akra는 broad swarm
harness보다 더 trustworthy할 수 있다.

이 sequence는 runtime distributor를 설명한다. 수동으로 관리하는 feature lane은 repository의 local rebase 및
base fast-forward policy를 계속 따른다. 두 delivery path가 misleading provenance label을 공유해서는 안 된다.

### 운영자 소유 의도

accepted planning은 intended work의 durable source로 남는다. operator는 다음을 볼 수 있어야 한다.

- 어떤 direction과 task가 turn을 야기했는가;
- 무엇이 supersede됐는가;
- automatic continuation을 선택한 이유;
- 어떤 validation/review evidence 또는 explicit review-skipped policy가 task를 닫았는가;
- 다음 remaining task가 무엇인가.

### 운영 Game Board

Admin diorama로 decoration이 아닌 실제 progress를 표현한다.

- worker는 actual lifecycle state에 따라 station을 차지한다;
- blocker 및 review wait에는 서로 다른 visible state가 있다;
- completed delivery는 project/company progression을 바꾼다;
- throughput, success rate, queue wait, integration latency는 persisted event에서 가져온다;
- data가 없으면 fabricated percentage 또는 API speed를 표시하지 않는다.

<a id="implementation-slices"></a>

## 구현 단위

각 slice는 독립적으로 review 가능하다. file list는 ownership hint이지 모든 row를 한 PR에 섞을 권한이 아니다.

<a id="p0-native-performance-evidence-contract"></a>
### P0: 네이티브 성능 증거 계약

**소유 경계**

- `scripts/` 아래 benchmark driver
- 기존 native-validation capture/manifest schema 및 `docs/validation/` artifact index
- focused Rust integration test 및 기존 native-validation script test

**계약**

- cold-start 및 warm-resume mode를 각각 최소 30개 attempted sample로 실행한다;
- fixed 80x24 PTY를 구동하고 terminal query handling을 기록한다;
- first frame, first input echo, ready-to-submit, first event, first assistant delta를 기록한다;
- process set을 Akra와 owned descendant로 정의하고 shared-daemon attribution을 기록하며 client 간 double
  counting을 피한다;
- idle, active stream, three-slot parallel profile에 대해 Linux PSS, macOS physical-footprint/RSS evidence,
  Windows private-working-set evidence를 sample한다. platform-specific metric name을 유지하고 서로 다른
  memory metric을 한 숫자로 비교하지 않는다;
- startup failure, timeout, censored memory sample을 denominator에서 빼지 않고 raw output에 유지한다;
- check-in된 deterministic nearest-rank rule로 p50/p95를 계산하고 각 percentile 옆에 successful/attempted
  sample count를 보고한다;
- raw JSON artifact 하나와 deterministic summary를 emit한다;
- OS, 커널, CPU, 메모리, 터미널/PTY, git SHA, Akra 버전, Codex 버전, 인증 상태, app-server warm 상태,
  명령, 환경 재정의, 실패를 포함한다;
- 두 번째 evidence format을 만들지 않고 기존 native-validation artifact schema를 확장한다;
- cross-product threshold 전에 Akra baseline ratchet을 확립한다.

**필수 증명**

- 결정론적 파서/요약 테스트;
- 실제 Linux baseline artifact;
- 대표 macOS 터미널 기준선 아티팩트;
- terminal semantic이 적용될 때 Windows Terminal/PowerShell 및 Windows Terminal/WSL profile;
- cross-OS memory value를 false comparison으로 normalize하지 않았다는 명시적 evidence;
- 비교할 수 없는 각 competitor sample의 문서화된 이유.

**주장 금지**

- current binary를 동일 boundary와 environment에서 실행하기 전에는 Akra가 jcode보다 빠르다는 주장.

<a id="p0-protocol-native-live-execution-rail"></a>
### P0: 프로토콜 네이티브 실시간 실행 레일

**소유 경계**

- `src/domain/conversation*.rs`
- `src/application/service/conversation_runtime_event.rs`
- `src/core/app/turn_stream.rs`
- `src/adapter/outbound/app_server/protocol/turn_notifications.rs`
- `src/adapter/inbound/tui/app/conversation_model/turn_activity.rs`
- `src/adapter/inbound/tui/app/shell_presentation/status_panels/`

**계약**

출시된 completed-item activity path를 보존한다. typed command/file-change event, current/last count, latest
summary, compact tail line이다. 해당 state를 다음 typed form으로 확장한다.

- `item/started`
- `item/commandExecution/outputDelta`
- `item/fileChange/patchUpdated`
- `turn/diff/updated`
- `turn/plan/updated`
- `thread/tokenUsage/updated`

item event는 item ID로, turn-level event는 turn ID로 reduce하여 bounded state로 만든다. rail은 recent
action, active count, changed-file count, diff stat, plan step, context pressure를 세 개에서 다섯 개 row로 보여 준다.
selected drilldown은 retained aggregated diff 전체를 render하고 provider payload가 enforced memory bound를
초과하면 explicit truncation state를 표시한다. raw delta와 full diff가 permanent scrollback을 범람하게
해서는 안 된다. completion은 하나의 stable transcript summary를 기록한다.

**필수 증명**

- identity, ordering, bound, schema drift용 protocol fixture;
- coalescing 및 memory-bound reducer test;
- 컨텍스트 임계값 테스트;
- diff 교체, 크기 제한, 잘림, 선택 상세보기 테스트;
- wide 및 narrow snapshot;
- vt100/인라인 기록기 검증 범위;
- integration 전 real-terminal capture.

<a id="p0-active-turn-steering"></a>
### P0: 활성 턴 조정

**소유 경계**

- `src/application/port/outbound/interactive_turn_runtime_port.rs`
- `src/application/service/conversation_service.rs`
- `src/adapter/outbound/app_server/`
- `src/core/app/`
- `src/adapter/inbound/tui/app/turn_submission_runtime.rs`

**계약**

- running `Enter`가 `threadId` 및 `expectedTurnId`를 사용해 current app-server turn에 input을 보낸다;
- active streaming connection이 control channel을 소유해 ordering과 correlation이 동일 runtime path에
  유지되게 한다;
- success는 draft를 정확히 한 번 clear한다;
- stale turn, non-steerable turn, rejection, disconnect, transport failure는 draft를 보존하고 actionable
  state를 보여 준다;
- steering을 새 planning task 또는 continuation turn으로 계산하지 않는다.

**필수 증명**

- fake app-server serialization 및 correlation test;
- 오래됨/조정 불가/실패 시 보존 테스트;
- 코어 리듀서 테스트;
- running-turn TUI snapshot 및 terminal capture;
- approval input ownership에 regression 없음.

<a id="p0-active-turn-exit-and-restart-recovery"></a>
### P0: 활성 턴 종료 및 재시작 복구

**소유 경계**

- app-server 연결/런타임 생명주기
- conversation application service 및 core recovery state
- 새 recovery-journal port 및 SQLite adapter
- TUI 종료/시작 흐름
- parallel worker supervision 및 persisted session detail

**계약**

- `thread/start` 전에 workspace, catalog watermark, requested thread metadata, request nonce, session kind를
  포함하는 new-thread intent를 durably write한 뒤 반환된 thread ID를 CAS-bind한다;
- `turn/start` 전에 thread ID, preceding-turn identity, input fingerprint, request nonce, session kind를
  포함하는 start intent를 durably write한다;
- main 또는 parallel turn을 running으로 projection하기 전에 반환된 turn ID를 해당 intent에 CAS-bind하고,
  projection 전에 last-observed 및 terminal event를 persist한다;
- `thread/start` 중 crash 뒤에는 실제 노출되는 workspace, time/watermark, thread metadata만 사용해 recent
  official-session catalog를 reconcile한다. 이 단계에는 input이 없으므로 unique catalog match만 bind하고
  아니면 unknown으로 남는다;
- remote acceptance와 turn-ID binding 사이 crash 뒤에는 preceding turn 및 input fingerprint에 대해
  `thread/read`를 reconcile한다. unique match만 bind하고 아니면 explicit unknown state를 유지한다;
- running turn이 있는 controlled TUI exit에서 bounded wait, protocol interrupt-and-exit, explicit force-exit
  path를 제공한다. child termination을 successful completion으로 조용히 표시하지 않는다;
- transport loss, child death, process exit 시 replacement child를 시작하기 전에 last observed identity와 함께
  turn을 `recovery_pending`으로 표시한다;
- restart 뒤 official thread truth를 읽고 가능하면 terminal result를 정확히 한 번 apply한다. 아니면 prior
  turn을 interrupted 또는 unknown으로 표시하고 explicit operator decision을 요구한다;
- start intent 또는 prior outcome이 unknown인 동안 replacement turn을 자동 submit하지 않는다;
- lease recovery 또는 retry 전에 parallel worker에도 동일 terminal/unknown 구분을 적용한다;
- official attach contract를 구현하고 재현하지 않는 한 Akra daemon을 도입하거나 client-independent
  continuation을 주장하지 않는다.

**필수 증명**

- wait, interrupt, force exit, transport loss, child death, restart를 다루는 fake-runtime test;
- intent persistence 전, RPC acceptance 후 ID binding 전, binding 후, last-event/terminal persistence
  전후의 crash-boundary test;
- unique-match, no-match, ambiguous session-catalog 및 `thread/read` reconciliation fixture;
- automatic duplicate turn 또는 duplicate completion이 없음을 입증하는 main 및 parallel recovery test;
- explicit unknown 및 recovered state가 있는 TUI shutdown/restart snapshot;
- pinned app-server version에 대한 실제 child-kill/restart capture.

<a id="p0-frozen-source-validation-evidence"></a>
### P0: 고정 소스 검증 증거

**소유 경계**

- 새 domain validation-evidence 및 policy type
- 새 application validation service 및 outbound runner/checks port
- host subprocess 및 GitHub-check adapter
- 병렬 세션 상세, distributor 준비 상태, TUI, Admin 투영

**계약**

- 현재 planning-file-change `validation_summary`를 test 또는 check가 실행됐다는 proof가 아닌 legacy
  context로 취급한다;
- command 및 required-check policy를 operator-owned authority 또는 frozen integration-base revision에서
  load하고 candidate-controlled configuration에서는 절대 load하지 않는다. candidate policy change가 자체
  validation을 authorize할 수 없다;
- worker가 중지한 뒤 exact frozen source SHA 또는 CAS-bound re-freeze candidate의 clean worktree에서
  host-owned runner로 configured local validation을 실행한다. 또는 해당 exact SHA에 bind된 explicitly
  trusted CI check를 consume한다;
- publication 전에 configured local validation을 evaluate하여 clear 또는 not-required로 표시한다. trusted
  CI는 lease-bound publication/PR creation 뒤에만 collect하고 모든 required phase가 clear할 때까지
  integration을 block한다;
- candidate code, build script, test binary를 untrusted로 취급한다. credential과 ambient environment를
  scrub하고 network를 default-deny하며 write를 candidate worktree 및 private temp/cache root에 한정하고
  process-tree termination과 함께 timeout, output, memory/CPU, child-process limit을 enforce한다;
- required sandbox가 없는 platform에서는 candidate code를 operator host에서 직접 실행하는 대신 local
  validation unavailable을 보고한다;
- local command와 remote check를 구분하고 command/check identity, source SHA, start/finish time,
  exit/conclusion, runner/tool version, artifact reference, content hash를 저장한다;
- worker prose, unbound artifact, dirty 또는 moved source, unknown/failing required validation을 delivery
  readiness로 거부한다;
- source tip 또는 frozen range가 바뀔 때마다 prior validation evidence를 invalidate한다;
- gate state를 summary에서 도출하지 않고 presentation을 위한 bounded human summary를 유지한다.

**필수 증명**

- success, failure, timeout, missing, malformed, stale-SHA evidence용 fake runner/check adapter test;
- 깨끗한 worktree/소스 식별, 기준선 결합 정책, 후보 정책 변조, 명령 정책 테스트;
- 자격 증명/환경 읽기, 루트 밖 쓰기, 네트워크 접근, fork된 자식 프로세스, 자원 고갈, 과대 출력,
  불완전한 프로세스 트리 정리를 다루는 적대적 runner 테스트;
- execution, artifact persistence, readiness transition 사이 crash recovery;
- missing/failing validation이 block하고 exact-SHA success가 unblock함을 입증하는 distributor test;
- TUI/Admin projection 및 실제 frozen-commit validation artifact 하나.

<a id="p1-authoritative-delivery-outcomes"></a>
### P1: 권위 있는 전달 결과

**소유 경계**

- domain delivery-attempt outcome 및 failure-classification type
- 계획 권한 포트, SQLite 저장소, 마이그레이션
- 애플리케이션 결과 명령/CAS 서비스
- distributor 복구/실패 분류
- TUI, CLI, Admin outcome projection 및 command

**계약**

- 각 delivery attempt에 task 및 prior attempt와 연결된 immutable identity를 부여한다;
- terminal outcome을 `integrated`, `terminal_failed`, `canceled`로 정의하고 skipped 또는 superseded work는
  별도로 보고한다;
- canonical remote integration ref를 fetch 또는 inspect하고 expected applied commit과 같음을 verify한 뒤에만
  `integrated`를 finalize한다. local application 및 push는 provisional state다;
- PR close, source-branch deletion, worktree cleanup, slot cleanup을 post-integration finalization으로
  추적한다. 이들의 failure는 recoverable로 남고 verified `integrated` outcome을 rewrite하지 않는다;
- manual recovery가 가능하면 retryable `blocked`, automatic-retry exhaustion, internal `failed` label을
  nonterminal로 유지한다;
- application-owned non-retryable classification 또는 authenticated explicit abandon command에서만
  `terminal_failed`를 허용하고 actor, reason, attempt, timestamp를 기록한다;
- compare-and-set을 통해 attempt를 한 번 finalize한다. 나중의 reopen은 old outcome을 rewrite하지 않고 linked
  attempt를 만든다;
- attempt 내부 retry를 reopen attempt 및 final outcome과 별도로 노출한다.

**필수 증명**

- retryable, non-retryable, abandon, cancel, integrate, reopen path용 transition test;
- local apply, push failure, remote-ref mismatch, remote verification, post-integration cleanup recovery용
  boundary test;
- stale-version, duplicate-command, concurrent-finalization, crash-recovery test;
- persistence migration 및 round-trip test;
- TUI/CLI/Admin projection 및 authorization test;
- blocked recovery 또는 abandon에서 final immutable attempt outcome까지의 audit trail 하나.

<a id="p1-admin-operational-metrics"></a>
### P1: Admin 운영 지표

**소유 경계**

- 새 `src/domain/operational_metrics.rs`
- 새 application-owned outbound metrics port
- SQLite 생명주기 집계 어댑터
- `src/adapter/inbound/admin_api/akra_dashboard.rs`
- Admin JSON/template 및 dashboard script

**계약**

다음을 persist하고 aggregate한다.

- daily integrated 및 explicitly finalized terminal-failed delivery attempt;
- 전달 시도 성공률;
- queue wait;
- dispatch부터 running까지;
- running부터 reported completion까지;
- commit ready부터 integrated까지;
- interval별 p50 및 p95;
- selected work의 lifecycle stage `n/N`.

data가 존재하기 전에는 현재 `None`과 `unmeasured` value를 정직하게 유지한다. UI가 human-readable event
summary를 parse하여 metric을 도출해서는 안 된다.

metric semantic은 contract의 일부다.

- timestamp를 UTC로 저장하고 explicit configured IANA timezone에서 daily bucket을 계산하며 default는
  UTC다;
- Authoritative Delivery Outcomes의 immutable outcome을 consume한다. finalized `integrated +
  terminal_failed` attempt를 success-rate denominator로 사용하고 canceled/skipped work를 별도로 보고한다;
- within-attempt retry와 linked reopen attempt를 별도 measure로 노출하면서 각 attempt의 final outcome을 한
  번 계산한다;
- still-running/censored work는 completed-duration percentile에서 제외하되 count를 보고한다;
- nearest-rank percentile을 사용하고 항상 sample count를 보여 주며 completed sample이 최소 20개가 될
  때까지 p95를 보류한다.

**필수 증명**

- day boundary 전반의 injected-clock test;
- timezone, attempt/retry/reopen, cancellation, censoring, percentile, empty-store test;
- Admin JSON 및 template test;
- desktop 및 mobile Playwright capture;
- game-board build 및 visual check.

<a id="p1-guarded-admin-control-actions"></a>
### P1: 보호된 Admin 제어 액션

**소유 경계**

- application control-plane command 및 port
- authenticated Admin POST handler 및 form
- Admin 보안, API, 감사 이벤트 투영

**계약**

- terminal abandon/reopen에는 Authoritative Delivery Outcomes command를 재사용하고, distributor pause/resume,
  blocked-delivery retry, review handoff, targeted cleanup recovery를 위한 나머지 application-owned command를
  정의한다;
- Admin을 통해 해당 command만 노출하고 TUI/CLI에서도 재사용 가능하게 한다;
- authenticated session, CSRF protection, explicit target/version, idempotency 또는 compare-and-set semantic을
  요구한다;
- actor, request, result, resulting lifecycle event를 persist한다;
- HTTP handler에서 SQLite, git, worktree, GitHub를 직접 변경하지 않는다;
- destructive 또는 remote-write action에는 confirmation을 사용하고 정직한 unavailable state를 유지한다.

**필수 증명**

- authorization, CSRF, stale-version, idempotency, concurrent-action test;
- non-Admin surface와 공유하는 application-service transition test;
- Admin API/template test 및 desktop/mobile Playwright capture;
- request에서 projection까지 실제 pause/resume 또는 blocked-retry audit trail 하나.

<a id="p1-parallel-activity-and-completion-evidence"></a>
### P1: 병렬 활동 및 완료 증거

**소유 경계**

- parallel session-detail domain 및 store
- control-plane 및 supervisor projection
- TUI 병렬 상세
- Admin 태스크/에이전트 상세

**계약**

- 이미 출시된 lifecycle history, completion/review/cleanup state, blocked reason, conflict-file projection,
  distributor timeline을 보존한다;
- lifecycle age와 last app-server activity를 구분한다;
- bounded current app-server item/tool activity 및 declared hotspot ownership을 저장한다;
- Frozen-Source Validation Evidence가 만든 exact-SHA gate와 bounded summary를 projection한다. 기존 free-form
  field는 labeled legacy context로만 보존한다;
- active slice가 named hotspot과 겹치면 allocation 전에 경고한다;

**필수 증명**

- persistence 및 recovery test;
- 동시 할당 충돌 테스트;
- supervisor/TUI/Admin 투영 테스트;
- 실제 two-lane delivery capture.

<a id="p1-critical-review-response-loop"></a>
### P1: 중요 리뷰 대응 루프

**소유 경계**

- `src/application/service/github_review_poller_service.rs`
- GitHub review port/adapter 및 identity guard
- 병렬 전달 선점, 대기열 리비전, 고정 소스 상태
- review-response turn 및 Frozen-Source Validation Evidence service
- Admin review center 및 TUI review projection

**계약**

- current diff context 및 resolution state와 함께 unresolved inline thread를 load한다;
- reviewer body, linked URL, diff context를 typed untrusted data로 나타낸다. system/developer instruction,
  shell command, validation policy, task authority에 interpolate하지 않는다;
- 각 comment를 valid, stale, incorrect, out of scope로 classify하고 rationale을 기록한다;
- delivery record가 여전히 review wait 상태일 때만 valid-fix transition을 accept한다. claim을 acquire하고
  expected frozen tip, remote branch, PR head, thread version을 비교한다;
- owning worktree에서 scoped review-response turn을 실행하고 실용적인 경우 separate clean commit을 만든다;
- old tip, candidate tip, idempotency key를 포함하는 re-freeze intent를 persist하며 old tip의 approval,
  required-check, clean-merge, validation readiness를 즉시 invalidate한다;
- publication 전에 exact candidate에 대해 policy-required local validation을 실행한 뒤 old remote tip에
  bind된 lease로 push한다;
- remote branch와 PR head가 intent와 일치한 뒤 candidate에 대한 required trusted-CI evidence를 collect하고
  full validation policy가 clear일 때만 old frozen-source revision을 atomically supersede한다. crash recovery는
  이 ordering을 보존해야 한다;
- 새로 frozen된 PR head에 대해 fresh default approval, CLEAN, required-check gate를 요구한다;
- integration이 시작됐거나 source가 이동했거나 다른 response가 CAS를 이겼으면 fail closed한다;
- GitHub credential 없이 owning task의 기존 sandbox 및 scope 아래에서 response turn을 실행한다. reviewer
  request는 tool, network access, file ownership, accepted intent를 확장할 수 없다;
- stale 또는 incorrect feedback은 code를 보존하고 concise evidence-backed rationale을 게시한다;
- intended GitHub identity 및 repository target을 verify한 뒤에만 reply 또는 resolve한다;
- comment, decision, commit, validation, reply, final thread/check state를 linked 및 idempotent 상태로
  유지한다;
- reviewer text를 architecture, safety, operator intent를 우회하는 instruction으로 절대 취급하지 않는다.

**필수 증명**

- unresolved/resolved, outdated diff, valid, incorrect, duplicate thread case용 fixture;
- prompt injection, scope escape, malicious patch context, credential request, shell text, URL, task 또는
  validation policy rewrite attempt용 adversarial fixture;
- identity mismatch 및 remote-write failure test;
- competing-response 및 integration-start race test;
- validation, re-freeze intent, push, queue-CAS, reply boundary 전반의 retry/idempotency test;
- old-tip approval, check, clean state, validation이 new tip을 authorize할 수 없음을 입증하는 test;
- comment에서 decision, commit/reply, refreshed final state까지 입증하는 실제 PR trail 하나.

<a id="p2-official-session-search-and-provenance"></a>
### P2: 공식 세션 검색 및 출처

**소유 경계**

- app-server 세션 카탈로그 포트/어댑터
- session application service 및 core projection
- 계획 출처 쿼리
- TUI session browser 및 Admin session detail

**계약**

- 출시된 query, project filter, local paging, preview, source/model/status/branch display, session selection
  behavior를 보존한다;
- 기존 result를 duplicate하거나 reorder하지 않고 provider-cursor `load more`를 추가한다;
- explicit time-range filtering을 추가한다;
- session과 연결된 accepted direction/task 및 delivery result를 보여 준다;
- imported, non-resumable, official resumable session을 classify하고 non-resumable context는 read-only로
  유지한다;
- 이 slice에 embedding을 추가하지 않는다.

**필수 증명**

- provider-cursor, deduplication, time-filter, provenance, classification, stale-session test;
- 저장소 간 격리 테스트;
- TUI 좁은/넓은 화면 스냅샷;
- accepted task에서 integrated PR까지 provenance round trip.

<a id="recommended-order"></a>
## 권장 순서

1. 하나의 scripts/validation lane이 다른 live lane과 겹치지 않고 기존 capture schema를 소유할 수 있을 때
   performance evidence contract를 확립한다.
2. 첫 app-server/core control-path change로 active turn steering을 구현한다.
3. steering 뒤 동일 ownership lane에서 automatic duplicate submission 또는 completion 없이 controlled exit,
   child loss, restart가 active main 및 parallel turn을 reconcile하게 한다.
4. disjoint protocol/TUI lane에서 delta, context, full-diff drilldown으로 live execution rail을 확장한다.
   별도 PR로 step 2-3과 함께 진행할 수 있다.
5. planning-file summary를 frozen-source validation evidence 및 실제 delivery gate로 교체한다.
6. crash-safe source re-freeze와 fresh validation/review/check gate를 포함해 critical GitHub review-response
   loop를 닫는다.
7. metric을 만들기 전에 authoritative delivery-attempt outcome을 추가한다.
8. shared application service를 통해 Admin operational metric 및 guarded control action을 추가한다.
9. 누락된 parallel activity evidence 및 hotspot collision warning을 추가한다.
10. semantic memory를 고려하기 전에 session provider-cursor/time/provenance gap을 추가한다.

## 성공 감사

이 비교는 이후 Akra evidence가 다음을 모두 입증할 때만 가치를 만든다.

- user가 running official Codex turn을 보고 steer하며 retained diff를 inspect할 수 있다;
- controlled exit, child failure, restart가 duplicate work 없이 prior active turn을 reconcile한다;
- first-frame/input/readiness/stream 및 platform-labeled process-tree memory performance가 versioned되고
  repeatable하다;
- TUI execution state는 dense, bounded, responsive하며 host scrollback을 오염시키지 않는다;
- required validation은 exact frozen source SHA에 bind된 host-run 또는 trusted-CI evidence다;
- 모든 parallel lane이 activity, validation, PR presence, review/check state 또는 explicit skipped-gate policy,
  integration, cleanup truth를 노출한다;
- valid review feedback은 fresh validation/review/check gate가 있는 crash-safe source revision과 traceable
  decision, commit 또는 rationale, reply, final thread state를 갖는다;
- operational failure rate는 authoritative finalized `integrated` 또는 `terminal_failed` delivery attempt만
  계산한다;
- Admin game progression은 stored lifecycle data로 구동되고 guarded action은 shared application service를
  사용한다;
- multi-provider runtime, same-checkout swarm, speculative memory graph가 이 목표를 밀어내지 않는다.
