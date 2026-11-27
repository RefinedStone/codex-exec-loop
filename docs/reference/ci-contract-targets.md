# 패키지와 target을 구분하는 native 계약 실행기

관련: #2905, #2930. 전체 workspace 선택은 #2903, 공통 계획/실행은 #2904/#2906/#2907과 연결한다.

## 실행 계약

`run_native_rust_contracts.mjs`는 --locked Cargo metadata에서 workspace package ID와 target kind/name을 얻는다. Cargo JSON artifact를 package_id, target name/kind, test profile에 모두 일치시켜 실행 파일을 고른다. 이름이 같은 다른 package, production binary, 중복·누락 artifact는 성공으로 처리하지 않는다. package ID 문자열의 내부 구조는 해석하지 않는다.

같은 build options와 target 종류를 쓰는 요청은 한 번의 --no-run으로 준비한다. feature/profile/target triple, 호출자의 CARGO_TARGET_DIR 및 환경, 동적 로더 경로를 실행까지 보존한다. 새 package의 시험이 dependency라는 이유만으로 자동 실행된다고 가정하지 않는다.

기본 CLI는 기존 문자열 필터를 받는다. 이동한 시험은 소유권 registry transfers의 oldTestPrefix/newTestPrefix 및 추가 testMappings로 실제 owner inventory에 역매핑한다. source만 이동하고 시험 prefix가 없는 항목은 시험 매핑으로 쓰지 않는다. 같은 필터가 root와 member에 나뉜 경우 각 owner에서 실제로 일치한 시험만 실행한다. 필수 필터가 0개 비-ignored 시험에 맞거나, 두 필터가 같은 package/target 시험을 겹쳐 선택하면 실행 전에 실패한다.

## CLI

```sh
node scripts/run_native_rust_contracts.mjs --include-members --all-features --target x86_64-pc-windows-msvc -- 'process_liveness::tests::'
node scripts/run_native_rust_contracts.mjs --package akra-issue-core --target-kind lib --target-name akra_issue_core --all-features -- 'issue_validation::tests::'
node scripts/run_native_rust_contracts.mjs --contract-id evidence-gate --all-features --
```

명시 package 선택은 registry의 old namespace 대신 해당 target의 실제 시험 이름을 받는다. --contract-id는 registry의 owner/target/filter/OS/feature/격리 조건을 적용한다.

선택 계획은 --include-members --package <member>를 사용할 수 있다. 뒤의 실제 계약 필터가 없으면 그 member의 모든 일반 시험·통합시험·doctest를 실행하며, 필터가 있으면 그 그룹과 나머지를 나누어 중복 없이 실행한다. root 전체 검증은 이 단축 경로를 거절하고 기존 full gate로 보낸다.

CI target matrix는 --all-tests --package <member> --target-kind <lib|test|bin> --target-name <name>를 쓴다. 이 명시 모드에서는 해당 harness 전체를 검사하므로 실제 0개 시험 harness도 빌드·실행한 결과로 기록한다. 필수 계약 필터의 0개 일치 거절은 계속 유지한다.

## 전체 member 보존과 격리

--include-members는 기존 root 계약 필터를 실행하고 새 member의 남은 비-ignored 단위시험, bin/integration harness를 각각 실행한다. 이미 계약으로 실행한 시험은 --exact --skip으로 제외하므로 부분 이름이 같은 시험을 실수로 제외하지 않는다. 이어 각 member의 doctest를 같은 Cargo 조건으로 실행한다. root의 전체 검증은 기존 Linux workspace gate가 유지하고, Windows/macOS의 기존 root native 계약과 별도 native confirmation helper 경로를 보존한다.

일반 ignored 시험을 활성화하지 않는다. Gateway 실행 파일과 정확히 지정한 native confirmation ignored helper는 기존 전용 스크립트가 같은 feature/target/profile로 검증한다. process-per-filter는 owner별로 유지하며 임의로 모든 시험을 병렬화하지 않는다. 표준 출력은 실시간으로 보내고 package/target/OS/features/profile/triple 및 plan hash를 진행 JSONL에 남긴다. 실패·취소·신호 종료, 실제 통과 개수와 inventory 불일치는 실패다.

compareContractInventory는 package/target별 기존·새 시험의 일대일 매핑과 ignored 상태를 검사한다. 이름 변경과 신규 시험은 명시 목록이 필요하고 누락·중복·ignored 전환을 거절한다.

## 검증 근거

- Node 회귀: runner/target/progress/CI scope/impact plan 84개 범위를 검사했고, 마지막 empty-harness 수정은 target 회귀11개와 실제 Cargo fixture로 재확인했다.
- 실제 Cargo fixture: 동일 lib 이름을 가진 2개 package, 이동한 필터, member 나머지, integration, doctest, ignored 유지, 명시 package/target 선택을 실행했다.
- fixture 결과: 3개 실행 파일 준비, 4개 그룹/5개 일반 시험 + member doctest1개 통과. 명시 integration 요청1개 추가 통과.
- 재현: `node scripts/check_ci_contract_targets.mjs`. 외부 dependency 없이 자체 임시 workspace를 만들고 그 디렉터리만 정리한다.
- 기록: docs/validation/ci-contract-targets-20261127.json. 실제 제품의 최종 조합 검증은 #2911에서 수행한다.

이 실행기 변경만으로 전체 CI 시간을 단축했다고 주장하지 않는다. 기존 필터/시험 소유권을 보존하는 실행 기반이며, 느린 시험 자체의 변경은 #2864/#2865에 분리되어 있다.
