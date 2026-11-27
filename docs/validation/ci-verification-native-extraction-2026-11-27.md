# Git verification native crate 경계와 시험 소유권

관련 이슈: [#2910](https://github.com/RefinedStone/codex-exec-loop/issues/2910). 선행: #2908 native 원시 기능, #2909 verification 규칙/Port, #2905 영향·시험 inventory.

## 실제 추출 범위

`akra-verification-native`는 고정된 `GitDiffCheckV1` host recipe, private Git context, 입력 object/경로 검사, 환경·명령 fingerprint, 그리고 공유 host Git 환경·config-key 규칙을 소유한다. `akra-native`의 pinned executable/directory/file, private temp, controlled subprocess API와 `akra-verification`의 runner Port 및 관측값만 사용한다.

루트는 기존 `GitTaskVerificationRunnerAdapter` 경로를 재export하며 composition이 같은 이름으로 새 adapter를 주입한다. SQLite, parallel worker pool, issue admission, current lease/authority 판단을 새 adapter에서 참조하지 않는다. 일반 사용자 recipe나 shell runner를 추가하지 않았다.

다음 계약은 기존 구현을 유지한다.

- 고정 Git 실행파일의 trust/identity/hash pin과 실행 전후 재확인.
- source head/tree/object-format 검증, source index·HEAD·refs 보존.
- 고정 private config와 격리한 HOME/TEMP/worktree/metadata, filesystem pin과 no-replace stable file.
- 환경·command fingerprint에 실제 temporary 경로가 섞이지 않는 규칙.
- 기존 deadline/cancellation/stream byte limit와 실행 전 NotRun, 실행 후 관측값 실패 구분.
- worker가 준 명령이나 저장소 script를 검증 recipe로 실행하지 않는 경계.

## 공유 Git helper와 루트 경계

`git_command`는 기존 production host Git builder와 환경 hardening을 한 곳에서 구현한다. 루트의 `cfg(test)` facade는 기존 fixture 실행파일 선택과 `NativeTestTimer`의 git_prepare/git_harden 계측 범위를 유지한다. production에서는 새 crate의 같은 builder를 사용한다.

`git_config`는 NUL key stream의 크기·UTF-8 검사, executable-key 분류와 값 없는 bounded diagnostic을 수행한다. strict 검사와 이미 검증된 submodule용 core.worktree 예외의 내부 bool은 private이다. 공개된 순수 검사 결과가 실제 worktree 검증이나 권한을 대신하지 않는다.

root `git_execution_guard`의 repository context 선택, submodule canonical identity/containment 재확인, configured subprocess timeout과 실제 config audit는 그대로 남는다. 기존 30초 기본값을 쓰던 Git builder fixture 한 곳은 독립 package에서 같은 30초를 명시한다. 제품 recipe deadline이나 CI timeout은 변경하지 않는다.

기존 Git 환경 격리 architecture guard는 src/tests에 더해 crates도 검사한다. 이동한 poisoned-environment fixture의 raw Git 5곳과 단일 fixture builder 1곳을 정확한 개수로 등록했다.

## 시험 이전과 합집합

| 기존 시험군 | 새 위치 | Windows 수 | Linux/macOS 수 | 처리 |
| --- | --- | ---: | ---: | --- |
| `adapter::outbound::git::task_verification::tests` | `task_verification::tests` | 15 | 14 | Git adapter 계약 이동 |
| 실제 dirty snapshot 및 source index 보존 | 기존 root 위치 | 1 | 1 | 실제 parallel runtime과의 통합 보존 |
| `git_subprocess::tests` | `git_command::tests` | 5 | 5 | 환경·인자·malicious routing 계약 이동 |
| config-key vocabulary | `git_config::tests` | 1 | 1 | 순수 Git policy 회귀 이동 |
| audit context 및 실제 worktree-config audit | 기존 root 위치 | 2 | 2 | 루트 업무·설정 경계 보존 |

기존 합집합은 Windows 24개 = 이동 21개 + root 3개, Linux/macOS 23개 = 이동 20개 + root 3개다. Windows 경로 정규화 시험 1개는 기존 플랫폼 조건을 유지한다. 추가한 submodule 예외·동일 byte/key 개수 제한 회귀 1개와 fixture의 실제 local Git 설정 확인 회귀 1개를 포함하면 새 crate는 Windows 23개다. Linux/macOS는 소스의 기존 cfg 조건 기준 22개로 예상하며, 아직 해당 OS에서 목록이나 실행을 확인하지 않았다. 이동한 원래 시험에는 ignored 항목이 없다.

원래 helper가 확인하던 output hash와 실제 dirty source/index 보존 assertion도 유지했다. 루트에 남긴 dirty snapshot 시험은 새 adapter의 공개 runner Port를 통해 실행한다. 기존 portable prefix는 이 root 시험 1개를 계속 선택하며, 이동한 시험은 workspace member test에 포함된다.

## 상위 네 고비용 업무 시험군

| 시험군 | 이전 소유 | 이후 소유 | 필수 OS 및 검증 |
| --- | --- | --- | --- |
| evidence_gate | root lib | root lib | Linux/Windows/macOS, all features, full 유지 |
| post_turn automatic_verification | root lib | root lib | Linux/Windows/macOS, all features, full 유지 |
| orchestrator automatic_verification | root lib | root lib | Linux/Windows/macOS, all features, full 유지 |
| pool lease task_verification | root lib | root lib | Linux/Windows/macOS, all features, full 유지 |

이 네 군은 SQLite/Git과 업무 admission/continuation/lease를 묶으므로 adapter 시험으로 통째 옮기지 않았다. registry의 네 기존 root-integration 계약과 consumerCoverage full을 유지하고, adapter/Git builder/config-key native 계약 및 root dirty-snapshot 계약을 추가했다.

## Fixture 설정 비용과 Windows 시험 입력 보정

새 private Git fixture마다 `git config`를 네 번 실행하던 준비 코드를 한 번의 local config append로 바꿨다. 고정 `user.name`, `user.email`, `core.autocrlf=false`, `commit.gpgsign=false` 값은 동일하다. 새 회귀는 **실제 Git의 `config --local --list`**로 네 값을 읽어 확인한다. fixture당 설정용 Git 프로세스 4개를 없앴으며 제품 recipe, 실행 신뢰 검사, timeout/cancellation 또는 시험 선택은 바꾸지 않았다.

첫 독립 Windows 실행에서는 22개 중 환경 격리 시험 두 개가 실패했다. 이동 전 원본과 새 구현은 모두 `Path::is_absolute()`를 요구하는데, 기존 시험이 `/tmp/agent.sock`과 `/home/operator`를 Windows에서도 허용되는 절대 경로로 가정했다. 해당 fixture만 OS별 절대 경로로 보정하고, 상대 경로 거절도 같은 시험에서 확인했다. 인증 전달 보존과 명시적 HOME 삭제 우선 assertion은 유지했고 production 허용 정책은 변경하지 않았다. 보정 후 관련 Git builder 시험 5개가 전부 통과했다.

## 같은 대표 시험의 전후 관찰

측정 자료: [ci-verification-native-extraction-2026-11-27.json](ci-verification-native-extraction-2026-11-27.json).

- 기준: `ce02eb62f163086c4bb54be41de92727c2d37126` — Windows fixture 입력 보정 후, 설정 프로세스 최적화 전.
- 후보: `79bcfcafd5a9886acecc2b609566aa7eb8f193c1` — 설정 프로세스 최적화와 새 회귀 추가 후.
- 같은 Windows 개발 PC, Rust/Cargo 1.95.0, all features, 기본 test profile, `x86_64-pc-windows-msvc`, jobs=2, 같은 worktree 전용 target, `--test-threads=1`.
- 대표 시험은 `task_verification::tests::actual_git_check_binds_the_frozen_full_range_without_changing_index_head_or_refs`이며 각 3회 순차 실행했다. 시험이 검증하는 index·HEAD·refs 불변 assertion은 그대로다.

| 대표 시험 libtest 시간 | 기준 | 후보 |
| --- | ---: | ---: |
| 1회 | 1.33초 | 1.18초 |
| 2회 | 1.27초 | 1.22초 |
| 3회 | 1.25초 | 1.20초 |
| 관측 중앙값 | 1.27초 | 1.20초 |
| 관측 범위 | 1.25–1.33초 | 1.18–1.22초 |

이 세 표본의 중앙값은 0.07초, 약 5.5% 줄었다. libtest의 소수 둘째 자리 출력 기준이며 일반 개발 PC의 소수 표본이다. 기준군을 먼저, 후보군을 나중에 측정했고 무작위 교차 실행이나 CPU 부하 통제 실험은 아니다. p90, 통계적 유의성, Windows 전체 native suite 또는 전체 CI 개선율을 추정하지 않는다. 다른 에이전트의 architecture 시험 실행은 대표 측정 구간과 겹치지 않도록 조정했다.

전체 독립 package 확인은 기준 **22/22, 16.16초**, 후보 **23/23, 15.29초**, 양쪽 ignored 0개다. 새 회귀가 추가돼 집합이 다르므로 이 두 전체 시간을 동등 suite 개선율로 계산하지 않는다. doctest 경로도 실행했으며 해당 package의 doctest는 0개다.

첫 독립 libtest 준비는 빈 전용 target과 기존 다운로드 캐시로 **18.73초**였다. 이 값의 소스는 fixture 입력을 보정하기 전 `d11838741da6e6c9697e7258f7737368bfa5e719`이며, 최종 후보의 cold 성능으로 바꾸어 쓰지 않는다. root libtest를 빌드하지 않았고 `akra-native`, `akra-verification`, `akra-verification-native`만 workspace에서 컴파일했다. compiler frontend/codegen/link 시간은 분리하지 않았다.

이 package 단독 실행은 독립 검증 가능성을 확인하는 명시적 `-p` 선택이다. root가 consumer인 실제 영향 계획은 여전히 **full**이며, standalone 비용을 전체 consumer 검증 시간으로 표현하지 않는다. #2864의 Windows subprocess wakeup 수정, package 선택량과 이 fixture의 실행 시간 관찰을 서로 다른 지표로 유지한다.

재현 명령은 다음과 같다. 각 SHA에서 동일한 명령과 전용 target을 사용한다.

```text
cargo test --offline --locked -p akra-verification-native --all-features --target x86_64-pc-windows-msvc --target-dir target/ci-verification-native -- --test-threads=1

cargo test --offline --locked -p akra-verification-native --all-features --target x86_64-pc-windows-msvc --target-dir target/ci-verification-native --lib task_verification::tests::actual_git_check_binds_the_frozen_full_range_without_changing_index_head_or_refs -- --exact --test-threads=1

cargo clippy --offline --locked -p akra-verification-native --all-targets --all-features --target x86_64-pc-windows-msvc --target-dir target/ci-verification-native -- -D warnings

node --test scripts/crate-boundaries.test.mjs scripts/native-rust-contracts.test.mjs scripts/ci-contract-targets.test.mjs
```

## 검증 결과와 남은 인수

- 실제 locked/offline Cargo metadata와 crate 경계 검사 통과. active workspace 5개이며 기존 transfer 44개를 보존하고 이 추출의 8개를 추가했다. root normal/dev/build 역참조를 만들지 않았다.
- Windows 후보 package **23개 통과, 실패·ignored 0개**. 실제 Git/trust/private filesystem/source identity/취소/출력 한도 계약과 새 fixture 회귀를 포함한다.
- standalone all-targets/all-features Clippy `-D warnings`, `cargo fmt --all -- --check`, `git diff --check` 통과.
- ownership·target/runner 관련 Node **36개 통과**, 실패·취소·skip 0개.
- 기존 Windows 시험 24개의 이름을 원본 소스에서 추출하고 **이동 21개 + root 잔류 3개**와 대조했다. 새 crate의 실행 inventory 23개는 이동 21개와 명시적 추가 2개의 합집합이고 ignored는 0개다. root 잔류 3개의 실행 성공을 이 목록 대조만으로 주장하지 않는다.
- 기본 feature와 all-features가 다른 기존 root/다른 package의 전체 합집합, root 소비자 all-targets compile/full gate, Linux/macOS 관련 native 시험과 최종 조합 SHA의 통합 인수는 남아 있다. 모든 완료 기준을 충족했다는 선언이나 이슈 자동 종료는 하지 않는다.
- 공통 Port shim guard 수정은 부모의 공통 기반에서 처리한다. 이 branch에 중복 적용하지 않았으며 기존 head의 architecture 실패를 이 독립 package 성공으로 해결됐다고 표시하지 않는다.

관련 근거: [#2864](https://github.com/RefinedStone/codex-exec-loop/issues/2864), [#2905](https://github.com/RefinedStone/codex-exec-loop/issues/2905), [#2908](https://github.com/RefinedStone/codex-exec-loop/issues/2908), [#2909](https://github.com/RefinedStone/codex-exec-loop/issues/2909), [최종 성능 인수 #2911](https://github.com/RefinedStone/codex-exec-loop/issues/2911).
