# 로컬 반복 빌드 기준선: workspace 추출 전

측정일: 2026-11-27 KST

관련 작업: [#2867](https://github.com/RefinedStone/codex-exec-loop/issues/2867), [#2903](https://github.com/RefinedStone/codex-exec-loop/issues/2903), [#2911](https://github.com/RefinedStone/codex-exec-loop/issues/2911)

기준 소스: [caf1da09aa561d1510b3271cc663b8e5bf3a0690](https://github.com/RefinedStone/codex-exec-loop/tree/caf1da09aa561d1510b3271cc663b8e5bf3a0690)

구조화 근거: [측정 JSON](ci-local-build-baseline-2026-11-27.json)

## 결과와 해석

같은 checkout과 전용 target에서 기본 feature의 라이브러리 시험 바이너리를 빌드했다.
빈 target에서는 162.63초, 변경 없는 반복 빌드는 0.48초였다.
도메인 또는 adapter 소스 파일 하나에 주석을 추가하면 의존성은 재사용했지만
단일 root library test target을 다시 컴파일해 각각 33.43초가 걸렸다.

| 조건 | 표본 수 | Cargo 명령 wall / 중앙값 | 바깥 wrapper wall | root 컴파일 단위 | Compiling / Fresh 표시 줄 | 시험 본문 실행 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 빈 target, 기존 다운로드 캐시 | 1 | 162.6318초 | 162.6876초 | 135.80초 | 289 / 0 | 0개 |
| 변경 없음 | 1 | 0.4765초 | 0.5193초 | 0초 | 0 / 289 | 0개 |
| 작은 domain 변경 | 1 | 33.4256초 | 33.4683초 | 32.99초 | 1 / 288 | 0개 |
| 작은 adapter 변경 | 1 | 33.4296초 | 33.4733초 | 32.96초 | 1 / 288 | 0개 |

각 조건을 한 번씩만 실행했으므로 중앙값은 해당 관측값이다. 분산, p90, 일반적인 개선율은
추정하지 않는다. 표는 구조 변경 전 기준선이며 workspace 추출의 성능 개선 결과가 아니다.
시험을 실행하지 않은 빌드 결과이므로 Windows CI 시험 전체 소요 시간이나 60분 제한과도
직접 비교할 수 없다.

Cargo 명령 wall은 기존 `scripts/ci-performance.mjs`가 측정한 자식 프로세스 시간이다.
바깥 wrapper wall은 계측 Node 프로세스의 시작 비용을 포함한다.
root 컴파일 단위는 stable Cargo `--timings` HTML의
`akra-ade / akra_ade "lib" (test)` duration이다.
각 표본의 timing `sections`가 `null`이어서 frontend·codegen·link 시간을 따로
구하지 못했다. root 시간에는 해당 단위의 링크가 포함되며, 이를 순수 Rust frontend
시간으로 해석하지 않는다.

`Compiling`과 `Fresh`는 Cargo 표시 줄 수다. 전체 빌드 그래프 단위 수와 다르다.
timings에는 각 조건에 369개 단위가 기록됐다. 작은 변경에서는 root만 duration이 양수이고
나머지 의존 단위 duration은 0이었다. Cargo JSON의 root `compiler-artifact`도
변경 없음에서는 `fresh: true`, 작은 변경에서는 `fresh: false`였다.
stdout/stderr를 합친 로그에서 파싱한 JSON 메시지 수는 보조 근거로만 보관한다.

## 측정 환경과 범위

| 항목 | 값 |
| --- | --- |
| OS / target | Windows 10.0.26200 / x86_64-pc-windows-msvc |
| CPU / 메모리 | Ryzen 9 7900X3D, logical CPU 24개 / 67,767,197,696 bytes |
| Rust / Cargo | rustc 1.95.0 (59807616e 2026-04-14) / cargo 1.95.0 (f2d3ce0bd 2026-03-21) |
| Node | v24.13.0 |
| feature / profile | default / test (dev 상속), root debug line-tables-only, dependency debug false |
| 빌드 환경 | CARGO_INCREMENTAL=1, CARGO_BUILD_JOBS=8 |
| 전용 target | C:/dev/tmp/akra-ci2867-baseline-20261127 |
| 측정 구간 | 2026-11-27 06:35:14–06:41:39 UTC, 15:35:14–15:41:39 KST |
| source inventory | Cargo root package 1개, lib target 1개, bin 5개, integration test target 9개 |
| 이번 빌드 범위 | root libtest 하나만; bin·integration·all-features·다른 OS 제외 |
| registry 상태 | 기존 다운로드 캐시 사용, offline; 첫 target만 비어 있음 |

다른 에이전트의 대형 Rust 빌드가 끝난 뒤 전용 빌드 슬롯에서 순서대로 실행했다.
일반 데스크톱 앱은 동작 중이었고 엄격한 유휴 환경은 아니다.
소규모 Node 정책 시험과 의존성이 없는 Cargo 캐시 fixture가 동시에 실행된 구간이 있다.
특히 small-adapter 마지막 수 초와 작은 Cargo fixture가 겹쳤다.
CPU 사용률과 peak memory는 수집하지 않았다. JSON의 시작/끝 가용 메모리는 peak
사용량을 대신하지 않는다.

기존 작업의 target을 복사·삭제하거나 공유하도록 강제하지 않았다.
측정용 target은 측정 이후에도 보존했으며 2026-11-27 06:51:58 UTC에
5,987,186,982 bytes, 3,223 files였다. 다운로드 시간, 캐시 압축·업로드 시간,
GitHub runner 비용은 이 수치에 포함되지 않는다.

## 정확한 변경과 순서

각 조건의 공통 명령은 다음과 같다.

```text
cargo test --locked --offline --lib --no-run --timings -v --message-format=json-render-diagnostics
```

진행 순서는 다음과 같다.

1. 기준 commit의 깨끗한 별도 worktree와 존재하지 않던 전용 target을 준비했다.
2. 빈 target에서 공통 명령을 실행해 cold를 측정했다.
3. 파일과 환경을 변경하지 않고 같은 명령으로 unchanged를 측정했다.
4. `src/domain/issue_validation.rs` 원본 바이트 뒤에 아래 domain 주석을 추가하고
   small-domain을 측정했다.
5. domain을 원본 바이트로 복원한 뒤 같은 명령을 한 번 실행했다. 이 reset 빌드는
   33.3728초이며 네 조건의 비교 표에는 포함하지 않았다.
6. `src/adapter/outbound/git/task_verification.rs`에 아래 adapter 주석을 추가하고
   small-adapter를 측정했다.
7. adapter를 원본 바이트로 복원하고 `git status --porcelain`이 빈 문자열인지 확인했다.

정확한 추가 문자열은 JSON의 `patches[].append`와 `patches[].patch`에 보관했다.
각 문자열은 LF 하나로 시작해 빈 줄을 만든 다음 주석과 LF로 끝난다.

```rust
// Local CI baseline probe: domain only; no behavior change.
// Local CI baseline probe: adapter only; no behavior change.
```

두 줄은 서로 다른 파일에 각각 적용했다. 주석 변경은 해당 source input의 재컴파일 비용을
측정하기 위한 것이다. 함수 구현·공개 API·실행 경로가 바뀌는 모든 변경의 비용을
대표하지 않는다. 끝에서 source만 원복했으므로 전용 target에는 마지막 adapter 주석을
포함해 컴파일한 바이너리가 남아 있다. 그 후의 unchanged 측정은 재사용 조건이 달라지므로
이번 unchanged 표본에 섞으면 안 된다.

## 재현과 후속 비교

기준 commit을 별도 worktree에 준비하고 동일 toolchain을 활성화한다. 기존 target을 지우지
않고 새로운 경로를 사용한다. PowerShell에서 첫 측정의 환경과 실행은 다음과 같다.

```powershell
$env:CARGO_TARGET_DIR = "C:/dev/tmp/akra-ci2867-reproduction-" + [guid]::NewGuid().ToString("N")
$env:CARGO_INCREMENTAL = "1"
$env:CARGO_BUILD_JOBS = "8"
$env:AKRA_CI_MEASUREMENTS = "artifacts/ci-local-baseline-reproduction"
if (Test-Path $env:CARGO_TARGET_DIR) { throw "Expected a new target directory" }
New-Item -ItemType Directory -Path $env:CARGO_TARGET_DIR | Out-Null
New-Item -ItemType Directory -Force -Path $env:AKRA_CI_MEASUREMENTS | Out-Null
node scripts/ci-performance.mjs run cold -- cargo test --locked --offline --lib --no-run --timings -v --message-format=json-render-diagnostics
if ($LASTEXITCODE -ne 0) { throw "Cold build failed" }
```

이후 위 순서에 따라 동일 명령의 label을 `unchanged`, `small-domain`,
`reset-baseline-before-adapter`, `small-adapter`로 바꾸어 실행한다.
수정할 파일의 원본 바이트를 별도 보관하고 외부 수정이 없는지 확인한 뒤 원복한다.
각 실행 직후 target의 `cargo-timings/cargo-timing.html`을 조건별 이름으로 복사한다.
이미 사용 중인 source/target에서 파일을 원복하거나 `cargo clean`을 실행하지 않는다.

[#2911](https://github.com/RefinedStone/codex-exec-loop/issues/2911)의 비교에서는 동일 OS,
toolchain, feature, profile, jobs, incremental 설정과 대표 편집 문자열을 유지한다.
추출 후 선택된 member만 빌드한 시간과 root 소비자 호환 검증 시간을 각각 보고해야 한다.
시험 집합이 달라지면 inventory 차이를 함께 기록하며, 줄어든 빌드 시간이 전체 CI wall
개선이라고 단정하지 않는다.

## inventory와 근거 파일

측정된 Windows 기본 feature libtest 실행 파일의 `--list --format terse`에는
6,418개, `--list --ignored --format terse`에는 24개가 있었다.
목록 수집은 시험 본문을 실행하지 않는다. Linux 또는 all-features inventory와 같은 수로
취급하지 않는다.

원본 artifact는 측정 worktree의 `artifacts/ci-local-baseline/`에 보존했다.
이 보고서와 JSON에는 검토에 필요한 수치·명령·진단·timing unit·해시를 포함했다.

| 파일 | 내용 |
| --- | --- |
| environment.json, metadata.json | 환경과 root target 목록 |
| steps.jsonl, *-result.json | Cargo wall, wrapper wall, exit code, 시작/끝 메모리 |
| *.log, *-timing.html | Cargo verbose/JSON, stable Cargo timings |
| small-domain.patch, small-adapter.patch | 각 대표 변경 |
| tests-all.txt, tests-ignored.txt | libtest inventory |
| source-inventory.txt | 기준 tracked source inventory |

`cargo metadata --no-deps`는 여기서 root target 목록 확인에만 사용했다.
전체 resolve/feature dependency graph를 확보했다는 뜻이 아니다.
구조화 JSON은 원본 artifact의 SHA-256과 입력 manifest/lock/inventory SHA-256을 기록한다.

이 기준선은 [기존 CI 빌드 비용 기록](ci-build-cost.md)의 로컬 미측정 항목을 보완한다.
과거 원격 CI의 cache 복원·시험 시간·실행 환경 수치를 이 로컬 결과로 덮어쓰지 않는다.
