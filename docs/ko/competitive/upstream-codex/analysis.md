# 업스트림 OpenAI Codex v0.144.1 기준선 감사

[English](../../../competitive/upstream-codex/analysis.md)

이 감사는 업스트림 OpenAI Codex 릴리스 `rust-v0.144.1`과 prerelease 커밋
`226e4794b84107704378ecc1ea65f7d5c27750e5` 시점의 Akra를 비교합니다. 업스트림 Codex는 일반적인
경쟁 제품이 아닙니다. 업스트림의 `codex app-server`는 Akra의 런타임 권한이며, 업스트림의 자사
TUI는 그 권한을 위한 가장 빠르게 발전하는 참조 클라이언트입니다. 변경 불가능한 소스 링크,
아티팩트 해시, 재현 명령, 원시 측정값, 한계는 [evidence.md](evidence.md)에 있습니다. Akra 관점의
결정과 구현 계약은 [gap-matrix.md](gap-matrix.md)에 있습니다.

## 종합 판정

업스트림 Codex v0.144.1은 단순히 JSON 프로세스 경계 뒤에 있는 CLI가 아니라 이미 폭넓은 네이티브
코딩 환경입니다. 기본 TUI는 제한된 typed channel을 통해 app-server를 프로세스 내에서 실행하며,
명시적인 원격 및 로컬 데몬 경로도 동일한 thread, turn, item, approval, session 의미론을
유지합니다. 공식 런타임은 모델 실행, 도구, sandboxing, approval, MCP, skill, plugin, hook,
account/config 상태, compaction, session, fork, goal, subagent, remote control, 범용
command/filesystem/process API를 소유합니다.

Akra에 가장 강력한 위협은 구조적입니다. 공식 클라이언트는 child-process와 외부 JSON 경계를
제거하면서 프로토콜 변경을 도입한 바로 그 릴리스에서 해당 변경을 노출할 수 있습니다. 현재
Akra는 별도 app-server 프로세스 비용을 지불하면서도 실제 model/runtime configuration, 대부분의
item kind, command 및 patch 진행 상황, token usage, failed와 interrupted terminal state를 비롯한
중요한 protocol truth를 잃습니다. 가장 시급한 발견은 기능 누락이 아니라 truth bug입니다. 공식
프로토콜은 typed terminal status를 전달하고 retrying error를 명시적으로 nonterminal로 표시하지만,
Akra는 일치하는 모든 `turn/completed` notification을 성공으로 변환하고 모든 `error` notification을
terminal로 취급합니다.

Akra는 Codex의 도구, provider, session storage, remote control 또는 실험적 daemon을 복제하는
방식으로 대응해서는 안 됩니다. 공식 런타임 위에서 가장 충실하게 검토되는 운영 계층이 된 다음,
업스트림이 권한을 주장하지 않는 영역인 승인된 planning, 격리된 worktree lease, frozen-source
validation, pull-request review, integration, remote verification, cleanup에서 차별화해야 합니다.
즉시 수행할 순서는 terminal truth, 폐쇄형 typed live projection, steering과 recovery, 공식 model
capability projection, 그리고 공유 TUI/Admin/parallel view입니다. 이러한 기반이 정확해진 뒤에야
session search, fork, approval form 또는 추가 input modality를 심화해야 합니다.

성능 승자는 확정되지 않았습니다. 인증되지 않은 direct app-server probe 10회는 하한 기준선으로는
유용하지만 어느 interactive TUI와도 비교할 수 없습니다. 공식 TUI의 in-process topology는 Akra의
현재 transport path보다 검증된 아키텍처상 이점이 있지만, 최종 사용자 latency와 memory에 미치는
영향을 확인하려면 같은 머신에서 통제된 interactive benchmark가 필요합니다.

## 스냅샷

| 필드 | 값 |
| --- | --- |
| 제품 | OpenAI Codex |
| 공식 저장소 | <https://github.com/openai/codex> |
| 릴리스 | [`rust-v0.144.1`](https://github.com/openai/codex/releases/tag/rust-v0.144.1) |
| 태그가 최종적으로 가리키는 소스 커밋 | `44918ea10c0f99151c6710411b4322c2f5c96bea` |
| 릴리스 게시 시각 | 2026-07-09 23:02:40 UTC |
| 감사 날짜 | 2026-07-12 (Asia/Seoul) |
| Akra 기준선 | `prerelease`의 `226e4794b84107704378ecc1ea65f7d5c27750e5` |
| Akra 버전 | 1.3.5 |
| 감사 환경 | Ubuntu 24.04.2 WSL2, Linux 6.18.33.2, x86_64 |
| 도구 체인 | Rust/Cargo 1.95.0, Node 24.14.1, npm 11.6.1 |

릴리스 소스는 peeled tag commit에 detached 상태로 checkout했습니다. 공식 x86_64 Linux musl CLI 및
app-server archive를 내려받아 릴리스 checksum과 일치하는지 확인했습니다. Cargo test 설정이 임시
source checkout의 lockfile version field를 갱신했으므로, source conclusion과 link에는 이후의 로컬
working-tree 상태 대신 변경 불가능한 commit을 사용합니다. 근거를 수집하는 동안 Akra source file은
수정하지 않았습니다.

## 제품과 대상

공식 제품은 최신 Codex model과 tool behavior를 terminal-first interface, noninteractive execution,
IDE/app-server integration 또는 remote-control experiment를 통해 사용하려는 operator를 대상으로
합니다. 설치된 `codex` binary는 interactive TUI, `exec`, `review`, session
resume/archive/delete/unarchive/fork, MCP, plugin, app-server, cloud, remote control, sandbox,
doctor, feature inspection을 제공합니다. 이 범위는 Akra가 app-server session, model picker 또는
범용 agent transcript를 노출하는 것만으로는 차별화할 수 없음을 의미합니다.

공식 제품은 코딩 상호작용 자체를 최적화합니다. Codex rollout을 유지하고 runtime reconstruction을
소유하지만, 의미 있는 모든 slice에 전용 worktree, commit, pushed branch, pull request, critical
review response, rebase integration, remote verification, cleanup을 부여하는 Akra의 repository
policy는 제공하지 않습니다. 이 차이는 권한에 관한 것이며, 업스트림이 turn 안에서 Git이나 GitHub
도구를 호출할 수 없다는 주장이 아닙니다.

Akra의 목표 operator는 더 좁습니다. 즉, 공식 Codex 동작이 검토 가능한 intent와 cross-surface
operational state를 갖춘 내구성 있는 delivery system에 내장되기를 원하는 사람입니다. 따라서
제품 작업은 공식 capability를 충실하게 투영하고 Akra가 소유한 complexity를 provider나 tool
breadth가 아니라 delivery truth에 사용해야 합니다.

## 런타임 아키텍처

### 런타임 권한

Codex core는 configuration, prompt, model/provider request, tool execution, sandbox 및 approval
policy, MCP/app/plugin, compaction, rollout persistence, thread lifecycle, subagent의 권한입니다.
App-server는 이 권한을 versioned JSON-RPC 개념인 thread, turn, item, server request,
notification, model, permission profile, goal, hook, remote-control state로 매핑합니다.

Akra는 model 및 tool execution을 이 경계에 올바르게 위임하지만, adapter는 아직 application을
향한 무손실 projection이 아닙니다. 통과하는 method-vocabulary classification test는 method name을
인지했다는 사실만 증명할 뿐, payload의 의미가 adapter, reducer, persistence, inbound surface를
거치며 보존된다는 사실은 증명하지 않습니다.

### 자사 TUI의 세 가지 경로

자사 TUI는 세 가지 runtime shape를 지원합니다.

1. 기본으로 사용하는 embedded in-process app-server.
2. launch configuration이 호환되고 replay 가능한 경우 Unix-domain socket을 통해 연결하는 로컬
   app-server daemon.
3. Unix socket 또는 WebSocket을 통한 명시적 remote app-server endpoint.

embedded path는 typed `ClientRequest`, `ClientNotification`, `InProcessServerEvent` channel을
사용합니다. JSON-RPC result envelope은 남아 있지만 serialization과 process boundary는 hot path에서
제거됩니다. Queue에는 bound가 있고 overload나 lag가 명시적으로 드러납니다. 이는 두 번째 semantic
protocol을 도입하지 않으면서 latency와 memory pressure에 대응하는 신뢰할 만한 설계입니다.

local daemon path는 Akra가 지금 채택해야 할 production contract가 아닙니다. README는 이를
experimental로 표시하고, Unix 전용이며, updater는 reboot-persistent하지 않고, auto-update가 활성
app-server를 재시작할 수 있습니다. remote client는 disconnect를 event로 노출하며 범용 reconnect
loop를 포함하지 않습니다. 이러한 한계 때문에 daemon/UDS는 제한적인 recovery experiment이지,
client-independent continuation을 약속할 기반이 아닙니다.

### 외부 app-server 경계

릴리스된 app-server는 stdio, Unix socket 또는 plain `ws`를 통해 listen하며, remote client는
`wss`도 허용합니다. Capability-token 또는 signed-bearer authentication mode가 WebSocket을
보호합니다. 감사 환경에서 binary는 인증되지 않은 non-loopback bind를 거부했으며, source는 Origin
header를 포함한 WebSocket upgrade request를 거부합니다. plain `ws`는 loopback 또는 SSH-forwarded
path용입니다. nonlocal 사용에는 TLS reverse proxy와 authentication이 필요합니다.

이 transport protection만으로 raw app-server가 안전한 Akra Admin API가 되지는 않습니다. Stable
method에는 host filesystem 및 command operation이 포함되며, 초기화한 isolated probe는
`fs/readFile`을 통해 `/etc/hostname`을 읽었습니다. Stable filesystem 및 thread shell-command path는
local-user authority로 동작할 수 있습니다. experimental process spawn은 unsandboxed이며 app-server
environment를 상속합니다. Akra는 범용 JSON-RPC surface를 Admin이나 Telegram을 통해 전달하지 말고
제한되고 인증된 application DTO를 계속 노출해야 합니다.

## 프로토콜과 기능 표면

생성된 v0.144.1 stable schema에는 client request 87개, server notification 68개, server request
10개, client notification 1개가 있습니다. `--experimental`로 schema export를 활성화하면 client
request가 122개, server request가 11개로 늘지만 notification 수는 그대로입니다. runtime capability
handshake는 별도로 experimental use를 gate합니다. experimental schema를 생성하거나 vendoring하는
것만으로는 해당 method가 활성화되지 않습니다.

Akra의 pinned schema는 experimental entry를 포함한 0.144.0에서 생성되었습니다. Akra 자체
normalizer를 사용해 새로운 0.144.1 experimental export를 정규화한 결과, definition은 byte 단위로
동일했고 generated date와 source CLI metadata만 달랐습니다. 이는 patch-level vocabulary currency를
검증하지만 semantic coverage나 stable-runtime guarantee를 검증하지는 않습니다. Akra는
`experimentalApi: false`로 app-server를 초기화하므로, product contract는 stable method와 단지
experimental definition에 존재하는 method를 구분해야 합니다.

격리된 released-binary handshake는 account, thread catalog, loaded thread, model, collaboration
mode, app, skill, plugin, remote-control status, permission profile, hook, config, MCP status,
experimental-feature catalog를 성공적으로 읽었습니다. Negative probe는 필수 initialization order,
one-time initialization, capability gating, notification opt-out을 검증했습니다. 이는 model
authentication이나 live turn 없이 런타임에서 직접 관찰한 결과입니다.

Stable discovery가 반환된 모든 capability의 stable application을 의미하지는 않습니다. 특히
`permissionProfile/list`는 stable이지만, named/custom `thread/start.permissions` selection과
`activePermissionProfile` response provenance는 experimental입니다. stable thread path에는 legacy
`sandbox`와 generic config override map이 남아 있으므로, 기존 custom default 또는 untyped
`config.default_permissions` override는 profile identity를 반환하지 않고도 여전히 execution에 영향을
줄 수 있습니다. Akra의 일반 stable product path는 permission selection을 위해 이 generic escape
hatch를 사용해서는 안 됩니다. 정확히 표현 가능한 built-in profile만 legacy `sandbox`에 매핑할 수
있으며, 별도의 experimental product decision이 없는 한 임의의 named profile selection을 사용할 수
없는 것으로 취급해야 합니다.

### 종료 상태는 Akra의 투영보다 풍부합니다

공식 `Turn` data는 `completed`, `interrupted`, `failed`, `inProgress` status, optional typed error,
timestamp, duration, item-view completeness를 전달합니다. `error` notification은 `TurnError`를
중첩하고 `willRetry`를 선언합니다. `willRetry: true`는 명시적으로 turn을 interrupt하지 않습니다.

감사 대상인 Akra 기준선에서는 다음과 같습니다.

- `error` handler가 존재하지 않는 top-level `params.message`를 찾고 stream을 종료합니다.
- `turn/completed`는 ID를 검증하지만 `turn.status`와 `turn.error`를 버립니다.
- terminal event send result를 버리고, connection/caller는 application sink가 disconnect된 경우에도
  completion을 transport-level `Result<()>`로 축소합니다.
- core reducer는 generic `TurnCompleted` event만 받습니다.
- parallel worker는 adapter가 `Ok`를 반환할 때마다 해당 thread를 archive합니다.
- prompt-log status는 stream이 `Ok`를 반환할 때마다 `completed`를 기록합니다.

interrupted turn은 이 경로에서 성공한 Akra completion으로 처리됩니다. failed `turn/completed`
payload도 이전 nonretrying `error`가 이미 loop를 abort하지 않은 채 handler에 도달하면 success와
구별할 수 없습니다. 이와 별개로 transient upstream error는 업스트림이 retry 중인 동안에도 Akra를
종료합니다. 이 문제가 해결될 때까지 completion에 의존하는 planning continuation, archive,
validation, delivery state는 현재 adapter의 generic success를 권위 있는 증거로 취급해서는 안
됩니다.

### 메서드 범위는 항목 범위가 아닙니다

Akra는 현재 모든 notification method name을 handled, deferred, diagnostic, ignored set으로
분류합니다. 그러나 adapter translation이 없으므로 active reducer는 모든 deferred method를 warning에
버립니다. Live completed item은 agent message, file change, command execution만 보존하며, historical
snapshot에는 user message도 추가됩니다. 공식 `ThreadItem` vocabulary는 reasoning, plan, MCP call,
dynamic tool, collaboration 및 subagent activity, web search, image generation, review-mode
transition, compaction, 기타 typed outcome도 전달합니다.

올바른 대응은 모든 wire struct를 `domain`에 그대로 복제하는 것이 아니라 explicit unknown variant와
bounded payload를 갖춘 폐쇄형 application projection입니다. Started/completed identity, terminal
truth, command output, patch 및 turn diff, plan, token usage, reroute, approval ownership, parallel
activity는 운영상 의미가 있습니다. 세밀한 reasoning text, raw secret, unbounded tool output에는
의도적인 redaction 및 size policy가 필요합니다.

## 모델, 도구, 컨텍스트, 세션

### 모델과 적용된 런타임 범위

공식 `model/list`는 model ID, display label, default selection, supported reasoning effort, input
modality, service tier, 관련 capability를 제공합니다. effort vocabulary는 업스트림이 소유하고
versioning합니다. 현재 enum에는 이미 Akra의 hard-coded picker를 넘어서는 `max`와 `ultra`가 있으며,
향후 릴리스에서 또 다른 wire value를 추가할 수 있습니다. Thread start 및 resume response는 실제로
적용된 model, provider, cwd, approval policy, sandbox, reasoning effort, service tier도 반환하며,
`model/rerouted`는 나중에 effective model truth를 변경할 수 있습니다.

현재 Akra는 model/effort choice를 hard-code하고 start 및 resume response 중 `thread` 부분만
deserialize합니다. 공식 catalog를 요청하고 투영하며 unknown effort value를 보존하고 requested
configuration과 applied configuration을 함께 표시해야 합니다. provider routing이나 Akra 소유의
model registry를 추가해서는 안 됩니다.

### 도구와 확장성

Codex는 shell/command, patch, filesystem, web, image, dynamic tool, MCP, app, plugin, skill, hook,
sandbox behavior를 소유합니다. 릴리스된 binary의 feature discovery는 stable, experimental,
under-development, removed compatibility entry에 걸친 92개 row를 반환했습니다. feature가 list에
있거나 enabled라는 사실은 stable app-server method 또는 성숙한 product workflow와 같지 않습니다.

Akra는 operator decision을 지원하는 경우에만 capability/status discovery를 노출해야 합니다. 예를
들어 model availability, MCP startup failure, plugin/skill availability, permission profile,
effective sandbox는 diagnostic 또는 live envelope에 속합니다. 범용 filesystem/process/plugin
management는 계속 업스트림 권한으로 둡니다.

### 세션 재구성과 압축

업스트림의 rollout persistence, resume reconstruction, rollback, fork, compaction은 상당한
수준입니다. reducer는 replacement-history checkpoint, world state, setting, compaction window를
재구성합니다. Fork는 aborted-turn marker를 삽입해 mid-turn snapshot boundary를 명시적으로
처리합니다. Manual 및 automatic compaction에는 provider-specific path가 있고 replacement history를
persist합니다.

감사 대상 소스에는 pre-turn compaction 한계도 있습니다. 새 input과 context diff를 추정하지 않고
기존 context usage를 확인하므로, 큰 input은 첫 sampling attempt 전에 threshold를 넘을 수 있습니다.
이는 error와 recovery truth를 통해 드러내야 할 업스트림 runtime risk이지, Akra가 자체 compactor를
만들 이유가 아닙니다.

Akra의 공식 session catalog projection은 cursor-driven paging, 여러 filter, lineage, provenance
field를 잃습니다. 이 gap은 기존 Official Session Search And Provenance slice에 속합니다. Akra가
experimental capability 없이 초기화되는 동안 experimental turn/item paging을 stable이라고 설명해서는
안 됩니다.

### 메모리

Automatic memory는 experimental이며 기본적으로 꺼져 있습니다. extraction path는 선택된 prompt
fragment를 제거하고 best-effort secret sanitization을 적용하며, isolated consolidation worker에는
강력한 sandbox restriction이 있습니다. sanitizer는 의도적으로 완전하지 않은 pattern-based
방식이며, memory rate-limit guard는 check가 실패해도 작업을 진행하도록 허용합니다. 이러한 사실은
두 번째 Akra semantic-memory store를 추가하지 말아야 한다는 근거입니다. Accepted planning과
explicit provenance가 더 안전한 단기 memory primitive입니다.

## TUI와 상호작용 설계

자사 TUI는 breadth와 protocol freshness의 참조 대상입니다. session resume/fork, goal, steering,
subagent, model/reasoning selection, permission profile, skill, plugin/app, hook, review, compaction,
command discovery, rich approval, remote operation을 지원합니다. Approval UI는 thread/environment
context를 보존하고 cross-thread navigation을 지원하며, 제공되는 경우 one-shot, session,
persistent-rule, host, decline, abort 의미론을 분리합니다.

여전히 Akra의 목표가 아니라 업스트림의 사실로 취급할 가치가 있는 reference-client 한계가
있습니다. modal이 이미 표시된 동안 추가된 approval request는 vector에 저장된 다음 나중에 pop되므로,
요청이 지속적으로 들어오면 새 요청이 우선될 수 있습니다. 일반 exec decision list는 TUI가 protocol의
decline-and-continue choice를 render할 수 있는데도 이를 생략할 수 있습니다. 따라서 폐쇄형 Akra
approval queue는 arrival order를 보존하고 자사 기본값을 무작정 복사하지 말고 권위 있게 제공된
decision만 render해야 합니다.

Akra는 의도적으로 다른 terminal contract에서 여전히 앞섭니다. inline rendering은 full-screen
transcript history를 소유하지 않고 host scrollback을 보존합니다. 또한 application state에 연결된
planning 및 parallel delivery overlay가 있습니다. live Codex item이 하나의 generic activity line으로
축소되면 이러한 이점이 약화됩니다. 올바른 설계는 host scrollback을 durable transcript surface로
유지하면서 typed application event 위에 bounded priority rail과 drilldown을 제공하는 것입니다.

## Admin과 원격 표면

업스트림 app-server, WebSocket remote TUI, `remote-control`은 범용 Codex access를 다룹니다.
remote-control과 daemon surface는 명시적으로 experimental입니다. 로컬 remote-control database는
bearer token이 아니라 server/environment identity를 저장합니다. 감사 대상 업스트림 surface 중 어떤
것도 Akra의 planning authority, lease board, validation evidence, GitHub review/integration state,
delivery game diorama를 제공하지 않습니다.

Akra는 TUI, CLI, Telegram, automation이 사용하는 것과 동일한 application service의 authenticated
projection으로 Admin을 유지해야 합니다. raw app-server proxy가 되어서는 안 됩니다. 추후 Admin
status view는 `codex doctor --json`의 제한되고 redacted된 fact를 수집할 수 있지만, output schema,
command cost, timeout, redaction, offline behavior를 고정한 뒤에만 가능합니다. 이 감사에서 Doctor는
reachability check를 수행했으므로 무해한 synchronous render helper가 아닙니다.

## 병렬 작업

Stable 1세대 Codex subagent는 기본적으로 enabled이며 default concurrency 6, depth 1로 제한되고,
live model/provider/reasoning, approval, cwd, permission profile, environment, compatible execution
policy를 상속합니다. 이는 upstream subagent event가 Akra가 투영해야 할 일반 runtime vocabulary의
일부라는 뜻입니다.

2세대 graph/fanout 구현은 under development이며 기본적으로 off입니다. Source inspection에서는
tool gate에 depth enforcement가 누락된 점과 execution limiter에 발생 가능한 check-then-increment
race를 발견했습니다. 이는 재현된 load failure가 아니라 조건부 source inference이며, shipped
defect로 평가하거나 Akra roadmap에 복사해서는 안 됩니다.

Codex subagent는 런타임 안에서 model work를 조정합니다. Akra의 accepted task authority,
one-worktree-per-lane lease, frozen source, validation, PR review, integration, cleanup을 대체하지
않습니다. Akra는 delivery state의 권한을 자체 service에 유지하면서 공식 subagent 및 collaboration
item을 current activity에 매핑해야 합니다.

TUI의 `/side` lifecycle은 ephemeral thread를 interrupt하고 unsubscribe합니다. app-server는
unsubscribed inactive thread를 thread별 no-subscriber-and-inactive delay 30분 동안 유지할 수 있으므로,
side session을 반복하면 resource가 일시적으로 누적될 수 있습니다. 이는 제한된 transient-risk
inference이지 lifetime leak evidence가 아닙니다.

## 성능

### 재현한 직접 app-server 하한

릴리스된 x86_64 Linux musl app-server는 인증이나 model turn 없이 새로 격리한 `CODEX_HOME`에서
측정했습니다. warm-cache run 10회 각각은 기본 app-server를 실행하고, 초기화하고,
`account/read`를 호출하고, `thread/list`를 호출한 다음 종료했습니다. 밀리초 단위 elapsed sample은
다음과 같습니다.

```text
120.781 122.712 123.812 120.261 117.292
120.708 122.124 120.605 119.439 120.829
```

최솟값은 117.292 ms, 중앙값은 120.745 ms, 최댓값은 123.812 ms였습니다. sampling한 process-tree
peak RSS의 KiB 단위 값은 다음과 같습니다.

```text
82440 90524 93328 92052 87676 88272 88352 88336 91224 87920
```

최솟값은 82,440 KiB, 중앙값은 88,344 KiB, 최댓값은 93,328 KiB였고 모든 sample은 process 4개에서
peak에 도달했습니다. 같은 default path를 수동으로 검사한 결과 app-server의 enabled plugin startup
warmup이 `git ls-remote https://github.com/openai/plugins.git HEAD`와 Git transport helper를 실행하는
것을 관찰했습니다. final response는 이 public-network work가 끝날 때까지 기다리지 않습니다.
checked-in harness는 2 ms 간격의 all-thread recursive `/proc` process-tree polling을 요청했지만,
event-loop scheduling과 RSS sampling은 더 짧은 peak를 놓칠 수 있습니다. Page cache는 warm이었고,
해당 path에는 TTY/rendering, authentication, model call, stream, MCP request 또는 Akra process가
없었습니다. 이 수치는 isolated protocol cost나 user-visible winner가 아니라 default-path 하한을
설명합니다.

### 구조적 위험과 메모리 위험

공식 embedded TUI는 child process와 외부 JSON serialization을 피하지만, Akra의 현재 TUI는
app-server를 실행합니다. 따라서 full process-tree accounting이 필수이고 신뢰할 만한 최적화 목표가
생기지만, 이를 정량화하려면 통제된 PTY benchmark가 필요합니다.

Codex core 내부에서 submission에는 bound가 있지만 bounded app-server downstream 앞에 있는 event
channel 하나는 unbounded입니다. 따라서 slow client나 delta-heavy multi-agent run은 core memory에
event를 축적할 수 있습니다. 이 source inference는 stress로 재현하지 않았습니다. Akra는 고정된
synthetic stream에서 event backlog와 full-tree RSS를 benchmark하고 자체 queue를 bound하며, 공식
runtime이 memory unboundedness를 불가능하게 만든다고 가정하지 말고 upstream lag/overload state를
보존해야 합니다.

## 안전성과 품질

### 샌드박스와 전송

implicit sandbox는 trust state에 따라 달라집니다. 기록된 project decision(`Trusted` 또는
`Untrusted`)이 있으면 built-in workspace profile을 선택하고, trust decision이 없으면 read-only를
선택합니다. 두 경우 모두 root read를 유지하고 network를 제한합니다. 명시적인 CLI policy는 이를
변경할 수 있습니다. Product copy는 하나의 보편적 default를 주장하지 말고 effective applied
envelope을 표시해야 합니다.

업스트림 generated-shell default는 Akra의 launch policy보다 넓습니다. 모든 parent environment
variable을 상속하고 default key/secret/token exclusion rule을 비활성화합니다. Akra는 이미 app-server
child environment를 clear한 뒤 allowlist하고, generated-shell inheritance를 `core`로 설정하며,
operator가 명시적인 elevated override를 선택하지 않는 한 secret exclude를 보존합니다. 이는 실제
구조적 이점이지만 constructed command argument만으로는 최종 tool-child environment 또는
login-profile behavior를 증명할 수 없으므로 released-binary canary가 필요합니다.

Filesystem policy는 project decision이 존재하기 전의 경우에만 반대 방향을 가리킵니다. 자사 runtime은
trust decision이 없으면 read-only를 선택하고 Trusted 또는 Untrusted decision이 기록된 뒤에는
workspace profile을 선택하지만, Akra의 일반 main 및 parallel work turn은 동등한 recorded decision
없이 기본적으로 workspace-write를 사용합니다. Akra thread setup과 hidden planning path는 여전히
read-only이므로 이는 보편적인 runtime default가 아닙니다. Akra는 더 강한 environment boundary와 더
안전한 default라는 포괄적 주장을 결합해서는 안 됩니다. requested 및 applied sandbox/profile과
operator decision은 명시적으로 유지되어야 합니다.

WebSocket non-loopback 및 Origin protection은 의미 있는 강점입니다. initialization 이후의 method
authority를 축소하지는 않습니다. Akra의 remote surface는 capability-specific, authenticated,
rate-limited, audited 상태를 유지하고 임의의 app-server JSON-RPC를 제출할 수 없게 해야 합니다.

Rollout file에는 message, reasoning, tool call, command, output이 포함될 수 있습니다. recorder는 일반
create semantics를 사용하고 자체적으로 mode `0600`을 강제하지 않으므로, 보호는 directory permission과
umask에 달려 있습니다. 이는 조건부 local confidentiality risk이지, 감사 대상 사용자의 rollout이
노출되었다는 증거가 아닙니다. Akra는 raw transcript persistence를 복제하지 않아야 하며, 자체
prompt/review log에 명시적인 file/database permission, retention, redaction, Admin authorization을
부여해야 합니다.

또한 Codex authentication은 API, OAuth, PAT 또는 agent-key material을 포함하는 plaintext
`$CODEX_HOME/auth.json` file을 기본으로 사용합니다. Unix create path는 mode `0600`을 요청하지만,
기존 file을 저장할 때는 더 느슨한 permission을 복구하지 않습니다. keyring 및 encrypted alternative가
존재합니다. Akra는 이 upstream store를 소유하지 않으며, environment scrubbing은 같은 home 아래의
읽을 수 있는 file을 보호하지 않습니다. Secret-canary 작업은 direct/shell read를 관찰하고 Akra의
secondary amplification surface에서 fail closed해야 하지만, authorized upstream read를 통제할 수
있다고 주장해서는 안 됩니다.

### 테스트와 릴리스 근거

정확한 tag commit에는 build, signing, packaging, publishing 작업이 중심인 44개의 attached check를
포함한 성공한 `rust-release` workflow가 있습니다. 이는 강력한 artifact provenance이지만 전체 일반
pull-request test matrix가 tag commit에서 다시 실행되었다는 증거는 아닙니다. 저장소에는 폭넓은
Cargo/Bazel workflow가 있지만, 이 감사는 exact-tag release evidence를 별도로 구분합니다.

문서화된 단순 x86_64 Linux CLI 및 app-server tar archive에는 각각 binary 하나가 있습니다. system
`bwrap`이 없었던 감사 host에서는 adjacent bundled helper가 없었기 때문에 둘 다 sandboxed execution에
exit 101로 실패했습니다. 업스트림은 helper를 포함하고 검증하는 complete package archive도
게시합니다. 이는 shipped install-shape defect이지 보편적인 Linux failure가 아닙니다. Akra는 Codex나
`bwrap`을 package하지 않습니다. 올바른 대응은 업스트림 resource를 조용히 vendoring하는 것이 아니라
정확한 install-compatibility receipt와 operator prerequisite입니다.

Release build/signing hardening은 폭넓지만, final release job은 일반 source-test matrix에 직접
의존하지 않고 Cargo release command는 `--locked`를 일관되게 사용하지 않습니다. commit된 tag
lockfile은 workspace package를 `0.0.0`으로 저장하고, 로컬 Cargo invocation은 132개 entry를
`0.144.1`로 다시 작성했습니다. external dependency graph 변화는 관찰되지 않았습니다. Akra는 이미
locked build를 사용합니다. 남은 차별점은 mutation 전에 source-test receipt, Codex compatibility,
archive integrity를 각각 release SHA에 결부하는 것입니다.

user-space `pkg-config`와 OpenSSL development file을 제공한 뒤, 정확한 source는
`codex-app-server-protocol` test 256개와 `codex-app-server-client` test 27개를 모두 통과했습니다.
v0.144.1 patch 자체는 installer와 code-mode behavior에 집중된 file 8개를 변경했습니다. 따라서
app-server protocol 결론은 이 patch가 이를 도입했다는 주장이 아니라 더 넓은 v0.144.0 release
lineage에도 의존합니다.

`codex doctor --json`은 격리된 home에서 structured redacted check를 생성했습니다. 해당 probe에서
예상된 credential, terminal, install-match check는 실패했지만 app-server, config, Git, state,
sandbox check는 실행되었습니다. 또한 reachability work를 시도했습니다. doctor를 언제나 안전한
data library가 아니라 side effect와 latency가 있는 진단 가능한 command로 취급해야 합니다.

## Akra 비교

| 차원 | 업스트림 Codex v0.144.1 | Akra `226e4794` | 판정 | 결과 |
| --- | --- | --- | --- | --- |
| 런타임 권한 | 공식 모델/도구/샌드박스/세션 권한 | 실행을 app-server에 위임 | 의도적으로 다름 | 위임을 유지하고 런타임 의미론을 절대 분기하지 않음 |
| 핫 패스 토폴로지 | 기본 타입 기반 프로세스 내 app-server | 자식 프로세스와 외부 JSON-RPC | 업스트림이 구조적으로 앞섬 | 전체 트리를 측정하고 제한적인 공식 클라이언트/UDS 실현 가능성 경로만 테스트 |
| 프로토콜 최신성 | 릴리스 자체의 stable/experimental 게이팅 | 0.144.0 experimental schema, stable runtime handshake | 어휘는 최신이고 기능 계약은 흐림 | stable 및 experimental 아티팩트를 별도로 고정 |
| 종료 상태의 진실성 | typed completed/interrupted/failed/retrying | generic completed, retrying error는 종료를 일으킴 | 업스트림이 결정적으로 앞섬 | 완료에 의존하는 모든 자동화보다 먼저 수정 |
| 실시간 항목 | 폭넓은 typed item 및 notification set | payload 의미 대부분을 버림 | 업스트림이 앞섬 | unknown/redaction을 갖춘 폐쇄형 typed projection 구축 |
| 모델 선택 | 공식 동적 기능 및 적용된 범위 | hard-coded catalog 및 effort enum | 업스트림이 앞섬 | 좁은 공식 catalog port와 effective-state projection 추가 |
| 세션 | persistence, paging, filter, lineage, fork, resume | 기본 list/read/resume projection | 업스트림이 앞섬 | P0 truth 이후 기존 session provenance slice 완료 |
| TUI 범위 | 대부분의 런타임 기능을 지원하는 참조 구현 | 더 좁은 Codex view, 더 풍부한 planning/delivery overlay | Codex 상호작용에서는 업스트림이 앞섬 | host scrollback을 보존하고 우선순위가 높은 실시간 세부 정보 개선 |
| Admin/전달 | 범용 remote/experimental control | planning, review, metric, Telegram, GitHub delivery | 소유 workflow에서는 Akra가 앞섬 | raw RPC가 아니라 권위 있는 application state를 공유 |
| 병렬 런타임 | 제한된 native subagent 및 개발 중인 graph mode | worktree-isolated lane 및 delivery service | 상호 보완적 | runtime activity를 투영하고 Akra lease/integration authority 유지 |
| 검토된 통합 | Codex는 Git/GitHub를 사용할 수 있지만 동등한 product invariant는 없음 | 명시적 bypass를 갖춘 commit/PR/review/integration/cleanup default | 검증된 contract에서는 Akra가 앞섬 | evidence와 bypass provenance를 표시 |
| 성능 근거 | 직접 측정한 하한 및 프로세스 내 아키텍처, 공정한 TUI 비교 없음 | 기존 P0 benchmark contract, 현재 비교 가능한 artifact 없음 | 승자 불명 | 하나의 공유 protocol/PTY/full-tree benchmark 생성 |
| 원격 안전성 | 보호된 전송이지만 초기화 후 host method가 광범위함 | bounded application API 및 identity-gated GitHub write | 의도한 boundary에서는 Akra가 더 안전함 | raw app-server를 private으로 유지 |
| Linux 설치 준비도 | system `bwrap` 없이는 simple tar의 sandbox가 실패하며 complete package는 helper를 포함 | 외부 Codex prerequisite, install-shape receipt 없음 | 업스트림 결함이 sandboxed command/runtime use를 중단시킬 수 있음 | 지금은 좁은 operator prerequisite를 두고 compatibility receipt를 추가하며 helper를 vendor하지 않음 |
| Shell 환경 | 기본적으로 모두 상속하고 secret exclude를 비활성화 | child를 clear/allowlist하고 기본 tool shell을 core로 설정 | Akra가 구조적으로 앞섬 | 모든 runtime path에 걸쳐 released canary 확장 |
| 파일시스템 샌드박스 기본값 | trust decision이 없으면 read-only, 어느 recorded decision이든 workspace profile 사용 | 일반 main/parallel work turn은 기본 workspace-write, setup/planning은 read-only | decision 전 일반 작업에서는 업스트림이 더 안전함 | 기존 permission P1에 명시적 trust/profile choice 추가 |

## 결정

### 채택

- Typed in-process semantics를 reference architecture로 삼습니다. transport optimization을 고려하기
  전에 하나의 app-server vocabulary를 유지하고 translation loss를 제거합니다.
- 별도의 generated artifact와 negative test를 갖춘 negotiated stable/experimental capability
  contract.
- 공식 model capability 및 실제 applied-envelope projection.
- bounded application type을 통한 typed terminal, item lifecycle, token, patch, diff, plan, reroute,
  compaction, approval-clear, collaboration, subagent fact.
- 기존 session provenance 작업을 통한 session paging, filter, lineage, fork.
- Akra가 소유한 모든 event path의 bounded queue, overload, lag, shutdown semantics.
- 권위 있게 제공된 decision과 FIFO ownership을 보존하는 자사 approval information hierarchy.
- Akra archive integrity 및 source-test evidence와 분리된 정확한 Codex install-shape 및
  sandbox-helper compatibility receipt.

### 거부

- Akra의 provider/model execution registry, tool engine, MCP runtime, compactor 또는 session store.
- Admin, Telegram 또는 automation을 통한 raw app-server JSON-RPC 노출.
- experimental local daemon 또는 remote-control behavior를 production continuity promise로 취급.
- operator가 설치한 runtime defect에 대응해 upstream `bwrap`, credential 또는 Codex package manager를
  Akra에 vendoring.
- Codex goal, plan item, review mode 또는 subagent completion을 Akra delivery authority로 전환.
- 공식 session provenance 및 planning link보다 먼저 두 번째 automatic semantic-memory system 구축.
- stage와 handshake proof 없이 feature flag 또는 source presence를 product claim으로 복사.

### 차별화

Akra는 delivery boundary에서 reference client보다 더 진실해야 합니다.

- requested runtime configuration과 applied runtime configuration을 계속 표시합니다.
- retrying, interrupted, failed, unknown, completed를 구분하고 영속화합니다.
- upstream item의 completion은 planning task나 delivery의 completion을 의미하지 않습니다.
- Codex `EnteredReviewMode`는 GitHub review evidence가 아닙니다.
- accepted planning intent, worktree lease, source freeze, validation, review, check, integration,
  remote verification, cleanup을 서로 분리된 authoritative state로 유지합니다.
- TUI, Admin, CLI, Telegram, automation은 동일한 application projection을 사용합니다.
- 모든 autonomous review/check/PR bypass를 명시적인 policy provenance로 유지합니다.

## Akra의 승부처

이 감사 이후의 지속 가능한 product statement는 다음과 같습니다.

> Akra는 공식 runtime truth를 왜곡 없이 투영하고, 승인된 intent를 격리되고 검증되고 검토되고
> 원격으로 확인된 integration으로 전환하는 Codex-first operating and delivery
> layer입니다.

"공식 runtime truth를 투영한다"라는 문구는 이제 positioning copy가 아니라 prerequisite입니다.
Akra가 공식 failed/interrupted status를 버리고 일치하는 completion을 success로 표시하는 동안에는 더
강한 delivery layer를 신뢰성 있게 주장할 수 없습니다. 이 문제가 해결되면 upstream velocity가
leverage가 됩니다. model, tool, session, subagent 개선은 공식 authority를 통해 도착하고, Akra는
업스트림이 소유하지 않는 operator 및 delivery guarantee에 집중합니다.

## 갱신 조건

다음 중 하나라도 발생하면 이 감사를 갱신합니다.

- Codex release가 app-server stable method, terminal status, applied thread envelope, TUI transport
  selection, daemon support 또는 remote authentication을 변경하는 경우.
- 공식 TUI가 daemon/remote reconnect를 지원되는 production contract로 만드는 경우.
- Akra가 child-process adapter를 교체하거나 experimental app-server capability를 opt in하는 경우.
- terminal-truth/live-projection P0 또는 official-model P1이 반영되는 경우.
- 통제되고 인증된 interactive TUI benchmark 또는 event-backpressure stress test를 사용할 수 있게
  되는 경우.
- 업스트림이 simple/complete Linux archive layout, sandbox-helper discovery, shell environment
  default 또는 auth storage default를 변경하는 경우.
- 업스트림 memory 또는 multi-agent V2가 experimental/development에서 stable default behavior로
  이동하는 경우.

## 출처

변경 불가능한 source ledger, release artifact identity, reproduction command, direct app-server
sample, test output, Akra baseline link, audit limit는 [evidence.md](evidence.md)를 참조하세요. 소유한
implementation delta와 proof requirement는 [gap-matrix.md](gap-matrix.md)를 참조하세요.
