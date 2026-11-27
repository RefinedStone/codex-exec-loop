# Windows native subprocess 종료 대기 개선 — #2864

## 확인한 실패와 범위

기준 소스는 `prerelease@caf1da09aa561d1510b3271cc663b8e5bf3a0690`이다.
[실행 45936932223](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45936932223)의
[Windows portable 작업](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45936932223/job/140302241610)은
60분 제한을 넘겨 취소됐다. 준비한 계약 필터는 126개이며, 80개를 시작하고 79개를 끝냈다.
마지막으로 시작한 `application::service::issue_coordinator::tests::auto_delivery::`에는 완료 결과가 없다.

로그의 빌드 시간은 production check 1분 24초, lib test build 4분 27초로 합계 5분 51초다.
완료된 테스트 그룹 중 오래 걸린 것은 다음과 같다. 이 값은 테스트 전체 합계나 테스트 한 개의 시간이 아니라,
당시 필터별 libtest 실행 시간이다.

| 계약 필터 | 통과 테스트 | 실행 시간 |
| --- | ---: | ---: |
| `parallel_mode::tests::distributor::evidence_gate::` | 23 | 1,016.41초 |
| `post_turn_evaluation::tests::automatic_verification::` | 5 | 531.61초 |
| `parallel_mode::orchestrator_loop::tests::automatic_verification::` | 5 | 364.72초 |
| `parallel_mode::tests::issue_automation_flow::` | 9 | 312.56초 |
| `parallel_mode::tests::pool::lease::task_verification::` | 1 | 224.68초 |

관련 이슈: [#2864](https://github.com/RefinedStone/codex-exec-loop/issues/2864),
[#2690](https://github.com/RefinedStone/codex-exec-loop/issues/2690).
전체 성능 인수와 다른 구조 변경의 영향 구분은
[#2911](https://github.com/RefinedStone/codex-exec-loop/issues/2911)에서 추적한다.

## 원인 분리

대표 테스트는
`application::service::parallel_mode::orchestrator_loop::tests::automatic_verification::automatic_verification_dispatch_epoch_closure_stops_before_run_and_refresh`이다.
실제 Git, SQLite, private filesystem, lease 경계를 사용하고 모델을 호출하지 않는다.

기준 소스에 같은 test-only 계측을 넣어 확인했을 때 이 테스트 한 번에 Git 명령 준비가 510회 있었다.
초기 임시 Git 저장소 생성은 0.626초였지만, Git 명령 준비는 누적 5.971초,
그중 PATH 정제는 누적 5.516초였다. 별도로 계측한 `command_output` 경로 190회는 누적 5.117초였다.
다른 command-output 진입점은 이 190회 합계에 포함되지 않는다.
초기 fixture 재사용만으로 큰 병목을 해결할 수 있다고 판단하지 않았다.

기존 일반 출력 수집과 취소 가능한 출력 수집은 출력·종료 상태를 확인한 뒤
항상 최대 10ms를 `thread::sleep`으로 쉬었다. 짧은 Git 명령이 그 사이 종료해도
다음 poll까지 기다려야 했고, 이 지연이 수백 회 반복됐다.

## 변경

Windows에서는 아직 종료 관측을 소비하지 않은 자식의 process handle을
`WaitForSingleObject`로 기다린다. 자식이 종료하면 즉시 다음 출력·취소 확인으로 진행한다.
한 번의 대기는 기존 10ms와 남은 절대 deadline 중 작은 값으로 제한된다.

- 명령 준비, 실행 파일 신뢰 확인, PATH 정제와 repository incarnation 재확인을 그대로 수행한다.
- 취소 판정, 출력 크기 제한, pipe 완료 판정, Job 격리와 후손 정리 순서를 유지한다.
- process handle 신호만 기다리며, exit status는 기존 capture loop가 소비한다.
- 부모 종료를 이미 관측했는데 후손이 pipe를 잡고 있으면 기존의 제한된 sleep으로 돌아간다.
  종료된 부모의 signaled handle을 반복해서 기다리는 busy loop를 만들지 않는다.
- 대기 실패는 기존 오류/프로세스 정리 경로로 전달한다.
- Unix/macOS의 대기 방식과 job timeout, 테스트 선택·ignored 상태는 바꾸지 않는다.

`AKRA_NATIVE_TEST_TIMINGS`를 설정한 test 빌드에서는 fixture 구성, Git 명령 준비·환경 정제,
일반 command-output 구간의 마이크로초를 stderr에 기록한다. 경로, 명령 인자, 환경변수 값은 기록하지 않는다.
이 진단은 기본으로 꺼져 있고 production 빌드에는 포함되지 않는다.
구간은 중첩되거나 다른 스레드에서 실행될 수 있으므로 합계를 전체 wall time으로 더하지 않는다.

## 같은 개발 PC에서의 전후 표본

Rust 1.95.0, `--locked --lib --all-features --target x86_64-pc-windows-msvc`,
기본 test profile, `--exact --nocapture --test-threads=1`로 동일한 테스트를 순차 실행했다.
기준과 후보 모두 같은 타이밍 계측을 켰다. 이 표는 각 1회 측정이다.

| 관측 | 기준 | 후보 |
| --- | ---: | ---: |
| 테스트 wall time | 21.52초 | 18.46초 |
| Git 명령 준비 횟수 | 510 | 510 |
| Git 명령 준비 누적 | 5.971초 | 6.150초 |
| 그중 PATH 정제 누적 | 5.516초 | 5.697초 |
| `command_output` 횟수 | 190 | 190 |
| `command_output` 누적 | 5.117초 | 3.733초 |
| 임시 Git 저장소 생성 | 0.626초 | 0.617초 |
| 자동 검증 fixture 구성 | 7.398초 | 5.935초 |

이 표본의 테스트 wall time은 3.06초, 약 14.2% 줄었다.
계측한 command-output 구간은 약 27.0% 줄었고 Git 준비 횟수는 같았다.
PATH 신뢰 검사 비용은 줄이지 않았다.

기존 다른 프로세스가 있는 개발 PC의 표본이며 CPU 부하를 완전히 고정한 벤치마크는 아니다.
이 수치를 GitHub-hosted Windows 전체 작업의 개선율, 중앙값 또는 p90으로 확대하지 않는다.

재현 명령(PowerShell):

```powershell
$env:AKRA_NATIVE_TEST_TIMINGS = "1"
cargo test --locked --lib --all-features --target x86_64-pc-windows-msvc application::service::parallel_mode::orchestrator_loop::tests::automatic_verification::automatic_verification_dispatch_epoch_closure_stops_before_run_and_refresh -- --exact --nocapture --test-threads=1
Remove-Item Env:AKRA_NATIVE_TEST_TIMINGS
```

## 검증

- 위 실제 Git/SQLite 단일 테스트: 전후 모두 통과.
- `subprocess::controlled::tests::`: 5개 통과, 기존 child-process helper 1개 ignored 유지.
  종료 신호를 관측한 뒤에도 취소 사실을 보존하는 Windows 회귀를 추가했다.
- `subprocess::tests::command_output_`: 3개 통과. 정상 출력, timeout, pipe buffer보다 큰 출력 배출을 확인했다.
- `subprocess::tests::exited_parent_with_inherited_pipe_still_obeys_the_deadline`: 통과.
- planner는 Rust 변경으로 분류하며 Windows `-Mode Rust` 게이트를 요구한다.
  이 작업에서 전체 native suite를 반복 실행하지 않았다. 최종 통합 검증 및 PR CI 결과와 구분해 보고한다.

## 남은 확인

Windows 전체 portable 작업의 같은 조건 전후 중앙값, 상위 네 필터 전체 시간,
macOS 실행 시간과 25~51분 편차의 모든 원인은 아직 확정하지 않았다.
이번 변경은 Windows의 반복 subprocess 대기 비용을 줄이는 한 조각이다.
이 문서와 한 개 로컬 표본만으로 #2864나 #2911 전체 완료를 선언하지 않는다.
