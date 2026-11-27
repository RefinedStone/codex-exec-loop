# 공통 계획을 사용하는 GitHub Actions 검증

관련 이슈: #2904, #2905, #2906, #2907, #2911.

## 선택과 전체 검증

`CI Scope`는 Rust 변경에 대해 `ci-plan-inputs.mjs`로 실제 checkout과 base의 입력을 수집하고, `ci-impact-plan.mjs`의 같은 계획을 사용한다. 문서와 Node만 바뀐 경로는 기존 가벼운 분류를 유지한다. PR의 계획은 임의 branch 최신 HEAD가 아니라 GitHub가 checkout한 정확한 merge SHA를 가리킨다.

`ci-workflow-plan.mjs`는 계획을 package/target/OS/features/profile/target triple 행으로 확장한다. library, binary, integration harness와 library doctest를 구분한다. 기본 조건은 Linux x64, Windows x64, macOS ARM64와 default/all features, test profile이다.

다음 중 하나면 기존 전체 Rust/portable 검증으로 돌아간다.

- 공통 계획이 full을 요구하는 root consumer, 공통 계약, manifest/lockfile/toolchain, 등록하지 않은 입력이나 불완전한 graph
- postmerge push 또는 수동으로 지정한 scope
- 테스트 harness로 컴파일 범위를 증명할 수 없는 compile-only target
- required-features 대상, 지원하지 않는 target나 계약의 기능/격리 조건, custom target triple
- 필수 contract가 대응하는 실행 행을 찾지 못함
- 선택할 Rust 행이 없거나 Actions matrix의 256개 한도를 초과함

현재 root 앱이 issue-core/native/verification의 consumer인 변경은 계속 full이다. crate 분리만으로 전체 앱 consumer 시험을 생략하지 않는다. 이 변경 자체가 현재 모든 PR의 job 수나 postmerge 대기시간을 줄였다는 뜻은 아니다.

## 실행과 결과 판정

Scope는 원본 계획과 matrix를 artifact로 저장한다. worker는 이 파일을 다시 계산해 검증하고, 지정한 행만 실행한다. package·target 선택은 #2905의 Cargo JSON executable 식별을 사용한다. 격리가 필요한 계약을 먼저 별도 process로 실행한 다음, 해당 시험을 정확하게 제외한 나머지 harness 시험을 실행한다. doctest는 별도 행에서 같은 package·features·profile·target triple로 실행한다.

각 worker는 시작과 완료 때 실제 HEAD와 clean checkout을 확인한다. Git에서 ignored된 등록 입력도 허용하지 않아 clean status 밖의 파일이 컴파일 입력을 바꿀 수 없게 한다. Windows는 기존에 신뢰 검사에 사용하던 사전 설치 Node를 유지한다.

`Selected Rust Results`는 다음을 모두 만족해야 성공한다.

1. matrix job 자체의 결과가 success이다.
2. 계획한 모든 행의 결과가 정확히 하나씩 존재한다.
3. 결과의 SHA, plan hash, matrix hash, package, manifest, target, OS, architecture, features, profile, target triple이 계획과 같다.
4. 각 행의 상태가 success이며 exit code가 0이다.

빠짐, 중복, 예상하지 않은 결과, 실패, 취소, skip, 다른 SHA나 설정의 결과는 거부한다. artifact directory를 합치지 않아 같은 이름의 결과 파일이 서로 덮어쓰여 중복 검사가 사라지는 것도 막는다. 선택 검증 중에도 workspace boundary, formatting, Clippy와 전역 architecture 계약은 실행한다.

기존 required context 이름인 `Fast Gate`, `CI Gate`, `Post-Merge Gate`는 그대로다. `CI Gate`는 full일 때 기존 Rust/portable 결과를, selective일 때 위의 정확한 결과 집계를 요구한다. `Fast Gate`의 기존 admission 범위와 Ruleset/bypass 설정은 바꾸지 않는다.

## 같은 계획의 실패 작업 재실행

Scope artifact의 실제 이름을 job output으로 전달한다. 실패한 작업만 재실행해 Scope가 다시 실행되지 않아도 원래 immutable 계획을 사용한다. 행 결과 artifact는 workflow run ID와 row ID를 이름으로 사용하며, 재실행한 행이 자신의 이전 결과만 교체한다. 이전에 성공한 행은 source SHA와 두 hash가 같을 때만 집계에 포함된다. 새로운 코드나 계획의 성공 결과로 승격하지 않는다.

## 캐시와 계측

기존 `ci-cache.mjs`가 OS/architecture/toolchain/manifest/lock/features/profile/target 조건을 구분한다. 선택 PR job은 복원만 하고 캐시를 저장하지 않는다. 신뢰된 full push가 기존 저장 정책을 유지한다. Linux 선택 행의 explicit target cache는 기존 implicit-host cache와 구분되고, Windows/macOS default-feature cache도 기존 all-feature producer와 구분된다. 현재 이 namespace를 채우는 신뢰된 producer가 없으므로 해당 target cache가 반복해서 cold일 수 있다. registry cache는 공유한다. 호환하지 않는 cache를 hit로 만들기 위해 key 조건을 낮추지 않는다. producer 추가나 실행 조건 정렬은 #2866/#2911의 남은 성능 인수다.

계약의 부분 진행 기록과 Cargo timing artifact는 실패 시에도 업로드한다. 필수 행 결과가 생성되지 못하면 결과 업로드와 집계가 실패한다. 기존 60분 native 제한과 각 전체 job의 제한 시간을 늘리지 않는다.

## 전체 경로로 복귀

workflow_dispatch의 `scope=full`은 선택 행을 사용하지 않고 기존 전체 경로를 실행한다. push/postmerge도 현재 full이다. 로컬은 `node scripts/agent-plan.mjs --run --full` 또는 기존 native gate의 Full 경로를 사용한다. postmerge 선택 전환은 #2911의 최종 SHA 인수 이후 별도로 판단한다.

## 현재 확인 범위

실제 Git fixture와 Node 정책 시험으로 선택 행 생성, compile-only 누락 반례, 필수 계약, 모든 결과 조건, 실패/취소/누락/중복, artifact 구조 및 source 변경 거절을 확인했다. workflow YAML을 파싱하고 기존 gate 정책 시험을 함께 실행했다.

2026-11-27의 최종 로컬 확인에서 공통 planner·collector·로컬 실행기·기존 CI scope·새 matrix 검사를 함께 실행해 **110개 통과, 실패·취소·skip 0개**를 확인했다. 이 중 matrix/결과 집계 회귀는 16개다.

```text
node --test scripts/agent-plan.test.mjs scripts/ci-plan-inputs.test.mjs scripts/ci-impact-plan.test.mjs scripts/ci-scope.test.mjs scripts/ci-workflow-plan.test.mjs
```

Windows tunnel 중단 후 이 연결 코드는 임시 실행 환경에서 작성했다. 실제 Actions의 새로운 선택 matrix 실행, Windows/macOS 실행과 전체 통합 SHA 성공은 아직 인수하지 않았다. 현재 PR은 이 제한을 명시하는 Draft로 제공하며, #2911의 전체 성능 인수 완료로 계산하지 않는다.
