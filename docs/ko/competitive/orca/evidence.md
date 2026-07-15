# Orca 증거 원장

[English](../../../competitive/orca/evidence.md)

이 원장은 변경 불가능한 소스 검사, 설치된 산출물 관찰, 문서화된 동작, 추론, 검증하지 않은 실험을
구분한다. 제품 결론은 [analysis.md](analysis.md)에, Akra 작업 결정은
[gap-matrix.md](gap-matrix.md)에 있다.

## 스냅샷

| 필드 | 값 |
| --- | --- |
| 제품 | StablyAI Orca |
| 공식 저장소 | <https://github.com/stablyai/orca> |
| 라이선스 | MIT |
| 안정 릴리스 | [v1.4.137](https://github.com/stablyai/orca/releases/tag/v1.4.137) |
| 릴리스 소스 커밋 | `6013055491943336660e12e5dec93c9ece4575bb` |
| 소스 커밋 날짜 | 2026-07-12 04:09:01 +00:00 |
| 릴리스 게시 | 2026-07-12 04:40:30 +00:00 |
| 관찰한 최신 prerelease | v1.4.138-rc.7, 2026-07-13 |
| 감사 날짜 | 2026-07-14 (Asia/Seoul) |
| Akra 기준선 | `prerelease`의 `6274a7fc9703f85e4fb6247541dc0d4ed6c5fb9b` |
| Akra 버전 | 1.3.5 |
| 감사 환경 | WSL2의 Ubuntu 24.04.2, Linux 6.18.33.2, x86_64, Git 2.43.0 |

public source는 read-only로 clone하고 release tag에 detached checkout했다. 설치된 Windows application은
스스로를 StablyAI Orca v1.4.137로 식별했으며 updater는 `stablyai/orca`를 가리켰다.

설치 산출물 식별 정보:

| 산출물 | 메타데이터 | SHA-256 |
| --- | --- | --- |
| `Orca.exe` | 제품 `Orca`, 회사 `stablyai`, 제품 버전 `1.4.137.0`, 파일 버전 `1.4.137` | `40dbd16873223a0c983c7f0453cd09943c44e0169dec43be0fd4c99c1e68a91f` |
| `resources/app.asar` | 패키지 버전 `1.4.137`, 저자 `stablyai`, 홈페이지 `https://github.com/stablyai/orca` | `4bb73a15d8426219df6aa5da80c4e01916393f01804c67d0d982a4739ee28f5e` |

설치된 updater 설정은 GitHub owner `stablyai`, repository `orca`, release channel `release`를 지정했다.
이 원장에는 username-specific installation path를 보존하지 않았다.

## 증거 등급

- `verified`: 변경 불가능한 source commit에서 검사했거나 설치된 artifact metadata에서 관찰했거나
  로컬에서 재현함;
- `documented`: 공식 Orca 문서가 명시했지만 재현하지 않음;
- `proposed`: experimental, beta 또는 안정적인 default contract가 아닌 것으로 표시됨;
- `inferred`: 종단 간 재현 없이 검증된 사실에서 논리적으로 도출한 제한적 결과;
- `unverified`: 필요한 environment, raw sample, dependency install, authenticated provider 또는
  external-policy evidence가 없음.

source presence는 implementation path를 입증하지만 모든 packaged configuration이 이를 실행했다는 뜻은
아니다. static audit에서 읽은 test는 quality intent이지 green release result가 아니다. inventory count와
file size는 runtime performance evidence가 아니다.

<a id="audit-environment-and-privacy"></a>
## 감사 환경 및 개인정보 보호

packaged CLI를 WSL에서 호출하는 동안 Windows desktop app이 실행 중이었다. `orca status
--json`은
application, runtime, graph가 ready라고 보고했다. package에는 유용한 CLI version subcommand가 없다.
따라서 release identity는 help text가 아니라 Windows file metadata, packaged application metadata,
updater configuration, release metadata, pinned source tag에서 가져왔다.

CLI fleet command는 private repository name, absolute path, agent prompt, terminal preview, runtime ID,
pane handle을 반환할 수 있다. 해당 raw response는 동작 검증을 위해서만 검사했으며 capture로 commit,
quote, retain하지 않았다. 이 원장은 public Akra repository에 대해 다음 sanitized observation만 기록한다.

- `git worktree list`와 Orca가 각각 8개 worktree를 보고했다;
- 집합은 primary checkout 1개, detached Akra pool slot 3개, branch worktree 4개였다;
- audit worktree는 Orca 밖에서 생성됐고 Orca를 restart하지 않아도 나타났다;
- repository policy는 external worktree가 계속 visible하도록 허용했다.

private repository, home-directory path, prompt, terminal content, repository UUID, runtime ID, user identity는
여기서 product conclusion의 evidence가 아니다.

## 재현 명령

### 설치된 식별 정보

Windows host의 PowerShell에서 실행한다.

```powershell
$root = Join-Path $env:LOCALAPPDATA 'Programs\orca'
(Get-Item (Join-Path $root 'Orca.exe')).VersionInfo |
  Select-Object ProductName, ProductVersion, FileVersion, CompanyName
Get-FileHash -Algorithm SHA256 (Join-Path $root 'Orca.exe')
Get-FileHash -Algorithm SHA256 (Join-Path $root 'resources\app.asar')
Get-Content (Join-Path $root 'resources\app-update.yml')
```

packaged `app.asar` metadata에서 `name`, `version`, `author`, `homepage`을 로컬로 검사했다. updater와
package identity는 public repository와 일치했다.

### 소스 및 릴리스 식별 정보

```bash
git clone https://github.com/stablyai/orca.git /tmp/orca-v1.4.137-source
git -C /tmp/orca-v1.4.137-source checkout --detach v1.4.137
git -C /tmp/orca-v1.4.137-source rev-parse HEAD
git -C /tmp/orca-v1.4.137-source show -s --format='%H%n%cI%n%s' HEAD
gh api repos/stablyai/orca/releases/tags/v1.4.137 \
  --jq '{tag_name,published_at,draft,prerelease,target_commitish}'
```

source conclusion을 기록하기 전에 annotated tag를
`6013055491943336660e12e5dec93c9ece4575bb`로 peel했다.

### 런타임 및 워크트리 인벤토리

exact Windows path는 `$env:LOCALAPPDATA`에서 resolve할 수 있다. 아래 example은 의도적으로
user-specific path를 피한다.

```powershell
$cli = Join-Path $env:LOCALAPPDATA 'Programs\orca\resources\bin\orca.cmd'
& $cli status --json
& $cli worktree ps --json
& $cli repo show --repo 'id:<repo-id>' --json
& $cli worktree list --repo 'id:<repo-id>' --limit 100 --json
```

어떤 output이든 공유하기 전에 repository ID, absolute path, prompt, terminal preview, runtime ID,
pane handle, non-public repository name을 제거한다. 감사에서는 public Akra repository subset만 다음과
비교했다.

```bash
git worktree list --porcelain
git branch -vv
git status --porcelain=v2 --branch
```

### Akra 기준선

```bash
git fetch origin
git worktree add -b docs/native-platform-analysis-orca-worktrees \
  ../codex-exec-loop-worktrees/docs-native-platform-analysis-orca-worktrees \
  origin/prerelease
git -C ../codex-exec-loop-worktrees/docs-native-platform-analysis-orca-worktrees rev-parse HEAD
```

audit worktree를 만들었을 때 baseline은 clean이었다. 감사에서는 Akra core, application service와 port,
TUI 및 Admin inbound adapter, app-server 및 Git/GitHub outbound adapter, parallel-mode implementation,
current operator contract, control-plane architecture를 읽었다.

<a id="product-release-and-topology"></a>
## 제품 릴리스 및 토폴로지

| 주장 | 등급 | 증거 |
| --- | --- | --- |
| 제품은 비슷한 이름의 다른 terminal product가 아니라 StablyAI Orca다. | verified | 설치 metadata, updater target, [package metadata](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/package.json) |
| v1.4.137은 embedded terminal 및 packaged CLI가 있는 Electron desktop ADE다. | verified | 설치 artifact layout, [package metadata](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/package.json), [install docs](https://www.onorca.dev/docs/install) |
| app은 local, WSL, SSH host를 조정한다. | verified/documented | host-provider 및 relay source, [SSH docs](https://www.onorca.dev/docs/ssh) |
| packaged runtime은 감사 중 ready였다. | verified | sanitized `orca status --json` observation |
| Mobile, automation, hibernation, orchestration이 desktop loop를 확장한다. | documented/proposed | [mobile](https://www.onorca.dev/docs/mobile), [automations](https://www.onorca.dev/docs/cli/automations), [hibernation](https://www.onorca.dev/docs/agents/hibernation), [orchestration](https://www.onorca.dev/docs/cli/orchestration) |

<a id="worktree-discovery-and-ownership"></a>
## 워크트리 탐색 및 소유권

다음 변경 불가능한 file을 검사했다.

- [`src/main/git/worktree.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/git/worktree.ts): Git 워크트리 목록 파싱,
  캐시/단일 실행, 변경 무효화, 희소 검사, 추가, 제거 기본 기능;
- [`src/main/ipc/worktrees.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/ipc/worktrees.ts): 목록/탐지/생성/제거 IPC 및
  조정;
- [`src/main/ipc/worktree-logic.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/ipc/worktree-logic.ts): 이름 정제, 생성 경로,
  WSL/워크스페이스 배치, 안전 도우미;
- [`src/shared/worktree-ownership.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/shared/worktree-ownership.ts): 관리, 외부, 레거시 소유권
  분류 및 가시성;
- [`src/shared/types.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/shared/types.ts): 워크트리 메타데이터, 출처, 링크, 계보;
- [`src/cli/handlers/worktree.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/cli/handlers/worktree.ts): CLI selector 및 worktree operation.

검증 결과:

- 새로운 discovery는 Git worktree inventory에서 시작해 path, `HEAD`, branch, detached, bare, lock state를
  parse하고 separate bounded probe로 sparse-checkout state를 추가한다;
- NUL-delimited porcelain을 선호하며 이전 Git behavior를 위한 compatibility handling이 있다;
- concurrent scan은 하나의 in-flight operation을 공유하고 mutation generation은 stale reuse를 막는다;
- sparse-checkout inspection은 bounded concurrency를 사용한다;
- path를 믿는 대신 ownership을 managed, external, unknown legacy state로 구분한다;
- local external mutation은 product view로 reconcile된다;
- 연결이 끊긴 remote host는 non-authoritative로 명시한 metadata fallback을 반환할 수 있다.

설치된 runtime observation은 audit worktree의 external discovery를 독립적으로 검증했다. 모든 ownership
transition 또는 remote fallback을 test하지는 않았다.

<a id="creation-bootstrap-and-metadata"></a>
## 생성, 초기 구성, 메타데이터

다음 생성 경로를 검사했다.

- [`src/main/git/worktree.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/git/worktree.ts);
- [`src/main/ipc/worktrees.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/ipc/worktrees.ts);
- [`src/main/ipc/worktree-remote.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/ipc/worktree-remote.ts).

검증 결과:

- base selection은 local ref, remote ref, commit identity를 허용하고 refresh 및 offline fallback 동작이 있다;
- 새 branch는 `git worktree add --no-track -b <branch> <path> <base>`를 사용한다;
- actual base는 branch configuration에 기록된다;
- `push.autoSetupRemote=true`는 user에게 explicit value가 없을 때만 기록된다;
- 충돌 처리, 기존 브랜치 보존, 풀 리퀘스트 시작점, 희소 설정, 롤백, 로컬/SSH 호스트 경로는
  별도 사례다;
- worktree metadata는 source, base, push target, issue/PR link, agent, prior ID, parent/child lineage를
  유지할 수 있다;
- setup은 생성된 worktree에서 읽어 visible terminal에서 실행한다;
- local creation에는 알려진 filesystem-stall case를 위한 bounded timeout이 있다.

공식 worktree creation, background progress, cancellation/retry, import, lifecycle behavior는
[Worktrees](https://www.onorca.dev/docs/model/worktrees)에 문서화돼 있다. live Orca installation에 active
user workspace가 있었고 source design 확립에 destructive product mutation이 필요하지 않았으므로 감사에서는
disposable Orca-managed worktree를 생성하거나 삭제하지 않았다.

<a id="base-and-drift-reconciliation"></a>
### 기준점 및 변경 조정

[`src/main/runtime/orca-runtime.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/runtime/orca-runtime.ts#L16035-L16240)에
현재 base-status reconciliation과 drift probe가 있다.

- 생성 뒤 current optimistic token이 remote fetch result를 guard하여 stale reconciliation이 더 새로운
  worktree state를 덮지 못하게 한다;
- created base SHA를 refreshed remote-tracking ref와 비교해 `current`, `drift`, `base_changed`, `unknown`으로
  projection하며 bounded behind count와 recent subject를 포함한다;
- on-demand drift probe는 stored/default base metadata를 resolve하고 best effort로 fetch한 뒤 `HEAD`가
  얼마나 뒤처졌는지 보고한다;
- signal을 확립할 수 없으면 unknown을 반환한다. local helper가 동등한 remote-ref/log plumbing을 구현하지
  않으므로 SSH repository는 명시적으로 unknown이다;
- unknown이 immutable delivery gate가 되지는 않는다. 이는 interactive drift information이지 Akra식
  frozen-base authority가 아니다.

<a id="terminal-session-and-remote-continuity"></a>
## 터미널, 세션, 원격 연속성

변경 불가능한 구현 증거:

- [`src/main/runtime/orca-runtime.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/runtime/orca-runtime.ts): 워크트리 범위 터미널
  조회/생성/중지, 백그라운드 런타임, 워크트리 수명 주기 구성;
- [`src/main/persistence.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/persistence.ts): PTY 연결, 직렬화된 플러시,
  세대 보호, 원자적 교체, 순환 백업, 복구, 호스트 분리;
- [`src/main/providers/local-pty-provider.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/providers/local-pty-provider.ts) 및
  [`src/main/pty/wsl-orca-env.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/pty/wsl-orca-env.ts): PTY environment의 worktree 및
  창/탭/터미널 핸들;
- [`src/main/providers/ssh-git-provider.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/providers/ssh-git-provider.ts) 및 remote relay
  handler: remote worktree Git operation과 reconnection;
- [`src/shared/ssh-types.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/shared/ssh-types.ts): relay disconnect-policy bound 및
  기본값.

검증한 소스 결과:

- terminal과 saved layout은 worktree 및 execution host를 key로 한다;
- PTY 생성 전에 stable identifier를 할당하고 spawn success를 반환하기 전에 binding을 동기적으로
  persist한다;
- persistence는 write를 serialize하고 stale generation을 거부하며 temporary file을 통해 replace하고
  rotating recovery backup을 유지한다;
- persistence는 local, SSH, remote-runtime host를 partition한다. WSL은 local session partition을
  공유하지만 Git/path/cache/environment routing은 explicit 상태로 남는다;
- SSH provider는 local directory fiction을 mirror하지 않고 remote host에서 실제 worktree operation을
  수행한다;
- established SSH relay는 configurable disconnect policy를 따른다. 감사한 default는 “until reset”이고
  bounded value는 60초~7일이다. 현재 공식 [SSH page](https://www.onorca.dev/docs/ssh)는 대신 5분
  default를 명시한다. 이 v1.4.137 snapshot에서는 tag-source fact가 우선하며 mismatch는 explicit하게
  남는다.

공식 문서는 [session restore](https://www.onorca.dev/docs/model/session-restore)와 experimental
[hibernation](https://www.onorca.dev/docs/agents/hibernation)을 구분한다. 여기서는 app-close PTY survival,
machine-reboot restoration, agent conversation resume를 하나의 보장으로 취급하지 않는다. 이 감사 중
강제 process crash, host reboot, SSH disconnect는 재현하지 않았다.

<a id="removal-and-recovery"></a>
## 제거 및 복구

변경 불가능한 구현 증거:

- [`src/main/worktree-removal-safety.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/worktree-removal-safety.ts): 위험한 대상,
  중첩 워크트리, `.git` 역링크, 관리 항목, 심볼릭 링크, 소유권 증명;
- [`src/main/local-worktree-removal-recovery.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/local-worktree-removal-recovery.ts): Git for Windows
  부분 제거 복구;
- [`src/main/git/worktree.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/git/worktree.ts): 워크트리 제거, 안전한 브랜치
  삭제, 보존, 가지치기 재시도;
- [`src/main/ipc/worktrees.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/ipc/worktrees.ts) 및
  [`src/main/runtime/orca-runtime.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/runtime/orca-runtime.ts): 전체 removal ordering 및
  런타임 자원 정리.

검증 결과:

- concurrent duplicate removal은 coalesce된다;
- registered target은 canonical Git inventory에서 다시 resolve한다. 모든 registered external worktree에
  Orca ownership이 필요한 것은 아니다;
- primary, dangerous, nested, locked target은 거부하고 unregistered/orphan recursive recovery에는
  structural `.git` 및 Orca provenance proof를 추가로 요구한다;
- non-force dirty/untracked preflight는 worktree PTY teardown 전에 일어난다;
- target-owned PTY/watcher만 best effort로 stop을 요청한다. close error는 log하고 removal은 계속할 수 있다;
- normal branch cleanup은 `git branch -d`를 사용해 unmerged 또는 unpublished history를 보존한다;
- force는 worktree removal에 적용된다. 보존된 branch를 나중에 별도로 삭제할 때 이전에 관찰한 `HEAD`와
  `git update-ref -d`를 비교할 수 있다;
- moved branch와 다른 곳에서 checkout된 branch는 보존한다;
- stale registration, orphan path, already-removed worktree, Windows partial deletion은 서로 다른 recovery
  path가 있다;
- authoritative refresh는 live view에서 missing entry를 제거하고 missing child lineage를 prune하며
  missing-parent instance identity를 rotate할 수 있지만 모든 durable `worktreeMeta` entry를 즉시
  삭제하지는 않는다.

대표적인 static test 증거:

- [`src/main/git/worktree.test.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/git/worktree.test.ts);
- [`src/main/worktree-removal-safety.test.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/worktree-removal-safety.test.ts);
- [`src/main/local-worktree-removal-recovery-live.test.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/local-worktree-removal-recovery-live.test.ts);
- [`src/main/ipc/worktrees.test.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/ipc/worktrees.test.ts);
- [`tests/e2e/worktree-lifecycle.spec.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/tests/e2e/worktree-lifecycle.spec.ts).

이 test는 dependency 없는 source checkout에서 실행하지 않고 읽기만 했다.

<a id="git-hosted-review-and-delivery"></a>
## Git, 호스팅 리뷰, 전달

변경 불가능한 구현 증거:

- [`src/main/source-control/hosted-review-creation.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/source-control/hosted-review-creation.ts): 사전 검사 및
  생성 직전 검증;
- [`src/main/github/pr-start-point.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/github/pr-start-point.ts): 가져와 검증한 PR/포크
  시작점;
- [`src/main/github/pr-refresh-coordinator.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/github/pr-refresh-coordinator.ts): 연결/브랜치 새로 고침
  병합, 호스트 범위, 가시성, 요청 한도, 재시도 지연, 푸시 후 지연;
- [`src/main/github/client.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/github/client.ts): PR 생성/조정, 검사,
  검토, 댓글, 병합;
- [diff viewer](https://www.onorca.dev/docs/review/diff-viewer),
  [commit and push](https://www.onorca.dev/docs/review/commit-push),
  [hosted review](https://www.onorca.dev/docs/review/github) 문서.

검증 결과:

- creation은 branch/dirty/upstream/ahead-behind/auth/existing-review state를 확인하고 side effect 전에
  다시 확인한다;
- fork start point는 remote SHA를 fetch하고 verify한다;
- draft/template creation과 ambiguous-create reconciliation을 지원한다;
- normal push는 조용히 force하지 않고 force는 explicit 및 lease-aware다;
- merge blocker를 확인하고 provider-side automatic branch deletion을 피한다;
- terminal에서 관찰한 PR URL은 linking 전에 branch association이 필요하다.

하나의 Orca authority contract로서는 검증하지 못한 항목:

- 자격 증명이 정제된 원격 URL, 저장소 식별 정보/가시성, 통합 브랜치 및 OID의 작업자 실행 전 고정;
- 정확히 검토된 소스 SHA 및 순서가 있는 커밋 범위 권한;
- 명시적인 우회 출처가 있는 검토 기본 관문으로서의 검토/검사/병합 가능성;
- 직렬화된 분리 통합, 원격 통합 참조 검증, 고정 SHA 정리.

감사에서는 완전한 Akra식 chain의 증거가 아니라 별도의 유능한 implementation path를 찾았다. 이 감사에서
찾지 못했다는 사실은 어떤 optional automation도 그 일부를 근사할 수 없다는 주장이 아니다.

<a id="automation-permissions-and-telemetry"></a>
## 자동화, 권한, 텔레메트리

| 주장 | 등급 | 증거 |
| --- | --- | --- |
| Worktree selector, JSON output, comment/checkpoint, creation을 CLI로 구동할 수 있다. | verified/documented | CLI source, [CLI overview](https://www.onorca.dev/docs/cli/overview), [checkpoints](https://www.onorca.dev/docs/cli/worktree-checkpoints) |
| Persistent task/message/dispatch/decision-gate orchestration이 존재하지만 experimental이다. | proposed | source 및 [orchestration docs](https://www.onorca.dev/docs/cli/orchestration) |
| Scheduled automation은 repository 또는 existing worktree를 대상으로 할 수 있다. | documented | [automation docs](https://www.onorca.dev/docs/cli/automations) |
| 새 supported-agent launch는 high-autonomy approval/sandbox bypass argument를 미리 채운다. global Manual mode와 agent override가 이를 바꿀 수 있다. | documented/source-verified | [supported agents](https://www.onorca.dev/docs/agents/supported), agent launch 및 persistence source |
| worktree는 OS process, credential, networking, Git config, hook, filter, service를 격리하지 않는다. | verified/inferred | Git worktree semantic 및 검사한 shared-config/setup path |
| Orca가 anonymous product-usage telemetry로 문서화한 항목에는 fixed enum, version string, local random ID가 포함되고 PostHog US를 사용하며 disable할 수 있다. prompt, file content, terminal output, path name은 제외된다고 문서화돼 있다. PostHog plan-level default retention이 적용되며 Orca custom window는 문서화되지 않았다. | documented | [telemetry docs](https://www.onorca.dev/docs/telemetry) |

packet capture 또는 third-party agent-provider audit는 실행하지 않았다. 따라서 telemetry conclusion은
독립적인 network verification이 아니라 documentation claim이다.

<a id="quality-evidence"></a>
## 품질 증거

static audit에서는 다음을 위한 focused test를 찾았다.

- 동시 검사 공유, 변경 세대, NUL/줄 바꿈 경로, 이전 Git 대체 경로, 제한된 생성 정체;
- 충돌 접미사, PR SHA 생성, 희소 설정, WSL 라우팅, SSH 생성, 무효화;
- 위조된 소유권, 중첩 워크트리, 위험한 루트, 심볼릭 링크, 별도 Git 디렉터리;
- 실제 Git for Windows 부분 제거 복구;
- 원자적 영속성, 오래된 쓰기 거부, PTY 연결, 백업 순환, 손상 복구, 호스트 범위 세션 분리;
- 포크 PR 시작점, 새로 고침 격리/재시도 지연, 생성 전 재검증;
- E2E spec의 worktree-scoped tab, file, browser, state restoration, terminal isolation.

removal ledger 밖의 대표적인 변경 불가능한 test:

- [`src/main/persistence.test.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/persistence.test.ts);
- [`src/main/providers/ssh-git-provider.test.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/providers/ssh-git-provider.test.ts);
- [`src/relay/git-handler-worktree-ops.test.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/relay/git-handler-worktree-ops.test.ts);
- [`src/main/github/pr-start-point.test.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/github/pr-start-point.test.ts);
- [`src/main/github/pr-refresh-coordinator.test.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/github/pr-refresh-coordinator.test.ts);
- [`src/main/source-control/hosted-review-creation.test.ts`](https://github.com/stablyai/orca/blob/6013055491943336660e12e5dec93c9ece4575bb/src/main/source-control/hosted-review-creation.test.ts).

source checkout에는 의도적으로 `node_modules`가 없었다. Orca unit, integration, E2E, package, signing,
release workflow는 실행하지 않았다. 설치된 artifact는 수정하지 않았다. static test presence에서 어떤
release gate도 green이라고 주장하지 않는다.

<a id="akra-baseline-evidence"></a>
## Akra 기준선 증거

Akra 비교에서는 `6274a7fc9703f85e4fb6247541dc0d4ed6c5fb9b`의 다음 현재 boundary를 읽었다.

- [`src/core/`](../../../../src/core): 헤드리스 애플리케이션 명령/효과/스냅샷 런타임;
- [`src/application/service/`](../../../../src/application/service) 및
  [`src/application/port/`](../../../../src/application/port): 애플리케이션 소유권 및 아웃바운드 경계;
- [`src/adapter/inbound/tui/`](../../../../src/adapter/inbound/tui) 및
  [`src/adapter/inbound/admin_api/`](../../../../src/adapter/inbound/admin_api): 현재 운영자 투영;
- [`src/adapter/outbound/app_server/`](../../../../src/adapter/outbound/app_server): 공식 Codex 런타임 경계;
- [`app_server/mod.rs`](../../../../src/adapter/outbound/app_server/mod.rs),
  [`execution_policy.rs`](../../../../src/adapter/outbound/app_server/execution_policy.rs),
  [`connection.rs`](../../../../src/adapter/outbound/app_server/connection.rs): 격리된 작업자 자식,
  정확한 워크스페이스/신뢰 검증, 실행 정책, 무인 승인 거부;
- [`src/application/service/parallel_mode/`](../../../../src/application/service/parallel_mode):
  풀, 슬롯 수명 주기, 완료, 오케스트레이션, 분배기, 제어 평면;
- [`pool/reconcile.rs`](../../../../src/application/service/parallel_mode/pool/reconcile.rs): 고정된
  분리 슬롯 준비 및 보수적 조정;
- [`pool/cleanup.rs`](../../../../src/application/service/parallel_mode/pool/cleanup.rs): 경로, 소유권,
  임대, 깨끗함/통합됨, 정리 식별 정보 검사;
- [`slot_lifecycle.rs`](../../../../src/application/service/parallel_mode/slot_lifecycle.rs): 고정된
  대상 획득 및 세대에 묶인 임대 전이;
- [`parallel_worker_commit.rs`](../../../../src/adapter/outbound/git/parallel_worker_commit.rs):
  정확한 루트, 브랜치, `HEAD`, 기준점, 파일 시스템, 스테이징, 깨끗한 결과 검증;
- [`distributor/`](../../../../src/application/service/parallel_mode/distributor): 직렬화된 소스,
  PR, 통합, 검증, 정리 전달;
- [`github/automation.rs`](../../../../src/adapter/outbound/github/automation.rs): 격리되고 고정된
  Git/GitHub 전달 대상 및 변경되지 않은 경우에만 비교하는 원격 작업;
- [`current-product.md`](../../reference/current-product.md) 및
  [`architecture.md`](../../reference/architecture.md): 출시된 운영자 및 아키텍처 계약.

검증한 Akra 기준선 결과:

- parallel automation은 idle일 때 detached baseline으로 provision된 generated slot worktree 정확히 3개와
  generated detached integration worktree를 소유한다. repository의 모든 developer worktree를 소유하지
  않는다;
- 각 parallel worker는 lease worktree의 exact absolute cwd에서 별도 unattended app-server child를
  시작한다. bootstrap은 read-only이고 turn은 shared workspace-write execution policy를 사용하며
  ancestor trust는 forced untrusted, applied cwd는 revalidate, approval request는 decline된다;
- repository-scoped OS mutation lock 및 SQLite lease authority가 pool mutation을 보호한다;
- 모든 새 slot allocation에는 lifecycle, completion, delivery, cleanup CAS check 전반에 전파되는
  64-lowercase-hex CSPRNG generation이 있다;
- lease acquisition은 worker 시작 전에 push remote, credential-redacted canonical GitHub URL,
  repository identity/visibility, integration branch, remote base OID를 freeze한다;
- worker contract는 agent에게 edit를 uncommitted로 남기도록 요청한다. host-owned bounded path는 commit
  readiness 전에 exact workspace, branch, base, `HEAD`, index/filesystem constraint, clean output을
  validate하고, clean existing linear descendant commit은 explicit compatibility path로 남는다;
- reviewed delivery는 기본적으로 PR approval, check, clean mergeability, matching frozen PR head를
  요구하며 explicit parent-level high-risk exception이 있다;
- integration은 dedicated detached worktree에서 serialize되고 identity-checked branch/slot cleanup 전에
  remote state를 verify한다;
- `:parallel off`는 automation을 중지하지만 의도적으로 worktree를 삭제하지 않는다;
- TUI board와 authenticated Admin dashboard는 managed pool, roster, lifecycle, queue, distributor,
  delivery state를 projection하지만 repository-wide managed/external worktree portfolio 또는 general
  worktree jump는 없다.

<a id="audit-limits"></a>
## 감사 한계

- 설치된 Orca CLI는 identity 및 status/inventory observation에 read-only로 사용했다. live user worktree,
  branch, terminal, prompt, repository setting, pull request, automation을 변경하지 않았다.
- disposable end-to-end create/setup/dirty/lock/remove/orphan/recovery sequence는 실행하지 않았다. 해당
  conclusion은 변경 불가능한 source로 검증하고 static test가 선택적으로 뒷받침했으며 설치된 app에서
  동적으로 재현하지 않았다.
- SSH 호스트, WSL Git 변경, 모바일 기기, 최대 절전 에이전트, 예약 자동화, 오케스트레이션 조정기를
  실행하지 않았다.
- authenticated GitHub/GitLab PR creation, review, merge, force-push, branch retirement를 실행하지 않았다.
- 텔레메트리 패킷 캡처, 시크릿 카나리, 권한 우회 카나리, 저장소 훅 공격, 악성 설정 파일을
  실행하지 않았다.
- read-only source checkout에 dependency가 없어 Orca test를 실행하지 않았다.
- complete process-tree performance benchmark를 capture하지 않았다. Startup, render, memory, worktree
  creation, switching, removal, disk-growth 승자는 알 수 없다.
- 최신 prerelease는 freshness 확인용으로만 기록했다. 모든 behavior conclusion은 stable v1.4.137
  source commit에 결합된다.
