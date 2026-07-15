# OpenCode에서 Akra로의 갭 매트릭스

[English](../../../competitive/opencode/gap-matrix.md)

이 매트릭스는 [v1.17.18 분석](analysis.md)과 [증거 원장](evidence.md)을 Akra 기준 결정으로
전환한다. `Ahead`는 고정된 스냅샷의 해당 차원에서 더 강한 증거가 있다는 의미이지 전체 제품
점수가 아니다. 수용하는 작업은 OpenCode의 플랫폼 폭을 복제하지 말고 Akra의 Codex 우선 운영과
리뷰 기반 전달 지위를 강화해야 한다.

## 상대 매트릭스

| 역량 | OpenCode v1.17.18 | Akra `354f4782` | 결정 | 우선순위 | 증거 |
| --- | --- | --- | --- | --- | --- |
| 런타임 권한 | provider, inference, tool, session, plugin, server, client 소유 | model/tool runtime은 공식 app-server에 위임하고 operator/delivery flow 소유 | runtime 중복 거부 | invariant | [OpenCode](evidence.md#product-release-and-topology), [Akra](evidence.md#akra-baseline-evidence) |
| Client detach | 별도 `serve`와 `attach`로 client 수명 분리 가능, 기본 TUI는 worker 중지, live detach 미재현 | child connection이 turn runtime 소유, transcript resume 가능 | 생존을 주장하지 않고 기존 recovery contract 확장 | existing P0 | [continuity](evidence.md#session-continuity-forks-and-background-work), [Akra](evidence.md#akra-baseline-evidence) |
| 외부 재연결 | replay cursor나 완전한 TUI resync 없이 SSE 재시도 | child restart reconciliation 미완성 | protocol-guaranteed field를 조정하고 생략된 history 표시 | existing P0 | [events](evidence.md#product-release-and-topology), [existing slice](../jcode/gap-matrix.md#p0-active-turn-exit-and-restart-recovery) |
| TUI 명령 탐색 | command palette, leader binding, which-key group, contextual session action | 검색 가능한 단일 `:` registry와 palette, availability/reason metadata 없음 | 핵심 protocol 작업 뒤 기존 registry 확장 | P2 | [TUI](evidence.md#tui-commands-and-long-sessions), [Akra](evidence.md#akra-baseline-evidence) |
| 실시간 실행 | 응집력 있는 command, tool, diff, permission, question, timeline, child view | 제한된 completed-item summary, 더 풍부한 native event 계획됨 | 기존 live rail 재사용 | existing P0 | [TUI](evidence.md#tui-commands-and-long-sessions), [existing slice](../jcode/gap-matrix.md#p0-protocol-native-live-execution-rail) |
| 긴 세션 | TUI는 최근 100개 message hydrate, browser는 이전 history paging | 공식 session list/search와 local paging, full protocol provenance 계획됨 | 적응형 projection과 명시적 truncation | existing P0/P2 | [history](evidence.md#tui-commands-and-long-sessions), [existing session slice](../jcode/gap-matrix.md#p2-official-session-search-and-provenance) |
| 승인 선택 | typed permission/question UX와 풍부한 action choice | binary accept/decline, 검사 불가 file/MCP request decline | command/permission choice부터 보존, form은 fail closed 유지 | P1/P2 | [OpenCode UI](evidence.md#tui-commands-and-long-sessions), [Akra](evidence.md#akra-baseline-evidence) |
| Fork | 명시적 source lineage 없는 transcript-copy fork | 공식 schema에 `thread/fork`와 `forkedFromId`가 있으나 Akra 경로 없음 | 공식 session provenance의 제한된 sub-slice로 추가 | existing P2 | [fork](evidence.md#session-continuity-forks-and-background-work), [Akra schema](evidence.md#akra-baseline-evidence) |
| Background agent | child session과 shared directory의 experimental process-local job | worktree isolation 및 delivery detail이 있는 durable fixed lane | lineage UX 도입, shared-directory authority 거부 | existing P0/P1 | [background](evidence.md#session-continuity-forks-and-background-work), [Akra delivery](evidence.md#akra-baseline-evidence) |
| Worktree UX | create/reset/remove/startup command, reset은 destructive | leased lane identity와 guarded cleanup | 다름, Akra lane contract 유지 | shipped | [worktrees](evidence.md#worktrees-github-and-delivery), [Akra](evidence.md#akra-baseline-evidence) |
| GitHub intake | issue/PR/comment/schedule/dispatch event | 출시된 schedule/webhook intake 없음 | Canvas automation plan 재사용 | existing P1 | [GitHub](evidence.md#worktrees-github-and-delivery), [automation slices](../agent-canvas/gap-matrix.md#p1-automation-intake-core) |
| GitHub 전달 | prompt-driven commit/push/PR, mutable action, 검증된 reviewed integration authority 없음 | frozen source, review/check default gate, serialized integration, remote verification, cleanup | 차별화하고 provenance deep link 추가 | shipped plus existing P0/P1 | [OpenCode](evidence.md#worktrees-github-and-delivery), [Akra](evidence.md#akra-baseline-evidence) |
| 서버 인증 | password optional, default loopback, password 없이 raw secret response 재현 | capability/session auth와 origin guard가 있는 loopback-only Admin | fail-closed boundary 보존, release invariant에 negative proof 추가 | invariant | [canary](evidence.md#resolved-secret-canary), [Akra](evidence.md#akra-baseline-evidence) |
| 시크릿 처리 | resolved config/provider value 노출, full plugin/MCP trust, repo Git config token | explicit full-env override와 default scrubbed child env, bounded egress는 완전 입증 전 | 기존 canary experiment를 release invariant로 확장 | invariant | [security](evidence.md#authentication-secrets-permissions-and-extensions), [Akra](evidence.md#akra-baseline-evidence) |
| 권한 합성 | 직접 `.env` read는 묻지만 in-workspace shell 경로는 해당 rule 미합성 | upstream Codex가 tool 소유, Akra는 child env와 app-server approval projection 소유 | canary로 공식 capability 검증, 두 번째 tool broker는 만들지 않음 | existing experiment/invariant | [OpenCode](evidence.md#authentication-secrets-permissions-and-extensions), [existing canary](../agent-canvas/gap-matrix.md#permission-and-secret-canary) |
| Browser/desktop | 공유 full product app과 6개 desktop target | authenticated operational Admin, desktop 없음 | surface 경쟁 거부, operator projection 강화 | invariant | [surface](evidence.md#web-desktop-ide-and-sharing), [Akra](evidence.md#akra-baseline-evidence) |
| Remote node | generic server connection과 optional credential | local Admin 및 Telegram, read-only node plan 존재 | 기존 read-only plan 유지 | existing P2 | [server](evidence.md#product-release-and-topology), [node plan](../agent-canvas/gap-matrix.md#p2-read-only-node-snapshot-protocol-and-server) |
| 공유 | manual/auto upload, GitHub Action은 public-repo session 기본 공유 | 대응하는 public share 없음 | 별도 redacted export use case가 생길 때까지 거부 | reject | [share](evidence.md#web-desktop-ide-and-sharing) |
| 성능 증거 | portable budget이나 packaged Electron coverage 없는 manual browser renderer suite | versioned complete process-tree baseline 없음 | 승자 불명, 하나의 evidence contract 재사용 | existing P0 | [limits](evidence.md#performance-and-quality), [existing slice](../jcode/gap-matrix.md#p0-native-performance-evidence-contract) |
| 릴리스 품질 | green Turbo core/app task와 별도 실행된 workflow-ungated desktop test, desktop/i18n/publish wiring gap | 폭넓은 Rust/native test, exact-SHA validation evidence 계획 상태 | green evidence를 released source에 결합해야 함 | existing P0 | [quality](evidence.md#performance-and-quality), [validation slice](../jcode/gap-matrix.md#p0-frozen-source-validation-evidence) |
| 게임 운영 | 비교 가능한 product surface 없음 | 차별화된 Admin diorama가 있으나 semantic truth 작업 계획됨 | 기존 truthful state-machine slice 유지 | existing P0 | [Canvas slice](../agent-canvas/gap-matrix.md#p0-truthful-diorama-state-machine) |

## 포트폴리오 규칙

OpenCode는 이미 소유자가 있는 여러 Akra 작업 항목에 새 증거를 제공한다. 이에 대해 병렬 backlog
이름을 만들면 안 된다.

| OpenCode 교훈 | 소유 중인 기존 Akra 항목 | 수정만 수행 |
| --- | --- | --- |
| 별도 server-scoped async prompt 및 attach | [Active Turn Exit And Restart Recovery](../jcode/gap-matrix.md#p0-active-turn-exit-and-restart-recovery) | external transport loss, field-level authority/omission mapping, event-gap/stale-state proof, 명시적 default-TUI/detach/crash label 추가 |
| diff/timeline/100-message hydration | [Protocol-Native Live Execution Rail](../jcode/gap-matrix.md#p0-protocol-native-live-execution-rail) | adaptive long-session window, truncation marker, 공식 older-history drilldown 추가 |
| browser session paging 및 child navigation | [Official Session Search And Provenance](../jcode/gap-matrix.md#p2-official-session-search-and-provenance) | fork/child lineage와 stable deep link 포함, embedding은 추가하지 않음 |
| GitHub event 및 schedule UX | [Automation Intake Core](../agent-canvas/gap-matrix.md#p1-automation-intake-core), 이후 schedule/webhook slice | trigger-to-session/branch/PR deep link 포함, typed template 및 idempotency 유지 |
| renderer throughput 및 frame-gap metric | [Native Performance Evidence Contract](../jcode/gap-matrix.md#p0-native-performance-evidence-contract) | 동일 raw artifact schema 아래 dense diff, long transcript, streaming burst, resize profile 추가 |
| agent completion 대 delivery | [Authoritative Delivery Outcomes](../jcode/gap-matrix.md#p1-authoritative-delivery-outcomes) | agent/session, source, review, integration, cleanup outcome을 각각 유지 |

OpenCode로 새 P0를 만들지 않는다. detach, live rail, performance, validation, activity,
automation, session 교훈은 기존 jcode/Canvas 포트폴리오를 수정한다. 새로 review 가능한 작업은
좁은 P1 approval-choice slice와 3개 P2 follow-up으로 한정한다. 검사 가능한 file approval, typed
request form, Akra의 기존 command registry를 위한 contextual metadata다. 공식 fork lineage는 기존
P2 session provenance contract의 제한된 sub-slice다. canary 작업은 각 소유 기능과 함께 matrix가
커지는 비교 experiment 및 release invariant로 남는다.

## 도입

### 1. 정확한 선택 충실도

OpenCode는 typed permission과 question form이 운영자에게 주는 가치를 보여 준다. Akra가 고정한
공식 schema는 더 풍부하다. command approval은 one-shot accept, session accept, exec-policy
amendment, network-policy amendment, decline, cancel을 제공할 수 있고 server request에는 typed
file, permission, input, MCP elicitation form이 포함된다.

정확한 app-server request method와 그 `availableDecisions`를 도입한다. 범용 approve button을
고안하지 않는다. one-shot acceptance는 기본값으로 유지한다. persistent 또는 session-scoped
choice는 server가 제공했을 때만 두 번째 명시적 operator action 뒤에 scope와 proposed rule을
보여 주며 나타난다. unattended worker는 계속 decline한다.

### 2. 공식 포크 계보

OpenCode의 fork UX는 유용하지만 명시적 lineage 없는 transcript 복사는 Akra schema에 이미 있는
공식 Codex primitive보다 약하다. session browser와 current session에서 fork action, 선택적 last-turn
selection, source thread로 돌아가는 stable link를 도입한다. fork를 accepted task 및 delivery truth와
연결하는 데 필요한 application provenance만 저장하며 app-server가 thread authority로 남는다.

<a id="3-contextual-command-discovery"></a>
### 3. 상황별 명령 탐색

Akra에는 이미 하나의 registry와 검색 가능한 palette가 있다. 다른 command system이 아니라 field를
추가한다.

- 안정적인 명령 식별자, 레이블, 별칭, 인수 형식, 그룹;
- current availability와 disabled일 때 제한된 reason;
- 같은 registry entry를 호출하는 discovery-only leader/which-key binding;
- hidden telemetry가 아닌 typed current mode와 최근 explicit use 기반 palette ranking;
- width-aware projection과 완전한 `:help` fallback.

Approval resolution, text editing, destructive confirmation은 modal로 남으며 오래된 palette entry로
trigger할 수 없다.

### 4. 조정된 외부 상태

OpenCode browser는 external TUI보다 더 많은 root, directory, catalog, status state를 복구하지만
cached-transcript 전체 reload를 입증하지 않는다. Akra는 공식 API가 허용하는 곳에서 더 나아가야
한다. gap 가능성이 있으면 오래된 selected window를 invalidate하고, 공식 thread/turn truth와 Akra의
durable planning/delivery state에서 reconcile하며, 남은 ambiguity를 표시한다. Akra가 app-server에
application-owned replay cursor를 내보내게 할 수는 없고 silence는 절대 completion을 입증하지 않는다.

### 5. 응집력 있는 상세 탐색

OpenCode는 선택한 session 주위에 diff, timeline, permission, question, child navigation을 둔다.
Akra는 이 information architecture를 자체 사실에 적용해야 한다. 제한된 live rail, 선택한 full diff,
현재 approval/elicitation, 공식 fork lineage, delivery provenance를 제공한다. 범용 file browser, PTY,
browser IDE는 복사하지 않는다.

## 거부

### 공급자 및 도구 런타임 중복

Akra core runtime으로 multi-provider routing, local model/tool ownership, 병렬 permission engine,
ACP를 거부한다. 이는 Codex wrapper를 선택할 이유를 약화하고 공식 app-server가 이미 소유한 semantic에
두 번째 authority를 만든다.

### 선택적 또는 원시 원격 제어

authentication-optional server, password를 저장하는 browser local storage, raw config, provider,
credential, file, PTY, shell, runtime plugin을 노출하거나 변경하는 API route를 거부한다. 계획된
remote-node surface는 read-only, mutually authenticated, versioned 상태로 유지되고 application DTO에서
도출된다.

public-repository default session sharing을 거부한다. 향후 explicit export는 payload를 열거하고
redact하며 destination과 policy를 보여 주고 operator action을 요구하고 delivery success와 분리돼야 한다.

### 임의 플러그인 및 변경 가능한 설치 프로그램

in-process arbitrary plugin, mutable default-branch archive에서의 자동 LSP install, `@latest` delivery
action, full-environment MCP inheritance를 Akra 소유 확장 pattern으로 거부한다. 공식 Codex가 자체
MCP/tool boundary를 선택하며 Akra는 자신이 소유한 child environment와 application surface만 filter한다.

### 동일 체크아웃 백그라운드 작업

대화형 fan-out을 parallel delivery authority로 거부한다. 현재 checkout을 공유하거나 restart 때
ownership을 잃거나 synthetic prompt로만 보고하는 background task는 Akra의 task, lease, worktree,
source, review, integration, cleanup chain을 대체할 수 없다.

### 프롬프트 전용 GitHub 전달

`git add .`, agent-managed branch bypass, commit/push/PR completion을 terminal delivery proof로
거부한다. Git 및 GitHub side effect는 frozen identity, CAS, gate, remote verification, recovery를 갖춘
application service와 adapter에 유지한다.

### 전략으로서의 표면 폭

native TUI와 reviewed-delivery loop가 측정되고 명확히 더 좋아질 때까지 browser IDE, general desktop
client, VS Code launcher, public transcript sharing, generic remote harness를 거부한다. Admin은 두 번째
editor가 아니라 operations surface로 남는다.

## 차별화

### 자격 증명 경계를 둔 인증 제어 평면

Akra에는 이미 더 강한 초기 구성이 있다. 루프백 전용 바인딩, 무작위 `.localhost` 출처, 필수
기능/세션 인증, 기본적으로 정제된 app-server 자식 환경, 식별 정보를 확인하는 GitHub 쓰기다. 정확한
`AKRA_APP_SERVER_PROCESS_ENVIRONMENT=all`은 명시적인 고위험 재정의이며 카나리 근거에서 눈에 띄게
구별돼야 한다. 기본 구조를 실행 가능한 제품 근거로 전환한다. 모든 공개 DTO는 허용 목록으로
제한하고 자격 증명/콘텐츠 분류마다 명시된 허용 도착점과 금지 도착점이 있어야 한다. 금지 경로는
카나리로 테스트하고 알 수 없는 필드는 자동 직렬화하지 말고 실패 시 닫혀야 한다.

### Codex 프로토콜 책임성

Akra는 provider-neutral translation layer 없이 공식 command/file/permission/MCP choice, fork lineage,
turn steering, diff, thread identity를 노출해야 한다. golden trace가 어떤 field를 보존, 축소, 거부,
미지원하는지 입증할 때만 직접 경계가 가치 있다.

### 에이전트 완료가 아니라 검토된 결과

OpenCode는 agent run 뒤 PR을 만들 수 있다. Akra는 더 강한 경로를 입증해야 한다.

```text
accepted intent
-> isolated lease/worktree
-> official session and fork lineage
-> exact frozen source and validation
-> source branch and PR
-> default review/check/mergeability gates, or explicit high-risk exception
-> serialized integration and remote verification
-> PR/source/slot cleanup
```

모든 transition은 identity, time, reason, retry/recovery state, deep link를 유지한다. Agent text는 절대
terminal proof가 아니다.

### 진실한 네이티브 운영

OpenCode의 session UI가 더 폭넓지만 Akra는 Codex delivery fleet에 더 나을 수 있다. TUI와 Admin은
같은 live item, approval, fork, source, review, integration, cleanup truth를 보여 줘야 한다. game layer는
persisted semantic transition만 animate할 수 있다. versioned process-tree contract가 green이 된 뒤에만
native performance를 주장한다. accepted asynchronous work/result는 wake generation이 있는 durable
mailbox 또는 queue에 들어가 drained되거나 명시적으로 pending이 돼야 한다. process-local synthetic
prompt는 task 또는 delivery authority가 될 수 없다.

## 릴리스 불변 조건

### 알려진 카나리 기반 제한 외부 전송 매트릭스

이는 기존 Canvas [Permission And Secret Canary](../agent-canvas/gap-matrix.md#permission-and-secret-canary)
experiment를 현재 출시된 Akra 소유 boundary의 release invariant로 전환한다. 이는 quality evidence이지
새 product P0나 대체 Codex tool sandbox가 아니다.

**소유 경계**

- 현재 애플리케이션 서비스와 인바운드 Admin/CLI/Telegram 어댑터의 공개 DTO;
- app-server 자식 환경 필터링과 정제된 추적/로그 도우미;
- Git/GitHub 자격 증명 주입 및 명령 진단;
- 현재 SQLite 계획, 세션 상세, 전달 영속성 어댑터;
- 캡처한 출력과 저장소 Git 상태를 검사하는 테스트 픽스처 및 네이티브 검증 스크립트.

**계약**

- 현재 Akra 소유 app-server API 키 전달, 자식 환경, GitHub 자격 증명 전송, Admin 기능/세션,
  Telegram 봇 자격 증명, 허용 목록에 든 공개 사용자 식별자, 프록시 URL 자격 증명, 프롬프트와
  유사한 신뢰할 수 없는 표식, 비공개 워크스페이스 콘텐츠 경계를 목록화한다;
- provider/MCP internal credential은 upstream observation으로만 기록한다. Akra는 raw storage 또는
  transport를 소유하지 않으며 matrix가 upstream sink를 enforce한다고 주장하면 안 된다;
- 적용 가능한 모든 credential/content class에 고유 synthetic canary를 생성한다. public identifier와
  control-injection marker는 secret과 별도로 분류한다;
- injection 전에 source-to-sink matrix를 정의한다. credential canary는 opted-in app-server child
  environment 또는 ephemeral outbound HTTPS authentication transport처럼 이름 붙은 raw sink에만
  도달할 수 있다. 실험이 해당 content를 요청하면 prompt/file canary는 명시적으로 승인된 disposable
  official transcript에 나타날 수 있으나 실제 credential은 사용하지 않는다;
- handle, redacted descriptor, approved raw sink, forbidden secondary sink를 서로 다른 expectation으로
  취급한다. 운영자가 읽도록 명시적으로 허용한 content를 공식 Codex가 반환하는 일을 Akra가 막을 수
  있다고 절대 주장하지 않는다;
- public DTO는 allowlist로 정의하고 raw provider/config/server-request structure를 serialize하지 않는다;
- credential canary가 declared raw sink 외부의 모든 TUI projection, Admin JSON/HTML, CLI JSON/text,
  Telegram payload, 현재 SQLite text/blob column, filesystem planning artifact, log/trace/crash report,
  prompt, Git command argument/diagnostic, PR title/body/comment, repository/worktree Git configuration에
  없음을 입증한다;
- prompt/file canary는 explicit disposable source transcript에서만 허용한 뒤, 별도 explicit user action
  없이 Akra가 해당 값을 unrelated Admin summary, Telegram message, planning/delivery persistence, log,
  Git metadata, GitHub content로 증폭하지 않음을 입증한다;
- valid capability/session authentication 없는 Admin request는 어떤 data route에도 도달할 수 없고
  explicit token 없는 noninteractive startup은 실패함을 입증한다;
- 현재 interactive Admin bootstrap stream을 generated capability token의 유일한 approved raw sink로
  취급하거나 별도로 review된 behavior change로 해당 output을 제거한다. 거기에 정상적으로 나타나는
  것은 leak-test failure가 아니다;
- 현재 exact opt-in API-key forwarding을 app-server에 유지하되 값을 log하거나 upstream process가
  사용할 수 없다고 주장하지 않는다;
- default-scrubbed와 exact `AKRA_APP_SERVER_PROCESS_ENVIRONMENT=all` 사례를 별도로 capture한다.
  후자는 elevated-risk override이며 default boundary의 green 결과를 상속할 수 없다;
- disposable workspace canary로 공식 Codex read/shell behavior를 관찰하고 unsupported live path를
  각각 `unknown`으로 기록한다. 이 experiment는 현재 Akra-owned sink에 gate를 걸지 않으며 Akra
  tool-policy layer를 추가하지 않는다;
- 테스트한 Akra boundary가 생성하는 것으로 알려진 URL/basic-auth/base64 표현만 포함하여 제한된 output
  artifact를 byte 및 UTF-8 text로 scan한다. 이는 제한된 regression scanner이지 일반 data-loss
  prevention이 아니다;
- expected capture 누락, scanner error, unknown public DTO field, forbidden match가 있으면 matrix check를
  실패시킨다. 빈 artifact directory에서 green을 보고하지 않는다;
- 실행 뒤 canary state를 삭제하고 source/worktree Git config와 process environment에 residue가 없음을
  검증한다;
- 각 disposable DB writer가 종료한 뒤 main SQLite file의 free page 및 제한된 `-journal`, `-wal`,
  `-shm`, temporary, backup candidate를 열거나 변경하지 않고 raw byte로 scan한다. active-row query와
  physical-file scan은 별도 증거이며 write/delete cycle은 빈 final query만으로 통과할 수 없다;
- credential mint, untrusted attachment download, 기타 remote side effect 전에 repository/actor identity를
  authorize한다;
- raw third-party credential을 Akra application persistence에 저장하지 않는다. durable lookup이 필요한
  미래 adapter는 자체 reviewed secret-store boundary를 정의하고 해당 feature PR에 matrix row를 추가해야
  한다;
- 모든 worktree에서 `.git`이 directory라고 가정하지 말고 `git rev-parse --git-common-dir`,
  `git rev-parse --git-path config`, worktree config, include, `GIT_CONFIG_*`, helper, askpass로 Git state를
  검사한다. ephemeral helper input 또는 outbound HTTPS auth header는 approved raw sink지만 repository
  config는 아니다.

**증거 확장**

초기 scanner/manifest는 공유 quality infrastructure이며 출시된 boundary만 다룬다. 이후 approval,
automation, webhook, remote-node, MCP, credential-bearing feature는 각 feature의 reviewable PR에서 자체
source/sink row와 capture를 추가한다. 기존 row는 green일 수 있고 future/unsupported row는 없거나
명시적으로 `unknown`일 수 있지만, 필요한 현재 capture가 누락되면 여전히 fail closed한다.

**사용자 결과**

운영자는 Akra가 알려진 credential value를 approved transport sink 밖으로 복사하지 않고 명시적으로
승인된 file/prompt content를 unrelated surface로 증폭하지 않는다는 exact-source evidence를 받는다.
주장은 canary matrix로 한정되며 범용 secret detector가 아니다.

**필수 증명**

- DTO 허용 목록, 재귀적 정제, 인코딩된 변형, 알 수 없는 필드 실패 단위 테스트;
- named current inbound/outbound surface마다 수행하는 integration test와 SQLite/filesystem round trip,
  각 approved raw sink가 실제 canary를 받았다는 positive assertion 포함;
- canary를 write한 뒤 delete하고 raw scanner가 residue를 계속 감지함을 입증하는 fixture를 포함한
  disposable SQLite active-row 및 post-writer physical main/sidecar/temp/backup scan;
- unauthorized Admin route matrix와 session idle/absolute-expiry test;
- approved sink가 canary를 받았지만 command argument, common/worktree config, include, diagnostic,
  repository file은 보존하지 않았음을 입증하는 fake Git credential helper/askpass capture;
- direct read, shell read, MCP, approval, interrupt, timeout 경로의 disposable official app-server canary
  trace와 명시적인 supported/unsupported 결과;
- process-tree teardown 및 residue scan;
- canary value를 저장하지 않고 source SHA, environment stamp, canary class, expected capture, scanner
  version, zero forbidden match를 포함하는 추적 manifest.

## 구현 단위

### P1: 명령 및 권한 선택 충실도

이는 Canvas와 OpenCode 모두에서 확인된 첫 번째 choice-fidelity gap을 해소한다. 의도적으로 command와
제한된 permission approval에 한정한다. File change, user input, MCP elicitation은 별도 P2 slice가
반영될 때까지 현재 fail-closed 동작을 유지한다.

**선행 조건**

- Active Turn Steering과 Active Turn Exit And Restart Recovery가 child/turn loss 및 불확실한 transport
  recovery를 소유한다;
- current-surface known-canary matrix와 protocol classification test가 green이다.

**소유 경계**

- `src/domain/conversation.rs`의 타입이 지정된 요청, 결정, 해결, 출처 타입;
- app-server approval/server-request parsing 및 response serialization;
- interactive runtime control port와 conversation service;
- TUI 모달 상태, 입력 컨트롤러, 명령/권한 승인 렌더링.

**계약**

- command/permission ownership에 server request ID, method, `threadId`, `turnId`, `itemId`를 요구한다.
  고유한 `approvalId` callback만 없을 수 있다. child generation, workspace ownership, request age,
  decision-source classification, 제한된 inspectable fact도 유지한다;
- pinned protocol의 network-only command approval shape는 command와 cwd를 생략할 수 있지만 그
  network/host/protocol fact는 필수다. 공식 request가 해당 shape를 식별하면 command 누락만으로
  malformed가 아니다;
- command decision을 one-shot accept, session accept, exec-policy amendment, network-policy amendment,
  decline-and-continue, cancel/interrupt로 각각 model한다;
- rendering 전에 `availableDecisions`를 분류한다. 비어 있지 않은 valid array는 제공한 variant만 정확히
  노출한다. missing/null이면 모든 required identity/detail을 검증한 뒤 pinned legacy one-shot
  accept/decline set만 사용한다. empty, unknown, mixed-unknown, malformed input은 UI에 도달하지 않고
  method-specific decline을 받는다;
- required thread/turn/item ownership이 없거나 불일치하면 UI에 도달하지 않고 하나의 method-specific
  decline 시도를 통해 settle한다;
- session-scoped 또는 persistent amendment에는 두 번째 confirmation을 요구하며 exact scope, rule,
  host/protocol, choice가 한 command 뒤에도 유지되는지를 보여 준다;
- network/filesystem detail을 잃지 않고 제한된 permission profile을 render한다;
- file-change, user-input, MCP elicitation은 기존 explicit decline response로 유지한다;
- main interactive TUI만 decide할 수 있다. parallel, planning, automation, Telegram, CLI, disconnected
  runtime은 미래의 explicit authority contract가 달리 정하지 않는 한 decline한다;
- local ownership을 child generation, method, server request, workspace, thread, turn, callback/item
  identity로 key한다. compare-and-set으로 local response write를 최대 한 번만 허용한다;
- `pending -> locally_decided -> write_attempted -> write_confirmed`를 local transport fact로 나타낸다.
  어떤 live local state에서든 일치하는 `serverRequest/resolved`는 마지막 local fact를 보존하면서 request를
  `remote_cleared`로 진행시킨다. write 전에 lifecycle cleanup이 일어난 경우를 포함해 server에 더는
  pending request가 없음을 입증하지만 decision이 accepted, consumed, applied됐음을 입증하지 않는다.
  correlated authoritative item/turn outcome만 effect를 settle한다. write 뒤 disconnect 또는 timeout을
  fabricated accepted/declined terminal result로 만들지 않는다;
- timeout, disconnect, interrupt, stale turn, duplicate response, late UI action은 local UI authority를
  닫고 remote가 response를 consumed했다고 주장하지 않은 채 audit 가능하게 남는다;
- operator decision 전 deadline 또는 UI-queue loss가 발생하고 같은 child generation과 transport가 여전히
  valid하면 하나의 method-specific decline write를 atomically claim한다. 그 write가 불가능하거나 consumption이
  uncertain하면 interrupt 또는 child termination ownership을 Active Turn Exit And Restart Recovery에 넘기고
  `resolution_unknown`을 보존한다. settlement owner 없는 live upstream request를 남긴 채 UI를 닫지 않는다;
- general log에서 raw command/permission value를 redact하면서 제한된 operator-visible fact와 decision
  provenance를 유지한다;
- 새 request method가 explicit support 또는 reject되기 전까지 test를 실패시키도록 protocol classification
  contract를 갱신한다.

**사용자 결과**

TUI는 공식 Codex가 실제 제공한 command 및 permission choice를 generic approve button으로 축소하지
않고 표시한다. one-shot, session, persistent, decline, cancel semantic이 눈에 띄게 구분되고 local write
ownership은 race-safe하며 remote consumption을 과장하지 않는다.

**필수 증명**

- 모든 command decision과 제한된 permission profile에 대한 schema fixture, 그리고 non-empty,
  missing, null, empty, unknown, mixed-unknown, malformed `availableDecisions` 사례;
- 고정된 공식 app-server fixture에 대한 round-trip response serialization;
- 좁고 넓은 terminal size에서 command 및 permission modal test;
- persistent decision을 위한 second-confirmation 및 scope-copy test;
- 중복, 시간 초과, 쓰기 전 연결 해제, 쓰기 후 정리 전 연결 해제, 중단, 오래된 자식/턴/워크스페이스,
  콜백 ID, 늦은 작업, 누락/불일치/중복 `serverRequest/resolved`, 쓰기 전후 수명 주기 정리,
  권한 있는 결과 경합 테스트;
- command/cwd가 생략된 network-only request fixture와 malformed lookalike;
- live child에서 decision 전 timeout이 발생하면 ownerless pending request가 아니라 하나의 decline write 또는
  explicit recovery-owned interrupt/termination transition이 생김을 입증;
- UI-queue loss 및 required thread/turn/item identity의 missing/mismatch가 approval을 보여 주지 않고 동일한
  decline-or-recovery settlement를 입증;
- 계획/병렬 처리 및 연결이 끊긴 주 런타임 전반의 무인 거부 테스트와 변경되지 않은 파일 변경,
  사용자 입력, MCP 거부 테스트;
- release-invariant canary matrix의 새 command/permission source-to-sink row;
- command 또는 permission, decline, cancel, session-accept 동작에 대한 하나의 실제 disposable app-server
  capture. 지원되지 않는 live case는 unverified로 표시한다.

### 기존 P0 의존성: Active Turn Exit And Restart Recovery

app-server 옆에 `attach` subsystem을 만들지 않는다. 기존
[계약](../jcode/gap-matrix.md#p0-active-turn-exit-and-restart-recovery)을 다음과 같이 수정한다.

- 깨끗한 TUI 연결 분리, 제어된 종료, app-server 자식 손실, stdio 전송 손실, Akra 프로세스 재시작,
  공식 스레드 비활성 상태를 구분한다;
- recovery 전에 field-authority map을 정의한다. official current thread/status/turn identity, 반환된
  stored `ThreadItem` history, event-only delta/tool detail, Akra durable state는 명시적 omission rule이
  있는 별도 source다;
- gap 뒤에는 더 많은 input을 받기 전에 해당 map에 필요한 사용 가능한 모든 official projection을
  query하되 pinned protocol이 보장하는 field만 authoritative로 부른다;
- resume 전에 expected thread/turn identity와 durable pending-submit identity를 비교한다;
- `thread/read`가 생략한 event-only detail은 `history_incomplete`로, 누락되거나 모순된 active-turn
  identity는 `unknown` 또는 `recovery_required`로 표시한다;
- 누락된 delta를 합성하거나 prompt를 자동 replay하거나 quiet에서 completion을 추론하지 않는다;
- unique reconciliation 뒤 다음 submission이 정확히 한 turn을 생성함을 입증한다.

필수 증명은 기존 slice에 남으며 event-gap, external-loss, stale projection, field-authority,
lossy-thread-read, `history_incomplete`, terminal-event-loss, bounded-saturation fixture를 추가한다.

### 기존 P0 의존성: Protocol-Native Live Execution Rail

기존 [live rail](../jcode/gap-matrix.md#p0-protocol-native-live-execution-rail)을 다음과 같이 수정한다.

- current item과 recent turn을 위한 제한된 hot window를 유지한다;
- window를 full history처럼 가장하지 말고 truncation을 표시하고 official older-turn loading을 노출한다;
- 기존 policy 아래 제한된 drilldown storage에 full aggregated turn diff를 유지한다;
- text parsing 없이 child/fork identity와 approval/elicitation phase를 projection한다;
- saturation 아래 progress delta를 coalesce하고 terminal/error transition을 보존한다;
- 또 다른 transcript store를 추가하지 않고 dense diff, 10k-message catalog metadata, resize, burst
  streaming을 test한다.

### 기존 P0 의존성: Native Performance Evidence Contract

하나의 [performance artifact schema](../jcode/gap-matrix.md#p0-native-performance-evidence-contract)를
유지한다. dense diff, long-session hydrate/load-more, event burst, palette filtering, repeated resize를
위한 OpenCode 기반 profile을 추가한다. Akra와 모든 app-server descendant를 측정한다. OpenCode bundle
size나 manual Chromium result를 Akra gate로 사용하지 않고, 두 제품이 호환되는 authenticated harness를
통과하기 전에는 승자를 발표하지 않는다.

### 기존 P2 하위 단위: Protocol-Native Forked Session Lineage

**선행 조건**

- 안정적인 공식 session selection/resume 경로;
- ambiguous outcome과 active-turn ownership이 하나의 recovery model을 갖도록
  [Active Turn Exit And Restart Recovery](../jcode/gap-matrix.md#p0-active-turn-exit-and-restart-recovery)가
  완료됨;
- app-server protocol drift check가 green;
- 계획된 semantic search 또는 remote-node 작업에 의존하지 않음.

**소유 경계**

- 기존 [Official Session Search And Provenance](../jcode/gap-matrix.md#p2-official-session-search-and-provenance)가
  catalog, lineage projection, Admin detail, stable navigation을 소유한다;
- 이 제한된 sub-slice는 app-server fork request/response adapter path만 소유한다;
- 대화/세션 애플리케이션 서비스;
- TUI current-session 및 Sessions-overlay fork action.

**계약**

- source thread 및 optional last turn으로 공식 `thread/fork`를 호출하고 Akra에서 message를 복사하지 않는다;
- 반환된 thread ID와 `forkedFromId`, source/fork display title과 creation time을 보존한다;
- fork를 선택하기 전에 반환된 lineage가 요청한 source와 일치하는지 검증한다;
- source와 fork를 기존 공식 catalog를 통해 서로 독립적으로 resumable하게 한다;
- source thread를 변경하지 않고 Akra planning 또는 delivery authority가 암묵적으로 전송되지 않음을
  입증한다;
- source session이 accepted task와 연결돼 있으면 fork에 명시적 provenance edge를 만든다. lane, lease,
  branch, PR, delivery record를 조용히 재할당하지 않는다;
- app-server가 요청한 turn boundary를 명시적으로 지원하고 Akra가 이를 입증할 수 있지 않은 한 unresolved
  approval/elicitation 또는 active submission 중 fork를 막는다;
- local action 주위에 idempotency/CAS를 사용하여 timeout retry가 무제한 duplicate fork를 만들지 않게
  한다. ambiguous server outcome에는 catalog reconciliation과 operator choice가 필요하다;
- broken, missing, imported, non-resumable lineage를 parent를 고안하지 않고 표시한다;
- 이 slice에 deprecated rollback semantic을 추가하지 않는다.

**사용자 결과**

운영자는 선택한 turn에서 공식 Codex conversation을 branch하고 정확한 출처를 확인하며 어느 thread로도
돌아가고 delivery ownership을 명시적으로 유지할 수 있다.

**필수 증명**

- full fork, last-turn fork, malformed lineage, timeout, duplicate retry, protocol drift용 adapter fixture;
- source-unchanged 및 independent-resume integration test;
- implicit authority transfer를 방지하는 planning/task/lease provenance test;
- restart/catalog reconciliation 및 ambiguous-outcome test;
- TUI narrow/wide fork-action snapshot. lineage navigation/Admin projection은 parent P2 session-provenance
  slice의 acceptance로 남는다;
- evidence policy가 요구하는 곳에서만 source 및 child thread ID를 redact한 실제 app-server fork/resume
  capture 하나.

### P2: 상황별 명령 레지스트리 메타데이터

이는 출시된 `:` palette를 확장하며 병렬 application command bus를 만들지 않는다.

**소유 경계**

- `src/adapter/inbound/tui/app/inline_shell_commands.rs` 레지스트리 메타데이터;
- 타입이 지정된 TUI 컨텍스트-사용 가능 여부 리듀서;
- 팔레트, 도움말, 선택적인 리더 키/키 안내 표시;
- 기존 shell controller/executor와 overlay key routing.

**계약**

- 하나의 registry entry를 typed command, alias, palette, help, discovery binding의 source로 유지한다;
- stable ID, group, argument synopsis, availability predicate, 제한된 unavailable reason, optional discovery
  binding을 추가한다;
- predicate는 typed current state를 사용하며 I/O 또는 mutation을 하지 않는다;
- item accept 시 실행 전에 최신 state에 대해 availability를 다시 검증한다;
- disabled item은 검색 가능하게 남고 이유를 설명한다. hidden은 transient state가 아니라 현재
  build/backend에서 사용할 수 없는 capability에만 사용한다;
- leader/which-key는 discovery surface일 뿐이며 동일 registry action을 호출한다;
- conflict, unreachable binding, duplicate alias/ID, palette-only executable action은 test를 실패시킨다;
- approval, editor-local key, destructive confirmation, overlay-owned navigation을 global registry로
  옮기지 않는다;
- leader key를 사용할 수 없거나 사용하면 안 되는 terminal에서도 `:help`와 direct typed command를
  보존한다;
- input buffer나 terminal geometry를 바꾸지 않고 narrow width에서 rendered row를 제한하고 group을
  결정적으로 collapse한다.

**사용자 결과**

운영자는 현재 context에서 올바른 action을 찾고 action을 사용할 수 없는 이유를 이해하며 두 번째
command vocabulary를 암기하지 않고 shortcut을 배울 수 있다.

**필수 증명**

- 레지스트리 고유성, 별칭, 충돌, 도달 가능성, 상태 매트릭스 테스트;
- stale availability revalidation 및 approval-modal non-bypass test;
- 타입이 지정된 명령, 팔레트, 도움말, 탐색 바인딩 동등성 테스트;
- 프롬프트 편집/오버레이 소유권 회귀 테스트;
- narrow/wide terminal snapshot과 실제 resize capture;
- 기존 performance contract에 추가한 palette filtering/input-latency sample.

### P2: 검사 가능한 파일 변경 승인

**선행 조건**

- [Protocol-Native Live Execution Rail](../jcode/gap-matrix.md#p0-protocol-native-live-execution-rail)이
  검사에 필요한 complete bounded patch와 target identity를 유지한다;
- Command And Permission Choice Fidelity가 child/request ownership lifecycle을 제공한다.

**소유 경계**

- app-server 파일 변경 승인 파서/직렬화기;
- 제한된 패치 검사 모델;
- TUI file-change modal 및 local response ownership.

**계약**

- 전체 patch, target identity, scope, advertised decision이 bounded inspection contract에 들어맞지 않으면
  현재 automatic decline을 유지한다;
- truncated patch, unknown target, stale child/turn, mismatched item은 절대 approve하지 않는다;
- command approval과 동일한 local write lifecycle 및 `resolution_unknown` 동작을 사용한다;
- unattended planning/parallel path는 automatic decline을 유지한다;
- 이 feature PR에 file-content canary row를 추가하여 explicit inspection modal은 허용하고 unrelated
  persistence/projection sink는 금지한다.

**필수 증명**

- 완전함/잘림/크기 초과/바이너리/경로 변경/오래된 턴 픽스처;
- narrow/wide patch modal snapshot 및 explicit decline fallback;
- disconnect-before/after-write race와 제공되는 경우 실제 app-server capture 하나.

### P2: 타입이 지정된 사용자 입력 및 MCP 추가 정보 요청 양식

**선행 조건**

- approval ownership lifecycle이 출시됨;
- release-invariant source/sink matrix가 form content를 credential과 별도로 분류할 수 있음.

**소유 경계**

- typed user-input 및 MCP elicitation request/response domain;
- app-server 서버 요청 파서/직렬화기;
- TUI form 및 URL-decision modal.

**계약**

- user-input과 MCP elicitation은 identity와 response shape가 다르므로 구분해 유지한다;
- 제한된 string, enum, number/integer, boolean, required/default/range, format metadata만 지원한다.
  unsupported schema feature는 free-form prompt로 바꾸지 않고 decline한다;
- URL elicitation을 눈에 보이는 host와 scheme allowlist를 갖춘 explicit external-open decision으로
  취급한다. 자동 fetch 또는 open하지 않는다;
- approval slice의 child/method/thread/turn/workspace/elicitation ownership 및 local write-state semantic을
  사용한다;
- parallel, planning, CLI, Telegram, automation, disconnected runtime은 계속 decline한다;
- 이 feature PR에 form-content 및 새 credential-handle canary row가 있으면 추가한다.

**필수 증명**

- 지원하는 모든 기본 타입, 필수값/기본값/범위 동작, 알 수 없는 스키마, URL 스킴/호스트, 오래된
  식별 정보, 시간 초과, 쓰기 후 연결 해제 픽스처;
- narrow/wide form snapshot 및 unattended decline test;
- disposable app-server/MCP setup이 실제 제공하는 case에만 live capture. 나머지는 fixture-verified 또는
  명시적으로 unverified로 남는다.

### 기존 P1 자동화 수정: 안정적인 결과 딥 링크

또 다른 automation slice를 추가하지 않는다. 기존 Canvas sequence를 수정한다.

```text
trigger definition -> intake event -> run/attempt -> accepted direction/task
-> lease -> official session/fork -> source branch/SHA -> validation -> PR/review
-> delivery attempt -> integration -> cleanup
```

Schedule 및 webhook UX는 이 chain으로 deep-link할 수 있지만 prompt string, GitHub event, agent commit에
delivery authority를 부여하지 않는다. 기존 idempotency, signed webhook, reconciliation,
authoritative-delivery 선행 조건은 바뀌지 않는다.

- `Automation Intake Core -> Schedule Intake / Signed Webhook Intake`;
- `Frozen-Source Validation -> Critical Review Response -> Authoritative Delivery Outcomes
  -> Automation Run Reconciliation`;
- `Guarded Admin Control Actions -> Admin Automation Run Trace`.

### 기존 P2 세션 수정: 계보 및 더 불러오기

[Official Session Search And Provenance](../jcode/gap-matrix.md#p2-official-session-search-and-provenance)에
공식 fork/child lineage, provider-cursor load more, stable source/fork navigation, trigger/task/PR deep link를
포함하도록 수정한다. imported/non-resumable session은 read-only로 유지하고 이 slice에 semantic
embedding을 추가하지 않는다.

## 비교 실험

### 권한 및 시크릿 카나리

기존 Canvas [실험](../agent-canvas/gap-matrix.md#permission-and-secret-canary)을 current-surface baseline으로
재사용한다. 각 owning P1/P2 또는 automation/remote feature는 해당 feature PR에서 새 matrix row만
추가한다. 다음을 추가한다.

- 공식 Codex policy 아래 동일 disposable `.env` canary의 direct read와 shell read 비교;
- Admin, log, SQLite, Git, PR adapter capture의 encoded secret variant;
- authenticated 및 unauthenticated Admin route matrix;
- P1 slice와 함께 one-shot, session, persistent-policy, decline, cancel, permission case;
- Inspectable File-Change Approval의 file-change row와 typed-forms P2 slice의 MCP/user-input/URL row;
- live path를 fixture로 대체하지 않고 명시적인 `not offered`, `unsupported`, `unverified` 결과.

### 연결 해제 및 조정 매트릭스

main 및 parallel session에서 submit write 전, write 후/response 전, streaming 중, approval 중,
terminal event 후/persistence 전, bounded receiver saturation 아래, shutdown 중에 loss를 주입한다. 각
case마다 사용 가능한 official projection과 그 declared omission, Akra durable state, 표시된
`history_incomplete`/recovery status, 허용되는 next action, duplicate-turn count를 검증한다.

### 긴 세션 상호작용 프로필

10k message, 1k tool item, dense diff, child/fork lineage, bursty output을 갖춘 deterministic fixture를
만든다. 기존 performance harness를 통해 hydration, palette search, selected timeline, load more, resize,
memory, event coalescing을 측정한다. fixture는 bound와 regression을 입증하며 model quality를 simulate하지
않는다.

## 권장 순서

1. active validation lane이 비면 기존 Native Performance Evidence Contract를 반영한다. artifact schema를
   바꾸지 않고 OpenCode 기반 profile을 추가한다.
2. 현재 출시된 Akra-owned sink를 위한 known-canary matrix를 quality evidence로 확립한다. 미래 surface는
   자체 PR에 row를 추가하며 존재하지 않는 matrix entry 때문에 block하지 않는다.
3. Active Turn Steering을 완료한 뒤 field-authority gap reconciliation과 explicit history omission을
   갖춘 Active Turn Exit And Restart Recovery를 완료한다.
4. Protocol-Native Live Execution Rail을 완료한 뒤 기존 dependency order에 따라 Parallel Current
   Activity Envelope와 Truthful Diorama State Machine을 완료한다.
5. recovery가 안정화되면 별도 protocol/TUI lane에서 Command And Permission Choice Fidelity를 구현한다.
   file-change와 typed form은 P2 선행 조건이 반영될 때까지 fail closed한다.
6. Frozen-Source Validation Evidence, Critical Review Response Loop, Authoritative Delivery Outcomes 순으로
   완료한다. 이 chain이 green이 되기 전에 run reconciliation은 terminal delivery를 주장할 수 없다.
7. 그다음 Automation Intake Core에서 Schedule Intake와 Signed Webhook Intake로 branch할 수 있다.
   Automation Run Reconciliation은 Authoritative Delivery Outcomes 뒤에, Admin Automation Trace는
   Guarded Admin Control Actions 뒤에 온다. DAG를 바꾸지 않고 stable deep link를 추가한다.
8. Admin operational metric/control과 기존 read-only remote-node sequence는 문서화된 순서로만 추가한다.
9. Active Turn Recovery 뒤 Official Session Search And Provenance를 제한된 protocol-native fork sub-slice와
   lineage/load-more projection으로 확장한다.
10. P0 protocol/delivery wedge가 측정되고 green이 된 뒤 Inspectable File-Change Approval, Typed User
    Input/MCP Forms, Contextual Command Registry Metadata를 P2 follow-up으로 취급한다.

quality-evidence, performance, protocol/live-event lane은 worktree와 validation artifact ownership이
겹치지 않을 때만 병렬로 진행할 수 있다.

## 성공 감사

이 비교는 이후 Akra 증거가 다음을 모두 입증할 때만 가치를 만든다.

- 모든 필수 current 또는 feature-added credential/content canary가 declared positive sink에 도달하고,
  forbidden surface에는 나타나지 않으며, capture 누락 상태로 matrix가 통과할 수 없다;
- 지원하는 모든 official approval 또는 elicitation choice가 typed, scoped, inspectable, redacted 상태로
  유지되고 local write가 최대 한 번 발생하며, unsupported input은 fail closed하고 불확실한 remote
  consumption은 `resolution_unknown`으로 남는다;
- event/child loss가 duplicate prompt나 fabricated delta 없이 protocol-guaranteed field를 reconcile하며,
  생략된 event-only detail은 `history_incomplete`로 남는다;
- fork는 공식 `thread/fork`를 사용하고 `forkedFromId`를 보존하며 restart 뒤 독립적으로 resume하고
  task/delivery authority를 암묵적으로 전송하지 않는다;
- palette, typed command, help, discovery binding은 하나의 registry를 공유하고 current availability를
  다시 검증한다;
- live diff/timeline/session projection은 bounded이고 눈에 띄게 truncated되며 공식 older history를
  불러올 수 있다;
- performance evidence는 raw sample과 함께 완전한 Akra/app-server process tree를 다루고 지원되지 않는
  cross-product claim을 하지 않는다;
- automation deep link가 authoritative source, review, integration, cleanup outcome으로 이어진다;
- Admin game view는 persisted semantic transition에 대해서만 움직인다;
- 공급자 런타임, 임의 플러그인 호스트, 브라우저 IDE, 선택적 인증 제어 서버, 공유 체크아웃
  백그라운드 권한, 프롬프트 전용 전달이 이 목표를 밀어내지 않는다.
