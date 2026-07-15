# jcode 증거 원장

[English](../../../competitive/jcode/evidence.md)

이 원장은 조사한 소스와 공급자 주장 및 제안된 설계를 구분한다. 해당 제품 결론은
[analysis.md](analysis.md)에 있고 Akra 결정은 [gap-matrix.md](gap-matrix.md)에 있다.

## 스냅샷

| 필드 | 값 |
| --- | --- |
| 제품 | jcode |
| 공식 저장소 | <https://github.com/1jehuang/jcode> |
| 릴리스 | [v0.43.0](https://github.com/1jehuang/jcode/releases/tag/v0.43.0) |
| Peeled 릴리스 커밋 | `649276753ae11948759192c067dfc4c90fafd47f` |
| 릴리스 게시 | 2026-07-11 06:00:50 UTC |
| 조사 날짜 | 2026-07-12 (Asia/Seoul) |
| Akra 기준선 | `prerelease`의 `66333152170124a42aca6f49ed2f72fa6a8293d7` |
| Akra 버전 | 1.3.5 |
| 조사 환경 | Linux x86_64, 소스 조사 및 정적 명령 |

jcode checkout은 release tag의 깨끗한 detached checkout이었다. Akra 기준선은
`origin/prerelease`에서 만든 깨끗한 worktree였다.

## 재현 명령

```bash
git clone --depth 1 --branch v0.43.0 \
  https://github.com/1jehuang/jcode.git /tmp/akra-jcode-v043-audit
git -C /tmp/akra-jcode-v043-audit rev-parse HEAD
git -C /tmp/akra-jcode-v043-audit describe --tags --exact-match
git -C /tmp/akra-jcode-v043-audit log -1 --format='%cI %s'
```

관찰 결과:

```text
649276753ae11948759192c067dfc4c90fafd47f
v0.43.0
2026-07-10T22:20:40-07:00 release: v0.43.0
```

과거 스냅샷:

```bash
git clone --depth 1 --branch v0.11.2 \
  https://github.com/1jehuang/jcode.git /tmp/akra-jcode-v0.11.2-audit
git -C /tmp/akra-jcode-v0.11.2-audit rev-parse HEAD
```

관찰 결과: `7e419c81a69936336c8ee9df6287f4919708d5c1`.

인벤토리 명령:

```bash
JCODE=/tmp/akra-jcode-v043-audit
AKRA=/path/to/codex-exec-loop-worktree

count_root_and_workspace_crates() {
  local workspace_crates=0
  if [ -d crates ]; then
    workspace_crates="$(find crates -mindepth 2 -maxdepth 2 -name Cargo.toml | wc -l)"
  fi
  printf '%s\n' "$((1 + workspace_crates))"
}

cd "$JCODE"
git ls-files | wc -l
git ls-files '*.rs' | wc -l
git ls-files '*.rs' | xargs wc -l | tail -1
count_root_and_workspace_crates
find src crates -type f -name '*.rs' \
  ! -path '*/tests/*' ! -name '*_test.rs' ! -name '*_tests.rs' \
  -print0 | xargs -0 wc -l | tail -1
rg -n '#\[(tokio::)?test\]' --glob '*.rs' . | wc -l
rg -n '#\[(tokio::)?test\]' --glob '*.rs' \
  crates/jcode-tui crates/jcode-tui-* crates/jcode-render-core | wc -l
git ls-files '*.rs' | awk '/^src\//' | wc -l
git ls-files '*.rs' \
  | awk '/^(src\/tui\/|crates\/jcode-tui[^/]*\/|crates\/jcode-render-core\/)/' \
  | xargs wc -l | tail -1
git ls-files '*.rs' \
  | awk '/^(src\/desktop\/|crates\/jcode-desktop[^/]*\/)/' \
  | xargs wc -l | tail -1

cd "$AKRA"
git ls-files | wc -l
git ls-files '*.rs' | wc -l
git ls-files '*.rs' | xargs wc -l | tail -1
count_root_and_workspace_crates
find src -type f -name '*.rs' \
  ! -path '*/tests/*' ! -name '*_test.rs' ! -name '*_tests.rs' \
  -print0 | xargs -0 wc -l | tail -1
rg -n '#\[(tokio::)?test\]' --glob '*.rs' . | wc -l
```

개수는 방향을 파악하기 위한 데이터이지 품질 점수가 아니다. `production Rust LOC`는 명백한 test
path와 test suffix 파일을 제외하지만 production 파일 안의 test module은 여전히 포함할 수 있다.

| 인벤토리 | jcode v0.43.0 | Akra 기준선 |
| --- | ---: | ---: |
| 추적된 파일 | 1,543 | 645 |
| Rust 파일 | 1,084 | 457 |
| 루트 + 작업 공간 crate | 77 | 1 |
| 전체 Rust LOC | 631,544 | 235,156 |
| 대략적인 production Rust LOC | 517,632 | 204,541 |
| 저장소 전체 Rust `#[test]` / `#[tokio::test]` marker | 6,205 | 2,239 |
| TUI 계열 test marker | 2,197 | 별도로 계산하지 않음 |

### 이전 내부 스냅샷 이후의 진화

삭제된 Akra의 2026-04 분석은 jcode v0.11.2를 사용했다. 두 번째 깨끗한 checkout으로 해당 tag를
`7e419c81a69936336c8ee9df6287f4919708d5c1`에 고정했다. 두 tag에 같은 path/count 정의를 적용했다.

| 인벤토리 | v0.11.2 | v0.43.0 | 변화 |
| --- | ---: | ---: | ---: |
| Rust 파일 | 735 | 1,084 | +47% |
| 전체 Rust LOC | 336,208 | 631,544 | +88% |
| 루트 + 작업 공간 crate | 35 | 77 | +120% |
| Root `src` Rust 파일 | 631 | 45 | -93% |
| TUI 계열 Rust LOC | 119,951 | 213,305 | +78% |
| Desktop 계열 Rust LOC | 11,880 | 71,788 | +504% |

`TUI-family`는 `src/tui/`, `crates/jcode-tui*` 또는 `crates/jcode-render-core` 아래에서 추적되는 Rust
path를 뜻한다. `Desktop-family`는 `src/desktop/` 또는 `crates/jcode-desktop*`를 뜻한다.

이는 실제 모듈 추출과 빠른 범위 증가를 모두 검증한다. Root source는 crate로 이동했지만 제품은
작아지지 않았다. 가장 큰 dependency spine은 여전히 `jcode-tui -> jcode-app-core -> jcode-base`를
통과하며 셋 모두 큰 compilation 및 ownership unit으로 남아 있다.

## 변경 불가능한 소스 원장

<a id="product-and-release"></a>
### 제품과 릴리스

- [릴리스 v0.43.0](https://github.com/1jehuang/jcode/releases/tag/v0.43.0)은 릴리스 커밋을 식별하고
  터미널 레이아웃 계산, 비동기 swarm 대기, 원격 작업 디렉터리 재정의, 유휴 TUI 힙 해제 변경을
  나열한다. 분류: `documented`.
- [README 제품 입지](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/README.md#L12-L13)는
  multi-session workflow, customizability, performance를 제품 논지로 제시한다. 분류: `documented`.
- [Cargo workspace](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/Cargo.toml#L1-L91)는
  v0.43.0과 runtime, provider, protocol, TUI, desktop, storage, memory, tool crate를 선언한다. 분류:
  `verified`.

<a id="runtime-architecture"></a>
### 런타임 아키텍처

- [Server 아키텍처](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/docs/SERVER_ARCHITECTURE.md#L9-L110)는
  session 및 state를 소유하는 daemon 하나, user socket을 통한 client, reconnect, reload, provider
  state, 공유 MCP pool을 설명한다. Source module과 protocol crate가 토폴로지를 뒷받침한다. 토폴로지
  분류: `verified`, 여기서 재현하지 않은 운영 보장 분류: `documented`.
- [Protocol crate](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/crates/jcode-protocol/src/lib.rs#L1-L22)는
  로컬 socket을 통한 newline-delimited JSON과 분리된 main/agent communication을 정의한다. 분류:
  `verified`.
- [Protocol agent snapshot](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/crates/jcode-protocol/src/lib.rs#L202-L273)은
  변경된 파일, 생명주기 상태, 완료 보고서, 첨부 파일, 활동 경과 시간, 현재 도구, 공급자/모델,
  토큰 변화량, 할 일 진행률을 전달한다. 분류: `verified`.
- [Multi-session client 아키텍처](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/docs/MULTI_SESSION_CLIENT_ARCHITECTURE.md#L1-L34)는
  하나의 client에 여러 session을 두는 workspace를 명시적으로 proposed로 표시한다. 현재 문서화된
  모델은 하나의 server와 대체로 single-session인 여러 client다. workspace 방향의 분류:
  `proposed`, 현재 client model의 분류: `documented`.

<a id="tui-and-visual-system"></a>
### TUI와 시각 시스템

- [README UI 절](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/README.md#L287-L300)은
  side panel, Mermaid, negative-space widget, 높은 render throughput, custom scrollback, alignment
  mode를 주장한다. 분류: `documented`, throughput은 `unverified`.
- [Info widget 인벤토리](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/crates/jcode-tui/src/tui/info_widget.rs#L69-L183)는
  workspace, todo, context, memory, swarm, usage, model, diagram, ambient work, git에 대한 명시적인
  kind, priority, side preference, minimum height를 구현한다. 분류: `verified`.
- [Side-panel tool](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/crates/jcode-app-core/src/tool/side_panel.rs#L11-L153)은
  agent runtime에 status, write, append, load, focus, delete action을 노출한다. 분류: `verified`.
- [Mermaid terminal crate](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/crates/jcode-tui-mermaid/src/lib.rs#L209-L272)는
  cached, deferred, stable-fit, viewport rendering path를 노출하고,
  [deferred-render state](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/crates/jcode-tui-mermaid/src/lib.rs#L415-L464)는
  공유 background queue를 소유한다. README의 `1800x` 주장은 재현하지 않았다. code 분류:
  `verified`, 비율 분류: `unverified`.
- TUI 중심 source에는 조사한 계산 규칙으로 Rust test marker가 2,197개 있으며 input/copy, remote
  reload, startup input, scroll/copy, smoothness, swarm plan, onboarding, visual path를 포함한다. 분류:
  `verified`.

<a id="performance"></a>
### 성능

- [README 성능 표](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/README.md#L48-L248)는
  local embedding 비활성 상태의 27.8 MB PSS, first frame까지 14.0 ms, first input까지 48.7 ms,
  multi-client 비교를 공개한다. 이 조사의 분류: `unverified`.
- [Visible-ready benchmark](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/scripts/bench_startup_visible_ready.py#L1-L330)는
  80x24 PTY를 구동하고 terminal capability query에 답하며 `pyte`로 렌더링하고 의미 있는 내용 후
  probe를 보내며 기본 10회 실행하고 JSON을 출력할 수 있다. 분류: `verified`.
- [Memory benchmark](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/scripts/bench_memory_cli.py#L1-L432)는
  process/session launch spec을 정의하고 version과 PSS를 기록한다. 분류: `verified`.
- README는 memory rerun에 대해서만 비교 version을 식별하며 그 목록은 v0.43.0이 아니라 jcode
  `v0.9.1888-dev`를 사용한다. startup에는 "this Linux machine"만 명시한다. 추적되는 파일에서 raw
  startup 또는 PSS JSON artifact, 전체 hardware stamp, current-tag rerun을 찾지 못했다. 분류:
  `verified` source-inventory limitation.

스크립트는 감사할 수 없는 marketing table보다 주장을 더 신뢰할 수 있게 하지만, 공개된 수치를
Akra와 직접 비교 가능하게 만들지는 않는다. Akra에는 TUI process와 공식 `codex app-server` child
경계가 포함되므로 비교는 전체 process tree를 계산하고 first frame, input echo, app-server ready,
first stream delta, parallel-worker memory를 분리해야 한다.

<a id="swarm-and-planning"></a>
### Swarm과 Planning

- [README swarm 절](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/README.md#L304-L318)은
  동일 저장소 에이전트, 읽기/변경 알림, 직접/브로드캐스트 메시징, 충돌 해결, 자율 에이전트 생성을
  주장한다. 분류: `documented`.
- [충돌 처리 설계](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/docs/SWARM_ARCHITECTURE.md#L295-L313)는
  조정이 optimistic하고 lock-free라고 말한다. file-touch notification이 위험을 감지하면 관련
  agent가 DM 또는 channel로 통신한다. 이는 모든 충돌이 자동으로 해결된다는 README의 더 강한
  진술을 증명하지 않는다. 문서 충돌의 분류: `documented`, notification이 deterministic resolution이
  아니라는 결론: `inferred`, automatic resolution: `unverified`.
- [Swarm 아키텍처 상태](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/docs/SWARM_ARCHITECTURE.md#L1-L26)는
  agent-first 설계가 대부분 구현되었다고 표시하고 DAG-first migration을 가리킨다. 분류:
  `documented`.
- [재귀 소유권 및 worktree 역할](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/docs/SWARM_ARCHITECTURE.md#L27-L94)은
  재귀적 에이전트 생성, 보고 소유권, 루트 계획 조정자, 선택적 worktree 관리자 에이전트를 정의한다.
  소스 조사에서는 일반 역할 문자열을 찾았지만 자동화된 git-worktree 생성, 병합 또는 정리
  생명주기는 찾지 못했다. worktree 관리자를 선택적 worktree 위에서 에이전트가 수행하는
  `documented` 역할로 취급한다. 자동 격리/통합은 `unverified`로 남는다.
- [DAG 설계 상태](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/docs/SWARM_TASK_GRAPH.md#L1-L16)는
  전체 reframe을 구현 중이라고 말하면서 DAG engine, deep/light mode, gate, graph growth, artifact
  dataflow, subtree broadcast를 live로 명시한다. channel/shared-context 제거는 pending이다. 명시된
  live 구현 분류: `documented`, 미완료 migration 분류: `proposed`.
- [Deep-mode 규모](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/docs/SWARM_TASK_GRAPH.md#L54-L91)는
  1,000-member cap까지 unbounded recursion 및 fan-out을 명시한다.
  [source constant](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/crates/jcode-swarm-core/src/lib.rs#L60-L63)는
  그 cap을 구현한다. 조사에서 unit 수준 cap 증거는 찾았지만 1,000-agent live load 결과는 재현하지
  못했다. 분류: `verified` configuration, `unverified` operational scale.
- [Plan graph protocol](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/crates/jcode-protocol/src/lib.rs#L311-L367)은
  ready, blocked, active, completed, failed, cycle, unresolved dependency, confidence, growth state를
  노출한다. 분류: `verified`.

<a id="memory-providers-and-extensibility"></a>
### 메모리, 공급자 및 확장성

- [Memory 아키텍처 상태](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/docs/MEMORY_ARCHITECTURE.md#L1-L20)는
  core memory를 implemented, graph 기반 hybrid를 planned로 표시한다. memory type, extraction,
  retrieval, UI activity를 소스가 뒷받침하는 `documented` 분류다.
- [README 공급자 인벤토리](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/README.md#L322-L365)는
  구독 기반 OAuth, 직접 공급자, 로컬 엔드포인트, OpenAI 호환 프로필, MCP 구성을 나열한다. Provider
  및 runtime crate가 그 폭을 뒷받침한다. code inventory 분류:
  `verified`, 여기서 실행하지 않은 live provider 동작 분류: `documented`.
- [Lifecycle hook](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/docs/HOOKS.md#L1-L87)은
  observer hook과 synchronous pre-tool gate를 정의한다. 명시적인 block code 이외의 nonzero error는
  설계상 fail open한다. 분류: `documented`.

<a id="desktop-and-remote-control"></a>
### Desktop 및 원격 제어

- [Desktop crate](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/crates/jcode-desktop/Cargo.toml#L1-L26)는
  `winit`, `wgpu`, custom rendering dependency를 사용하는 private v0.1.0 Rust package다. 분류:
  `verified`.
- [Desktop 아키텍처](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/docs/DESKTOP_APP_ARCHITECTURE.md#L1-L55)는
  명시적으로 proposed지만 이후 implementation note와 source는 상당한 prototype을 보여 준다.
  지원되는 public desktop release의 증거는 아니다. 분류: `proposed`, source prototype은 `verified`.
- Akra의 Axum/Askama 화면과 동등한 범용 operational Admin site는 찾지 못했다. jcode source는 대신
  TUI client, custom desktop client, gateway/mobile 방향, daemon/debug interface를 강조한다. 분류:
  `verified` inspected inventory이며 외부 서비스가 존재하지 않는다는 증거는 아니다.

<a id="quality-and-maintainability"></a>
### 품질과 유지보수성

- [CI workflow](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/.github/workflows/ci.yml#L1-L320)는
  서식 검사, 전체 target/feature 검사, clippy, 경고/크기/panic/오류/의존성 ratchet, 미사용 의존성
  검사, 플랫폼 빌드/테스트 작업을 실행한다. 분류: `verified` configuration.
- [Code-size ratchet](https://github.com/1jehuang/jcode/blob/649276753ae11948759192c067dfc4c90fafd47f/scripts/check_code_size_budget.py#L1-L27)은
  1,200 LOC를 넘는 새 production Rust 파일과 추적되는 oversized file의 증가를 차단한다. 분류:
  `verified`.
- 깨끗한 v0.43.0 tag에서 `check_code_size_budget.py`, `check_test_size_budget.py`,
  `check_panic_budget.py`, `check_swallowed_error_budget.py`가 모두 exit 1이었다. code-size check는 새
  oversized file 및 server, provider, swarm, TUI file 증가를 포함하여 regression 52개를 보고했다.
  `check_dependency_boundaries.py`와 `check_wildcard_reexport_budget.py`는 통과했다. 정확한 명령과
  캡처한 stdout/stderr는 [guard-results.txt](../../../competitive/jcode/guard-results.txt)에 저장되어 있다. 분류: `verified`
  local output.

이는 jcode가 매우 진지한 품질 계측을 갖지만 조사 중 tag source가 저장소 내 ratchet 4개를 충족하지
못했음을 뜻한다. 깨끗한 v0.11.2 checkout도 그곳에 존재하던 ratchet 4개를 실패했다. 이는
v0.43-only regression이 아니라 지속적인 baseline/release drift를 나타낸다. v0.11.2 명령과 출력은
[guard-results-v0112.txt](../../../competitive/jcode/guard-results-v0112.txt)에 캡처되어 있다. persistent-drift 결론은 두
`verified` snapshot에서 `inferred`했다. guard의 존재는 green release gate와 같지 않다.

<a id="akra-baseline-evidence"></a>
## Akra 기준선 증거

비교는 고정된 커밋의 현재 Akra 소스와 계약을 읽었다. 로컬 링크는 파일을 쉽게 열게 하고 인접한
변경 불가능 링크는 스냅샷 증거를 보존한다.

- [현재 제품 상태](../../reference/current-product.md) ([고정](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/docs/design/01-current-product-state.md))
- [문서 화면 지도](../../README.md) ([고정](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/docs/README.md))
- [현재 운영자 계약](../../reference/current-product.md) ([고정](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/docs/supersession/current-contract.md))
- [TUI 아키텍처](../../../design/07-tui-layered-architecture-and-aesthetic-contract.md) ([고정](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/docs/design/07-tui-layered-architecture-and-aesthetic-contract.md))
- [런타임 아키텍처](../../reference/architecture.md) ([고정](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/docs/design/05-parallel-control-plane-architecture.md))
- [app-server 어댑터](../../../../src/adapter/outbound/app_server/mod.rs) ([고정](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/adapter/outbound/app_server/mod.rs))
- [병렬 서비스](../../../../src/application/service/parallel_mode/mod.rs) ([고정](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/application/service/parallel_mode/mod.rs))
- [Admin inbound 어댑터](../../../../src/adapter/inbound/admin_api/mod.rs) ([고정](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/adapter/inbound/admin_api/mod.rs))

격차별 소스 증거:

- 고정된 app-server schema는 `turn/steer`와 `expectedTurnId`를 포함하지만
  [interactive runtime port](../../../../src/application/port/outbound/interactive_turn_runtime_port.rs)
  ([고정 schema](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/schema/codex_app_server_protocol.v2.schemas.json#L21238-L21284),
  [고정 port](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/application/port/outbound/interactive_turn_runtime_port.rs))는
  load, stop, approval, start-thread, start-turn operation을 노출하지만 steering method는 없다.
- [turn 제출](../../../../src/adapter/inbound/tui/app/turn_submission_runtime.rs)은 ready conversation에서만
  manual input을 수락한다
  ([고정](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/adapter/inbound/tui/app/turn_submission_runtime.rs)).
- [protocol 계약 테스트](../../../../src/adapter/outbound/app_server/protocol/contract_tests.rs)는 command
  output, patch update, item start, turn diff, plan update를 deferred로 분류하고 token usage는
  diagnostic-only로 분류한다
  ([고정](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/adapter/outbound/app_server/protocol/contract_tests.rs#L21-L78)).
- 완료된 command 및 file-change item은 이미
  [turn notification parsing](../../../../src/adapter/outbound/app_server/protocol/turn_notifications.rs)에서
  typed tool activity가 되고,
  [turn activity state](../../../../src/adapter/inbound/tui/app/conversation_model/turn_activity.rs)에서
  current/last-turn count와 latest summary로 누적되며,
  [공유 tail](../../../../src/adapter/inbound/tui/app/shell_presentation/status_panels/tail_shared.rs)에서
  running-turn priority line을 받는다
  ([고정 parsing](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/adapter/outbound/app_server/protocol/turn_notifications.rs#L485-L533),
  [고정 state](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/adapter/inbound/tui/app/conversation_model/turn_activity.rs#L8-L49),
  [고정 tail](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/adapter/inbound/tui/app/shell_presentation/status_panels/tail_shared.rs#L74-L127)).
- runtime distributor는 reviewed PR gate를 기본값으로 사용한다. 정확한 상위 수준 autonomous opt-in은
  PR이 있어도 approval, clean-merge, required-check gate를 건너뛰고 적격 PR mode는 direct
  integration 전에 PR automation도 건너뛸 수 있다
  ([고정 contract](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/docs/supersession/current-contract.md#L124-L141),
  [고정 policy](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/application/service/parallel_mode/distributor/delivery/github.rs#L177-L256),
  [고정 gate](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/application/service/parallel_mode/distributor/delivery/github.rs#L655-L719)).
- [app-server connection](../../../../src/adapter/outbound/app_server/connection.rs)은 transport failure와
  drop에서 child를 종료하고,
  [공유 runtime reconnect](../../../../src/adapter/outbound/app_server/runtime.rs)는 새 child를 시작한다.
  existing-thread submission은 이미 실행 중인 prior turn에 다시 attach하는 대신 thread를 resume하고
  새 turn을 시작한다
  ([고정 termination](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/adapter/outbound/app_server/connection.rs#L2438-L2462),
  [고정 reconnect](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/adapter/outbound/app_server/runtime.rs#L27-L58),
  [고정 resumed turn](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/adapter/outbound/app_server/mod.rs#L1228-L1275)).
- distributor queue는 base와 source tip을 고정하고 default PR readiness는 PR head, approval,
  clean-merge state, required check를 해당 고정 source에 묶는다
  ([고정 queue record](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/application/port/outbound/planning_authority_port.rs#L115-L133),
  [고정 readiness](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/application/service/parallel_mode/distributor/delivery/github.rs#L675-L719)).
- 현재 parallel `validation_summary`는 changed planning-file path에서 파생되고 validation이 보고되지
  않았다는 fallback을 사용한다. command/exit/artifact proof가 아니다
  ([고정 producer](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/application/service/parallel_mode/orchestrator_loop.rs#L1196-L1209),
  [고정 summary](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/application/service/parallel_mode/orchestrator_loop.rs#L1413-L1426),
  [고정 fallback](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/application/service/parallel_mode/completion.rs#L163-L168)).
- [Admin dashboard mapping](../../../../src/adapter/inbound/admin_api/akra_dashboard.rs)은 success,
  throughput, test success, error rate, selected-task percentage를 명시적으로 uncollected로 둔다
  ([고정](https://github.com/RefinedStone/codex-exec-loop/blob/66333152170124a42aca6f49ed2f72fa6a8293d7/src/adapter/inbound/admin_api/akra_dashboard.rs)).

지원 product-state 문서는 inline shell이 유일한 frontend라고 말하지만 code와 docs map은 Admin, CLI,
Telegram, automation을 포함한다. 따라서 비교는 그 오래된 문장을 반복하지 않고 source surface map을
사용한다.

<a id="audit-limits"></a>
## 조사 한계

- 이 조사에서는 jcode를 build하거나 launch하지 않았다.
- jcode 터미널 스크린샷, 입력 trace, 공급자 요청, 메모리 검색, swarm 실행, desktop 세션을 재현하지
  않았다.
- 공개된 performance number를 다시 실행하지 않았으며 current-tag measurement로 받아들이지 않는다.
- code 및 test를 조사했지만 source 존재가 default enablement 또는 release maturity를 증명하지 않는다.
- 여러 jcode design document는 implemented, migrating, proposed, design-only state를 명시적으로
  혼합한다. 분석은 이러한 구분을 보존한다.
- Akra는 고정된 기준선에서 반복 가능한 startup/input/PSS benchmark가 없으므로 성능 승자를 선언하지
  않는다.
