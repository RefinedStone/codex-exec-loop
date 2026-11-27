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

기존 합집합은 Windows 24개 = 이동 21개 + root 3개, Linux/macOS 23개 = 이동 20개 + root 3개다. Windows 경로 정규화 시험 1개는 기존 플랫폼 조건을 유지한다. 추가한 submodule 예외·동일 byte/key 개수 제한 회귀 1개를 포함하면 새 crate는 Windows 22개, Linux/macOS 21개다. 이동한 원래 시험에는 ignored 항목이 없다.

원래 helper가 확인하던 output hash와 실제 dirty source/index 보존 assertion도 유지했다. 루트에 남긴 dirty snapshot 시험은 새 adapter의 공개 runner Port를 통해 실행한다. 기존 portable prefix는 이 root 시험 1개를 계속 선택하며, 이동한 시험은 workspace member test에 포함된다.

## 상위 네 고비용 업무 시험군

| 시험군 | 이전 소유 | 이후 소유 | 필수 OS 및 검증 |
| --- | --- | --- | --- |
| evidence_gate | root lib | root lib | Linux/Windows/macOS, all features, full 유지 |
| post_turn automatic_verification | root lib | root lib | Linux/Windows/macOS, all features, full 유지 |
| orchestrator automatic_verification | root lib | root lib | Linux/Windows/macOS, all features, full 유지 |
| pool lease task_verification | root lib | root lib | Linux/Windows/macOS, all features, full 유지 |

이 네 군은 SQLite/Git과 업무 admission/continuation/lease를 묶으므로 adapter 시험으로 통째 옮기지 않았다. registry의 네 기존 root-integration 계약과 consumerCoverage full을 유지하고, adapter/Git builder/config-key native 계약 및 root dirty-snapshot 계약을 추가했다.

## 실행 및 측정

```sh
cargo test --locked -p akra-verification-native --all-features
```

새 package의 준비와 시험 자체 비용, 변경 영향으로 선택한 package/target 수, 전체 CI 시간은 서로 다른 지표다. #2864의 Windows process wait 비용 수정과 이 package 선택 범위를 혼합해 개선율을 계산하지 않는다.

## 검증 기록

- source 이동과 시험 이름 매핑은 ownership registry에 기록했다.
- 실제 native 연결 뒤 metadata/단위시험/Clippy/전체·ignored inventory 결과를 추가한다.
- root 소비자 all-targets compile와 full gate 및 세 OS의 관련 native 시험은 최종 통합 인수 범위다.
