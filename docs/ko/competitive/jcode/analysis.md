# jcode v0.43.0 심층 분석

[English](../../../competitive/jcode/analysis.md)

이 분석은 jcode v0.43.0과 prerelease 커밋 `66333152170124a42aca6f49ed2f72fa6a8293d7`의
Akra를 비교한다. 소스 링크, 명령, 개수, 제한 사항은 [evidence.md](evidence.md)에 있다. 제품 결정과
구현 단위는 [gap-matrix.md](gap-matrix.md)에 있다.

## 총괄 결론

jcode는 "TUI가 있는 네이티브 Rust Codex 래퍼"가 더는 하나의 입지가 아니라는 가장 강력하고
직접적인 경고다. 광범위한 에이전트 런타임을 소유하고, 주장하는 속도, 공유 다중 세션 아키텍처,
고밀도 TUI 계측, 메모리, 공급자 선택권, 재귀적 swarm 조정을 경쟁력으로 삼는다. 모델을 둘러싼
세련된 채팅 화면에 불과하지 않다.

Akra보다 강한 장점은 다음과 같다.

1. 공개 제품 가치로 성능을 다루고 실행 가능한 측정 스크립트로 뒷받침한다. 다만 공개된 수치는
   현재의 공정한 비교에 충분하지 않다.
2. TUI는 우선순위와 공간을 고려한 계측으로 모델, 컨텍스트, 메모리, git, 작업 공간, 다이어그램,
   todo, 백그라운드 작업, swarm 상태를 노출한다.
3. daemon과 프로토콜은 세션 간 공유 상태 비용을 분산하고 쉽게 attach, reconnect, inspect,
   coordinate하도록 설계되었다. 다만 이 조사에서는 그 비용을 재현하지 않았다.
4. 공급자, 도구, hook, 메모리, swarm의 폭은 power user가 하나의 하네스 안에 머물 이유를 많이 준다.

공략할 수 있는 약점도 똑같이 구조적이다.

- jcode는 provider, auth, tool, memory, daemon, TUI, desktop, swarm, self-development 스택을
  소유한다. 이로 인해 보안 및 유지보수 표면이 매우 커진다.
- 현재 동작과 제안된 동작이 같은 제품 설명에서 서로 인접해 있는 경우가 많다.
- 동일 저장소 swarm 조정은 강력하지만 그 자체로 격리된 전달, 리뷰 반영 또는 보호된 base로의 선형
  통합을 증명하지 않는다.
- 조사한 릴리스는 광범위한 품질 도구가 있는데도 저장소에 포함된 코드 크기 ratchet을 실패한다.
- 공개된 성능 데이터는 이전 jcode 빌드, 불완전한 환경 증거를 사용하며 추적되는 원시 결과
  artifact가 없다.

Akra는 jcode의 런타임 범위를 복제하는 방식으로 대응하면 안 된다. 더 좁고 날카로운 제품으로
대응해야 한다.

> 기본적으로 의도에서 검토된 통합까지 공식 Codex 세션을 운영하는 가장 빠르고 신뢰할 수 있는 방법.

이를 위해 Akra는 측정 및 상호작용 격차를 줄인 다음 jcode가 중심에 두지 않는 요소를 확대해야 한다.
Codex 프로토콜 충실도, 운영자 소유 planning authority, worktree 격리, 직렬화된 reviewed-range 전달,
중요 리뷰 처리, TUI와 Admin의 공유 운영자 진실이 그것이다. Akra의 명시적인 상위 수준 자율 전달
opt-in은 review/check gate와 적격 PR 모드에서는 PR 자동화 자체도 우회할 수 있다. 이러한 고위험
탈출 경로는 검토 전달 약속을 이행한 것처럼 제시하지 말고 review-skipped로 분명히 유지해야 한다.

## jcode의 실제 모습

jcode는 자신을 다중 세션 워크플로, 사용자화, 성능을 위한 차세대 coding-agent 하네스라고 설명한다.
소스는 이 설명을 뒷받침한다. 이 조사의 거친 계산 규칙에 따르면 약 518k production Rust LOC를 가진
77-crate Rust workspace다.

제품이 소유하는 것은 다음과 같다.

- 장시간 실행되는 server/daemon 및 로컬 클라이언트 프로토콜
- 여러 공급자 및 subscription login 경로
- 에이전트 및 도구 런타임
- TUI 표현, 터미널 이미지, Markdown, Mermaid, 권한, 계정, 세션, 사용량, workspace crate
- 메모리 추출, 검색, 그래프 타입, 임베딩, 세션 검색
- swarm coordination, messaging, plan graph, task state, 선택적 worktree 동작
- background, ambient, overnight, telemetry, self-development, gateway, desktop, mobile-facing 방향

이러한 폭 자체가 제품이다. TUI 클라이언트라고 부르면 경쟁 위협을 과소평가하고 유지보수 비용도
설명하지 못한다.

## v0.11.2 이후의 진화

이전 내부 jcode 분석은 v0.11.2를 기준으로 했다. 해당 태그와 v0.43.0 사이에서 다음과 같이 변했다.

- 추적되는 Rust 파일: 735개에서 1,084개
- Rust LOC: 336k에서 632k
- root + workspace crate: 35개에서 77개
- root `src` Rust 파일: 코드가 crate 경계 뒤로 이동하면서 631개에서 45개
- TUI 계열 Rust LOC: 약 120k에서 213k
- desktop 계열 Rust LOC: 약 12k에서 72k

이는 피상적인 churn이 아니다. jcode는 전용 TUI presentation crate, 공유 render model, smoothness 및
anchor-stability 작업, 확장된 공급자 런타임과 진단, 훨씬 큰 desktop prototype, DAG 중심 swarm
방향을 추가했다.

Akra가 대비해야 하는 경쟁 속도도 드러난다. 일회성 기능 비교는 빠르게 낡는다. 지속되는 교훈은
jcode의 투자 패턴이다. 성능 측정, 더 풍부한 운영자 가시성, 더 자율적인 병렬 작업에 지속적인 제품
작업을 투입한다.

모듈화 결과는 혼재되어 있다. 추출로 root crate는 줄었지만 `jcode-tui`, `jcode-app-core`,
`jcode-base`를 지나는 큰 직렬 의존성 spine은 남아 있다. 범위 증가는 더 많은 경계 뒤로 이동했지만
큰 소유 단위를 제거하지 못했다. Akra는 측정과 집중된 crate/module 규율을 따라야 하며, crate가
늘어나면 넓은 범위가 저렴해진다는 가정은 따라 하면 안 된다.

## 런타임 아키텍처

### jcode

jcode는 하나의 server process를 세션 및 런타임 authority로 사용한다. TUI client는 로컬 socket으로
연결하고, 보통 한 세션에 attach하며, server reload 또는 transport loss 후 reconnect한다. server는
provider state, tool, memory work, persistence, background work, swarm state를 소유한다.

이 아키텍처는 사용자에게 중요한 결과를 만든다.

- 추가 client가 완전한 런타임을 하나 더 시작하는 대신 server-owned state를 재사용할 수 있다. 낮은
  증분 비용은 재현하기 전까지 topology 기반 추론이다.
- client를 종료해도 session runtime은 종료되지 않는다.
- server가 session과 file-read/file-change 인식을 중앙에서 조정할 수 있다.
- self-development가 server binary를 교체하고 client를 재연결할 수 있다.
- protocol snapshot이 풍부한 session 및 swarm status 화면을 투영할 수 있다.

제안된 multi-surface client와 custom desktop은 이 토폴로지를 확장하지만, 이번 조사에서 하나로
검증되어 일반 배포된 workspace product는 아직 아니다.

### Akra

Akra는 의도적으로 모델 및 도구 런타임 authority를 공식 `codex app-server`에 위임한다. 자신이
소유하는 도메인은 해당 런타임 주변의 운영자 워크플로다.

- `src/core`가 app command, effect, completion, stream, snapshot을 조정한다.
- 애플리케이션 서비스가 conversation, planning, GitHub review polling, continuation, parallel
  delivery를 소유한다.
- SQLite가 accepted planning, queue, lease, runtime event, session detail을 소유한다.
- filesystem planning state는 운영자가 편집할 수 있는 mirror다.
- worktree와 GitHub 어댑터는 기본적으로 reviewed PR 경로를 사용하고 명시적인 상위 수준 고위험
  autonomous exception을 두어 병렬 태스크를 `prerelease`로 전달한다.

이는 소유하는 신뢰 경계가 더 좁지만 현재 제품 설명은 더 약하다. Akra가 방어해야 할
provider/tool/auth 런타임의 양을 줄이지만, 이 조사에서는 비교 threat model을 실행하지 않았으므로
보안 승자를 선언하지 않는다. 사용자는 jcode의 단일 daemon을 capability multiplier로 본다. Akra가
Codex 네이티브 기능과 전달 권한을 눈에 보이게 투영하지 않으면 더 느린 추가 shell처럼 보일 수 있다.

Akra의 세션 연속성에도 단단한 프로세스 경계가 있다. 공식 thread history는 resume할 수 있지만
connection은 app-server child를 소유하고 drop 또는 transport failure 시 종료한다. 런타임 reconnect는
새 child를 시작한다. 이 조사에서는 이전 child에서 이미 실행 중인 turn에 다시 attach하는 경로를
찾지 못했다. 따라서 현재 연속성은 thread 및 transcript 연속성이지 client-independent active-turn
survival이 아니다.

### 결정

런타임 authority로 `codex app-server`를 유지한다. 병렬 provider/tool daemon을 만들지 않는다. 대신
app-server 경계가 attach 및 observe 가능한 것처럼 느껴지게 만든다.

- 타입이 있고 resumable한 core snapshot을 보존한다.
- 더 많은 protocol-native execution event를 소비한다.
- turn 경계를 기다리지 않고 live turn steering을 노출한다.
- 지원되지 않는 background survival을 주장하지 않으면서 active-turn exit, child-failure, restart
  recovery를 정의한다.
- 전체 TUI + app-server process tree를 측정한다.
- overlay를 연속해서 열지 않고 planning, delivery, review state를 볼 수 있게 한다.

## TUI 및 상호작용 설계

### jcode가 앞선 부분

jcode의 TUI에는 응집력 있는 계측 전략이 있다. `WidgetKind`는 workspace, todo, context, memory,
swarm, background work, compaction, usage, cache, model, diagram, ambient work, tip, git에 명시적인
우선순위, 선호 방향, 최소 높이를 할당한다. 동적 상태가 widget 우선순위를 높일 수 있다. 이는 panel을
추가하는 것 이상이다. 레이아웃이 부족한 terminal space를 어디에 쓸지 결정한다.

side panel은 에이전트가 사용하는 기능이기도 하다. Markdown을 load하거나 receive하고, 파일을
follow하고, diff surface로 동작하며, focus를 받고, diagram을 렌더링할 수 있다. Session selection과
TUI test는 광범위한 화면을 다룬다. Custom scrollback은 더 풍부한 in-app 동작을 허용하며, jcode는
부드러운 부분 줄 스크롤에 관한 terminal 한계를 공개적으로 인정한다.

가장 강한 패턴은 **점진적 계측**이다.

- 일반 상태는 작고 지속적으로 유지된다.
- 세션을 떠나지 않고 더 풍부한 상세를 볼 수 있다.
- usage, memory 또는 관리되는 swarm 작업이 긴급해지면 우선순위가 바뀐다.
- TUI는 작업 목록만 보여 주는 대신 병렬 작업에 공간감을 줄 수 있다.

### Akra가 앞선 부분

Akra의 inline shell은 host-terminal scrollback을 영속적인 이력으로 보존한다. terminal adapter,
scroll-region insertion, vt100 coverage, Windows Terminal/WSL 검증 방법, 명시적인 layering contract는
alternate-screen app을 가정하지 않고 실제 terminal 동작을 다룬다.

Akra에는 실제 병렬 전달 상태에 연결된 목적별 supersession board와 선택 session timeline도 있다.
모든 런타임 하위 시스템에 generic widget을 도입할 필요가 없다.

### Akra가 뒤처진 부분

Akra는 완료된 `commandExecution` 및 `fileChange` item을 current/last-turn count와 최신 요약을 갖춘
거친 activity line으로 이미 축약한다. 하지만 더 가치가 높은 `codex app-server` 알림을 deferred 또는
diagnostic-only로 남겨 두므로 운영자는 이 rail에서 item start, command-output delta, patch progress,
aggregated turn diff, plan change, context pressure를 볼 수 없다. Aggregated turn diff를 보존하고
drilldown에서 검사하는 protocol-native 경로가 없다. 고정된 프로토콜에 `turn/steer`가 있어도 실행 중
입력은 turn이 끝날 때까지 기다린다.

Akra의 overlay 중심 기능 증가는 운영자가 다음 관련 사실을 보는 대신 "어디서 열어야 하지?"라고
묻게 만들 수 있다. 더 엄격한 정보 계층이 필요하다.

1. conversation 및 host scrollback을 primary로 유지한다.
2. live execution과 context pressure를 제한된 status rail에 표시한다.
3. 활성 상태에서는 planning 및 parallel state에 persistent compact projection을 사용한다.
4. detail view는 permanent dashboard chrome이 아니라 선택형 drilldown으로 유지한다.
5. 좁은 terminal은 임의 truncation이 아니라 priority에 따라 내용을 접는다.

해답은 jcode의 widget framework를 이식하는 것이 아니라 Akra 소유 사실에 priority 규율을 적용하는
것이다.

## 성능

### jcode의 장점

jcode는 startup과 memory를 공개 제품 정체성의 일부로 만든다. 저장소에는 PTY startup/input 측정,
PSS 측정, startup budget 검사, memory regression gate, render benchmark, compile probe, code-size
ratchet, 광범위한 TUI test가 있다. 개별 공개 주장을 검토할 필요가 있어도 강한 engineering feedback
loop를 만든다.

공개된 값에는 first frame까지 평균 14.0 ms, first input까지 평균 48.7 ms, local embedding을
비활성화한 상태의 27.8 MB PSS, session당 약 9.9 MB의 추가 PSS가 포함된다. 다음 이유로 이 숫자를
현재 비교 사실로 받아들이지 않는다.

- memory table은 조사한 v0.43.0 릴리스가 아니라 jcode `v0.9.1888-dev`를 명시한다.
- startup은 "this Linux machine"과 PTY 실행 10회만 명시한다.
- 원시 JSON과 완전한 machine/terminal stamp가 table과 함께 추적되지 않는다.
- 한 경쟁 제품은 다른 input-ready signal을 사용했고 인증되지 않았다.
- Akra 비교에 필요한 daemon warm state와 전체 process-tree attribution이 공개 table에 충분히
  드러나지 않는다.

올바른 결론은 "jcode가 Akra보다 63x 빠르다"가 아니다. "jcode는 성능 주장을 할 수 있지만 현재
Akra는 할 수 없다"가 올바른 결론이다.

### Akra의 격차

Akra에는 실제 terminal 검증과 즉각적인 first frame을 요구하는 scheduler test가 있지만 다음을 위한
반복 가능한 benchmark가 없다.

- process spawn에서 처음 보이는 shell frame까지
- first frame에서 prompt echo까지
- process spawn에서 app-server ready-to-submit까지
- submit에서 first protocol event 및 first assistant delta까지
- 전체 Akra + app-server process tree의 idle 및 streaming PSS
- resumed session 및 3-slot parallel pool에서의 memory scaling
- 빽빽한 tool output 중 frame cost 및 event backlog

이를 측정하기 전까지 "native"는 성능 장점이 아니라 구현 세부 사항이다.

### 필요한 대응

Akra는 최적화 전에 증거 계약을 만들어야 한다. raw sample, 정확한 binary 및 Codex version, auth
state, daemon state, terminal geometry, environment stamp, process tree, p50/p95를 저장해야 한다.
초기 gate는 Akra 자체 기준선에 대해 ratchet해야 한다. 동일한 하네스가 두 도구를 공정하게 실행할
수 있을 때까지 제품 간 주장을 미뤄야 한다.

## 병렬 작업과 전달

### jcode swarm

jcode의 구현 중이거나 전환 중인 swarm 화면은 야심 차다.

- parent/report-back 소유권을 갖춘 재귀적 spawning
- 직접 메시지, 브로드캐스트, 범위 지정 채널
- lifecycle, activity age, current tool, token, todo, completion report 투영
- 동일 저장소 read/change notification
- 문서화된 선택적 worktree-manager 역할. 자동화된 worktree 생성/통합은 소스에서 검증되지 않음
- legacy coordinator-gated shared plan과 live owner-partitioned DAG operation의 공존. migration은
  여전히 진행 중
- ready, blocked, failed, cyclic, unresolved, confidence, growth state를 갖춘 DAG 모델

이는 에이전트에 저마찰 조정을 제공하고 TUI가 그룹이 지금 하는 일을 보여 줄 수 있게 한다. v0.43.0의
asynchronous swarm-wait 변경은 병렬성이 커질 때 coordinator 응답성을 지키는 좋은 예다.

동일 저장소의 모든 충돌을 자동으로 해결한다는 README의 더 강한 주장은 아키텍처 계약으로
뒷받침되지 않는다. 해당 계약은 optimistic, lock-free detection 후 직접 agent negotiation을
설명한다. 이를 deterministic conflict resolution이 아니라 coordination assistance로 취급한다.

Deep mode는 1,000-member swarm cap까지 unbounded recursion 및 node별 fan-out으로 구성된다. constant와
cap 검사는 구현되어 있지만 이 조사에서는 1,000-agent live load를 찾거나 재현하지 못했다. 그 규모는
기능인 동시에 운영 비용 및 rate-limit 위험이다.

### Akra 병렬 모드

Akra의 병렬 시스템은 더 좁지만 전달에서는 더 강하다. 기본 경로는 다음과 같다.

- accepted task가 operator-owned planning authority에서 나온다.
- 하나의 slot/worktree/branch가 reviewable slice를 소유한다.
- lease 및 session detail을 영속적으로 저장한다.
- completion이 공식 session truth를 갱신한다.
- Git/GitHub 어댑터가 source range를 고정하고, push하고, PR을 열고, 고정된 head에 대해 review와
  check를 검증하고, 직렬화된 integration worktree로 range를 적용하고, integration ref를 push하고,
  PR을 닫고, slot을 정리한다.
- supervisor 및 distributor state를 TUI와 Admin에 투영한다.

이는 fail-closed 기본값이지 유일하게 구성 가능한 경로는 아니다. 명시적인 상위 수준 고위험
autonomous-delivery opt-in을 사용하면 Akra는 PR을 유지할 때도 approval, clean-merge, required-check
gate를 건너뛴다. 적격 PR 모드에서는 PR 자동화도 건너뛰고 직접 통합할 수 있다. 이러한 예외는 통합
mechanic을 보존하지만 리뷰 증거를 약화하므로 운영자 화면에 policy, PR presence, skipped gate를
기록해야 한다.

Worktree 격리는 jcode가 알림으로 조정하려는 same-checkout file-shift 문제를 방지한다. Akra는 이
장점을 보존해야 한다.

### 차용할 것

동일 checkout 협업이 아니라 더 풍부한 activity 및 completion 의미를 차용한다.

- slot별 typed current tool 및 제한된 최근 활동
- lifecycle-state age와 구분되는 last-activity age
- 자유 형식 "done" 요약이 아닌 구조화된 completion evidence
- 명시적인 failed reason 및 blocked dependency 투영
- 계획된 두 slice가 겹치는 hotspot을 명시할 때의 사전 ownership/collision warning
- 다른 slot을 선택하거나 감독할 때 coordinator를 멈추지 않는 nonblocking wait 경로

agent DM을 primary UX로 추가하지 않는다. operator, planning authority, integration queue를 조정
source로 유지해야 한다.

## 메모리와 연속성

jcode의 메모리 시스템은 의미 있는 제품 투자다. automatic extraction, semantic retrieval, optional
verification, explicit memory tool, session search, UI activity, graph direction을 갖는다. Core memory는
구현된 것으로 문서화되어 있고 완전한 hybrid graph는 계획 상태다.

Akra에는 영속적인 planning 및 session continuity가 있지만 동등한 cross-session semantic memory는
없다. jcode의 graph를 복사하는 것은 잘못된 첫 단계다.

- embedding은 CPU, memory, packaging, privacy, invalidation, relevance 의무를 추가한다.
- automatic user memory는 신뢰 및 수정 요구 사항을 만든다.
- `codex app-server`가 이미 conversation/session truth를 소유한다.
- Akra에서 가장 강한 continuity primitive는 추론된 personal memory graph가 아니라 accepted operator
  intent와 delivery history다.

Akra는 먼저 출처를 명시적으로 만들어야 한다.

- 어떤 accepted direction, task, prior session이 continuation을 만들었는지 보여 준다.
- imported data에 replay fidelity를 주장하지 않고 공식 Codex session 및 summary를 index한다.
- review feedback과 delivery outcome을 구조화된 operational history로 유지한다.
- superseded planning state를 조용히 교체하지 않고 노출한다.

측정된 search failure와 명시적인 privacy/repair model이 생긴 후에만 semantic retrieval을 추가한다.

## 공급자, 도구, Hook, 사용자화

jcode의 provider 및 auth 범위는 명백한 사용자 확보 장점이다. 사용자는 여러 subscription 및 direct
API를 가져오고, compatible endpoint를 추가하고, MCP를 사용하고, lifecycle hook을 연결할 수 있다.
self-development 및 reload model도 하네스 자체를 재구성하고 싶은 사용자에게 매력적이다.

Akra에는 대부분 함정이다. provider transport, subscription credential, model catalog, tool schema,
compaction semantic, safety translation을 지원하면 런타임 경계를 복제하고 제품이 존재할 가장 분명한
이유를 없앤다.

유용한 패턴은 더 작다.

- capability discovery는 명시적이고 machine-readable이어야 한다.
- 선택적 integration이 first input을 지연하면 안 된다.
- lifecycle event는 제한된 구조화 payload를 가져야 한다.
- 실행을 차단할 수 있는 hook에는 명확한 fail-open 또는 fail-closed 계약이 필요하다.
- configuration diagnostic은 secret을 노출하지 않고 credential source와 active runtime을 설명해야
  한다.

Akra는 Codex-first로 남고 공식 interface를 사용해야 한다. Provider 범위는 이 wrapper가 아니라
upstream Codex 또는 별도로 정당화된 제품에 속한다.

## Admin, Desktop 및 원격 운영

jcode에는 상당한 custom Rust desktop prototype과 야심 찬 spatial workspace 설계가 있지만 조사한
아키텍처 문서는 여전히 그 방향을 proposed로 표시한다. 공개 release path는 검증된 범용 Admin
site가 아니라 `jcode` binary를 중심에 둔다.

Akra에는 이미 뚜렷한 장점이 있다.

- Axum/Askama Admin 및 JSON API
- Game Development Country에서 영감을 받은 operational diorama
- planning/draft/task/review page
- Telegram control
- GitHub review polling 및 delivery state
- 이러한 inbound adapter 아래의 동일 application service

현재 parallel dashboard는 read-only projection이고 review center는 critical response loop를 소유하지
않은 채 review state를 poll하고 표시한다. 따라서 약점은 truth density와 guarded actionability 모두다.
일부 Admin headline metric 및 progress value는 측정되지 않거나 비어 있다. lifecycle-derived data가
없는 시각적 야심은 집중된 네이티브 클라이언트를 이기지 못한다.

Akra는 Admin을 실제 operations console로 발전시켜야 한다.

- task throughput 및 outcome trend
- 대기열 대기 시간과 배정부터 실행, 실행부터 보고, 보고부터 통합까지의 지연 시간
- 하나의 장식적 speed number가 아닌 p50/p95
- 만들어 낸 percentage 대신 명시적인 lifecycle stage `n/N`
- 선택 agent activity, validation evidence, review state, merge readiness
- 정직한 empty/error state

이 projection이 권위 있게 된 후 guarded Admin action은 기존 control-plane 경계에서 pause/resume,
retry/handoff, review response를 위한 공유 application-service command를 정의해야 한다. HTTP handler가
SQLite나 git을 직접 변경하면 안 된다.

game layer는 실제 company/work progression을 시각화해야 한다. operational truth를 대신하면 안 된다.

## 안전과 신뢰

jcode의 넓은 런타임은 안전을 어렵게 만든다. permission UI, tool policy, credential diagnostic,
safety design, synchronous pre-tool hook을 갖는다. hook은 timeout 및 예상치 못한 failure에서 의도적으로
fail open한다. 관련 runtime code가 있지만 ambient safety는 design으로 표시된 상태다.

Akra의 공식 app-server 경계와 unattended-decline policy는 구조적 표면을 더 좁게 만든다. 실제로 더
안전한지는 두 제품을 같은 threat model로 평가하기 전까지 `inferred` 가설로 남는다. Akra는 여전히
검증 가능한 policy를 눈에 보이게 만들어야 한다.

- interactive approval은 제한되고 눈에 보이게 유지한다.
- hidden planning 및 parallel worker는 지원되지 않는 interaction을 거부한다.
- credential은 공식 Codex 또는 명시적인 outbound adapter가 계속 소유한다.
- remote write에는 검증된 GitHub identity가 필요하다.
- default-path delivery는 commit, PR, review, integration, cleanup state를 영속화한다. 현재
  `validation_summary`는 command result가 아니라 planning-file-change context를 기록한다.
- autonomous operator surface에는 구조화된 high-risk policy, PR-presence, skipped-gate 출처가 여전히
  필요하다.

그 대가는 모호한 환경에서 자율성이 낮아지는 것이다. 받아들일 수 있다. 제품은 놀라게 할 만큼
강력하기만 한 것이 아니라 충분히 오래 실행할 수 있을 만큼 신뢰받아야 한다.

## 품질과 유지보수성

jcode는 test와 ratchet에 많이 투자한다. 조사한 workspace에는 6,000개가 넘는 Rust test marker와
크기, panic 위험 사용, 무시된 오류, wildcard export, 의존성, 컴파일 성능, 시작 시간, 메모리에 대한
명시적인 budget이 있다.

규모 부채는 눈에 보인다.

- 이 조사의 거친 규칙에 따른 약 518k production Rust LOC
- 77개 crate와 많은 provider/runtime variant
- 수천 줄에 달하는 수많은 server, provider, tool, TUI 파일
- v0.43.0 태그가 로컬에서 저장소에 포함된 quality ratchet 4개를 실패하며 그중 하나에는 code-size
  regression 52개가 있음

Akra는 더 작지만 작지는 않다. 같은 거친 규칙에 따르면 production Rust LOC가 약 205k이고 Rust test
marker가 2,200개가 넘는다. architecture boundary 및 TUI layering check는 feature work를 로컬에
유지할 때만 장점이다.

교훈은 양면적이다.

- Akra의 화면이 더 커지기 전에 jcode의 측정 가능한 budget을 채택한다.
- jcode의 범위를 복사한 다음 ratchet으로 억제하려 하지 않는다.

## 존중해야 할 강점

- 성능과 resource use를 제품 수준 관심사로 다룬다.
- server/client model은 다중 session runtime state의 비용을 분산하고 reconnect를 지원하도록
  설계되었다.
- TUI 계측은 우선순위와 공간을 고려하고 상태를 인식한다.
- memory와 session search는 transcript list를 넘어 실제 continuity를 제공한다.
- swarm status는 agent와 operator 모두에게 충분히 풍부하다.
- provider 및 customization 범위는 까다로운 power user에게 유용하다.
- test, benchmark, guardrail이 이례적으로 넓은 native surface를 다룬다.

## 공략할 약점

- runtime 및 credential surface가 극도로 넓다.
- shipped, migrating, proposed, design-only concept을 구분하기 어려울 수 있다.
- performance table이 완전한 raw evidence와 함께 조사한 release에 고정되어 있지 않다.
- 동일 저장소 collaboration은 protected integration보다 coordination을 최적화한다.
- desktop 및 ambient 방향은 release maturity가 분명해지기 전에 큰 의무를 추가한다.
- 태그된 source가 자체 저장소 내 size ratchet을 충족하지 않는다.
- 넓은 provider compatibility는 공식 protocol specialization을 더 어렵게 만든다.

## Akra의 쐐기

Akra가 존재해야 할 날카로운 이유는 하나의 end-to-end 약속으로 표현해야 한다.

> 공식 Codex 세션을 시작하고, protocol-native control로 조종하고, accepted intent를 보존하고,
> 오래 실행되는 모든 lane을 관찰하고, 하나의 operator system에서 기본적으로 reviewed linear
> integration으로 마무리한다.

이 약속은 jcode보다 좁고 범용 하네스로 대체하기는 더 어렵다. Akra가 다음을 증명할 때만 신뢰할 수
있다.

- first frame, input, readiness, stream, memory 성능
- transcript noise 없는 완전한 live execution visibility
- 안전한 steering 및 approval
- duplicate turn submission이 없는 명시적인 active-turn exit 및 restart recovery
- 영속적인 planning provenance
- 격리된 parallel execution
- 검증, PR, 중요 리뷰 대응, 직렬화된 통합, 정리
- 모든 고위험 autonomous exception에 대한 명시적인 review-skipped policy provenance
- Admin의 실제 operational metric

따라서 다음 작업은 "도구를 더 추가"하는 것이 아니다. 기존 Codex-to-integration loop를 광범위한
런타임이 감당할 수 있는 것보다 측정 가능하게 더 빠르고, 더 잘 보이고, 더 신뢰할 수 있게 만드는
것이다.
