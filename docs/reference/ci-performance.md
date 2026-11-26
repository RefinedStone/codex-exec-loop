# CI 성능 계측과 비교

관련: [#2690](https://github.com/RefinedStone/codex-exec-loop/issues/2690).

선머지 CI 실패 다섯 종류의 수정·검증 근거와 #2749 종료 판정은
[2026-11-26 실패 후속 인수](../validation/ci-failure-followup-2026-11-26.md)에 따로 기록한다.
그 회귀·진단 보강의 완료를 아래 성능 개선의 측정 결과로 간주하지 않는다.

## 수집 범위

`Native PR Checks`는 기존 검사 명령을 `scripts/ci-performance.mjs run`으로 감싼다.
명령의 실패 종료 코드를 전달하고, 출력은 기존 Actions 로그에 남긴다. 별도 시험을 추가로 실행하지 않는다.
wrapper가 SIGINT/SIGTERM을 받으면 자식이 정리 후 0으로 끝나도 각각 130/143을 반환한다.
기록의 exitCode는 wrapper 결과이며 childExitCode와 requestedSignal에 원래 종료와 취소 요청을 따로 남긴다.
각 job의 artifact에는 명령별 시작/종료와 경과 시간, 실제 Cargo `Compiling` 줄 수,
`Finished` 빌드 요약과 `test result` 실행 요약, commit/run/attempt, runner OS/아키텍처,
toolchain과 feature/target/profile 조건을 기록한다. Cargo HTML timings도 가능한 빌드에서 함께 보존한다.

명령 wall time에는 컴파일과 시험 실행이 포함된다. 둘을 구분할 때 Cargo Finished 및 test-result와
HTML timings를 사용한다. Compiling 줄 수는 관측된 compilation unit 수이며 캐시 hit 비율이나
재사용된 unit 수가 아니다. Fresh는 verbose Cargo 출력이 없는 명령에서 0으로 보일 수 있다.

캐시 복원/저장, checkout/toolchain 설치, job 준비·종료 시간은 GitHub Actions jobs API의 timestamps로
구분한다. wrapper만으로 캐시나 전체 CI가 빨라졌다고 판단하지 않는다.
job 시간 합은 점유 runner 초의 근사치이고 GitHub의 플랫폼 배율·반올림을 적용한 청구 분량이 아니다.

## 기존 관측 기준

#2690에 인용된 성공 실행 [45231960486](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45231960486),
commit `c63bec6e59b8c20e7247eb2ba597376d24805369`, 표본 **1회**:

| 구간 | 경과 |
|---|---:|
| workflow 시작 → CI Gate | 1,349초 |
| macOS portable | 1,319초 |
| Node/Admin | 1,116초 |
| Desktop 화면 단계 | 1,057초 |
| Linux Rust | 958초 |
| Windows portable | 510초 |

이 자료는 아직 미병합된 #2686 기능 스택의 실행이다. 현재 prerelease와 소스가 같지 않으므로
그 시간과 새 prerelease 실행을 같은 조건의 개선 전후로 단정하지 않는다.
이 표에는 직접 재조회하지 않은 이슈 본문의 관측값을 옮겼다. 평균·중앙값·개선율을 주장하지 않는다.

## 재현

기존 개발용 target과 캐시는 보존한다. 전용 측정 target을 고르고 동일한 runner 종류, 아키텍처,
Rust toolchain, features, profile, target triple을 고정한다.
다음 네 경우를 섞지 않는다: `cold`(처음 사용하는 별도 target), `cache-reuse`(복원 후),
`unchanged`(동일 checkout 재실행), `source-change`(하나의 작은 변경 후).

```bash
CARGO_TARGET_DIR=target-ci-measurement node scripts/ci-performance.mjs run rust-tests -- cargo test --locked --timings
node scripts/ci-performance.mjs report
```

cold 측정을 위해 평소 캐시를 지우거나 `cargo clean`을 호출하지 않는다.
실행별 artifact를 구분하여 보존하고, 비교 측정은 명시적으로 필요할 때에만 반복한다.

완료된 실행 한 번에 대한 offline 요약:

```bash
gh api repos/RefinedStone/codex-exec-loop/actions/runs/RUN_ID > run.json
gh api --paginate --slurp repos/RefinedStone/codex-exec-loop/actions/runs/RUN_ID/attempts/ATTEMPT/jobs > jobs.json
node scripts/ci-performance.mjs summarize run.json jobs.json summary.json
```

`RUN_ID`와 `ATTEMPT`는 실제 실행으로 교체한다. 마지막 시도와 예전 시도의 jobs를 합치지 않는다.
`summary.json`은 CI Gate 도달 시간과 runner 시간 합, job/step 상세, 미완료 job 수와 duration 누락 수를 분리한다.
실행 중이거나 성공한 CI Gate가 없으면 해당 경과는 `null`이고 성공으로 간주하지 않는다.
미완료 job 또는 시작·종료 시간이 빠진 실행 job이 있으면 runner 합계도 `null`이다.
실행되지 않은 skipped job만 0으로 처리하며, 불완전한 표본은 비교 명령에서 거부한다.

여러 summary에 다음 metadata를 붙여 JSON 배열 `samples.json`으로 보존한 뒤 비교한다.

```json
{
  "cohort": "baseline",
  "scenario": "cache-reuse",
  "environment": {
    "runner": "macos-15",
    "architecture": "arm64",
    "toolchain": "1.95.0",
    "features": "all",
    "profile": "test",
    "target": "aarch64-apple-darwin"
  }
}
```

```bash
node scripts/ci-performance.mjs compare samples.json comparison.json
```

cohort에는 개선 전 `baseline`, 개선 후 `candidate`처럼 명시적인 비교 집단을 지정한다.
cohort가 없으면 비교를 거부한다. 서로 다른 cohort는 같은 환경·scenario여도 한 중앙값으로 섞지 않는다.
도구는 같은 cohort·환경·scenario끼리만 묶어 표본 수, 중앙값, 최솟값·최댓값과 실행 URL/commit/attempt를 남긴다.
summary의 gate/workflow/runner 시간과 미완료·duration 누락 개수를 함께 보존해야 한다.
실패·중단된 실행을 빠른 성공 표본으로 섞지 않는다. 작은 소스 변경의 정확한 patch도 함께 보존한다.
캐시 hit/miss, 크기·복원·저장 시간, compiled unit 수, runner 사용량과 실패율을 같이 비교한다.

## 이번 적용의 한계

계측 경로와 비교 형식을 구현한 것이며, 반복 cold/warm 실험이나 로컬 Rust 전체 빌드를 새로 실행하지 않았다.
사용자의 요청에 따라 단계별 CI를 조회하지 않고 최종 단계에서 한 번만 확인한다.
속도 개선의 확정값은 같은 조건의 후속 표본이 쌓인 뒤 기록한다.

2026-11-24~26 CI 표본의 cold·캐시 재사용·소스 변경 비교와 `lanes`(lane·명령·target 캐시별 빌드/시험 중앙값·p90) 사용법은 [CI 빌드 비용 비교](../validation/ci-build-cost.md)에 있다.
