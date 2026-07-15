# Orca v1.4.137 심층 분석

[English](../../../competitive/orca/analysis.md)

이 분석은 StablyAI Orca v1.4.137과 prerelease 커밋
`6274a7fc9703f85e4fb6247541dc0d4ed6c5fb9b`의 Akra를 비교한다. 변경 불가능한 소스 링크,
재현한 로컬 관찰, 산출물 식별 정보, 감사 한계는 [evidence.md](evidence.md)에 있다. Akra를
기준으로 한 결정과 리뷰 가능한 단위는 [gap-matrix.md](gap-matrix.md)에 있다.

## 요약 결론

Orca가 worktree를 탁월하게 다룬다는 운영자의 관찰은 검증됐다. 중요한 혁신은
`git worktree add`를 더 빠르게 감싼 wrapper가 아니다. Orca는 worktree를 coding task의 주요
application aggregate로 승격한다.

```text
repository and base ref
-> worktree, branch, and ownership
-> agent terminal, editor, browser, and setup
-> diff, commit, push, pull request, and checks
-> archive, safe removal, or recoverable blocked state
```

세 가지 구현 선택이 이 모델을 이례적으로 완전하게 만든다.

1. Git은 live worktree inventory의 authority로 남고 Orca는 durable product metadata, ownership,
   link, lineage, terminal layout, session state를 더한다.
2. 제거는 failure state를 중심으로 설계됐다. dirty file, Git lock, nested worktree, stale
   registration, orphan directory, moved branch, Windows file handle, interrupted cleanup은 하나의
   범용 force flag가 아니라 명시적 case다.
3. desktop UI와 machine-readable CLI가 local, WSL, SSH execution host 전반에서 같은 worktree
   model을 다룬다.

따라서 Orca는 Akra의 operator experience에 심각한 위협이다. 사람이 만든 변화하는 workspace
fleet을 쉽게 보고, 들어가고, 재개하고, 비교하고, 폐기할 수 있게 한다. Akra는 현재 동등한
repository-wide worktree portfolio를 제공하지 않는다. parallel board는 의도적으로 runtime이
소유한 3개 slot으로 범위를 한정한다.

그렇다고 Orca가 더 강한 delivery authority인 것은 아니다. Orca의 hosted-review path는 훌륭한
interactive Git client지만, 이 감사에서는 Akra의 fixed lease generation, frozen remote 및 source
identity, host-owned bounded commit, explicit parent-level exception이 있는 reviewed-by-default gate,
serialized detached integration worktree, remote-verification contract를 찾지 못했다. Orca는 global
Manual mode나 agent-specific override가 바꾸지 않는 한 새 launch에서 지원 agent용 high-autonomy
argument도 미리 채운다. 공식 guidance는 diff를 review하거나 discard할 수 있는 disposable checkout을
근거로 그 기본값을 정당화한다. 이는 유용한 containment지만 Git worktree는 file과 branch를 격리할 뿐
OS, credential, network, process, hook sandbox는 아니다.

전략적 대응은 좁다.

- Orca의 authoritative repository-wide inventory와 failure-state information design을 도입한다;
- Akra의 managed-slot, lease, security, reviewed-delivery authority를 보존한다;
- read model이 ownership과 staleness를 입증한 뒤에만 destructive worktree action을 추가한다;
- desktop/provider/terminal surface 경쟁과 worktree-as-security-sandbox framing을 거부한다.

공정한 동일 머신 종단 간 performance benchmark는 실행하지 않았다. worktree 결론은 functional 및
safety-design 결론이지 startup, memory, latency 주장이 아니다.

## 제품과 대상 사용자

Orca는 terminal-only TUI가 아니다. embedded terminal과 companion CLI를 갖춘 Electron desktop
Agent Development Environment다. 여러 coding agent를 병렬로 실행하고, 각 task에 isolated branch와
directory를 제공하며, terminal 및 workspace state를 유지하고, 결과를 비교하고, 선택한 결과를 source
control로 보내려는 개발자를 대상으로 한다.

선택할 이유는 응집력 있는 workspace loop다. repository sidebar, worktree creation, agent launch,
terminal tab, file 및 browser state, diff review, hosted pull-request state, cleanup이 모두 하나의
user-visible object를 참조한다. 공식 recipe는 동일 prompt를 3개 worktree에서 시작하고 diff를 비교한
뒤 하나를 ship하고 나머지를 삭제하는 과정을 명시적으로 보여 준다.

표면은 처음 설명보다 넓다.

- local, WSL, SSH repository 및 terminal;
- 지원되는 여러 coding-agent CLI;
- 워크트리를 인식하는 편집기, 브라우저, 변경 차이, 소스 제어 패널;
- repository, worktree, terminal, file, browser control, automation, orchestration, computer-use operation을
  위한 JSON 지원 CLI;
- 예약 자동화, 실험적 오케스트레이션, 모바일 접근, 세션 최대 절전.

이 폭넓음은 Orca를 agent CLI 주변의 operator shell로 만든다. 해당 agent의 model 또는 tool runtime을
대체하지는 않는다.

## 아키텍처와 상태 소유권

감사한 릴리스는 큰 main-process runtime, renderer, CLI, terminal daemon, host provider, relay code를
갖춘 Electron application이다. 관련 소유권 구분은 다음과 같다.

| 상태 | 실질적 권한 |
| --- | --- |
| registered worktree path, `HEAD`, branch, sparse 및 lock state | Git worktree inventory |
| Orca 소유권, 생성 출처, 기준점, 푸시 대상, 링크, 계보, 설명 | Orca 메타데이터 |
| pane, tab, terminal binding, scrollback, editor/browser layout | Orca persistence 및 terminal daemon |
| process 및 filesystem execution | local, WSL, SSH host provider |
| PR, check, review, merge state | GitHub/GitLab 및 Orca refresh coordinator |
| agent conversation/runtime semantic | 선택한 agent CLI |

이는 유용한 합성이다. Orca는 metadata row가 path를 worktree로 만든다고 가장하지 않는다. 새로운
local discovery는 `git worktree list`에서 시작하고 그 뒤 product metadata를 join하며 mutation 뒤
cached scan을 invalidate한다. 연결이 끊긴 SSH host에서는 저장된 metadata를 보여 줄 수 있지만,
cache를 live Git truth로 제시하는 대신 결과를 non-authoritative로 표시한다.

모델은 path와 branch보다 풍부하다. Worktree metadata에는 creation time과 source, base ref, push
target, issue 및 pull-request link, agent state, sparse-checkout state, prior ID, task 또는 dispatch
identifier가 있는 parent/child lineage가 포함될 수 있다. 이점은 navigation과 recovery이고 위험은 Git
또는 filesystem이 Orca 외부에서 바뀔 때마다 생기는 큰 reconciliation surface다.

<a id="worktree-lifecycle"></a>
## 워크트리 수명 주기

### 탐색, 소유권, 외부 변경

Orca는 NUL-delimited Git porcelain을 선호하고 path, `HEAD`, branch, bare, lock, lock reason을 parse하며
이전 Git 동작에는 fallback한다. sparse-checkout state는 별도의 bounded probe로 annotate한다.
concurrent scan은 coalesce되고 mutation generation은 create/remove operation 전에 시작한 결과가 더
새로운 view를 덮어쓰지 못하게 한다.

worktree를 Orca-managed, external, unknown legacy state로 분류한다. 강한 metadata와 알려진 layout은
ownership evidence지만 임의 directory name은 아니다. 외부에서 만든 worktree는 import inbox에
나타나거나 repository policy에 따라 계속 visible할 수 있다. external deletion은 refresh 때 live Git
view에서 제거된다. scan은 누락된 child lineage를 prune하고 누락된 parent의 instance identity를 rotate할
수 있지만 모든 durable `worktreeMeta` entry를 즉시 지우지는 않는다.

감사 중 설치된 v1.4.137 runtime을 이 public Akra repository에 연결했다. Git과 정확히 동일하게 primary
checkout, 3개 detached Akra pool slot, 4개 branch worktree를 보고했다. application restart 없이 Orca
외부에서 만든 audit worktree를 감지했다. 이는 sanitized observation이며 repository UUID, home path,
prompt, terminal preview는 보존하지 않았다.

### 생성, 브랜치, 초기 구성

사용자는 local branch, remote branch, commit에서 시작할 수 있다. Orca는 가능하면 base를 resolve하고
refresh하며 bounded suffixing으로 branch/path collision을 처리하고 pull-request 또는 merge-request
start point를 인식한다. remote refresh의 offline failure가 이미 사용 가능한 local ref를 지우지는 않는다.

새 local branch는 `git worktree add --no-track -b`를 사용한다. inherited tracking을 피하는 것은 작지만
중요한 세부 사항이다. 새 task branch가 base branch를 push upstream으로 실수로 취급하지 않는다. Orca는
실제 base를 branch configuration에 기록하고 user가 이미 설정하지 않은 경우에만 shared repository
configuration에서 `push.autoSetupRemote`를 활성화할 수 있다. 이는 first-push ergonomics를 개선하지만
눈에 보여야 하는 repository-wide side effect다.

생성은 branch reuse, sparse checkout, shared/symlinked path, setup command, bootstrap 실패 시 rollback도
지원한다. setup은 새로 만든 worktree의 `orca.yaml`에서 가져와 visible terminal에서 실행한다. visible한
것이 hidden bootstrap process보다 낫지만 repository-controlled setup 실행은 여전히 trust decision이다.
worktree가 hook, filter, command를 안전하게 만들지는 않는다.

### 터미널, 세션, 워크스페이스 연속성

각 worktree는 terminal tab과 agent session의 scope가 된다. stable worktree, pane, tab, terminal handle을
terminal environment에 inject하여 CLI automation이 active workspace를 address할 수 있게 한다. PTY binding과
minimum layout은 spawn completion 전에 flush되어 recoverable UI ownership 없는 process가 존재하는 window를
좁힌다.

terminal daemon은 desktop renderer가 닫힌 뒤에도 process와 scrollback을 살려 둘 수 있다. machine reboot
뒤에는 process가 사라지지만 layout과 저장된 scrollback을 restore할 수 있다. Session hibernation은
명시적으로 experimental이며 agent의 resume primitive에 의존한다. 이는 혼동하면 안 되는 서로 다른 세
가지 continuity claim이다.

- 렌더러/애플리케이션 연결 분리 연속성;
- persisted layout 및 scrollback restoration;
- agent-process 또는 conversation resume.

persistence는 local, SSH, remote-runtime host를 partition한다. WSL은 explicit WSL Git, path, cache,
environment routing을 유지하면서 local session partition을 공유한다. SSH는 relay RPC를 사용해 실제 remote
worktree를 만들고 검사하며 설정된 disconnect policy 동안 terminal, file-event, diff state가 remote
workspace와 연결되게 한다. 감사한 source에서 established relay default는 “until reset”이며 bounded
60-second~seven-day timeout도 지원한다. 따라서 reconnect continuity는 policy-scoped이며 무조건적인
process-survival claim이 아니다. 현재 공식 SSH page는 default grace가 5분이라고 한다. 이 분석은 구현
주장을 v1.4.137 tag에 결합하고 documentation/source mismatch를 기록한다.

### 제거 및 복구

제거 구현이 가장 두드러진다.

destructive action 전에 Orca는 renderer가 제공한 path를 믿지 않고 Git에서 registered target을 canonical하게
resolve한다. primary checkout, dangerous location, nested worktree, Git-locked worktree를 거부한다. registered
external worktree는 Orca ownership metadata 없이 제거할 수 있지만 unregistered/orphan recursive recovery에는
ownership과 `.git` backlink proof가 필수다. normal delete는 worktree-owned PTY를 해체하기 전에 tracked 및
untracked dirtiness를 확인한다. 이 순서가 중요하다. preflight failure가 user의 여전히 유용한 terminal
context를 파괴하지 않는다.

preflight 뒤 target worktree의 PTY와 watcher만 닫으려 best-effort로 시도한다. 이는 특히 Windows file
handle에 중요하다. resource-close error는 log하고 Git removal은 계속할 수 있다. local branch deletion은
`git branch -d`를 사용하므로 unmerged 또는 unpublished branch를 조용히 버리지 않고 정확한 `HEAD`와 함께
보존한다. Force는 branch CAS가 아니라 `git worktree remove --force`에 적용된다. 보존된 branch를 폐기하는
별도의 이후 action은 이전에 관찰한 expected `HEAD`와 함께 `git update-ref -d`를 사용할 수 있으므로
confirmation 뒤 이동한 branch가 살아남는다.

recovery는 여러 partial state를 구분한다.

- prune할 수 있는 stale Git registration;
- provenance를 입증할 수 있는 Orca-owned orphan directory;
- Git for Windows가 registration은 제거했지만 file 삭제에 실패한 상태;
- instance identity가 live worktree와 더는 일치하지 않는 metadata;
- 이미 제거된 worktree와 여전히 보존해야 하는 branch.

product value는 이 state를 설명 가능하고 재시도 가능하게 유지하는 데서 나온다. generic `--force`
confirmation dialog보다 실질적으로 안전하다.

## 병렬 작업 및 조정

Orca의 주된 parallel pattern은 dynamic, human-directed fan-out이다. operator가 원하는 만큼 worktree를
만들고 agent를 시작하고 outcome을 비교하여 ship할 것을 고른다. worktree selector와 JSON output도
노출하므로 agent 및 automation이 workspace를 create, inspect, comment, address할 수 있다.

experimental orchestration layer는 persistent message, task, dispatch record, decision gate,
coordinator concurrency, worker completion identity를 추가한다. scheduled automation은 repository를
대상으로 하거나 기존 worktree를 재사용할 수 있다. worktree comment는 lightweight checkpoint 역할을
할 수 있다.

이는 유용한 control surface지만 Akra planning authority와 같지 않다. free-text checkpoint는 유용한
operator context이지 completion proof가 아니다. experimental orchestration과 dynamic worktree count는
durable capacity lease, stale-generation rejection, reviewed delivery를 확립하지 않는다.

Akra는 의도적으로 다른 automation model을 선택한다. fixed three-slot pool, repository-scoped OS mutation
lock, SQLite lease authority, 64-hex generation CAS, frozen delivery target, unattended worker policy, 하나의
serialized distributor다. Orca는 ad hoc workspace fleet ergonomics가 더 낫고 Akra는 bounded autonomous
ownership이 더 강하다.

## Git, 호스팅 리뷰, 전달

Orca는 cohesive interactive source-control path를 제공한다. staged 및 hunk-level diff review, commit,
push, existing-PR association, draft creation, check, review, comment, merge를 다룬다. host-scoped
coordination 및 rate/backoff policy로 PR state를 refresh한다. Pull-request creation은 side effect 직전에
detached/default branch, dirty state, upstream, ahead/behind state, authentication, existing match를 다시
확인한다. Fork PR start point는 remote ref를 fetch하고 verify한다.

구현은 여러 일반적인 함정을 피한다.

- normal push가 조용히 force하지 않는다;
- explicit force는 lease-aware path를 사용한다;
- ambiguous PR creation은 duplicate하지 않고 head/base lookup으로 reconcile할 수 있다;
- merge가 provider에 branch 삭제를 요청하지 않고 branch retirement를 더 안전한 worktree lifecycle에
  맡긴다;
- terminal에서 관찰한 PR URL은 branch association을 검증한 뒤에만 link한다.

이는 강력한 interactive Git/PR tooling이다. Akra distributor와 같은 contract는 아니다. 감사에서는
work 전에 credential-redacted remote URL, repository visibility, integration OID, exact source SHA,
commit range를 freeze하고, matching reviewed PR head와 passing gate를 기본으로 요구하며, detached
worktree에서 integration을 serialize하고, remote integration ref를 verify하고, frozen-SHA
compare-and-swap으로 source branch를 clean하는 하나의 durable authority chain을 찾지 못했다.

Orca는 “operator가 이 worktree를 ship하도록 돕는 것”을 최적화한다. Akra는 “이 accepted task가 이
reviewed remote result가 됐음을 입증하는 것”을 최적화한다. 후자가 보존할 차별점이다.

## 안전성과 신뢰 경계

Orca는 global Manual mode나 agent-specific override가 바꾸지 않는 한 새 launch에서 지원 agent용
high-autonomy argument를 미리 채우며 여기에는 Codex의 approval/sandbox bypass flag가 포함된다. 공식
guidance는 diff를 review하거나 discard할 수 있는 disposable checkout을 근거로 default를 설명한다.
이 containment 주장은 security boundary가 아니며 Akra permission policy로는 지나치게 넓다.

worktree는 별도 checkout과 branch를 제공한다. 다음은 격리하지 않는다.

- 프로세스, 포트, IPC, 네트워크 네임스페이스;
- 사용자 자격 증명, 환경 변수, 홈 디렉터리 파일;
- 저장소 훅, 필터, 도구, 설정 명령;
- 공유 Git 설정, 객체 데이터베이스, 참조, 원격 저장소, 자격 증명 도우미;
- agent가 접근하는 service 및 database.

Orca의 deletion safeguard는 accidental file loss를 줄이지만 workspace를 security sandbox로 바꾸지는
않는다. Akra는 main session에서 공식 app-server approval semantic을 유지하고, unattended approval은
fail closed하며, child environment를 기본 scrub하고, effective Git execution configuration을 audit하며,
격리된 frozen network delivery를 사용해야 한다.

Orca는 prompt, file content, terminal output, path name이 product telemetry에서 제외된다고 문서화한다.
공식 page에서 anonymous product-usage telemetry라고 부르는 항목에는 fixed enum, version string, locally
generated random identifier가 포함될 수 있고 US의 PostHog로 전송된다. Telemetry는 disable할 수 있다.
Orca는 PostHog plan-level default retention과 custom retention window가 없음을 문서화한다. 이 감사에서는
network traffic, remote-agent provider, mobile transport, retention enforcement, 모든 third-party agent
CLI를 독립적으로 검사하지 않았다.

## 성능, 품질, 유지보수성

worktree 영역에는 scanning, collision, NUL 및 newline path, old Git behavior, Windows partial deletion,
ownership forgery, persistence, terminal scope, PR start point, refresh coordination을 위한 광범위한 unit 및
end-to-end test code가 있다. 이는 engineering investment의 의미 있는 증거이지 모든 packaged path가
v1.4.137에서 통과했다는 증거는 아니다.

public tag는 dependency 없이 검사했으므로 test를 실행하지 않았다. 설치된 binary와 CLI는 identity,
daemon readiness, sanitized worktree inventory만 확인했다. cold/warm launch sample, input-to-render sample,
worktree-create latency, memory, disk growth, complete Electron/daemon/agent process-tree measurement는
capture하지 않았다. 따라서 이 감사에서 Akra와 Orca의 비교 가능한 performance 승자는 없다.

architecture cost는 local, WSL, SSH, desktop, mobile, CLI, browser, GitHub, GitLab, multiple agent path를
갖춘 매우 큰 Electron/TypeScript surface다. 빠른 release cadence 때문에 exact stable tag에 claim을
결합할 필요가 커진다. Akra는 이 surface area를 복사하지 않고 lifecycle model에서 배워야 한다.

## Akra 비교

| 축 | Orca v1.4.137 | Akra `6274a7fc` | 결론 |
| --- | --- | --- | --- |
| 주요 단위 | dynamic human-visible worktree workspace | accepted task와 fixed leased slot 및 delivery record | 서로 다른 aggregate |
| repository inventory | managed/external Git worktree, import 및 reconciliation | 3개 managed pool slot과 dedicated integration state | portfolio UX는 Orca가 앞선다 |
| terminal 연속성 | app detach 전반의 worktree-scoped PTY/layout, host-scoped persistence | 공식 thread/session resume와 하나의 inline shell, repository-wide worktree jump 없음 | Orca pattern을 좁게 도입할 가치가 있다 |
| delete safety | canonical Git target, lock/dirty/order check, branch preservation, orphan recovery | integration 뒤 identity-checked slot reset/cleanup, ordinary worktree는 Akra authority 밖 | 서로 다른 scope에서 둘 다 강하다 |
| automation capacity | dynamic workspace, experimental orchestration | durable lease generation 및 dispatch authority가 있는 fixed three-slot capacity | bounded automation은 Akra가 앞선다 |
| worker boundary | Manual/agent override가 바꾸지 않으면 새 launch에 high-autonomy argument 미리 입력 | unattended worker는 workspace-write 사용, approval decline, host가 bounded commit state 검증/준비 | Akra가 앞선다 |
| 전달 | interactive commit/push/PR/check/review/merge | frozen source/target, reviewed default, serialized integration, remote verification | authority는 Akra가 앞선다 |
| remote workspace | first-class SSH relay와 local/WSL | local pool, remote node work는 동등하지 않음 | 출시된 workspace reach는 Orca가 앞선다 |
| 성능 증거 | 비교 가능한 audit sample 없음 | 비교 가능한 complete process-tree baseline 없음 | 알 수 없음 |

## 결정

### 도입

- Akra 관리 슬롯, 통합 워크트리, 기타 등록된 워크트리, 외부/알 수 없음 항목을 포함하는 읽기 전용
  Git 권한 기반 저장소 워크트리 포트폴리오.
- 디렉터리 이름이나 하나의 권한 불리언에서 상태를 추론하지 않고 등록 권한, 상태 최신성, 관리 대상
  연결, 소유권, 잠금, 변경 여부, 브랜치/`HEAD`, 소유자가 선택적으로 제공한 링크 필드를 분리한다.
- import 또는 removal action이 존재하기 전에 external-change reconciliation과 operator-visible
  stale/orphan state를 제공한다.
- Akra가 나중에 manual retirement를 소유한다면 Orca의 destructive-action ordering을 도입한다.
  Git target과 ownership 입증, lock 및 dirty state 검사, scoped runtime resource 중지, remove,
  expected-`HEAD` CAS로 moved/unpublished branch 보존, reconcile 순이다.
- inventory contract가 안정화된 뒤 같은 application read model이 뒷받침하는 compact worktree detail
  flow와 linked-session selection. 공식 thread/workspace context 변경은 별도 P2 decision으로 남는다.

### 거부

- 승인 또는 샌드박스를 우회하는 근거로서의 워크트리 격리.
- Akra의 현재 감사 및 명시적 운영자 정책이 없는 저장소 제어 설정, 훅, 필터, 공유 Git 설정 변경.
- 작업, 임대, 완료, 검토, 전달 권한으로서의 자유 형식 체크포인트.
- 고정된 무인 작업 용량을 대체하는 무제한 동적 워크트리.
- Akra 제품 전략으로서의 다중 에이전트 공급자 데스크톱 IDE, 모바일 클라이언트, 브라우저 편집기,
  범용 터미널 제어 평면.
- Akra의 frozen reviewed-range patch comparison 및 cherry-pick integration delivery를 대체하는
  provider-default merge와 branch cleanup.

### 차별화

Akra의 더 강한 chain을 눈에 보이게 유지한다.

```text
accepted DB task
-> fixed-capacity slot with OS mutation lock and generation lease
-> official app-server worker in an exact worktree
-> host-owned bounded commit validation/preparation
-> frozen remote, repository, base, source, and commit range
-> matching PR, review, checks, and mergeability by default
-> serialized detached integration and remote verification
-> identity-checked source and slot cleanup
```

Orca가 worktree fleet ergonomics의 benchmark를 소유하게 한다. Akra는 그 authority를 더 쉽게 운영하게
하는 repository inventory 및 recovery information만 도입하면서 Codex-first reviewed outcome authority의
benchmark를 소유해야 한다.

## 갱신 조건

Orca가 worktree ownership/removal model을 변경하거나 orchestration 또는 hibernation을 정식화하거나
agent permission default를 바꾸거나 중요한 terminal-daemon architecture change를 출시하거나 새 major
release에 도달하면 이 분석을 갱신한다. Akra가 repository-wide worktree inventory 또는 manual worktree
action을 출시할 때 비교를 갱신한다.

## 출처

- [Orca repository](https://github.com/stablyai/orca)
- [v1.4.137 release](https://github.com/stablyai/orca/releases/tag/v1.4.137)
- [Worktrees](https://www.onorca.dev/docs/model/worktrees)
- [Agents and sessions](https://www.onorca.dev/docs/model/agents-sessions)
- [Supported agents and permissions](https://www.onorca.dev/docs/agents/supported)
- [Session restore](https://www.onorca.dev/docs/model/session-restore)
- [SSH](https://www.onorca.dev/docs/ssh)
- [CLI overview](https://www.onorca.dev/docs/cli/overview)
- [Orchestration](https://www.onorca.dev/docs/cli/orchestration)
- [Commit and push](https://www.onorca.dev/docs/review/commit-push)
- [Hosted reviews](https://www.onorca.dev/docs/review/github)
- [Telemetry](https://www.onorca.dev/docs/telemetry)
