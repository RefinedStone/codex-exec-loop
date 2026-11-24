# Cargo Workspace 분리 결정 기록

- 관련: [CI 성능 Epic #2690](https://github.com/RefinedStone/codex-exec-loop/issues/2690),
  [조건부 Issue Automation 추출 #2569](https://github.com/RefinedStone/codex-exec-loop/issues/2569)
- 조사 기준: prerelease@70966a27f27429f9134e467c98b7b09a9894e39d
- 결정: **Workspace 전환과 crate 추출을 보류한다.** 기존 모듈 경계를 유지하고,
  계측 및 빌드 조건 정리 뒤에 실제 편집 패턴의 재컴파일 자료로 재검토한다.
- 이 기록은 구조 검토 결과다. crate 분리, 성능 개선, 플랫폼 검증을 완료했다는 뜻이 아니다.

## 현재 구조와 비용의 구분

추적된 Cargo.toml은 root의 한 개다. 하나의 package가 src/lib.rs, src/main.rs 및
src/bin/akra.rs, akra-admin.rs, akra-gateway.rs, akra-telegram.rs를 제공한다.
기본 실행과 akra 실행은 library의 run()으로 들어가고 Gateway는 같은 library의 inbound
adapter로 진입한다. 따라서 binary wrapper만 package로 나누어도 중심 library의 컴파일
경계는 작아지지 않는다.

Cargo.toml에는 dev line-table debug, dependency debug 제거, 별도 debugging profile이
이미 있다. CI는 CARGO_INCREMENTAL=0을 쓰며 로컬 dev profile은 incremental=true다.
작은 소스 변경은 해당 crate의 Cargo 컴파일 단위를 다시 만들 수 있지만, 로컬 rustc 증분
컴파일의 실제 재사용량을 소스 줄 수만으로 판단할 수는 없다. 빌드·시험·링크 비용을 분리한다.

[#2690 기준 실행](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45231960486)은
다른 커밋 c63bec6e59b8c20e7247eb2ba597376d24805369에서 관측됐다. macOS feature 불일치와
캐시 miss, Linux 시험 실행 시간이 알려졌을 뿐, 같은 편집의 분리 전후 비교가 없다.
이 한 실행으로 Workspace 추출이 더 빠르다고 결론 내리지 않는다.

## 실제 경계 후보

| 후보 | 현재 의존 근거 | 예상 분리 범위와 비용 | 현재 판단 |
| --- | --- | --- | --- |
| 이슈 공유 값 | src/domain/issue_work.rs는 serde, schemars, sha2와 표준 타입을 사용 | 값과 해시 계약은 작지만 공용 타입 이동으로 import, 직렬화, schema 검증이 필요. 매우 작은 crate만 떼면 큰 root 재컴파일 비용이 남음 | 낮은 결합도 후보로 기록, 효과 미측정 |
| Issue Automation | domain/issue_automation, issue_* service, IssueWorkExecutionPort와 관련 ports | 공유 값·정책·검증·이벤트 경계를 함께 정리하고 adapter는 Akra에 남겨야 함 | #2569 착수 조건 전 보류 |
| UI/adapter 묶음 | src/lib.rs가 adapter/application/core/composition을 조립 | crate-private 타입과 조립 접근을 재설계해야 하고 TUI·Gateway·Admin 실행 경로 및 feature 전파에 영향 | 한 번에 분리하지 않음 |
| binary wrapper만 분리 | src/main.rs와 src/bin/akra.rs가 같은 run()을 호출 | 중심 library는 그대로 컴파일하고 manifest·배포 복잡도만 늘 가능성 | 우선 후보에서 제외 |

위 표는 소스 참조 분석이며 컴파일 성능 측정값이 아니다. 범용 shared/core crate를 먼저
만들고 임의의 공통 코드를 모으는 방식은 의존 방향과 성능 문제를 확인하기 어렵게 한다.

### Issue Automation의 숨은 순환 경계

현재 모듈 구조에서 허용되는 참조가 crate 사이에서는 순환을 만들 수 있다.

- src/domain/issue_automation/timeline.rs는 domain::kernel_event의 KernelEvent,
  RecordedKernelEvent 및 이슈 이벤트 기록을 읽어 timeline을 투영한다.
- src/domain/kernel_event.rs는 issue_automation::collection의 IssueCollectionMode와
  issue_automation::execution의 IssueExecutionIntegration/IssueExecutionProgress를 사용한다.
  동시에 domain::planning과 kernel_spec도 참조한다.
- issue_automation/source.rs는 issue_policy, triage.rs와 triage/basis.rs는
  issue_validation, triage/criteria.rs와 triage/paths.rs는 issue_markdown을 참조한다.
- issue_coordinator.rs와 issue_triage.rs는 여러 issue_* service와 ports를 조합한다.

따라서 issue_automation 디렉터리를 그대로 새 crate로 이동하고 root에 대한 의존을 남길 수 없다.
추출을 시작한다면 다음 둘 중 필요한 범위를 먼저 결정한다.

1. kernel journal을 읽는 timeline 투영은 Akra 쪽에 남기고 automation은 순수 상태와 port만 제공한다.
2. 이슈 전용 이벤트 DTO를 좁은 공용 계약으로 만들고 kernel event 변환은 Akra에서 수행한다.

어느 경우에도 모든 kernel_event/planning을 automation crate에 끌어넣어 순환을 해소하지 않는다.
공유본과 Vault 원문·키의 경계를 유지하며, 기존 이벤트 직렬화 이름과 저장된 레코드를 보존한다.

## #2569와의 관계

#2569의 착수 조건은 **독립 배포/추출을 결정했거나 두 번째 실행기가 필요할 때**다.
#2690의 빌드 성능 검토만으로 이 조건을 해제하지 않는다. #2555 선행 조건도 그대로다.

현재 production 구현은 src/composition/issue_work_execution.rs의
AkraIssueWorkExecutionAdapter다. 테스트의 FakeExecution, AnswerExecution,
InterruptedSubmission을 두 번째 production 실행기로 세지 않는다.
추후 추출 때는 실제 두 번째 adapter가 같은 port 계약 시험을 통과해야 한다.

tests/architecture_boundaries.rs의 issue_automation_stays_independent_and_stores_only_shared_issue_types
및 관련 semantic-reference 검사는 현재 모듈 경계를 지킨다. 그것이 곧 crate 단위 독립
빌드를 증명하지는 않는다. 파일 경로와 CARGO_MANIFEST_DIR에 의존하는 검사도 있으므로
이동 시 검사 범위를 함께 갱신하며 삭제하거나 제외 처리하지 않는다.

## 추출을 다시 검토할 때 수집할 증거

먼저 동일 runner/OS/아키텍처, rustc, Cargo.lock, features, profile, RUSTFLAGS를 고정한다.
CI의 증분 비활성 조건과 로컬 증분 활성 조건은 별도 표로 남긴다. 활성 worktree의 target은
보존하고 측정용 target을 따로 쓴다. cargo clean이나 모든 worktree의 target 공유를 넣지 않는다.

| 조건 | 비교할 편집 | 기록할 값 |
| --- | --- | --- |
| cold | 변경 없는 동일 source | Cargo timings, elapsed, peak memory, target 크기 |
| warm/unchanged | 변경 없는 재실행 | 실제 recompile 대상, cache restore/save 비용 |
| 작은 adapter 변경 | 비동작 변경 또는 작은 실제 수정 한 건 | root 및 의존 crate compile/link 시간 |
| 작은 domain 변경 | 동일 범주의 별도 실제 수정 | 역의존 재컴파일 범위와 unit/integration 시간 |

정확한 commit, 명령, Actions 링크, 표본 수, 중앙값·범위를 함께 기록한다. CLI 빌드와 lib test
빌드를 섞어 개선율을 계산하지 않는다. 후보 추출은 같은 편집을 분리 전후에 적용한 비교로 판단한다.
전체 경과 시간뿐 아니라 runner 총사용 시간, 메모리·디스크·artifact 전달 비용도 비교한다.
이 문서에는 그 실험을 실행하지 않은 상태임을 명시한다.

기존 명령의 Cargo timings를 켤 수 있는 최소 재현 예는 다음과 같다. 여기서는 실행하지 않았다.

~~~bash
cargo build --locked --bin akra --timings
cargo test --locked --lib --no-run --timings
~~~

## 조건이 충족되면 적용할 순서

1. 분리할 유스케이스와 필요 consumer를 정하고 #2569의 착수 조건 충족 여부를 확인한다.
2. 위 순환 경계와 공용 값의 소유자를 확정하고 crate import 방향을 architecture 검사로 고정한다.
3. 가장 작은 유효 경계를 별도 PR로 추출한다. root package와 기존 binary 이름을 유지한다.
4. workspace feature 전파, 기본/전체 feature 조합, doctest·ignored/native 경로를 대조한다.
5. Linux/macOS/Windows, 기존 Clippy·architecture, kernel simulation, CLI 및 native 배포를 확인한다.
6. 동일 편집의 compile/execute/runner 비용을 비교한다. 이득이 없으면 구조 변경을 되돌리고
   계측·문서는 남긴다. 두 번째 실행기 계약은 #2569 범위에서 별도로 충족한다.

scripts/package_native_release.sh는 root manifest 및 codex-exec-loop-native 이름을 사용하고,
release-native-assets.yml은 Cargo 버전과 배포 산출물을 연결한다. Workspace 변경 시 이
계약과 npm launcher/platform 패키지의 이름을 함께 검증한다. build cache 키도 전체
workspace manifest/lockfile/toolchain을 다루어야 한다.

## 이번 결정의 검증과 제한

tracked manifest, library/binary entrypoint, automation 참조, port 구현, architecture 검사,
배포 스크립트를 읽어 결정했다. 소스와 manifest를 이동하지 않았으므로 이번 PR은 문서 변경이다.
git diff --check만 수행한다. 전체 Rust 재빌드나 추가 CI 조회는 하지 않으며, 시험 축소나
ruleset 변경을 이 결정에 포함하지 않는다.

공식 참고: [Cargo Workspace](https://doc.rust-lang.org/cargo/reference/workspaces.html),
[Cargo build performance](https://doc.rust-lang.org/cargo/guide/build-performance.html).
