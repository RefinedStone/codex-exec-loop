# CI 빌드 비용 비교: cold, 캐시 재사용, 소스 변경

관련: [#2690](https://github.com/RefinedStone/codex-exec-loop/issues/2690) 작업 1, 4, 6.
측정 도구와 용어: [CI 성능 계측과 비교](../reference/ci-performance.md).

이 문서는 이미 끝난 CI 실행의 기록만 사용한다. 새 CI 실행, 로컬 빌드, 로컬 시간 측정은 하지 않았다.
결론의 결정 항목은 모두 **제안**이다. 사용자가 확정한다.

## 방법

- 대상: `Native PR Checks` 의 성공 실행 65건. 2026-11-24 13:18 UTC 부터 2026-11-26 06:10 UTC 까지
  생성됐고 `run_attempt` 가 1 이다. pull_request 49건, prerelease push 16건이다.
- 제외:
  - 실패·취소 실행. 2026-11-24 04:50 UTC 이후 PR 실행은 성공 63건, 실패 27건, 취소 56건이다.
  - 재시도(attempt 2 이상) 8건. 이전 시도의 성공 job 시간이 섞인다.
  - 2026-11-24 13:18 UTC 전 8건. Windows·macOS native 계약 필터가 늘기 전이며 시험 목록이 다르다.
  - 옛 workflow 로 실행된 PR 1건 (`45401451924`). 계측 artifact 가 없다.
- job 과 gate 시간: Actions jobs API 의 `started_at`/`completed_at`. `summarize` 와 같은 기준이다.
  gate 시간은 workflow 시작부터 해당 gate job 완료까지이다.
- 명령 시간: 각 job 의 `ci-performance-*` artifact.
  - `steps.jsonl`: 명령 wall 시간, Cargo `Finished` 줄(빌드 시간), `test result` 줄(시험 실행 시간).
  - `cache-snapshots.jsonl` 의 `restored` 기록: target 캐시의 `exact`/`fallback`/`miss`.
- 분류:
  - `miss` = cold. 같은 컴파일 경계의 target snapshot 이 없다. 의존 crate 까지 컴파일한다.
  - `fallback` = 캐시 재사용 + 소스 변경. 이전 prerelease SHA 의 snapshot 을 복원하고 바뀐 소스를 빌드한다.
  - `exact` = 변경 없는 재실행. 이 기간에 **0건**이다. target 키 끝이 source SHA 이고 PR merge SHA 는
    매번 다르기 때문이다.
- 백분위: nearest-rank p90. 표의 시간 단위는 분이다.
- Cargo 빌드 분해: run `45766265467` 의 Linux·Windows·macOS job artifact 에 있는 Cargo HTML timings 를 읽었다.
- 느린 필터: run `45766265467` 의 macOS·Windows job 로그에서 `==> Native contract:` 와 `test result` 를 짝지었다.

재현:

```bash
gh run download RUN_ID --repo RefinedStone/codex-exec-loop -p 'ci-performance-*' -D downloads/RUN_ID
node scripts/ci-performance.mjs lanes downloads lanes.json
```

`RUN_ID` 는 아래 [사용한 실행](#사용한-실행) 의 ID 로 바꾼다. gate·job 시간은
[재현 절차](../reference/ci-performance.md#재현) 의 `summarize` 로 만든다.

## 결과

### 검증 범위별 gate

범위는 실제 실행된 job 으로 나눴다. `full/mixed` 는 Rust, Node, portable 이 모두 실행됐다.
`rust` 는 Node 와 Desktop 이 skipped 다. `admin` 은 Node 와 Desktop 만 실행됐다.

| 범위 | 표본 | CI Gate 중앙값 | CI Gate p90 | Fast Gate 중앙값 | Fast Gate p90 | runner 합 중앙값 |
|---|---:|---:|---:|---:|---:|---:|
| full/mixed PR | 25 | 46.7 | 58.9 | 11.1 | 13.6 | 122.7 |
| rust PR | 11 | 55.2 | 58.6 | 3.2 | 3.8 | 112.2 |
| admin PR | 10 | 11.1 | 14.5 | 11.2 | 14.5 | 21.4 |
| docs PR | 3 | 1.0 | 2.3 | 0.8 | 2.4 | 0.5 |
| prerelease push (postmerge) | 16 | 43.1 | 58.9 | - | - | 127.8 |

- Rust 를 포함한 실행 52건에서 CI Gate 직전에 끝난 job 은 Windows portable 40건, macOS portable 12건이다.
- full/mixed 의 Fast Gate 는 Desktop 화면 시험 shard 가 정한다 (shard 2/2 중앙값 10.3분).

### job 시간 (Rust 를 포함한 실행 52건)

| job | 중앙값 | p90 | 최소 | 최대 |
|---|---:|---:|---:|---:|
| Portable Rust Check (windows-x86_64) | 45.6 | 57.6 | 29.1 | 59.7 |
| Portable Rust Check (macos-aarch64) | 36.4 | 39.9 | 22.7 | 42.5 |
| Rust Tests | 20.4 | 21.4 | 15.8 | 22.1 |
| Desktop Screen Tests (2/2) | 10.3 | 10.7 | 9.8 | 12.3 |
| Desktop Screen Tests (1/2) | 9.3 | 10.7 | 8.6 | 11.0 |
| Rust Lint and Architecture | 2.5 | 3.0 | 1.7 | 3.7 |
| Node and Admin Surfaces | 0.9 | 1.0 | 0.7 | 1.2 |

Desktop shard 는 41건(Node 를 포함한 실행)이다.

### cold 와 캐시 재사용: 빌드와 시험 실행의 분리

`빌드` 는 Cargo `Finished` 시간의 합이다. `시험` 은 `test result` 의 `finished in` 합이다.
명령 wall 시간은 둘의 합보다 조금 길다 (프로세스 시작, 목록 조회).

| lane / 명령 | 캐시 | 표본 | wall 중앙값 / p90 | 빌드 중앙값 / p90 | 시험 중앙값 / p90 |
|---|---|---:|---:|---:|---:|
| Windows `windows-portable` | miss | 17 | 44.1 / 57.7 | 7.1 / 8.2 | 36.3 / 49.8 |
| Windows `windows-portable` | fallback | 35 | 44.9 / 55.4 | 5.3 / 5.7 | 39.5 / 49.8 |
| macOS `native-check` | miss | 8 | 2.5 / 2.8 | 2.5 / 2.8 | 0 |
| macOS `native-check` | fallback | 44 | 1.3 / 1.7 | 1.3 / 1.7 | 0 |
| macOS `native-contracts` | miss | 8 | 31.9 / 35.4 | 9.2 / 12.3 | 22.9 / 23.3 |
| macOS `native-contracts` | fallback | 44 | 31.3 / 33.9 | 8.3 / 10.6 | 22.8 / 23.5 |
| macOS `native-confirmation` | miss | 8 | 2.5 / 3.1 | 2.4 / 3.0 | 0.1 / 0.1 |
| macOS `native-confirmation` | fallback | 44 | 2.7 / 3.2 | 2.6 / 3.1 | 0.1 / 0.1 |
| Linux `rust-tests` | miss | 6 | 18.6 / 20.0 | 3.6 / 4.0 | 14.5 / 15.8 |
| Linux `rust-tests` | fallback | 46 | 18.2 / 19.2 | 2.7 / 3.0 | 15.5 / 16.1 |
| Linux `native-confirmation` | miss | 6 | 1.5 / 1.7 | 1.5 / 1.7 | 0 |
| Linux `native-confirmation` | fallback | 46 | 1.4 / 1.5 | 1.4 / 1.5 | 0 |
| Linux `clippy` | miss | 5 | 2.7 / 3.1 | 2.7 / 3.1 | 0 |
| Linux `clippy` | fallback | 47 | 1.7 / 2.1 | 1.6 / 2.1 | 0 |

캐시 복원 step 은 중앙값 0.3분 이하, push 의 저장 step 은 0.3분 이하다. PR 은 저장하지 않는다.
복원된 target 크기 중앙값: Linux 시험 4.66 GB, macOS 2.86 GB, Windows 1.78 GB, lint 0.54 GB (압축 전 로컬 크기).

### 작은 소스 변경

CI 자료로는 "작은 변경" 을 따로 잴 수 없다. `fallback` 실행의 빌드 시간은 변경 크기와 관계가 없다.

- Linux `rust-tests` 빌드: snapshot 과 run 사이 `src`·`tests`·Cargo 파일 변경 0개인 push
  `45563935880` 이 2.8분이다. 2~5개 변경 run 4건은 2.4~2.8분, 100개 이상 변경 run 12건은 1.9~2.9분이다.
- run `45563935880` 은 Rust 입력 변경 없이(`desktop/`, `docs/` 만 변경) 본 crate 를 다시 빌드했다.
  원인은 확인하지 않았다. checkout 직후 소스 mtime 이 복원된 산출물보다 새로운 경우와 일치한다.
- Cargo timings (run `45766265467`, 모두 fallback): 컴파일된 unit 은 본 package `akra-ade` 뿐이다.
  의존 crate 의 컴파일 시간 합은 0초다.

| 플랫폼 | 본 crate 시험 lib 빌드 1회 | 본 crate 의 다른 빌드 |
|---|---:|---|
| Linux (기본 feature) | 178초 | `cargo test` 안의 비시험 lib 99초 (병렬), Gateway dev 빌드 88초 |
| Windows (all-features) | 224초 | `cargo check` 는 timings 없음 |
| macOS (all-features) | 600초 | `cargo check` 91초, Gateway dev 빌드 176초 |

따라서 CI 의 `fallback` 빌드 시간은 "본 crate 한 개를 처음부터 다시 컴파일하는 비용" 이다.
이 값이 CI 에서 관측할 수 있는 작은 변경 비용의 상한이다. 로컬 증분 빌드(`incremental=true`)의 값은 아니다.

### 시험 실행의 집중

run `45766265467` 의 native 계약 필터는 한 번에 하나씩 차례로 실행된다.

| 플랫폼 | 필터 수 | 시험 실행 합 | 상위 4개 필터 (시험 34·33개) | 비율 |
|---|---:|---:|---:|---:|
| Windows | 124 | 2,370초 | 1,640초 | 69% |
| macOS | 101 | 1,399초 | 1,021초 | 73% |

상위 4개 필터는 두 플랫폼에서 같다.

1. `application::service::parallel_mode::tests::distributor::evidence_gate::` (Windows 782초, macOS 524초)
2. `application::service::post_turn_evaluation::tests::automatic_verification::` (399초, 196초)
3. `application::service::parallel_mode::orchestrator_loop::tests::automatic_verification::` (272초, 165초)
4. `parallel_mode::tests::pool::lease::task_verification::` (시험 1개, 187초, 137초)

Linux 의 lib 시험 6,424개는 829초다. libtest 의 60초 초과 경고는 2건뿐이다.
시험별 시간은 stable libtest 출력에 없다. Linux 15분의 분포는 알 수 없다.

### 이미 반영된 변경의 관측

- macOS confirmation 의 시험 lib 재컴파일은 사라졌다. confirmation 의 시험 `Finished` 는 2초 미만이다.
  남은 2.6분은 Gateway 실행 파일의 dev 빌드다.
- 캐시는 의존 crate 를 재사용한다. cold 대비 절감은 Linux 0.9분, Windows 1.8분, macOS 약 2.1분
  (check 1.2분, 계약 빌드 0.9분)이다.
- Desktop 화면 시험은 shard 당 9.3분, 10.3분이다. #2690 기준 실행의 단일 단계는 17분 37초였다.
- CI Gate 는 #2690 기준 22분 29초보다 길다. 같은 조건의 비교가 아니다. Windows 계약 필터는 #2690 검토 기준
  (`99238e87`)의 30개에서 124개로, macOS 는 12개에서 101개로 늘었다.

## 판단 (제안)

### nextest: 측정 도구로만 쓴다. 기본 실행기 전환은 보류한다

- Rust job 의 비용은 시험 실행이다. 빌드가 아니다. 시험 실행은 명령 wall 의 Windows 88%,
  macOS 계약 73%, Linux 85% 이다 (fallback 중앙값).
- 실행 시간은 소수 시험에 모인다. 계약 필터 상위 4개가 Windows 69%, macOS 73% 이다.
  실행기를 바꿔도 느린 시험 자체는 빨라지지 않는다.
- 빠진 자료는 시험별 시간이다. 특히 Linux 15.5분의 분포다. nextest 의 시험별 시간 기록이 이 자료를 준다.
  [시험 목록 보존과 nextest 검토](../reference/ci-rust-tests.md) 의 격리 위험(환경 변수, 자식 프로세스,
  OS 키 저장소, PTY)은 측정 전용 실행에서도 그대로 확인해야 한다.
- 제안 순서:
  1. 상위 4개 필터의 시험이 왜 느린지 확인하고 고친다 (대기·timeout·실제 프로세스 사용 여부).
  2. Linux 에서 nextest 를 측정 전용으로 한 번 실행해 시험별 시간을 얻는다. 필수 gate 는 바꾸지 않는다.
  3. 계약 필터의 순차 실행을 한 libtest 프로세스로 합칠지 검토한다. 필터 사이의 공유 자원 격리 확인이 먼저다.
  4. 1~3 뒤에도 실행이 병목이면 nextest 기본 도입이나 샤딩을 다시 판단한다.

### 로컬 crate 분리: 나중에 한다

- 남은 컴파일 비용은 본 crate 한 개다. 의존 crate 는 캐시가 해결했다. 본 crate 시험 lib 빌드는
  Linux 178초, Windows 224초, macOS 600초이며 하나의 직렬 unit 이다. 이 비용을 줄이는 구조 변경은 분리뿐이다.
- 지금은 효과가 CI Gate 에 거의 닿지 않는다. CI Gate 를 정하는 Windows job 에서 빌드는 45.6분 중 5.3분(12%)이다.
- CI 에서 분리 효과는 불확실하다. Rust 변경이 없어도 본 crate 를 다시 빌드했다 (`45563935880`).
  분리한 crate 도 같은 이유로 다시 빌드될 수 있다. 확인하지 않았다.
- 로컬 반복 빌드 이득은 측정하지 않았다. 측정 당시 이 장비는 다른 빌드와 함께 쓰여 시간이 유효하지 않았다.
- 다시 판단할 조건:
  - 시험 실행을 줄인 뒤 macOS 빌드(현재 36.4분 중 약 12.2분, 33%)가 CI Gate 를 정할 때.
  - 유휴 장비에서 [Workspace 결정 기록](../reference/ci-workspace-decision.md) 의 로컬 작은 변경 표를 측정했을 때.
- #2569 의 착수 조건은 그대로다.

## 측정하지 않은 것

- 로컬 cold·증분·작은 변경 빌드. 이 장비의 동시 빌드 때문에 실행하지 않았다.
- 변경 없는 재실행(`exact`). CI 에 표본이 0건이다.
- 실패·취소 실행의 "실패를 알기까지의 시간".
- miss 의 run 별 원인 (manifest 변경, 첫 push 등). 캐시 경계 규칙만 확인했다.
- Windows 시험 실행의 큰 범위(25~51분)의 원인. runner 차이인지 시험 시간 변동인지 확인하지 않았다.
- 기간 안에서도 계약 필터가 추가됐다. 표본은 같은 시험 목록이 아니다.
- runner 합은 job 점유 시간의 합이다. 청구 분량이 아니다.

## 사용한 실행

모두 `https://github.com/RefinedStone/codex-exec-loop/actions/runs/<ID>` 이다.

- full/mixed PR (25): 45401991432 45404523004 45522603479 45522778516 45531415015 45540307881
  45549098533 45556838021 45557288953 45559239425 45564873701 45565127428 45565257726 45575649453
  45624929542 45626289006 45650797465 45654991380 45658390608 45659875707 45664384728 45690894283
  45744689046 45746279905 45766265467
- rust PR (11): 45525313322 45559163782 45559170488 45563129600 45570611916 45573800906 45575473250
  45576369090 45759529572 45761004115 45761293595
- admin PR (10): 45522786182 45531422664 45558357186 45639601985 45645009884 45647632878 45658780149
  45671611343 45674599450 45756849974
- docs PR (3): 45648454200 45655684154 45659424348
- prerelease push (16): 45530889726 45535702344 45546827981 45554177593 45563935880 45581960988
  45582364852 45582547887 45622266326 45632436260 45648596778 45649898979 45650254990 45754926866
  45759664398 45766388445
