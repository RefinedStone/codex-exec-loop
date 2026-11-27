# Linux Rust 시험 병목과 architecture 실행 비용 개선

관련: [#2865](https://github.com/RefinedStone/codex-exec-loop/issues/2865), [#2690](https://github.com/RefinedStone/codex-exec-loop/issues/2690)

기준 소스: `prerelease@caf1da09aa561d1510b3271cc663b8e5bf3a0690`. 2026-11-27 조사 및 Windows 로컬 비교이다. Linux CI의 개선율과 job 전체 중앙값은 아직 측정하지 않았다.

## 기존 Linux 실행 근거

[실행 45936932223의 Rust Tests job](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45936932223/job/140302241613) 기록:

| 구간 | 결과 | 실행 시간 |
| --- | --- | ---: |
| lib | 6,540 통과, 실패 0, ignored 27 | 843.95초 |
| architecture_boundaries | 144 통과, 실패 0, ignored 0 | 127.81초 |

같은 로그에서 `shell_chrome_state_has_one_typed_reducer_writer`가 60초 이상 실행 중이라는 경고가 나온다. 이는 이 시험의 정확한 최종 실행 시간이 아니다. 실제 stable 1.95 실행 파일은 `--report-time`를 nightly 전용 옵션으로 거절했다. 기존 병렬 로그만으로 전체 lib의 개별 시간을 복원할 수 없다.

## 확인한 비용과 변경

`tests/architecture_boundaries.rs`의 shell chrome writer 방문기는 파일·모듈·블록마다 전역 구조체 registry 전체를 깊게 복사했다. 파싱된 타입이 들어 있는 큰 registry를 선언이 없는 블록에서도 복사하고 버리는 비용이 반복됐다. 이미 구성된 함수 반환 registry도 TUI 파일마다 복제했다.

- scope 진입 때 그 scope에서 추가하거나 가리는 선언만 저장하고, 나갈 때 해당 항목만 복원한다.
- 완성된 함수 반환 registry는 읽기 전용으로 차용한다. fixture처럼 미완성인 registry는 기존처럼 소유 복사본을 만든 뒤 현재 파일과 trait default를 합친다.
- 기존 escape fixture에 중첩 블록의 구조체 가림과 이후 scope 복원을 확인하는 반례를 추가했다.
- production 소스, 시험 선택, 기본 runner, timeout, ignored 정책을 유지했다.

## 로컬 비교

Windows에서 같은 Rust 1.95.0, 기존 syn/quote/proc_macro2 rlib, `rustc --test --edition=2024 -C debuginfo=1`로 architecture 실행 파일만 만들었다. root library나 전체 제품을 새로 빌드하지 않았다. 전후 executable은 같은 worktree의 같은 production 소스와 문서를 읽는다.

| 측정 | 이전 | 이후 | 관찰 차이 |
| --- | ---: | ---: | ---: |
| 단일 병목, `--exact`, 1스레드 | 51.01초 | 26.55초 | 약 48% 감소 |
| architecture 144개 전체, 4스레드 | 64.99초 | 44.86초 | 약 31% 감소 |

각 조건은 전후 한 번씩 실행했다. 별도의 native lib 컴파일과 일부 시간이 겹쳤으며 호스트 부하를 통제한 벤치마크는 아니다. 위 수치는 로컬 표본의 관찰값이며 Linux CI job이나 전체 CI가 같은 비율로 빨라졌다는 뜻은 아니다.

두 전체 실행은 각각 144 passed / 0 failed / 0 ignored이다. `--list --format=pretty` 출력도 같으며 SHA-256은 `a75588ee78ee98ec36fbbe01ea1917000626032a2f7fd4f45a7f13783f29b868`이다.

실행 구간 재현 명령이다. 실행 파일은 동일 toolchain과 의존성 옵션으로 각 소스판에서 빌드하며 repository root에서 실행한다.

```text
architecture-before.exe --exact shell_chrome_state_has_one_typed_reducer_writer --test-threads=1
architecture-after.exe --exact shell_chrome_state_has_one_typed_reducer_writer --test-threads=1
architecture-before.exe --test-threads=4
architecture-after.exe --test-threads=4
```

## 개별 시간 측정 도구

`scripts/ci-rust-test-profile.mjs`는 이미 빌드한 local libtest executable을 받는 선택적 진단 도구다. 전체/ignored 목록을 읽고 선택한 일반 시험을 별도 프로세스로 순차 실행하여 wall time과 libtest 보고 시간을 JSON에 남긴다. 기본 CI 경로에는 연결하지 않았다.

```text
node scripts/ci-rust-test-profile.mjs <local-test-executable> <output.json> [exact-test-name ...]
```

이름을 생략하면 해당 executable의 일반 시험 전체를 측정한다. 큰 lib 전체를 반복하기 전에 기존 로그가 지목한 이름부터 선택한다.

분리 실행은 원래 병렬 libtest의 공유 상태·경합 조건과 다르다. 결과에 `equivalentToDefaultSuite: false`, executable SHA-256, OS, 작업 위치, 선택한 시험과 제외한 ignored 목록을 기록한다. wall time에는 프로세스 시작·종료 비용이 포함된다. 재빌드·nextest 설치·retry·일반 ignored 활성화·시험 timeout 변경을 하지 않는다. 실행 환경, Git SHA와 dirty 상태, Rust 옵션은 호출자가 비교 자료에 함께 보존해야 한다.

알 수 없는 이름·중복·ignored 요청·목록 실패는 거절한다. 시험 실패, signal 종료, 실행 오류, 0개 통과는 성공으로 계산하지 않고 나머지 측정을 멈춘다. 새 실행 전에 incomplete 영수증을 써서 중단/목록 오류 뒤에 이전 성공 결과가 남지 않게 한다. 신뢰하는 로컬 바이너리를 그 소스와 같은 작업 위치에서만 실행한다.

## 검증과 남는 범위

- 기존 architecture 목록과 144개 전체 전후 통과를 확인했다.
- 중첩 구조체 가림/복원과 기존 함수 반환·trait·macro escape 반례가 통과했다.
- 개별 시간 도구와 기존 inventory의 Node 계약시험 10개가 통과했다.
- 실제 architecture 실행 파일의 순수 parser 시험 두 개를 도구로 측정하여 정상 종료와 영수증을 확인했다.
- `cargo fmt --all`, `git diff --check`를 수행했다.
- planner는 Windows Full gate를 요구한다. 각 병렬 PR에서 전체 제품 검증을 반복하지 않는 작업 지시에 따라 전체 gate는 통합 인수에 모으며, 이 기록은 그 통과를 대신하지 않는다.

Linux runner의 같은 조건 전후 표본, 전체 lib 시험의 느린 순위, Rust Tests job 실행 구간 중앙값 감소는 아직 인수하지 않았다. #2865의 이 범위는 열어 둔다.
