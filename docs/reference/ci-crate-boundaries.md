# Crate boundaries and test ownership

기준: `caf1da09aa561d1510b3271cc663b8e5bf3a0690`, #2902. 실제 추출 여부는
`scripts/ci-crate-ownership.json`의 `state`와 Cargo workspace를 대조한다.
이 문서는 구조/검증 계약이며 성능 인수 결과가 아니다.

## 첫 경계

| 소유 package | 코드와 시험 | root에 남는 책임 |
| --- | --- | --- |
| akra-issue-core | issue_work/policy/template/markdown/validation/masking/secret_spans/review/publish_intent/creation와 함께 있는 단위시험 | 파일 읽기 Port, provider/DB/Vault adapter, 게시/승인 서비스 |
| akra-native | 프로세스 생존, 신뢰 실행 파일, private 파일/디렉터리 identity 원시부와 OS 계약시험 | SQLite schema/transaction, provider 의미, 제품 설정·조립 |
| akra-verification | 독립 가능한 evidence/verification 규칙·요청·결과와 fake Port 시험 | pool/issue admission/Kernel owner 및 저장소 조립 |
| akra-verification-native | 위 Port를 구현하는 Git 검증 adapter와 native 계약 | 다른 Git 기능과 업무 통합·최종 전달 판단 |
| akra-ade | 제품 bin, Kernel/Gateway/TUI, 아직 미이관 기능과 통합시험 | 배포 이름·root public API를 유지 |

`akra-native`의 실제 파일 이동, root 잔류 정책, Windows 전후 시험 목록과 기존 호스트 ACL 실패는
[Native crate 분리 검증](../validation/ci-native-crate-extraction-20261127.md)에 기록한다.
`subprocess`는 명시적 timeout 원시부가 native이고 제품 설정은 root facade가 소유한다.
`trusted_executable`의 Codex 정책·캐시도 root에 남는다.

순수 template parser 및 policy 해시는 실제 사용하는 좁은 API만 추출한다.
파일 SourcePort를 새 crate에 가져오기 위해 제품 서비스 전체를 옮기지 않는다.
큰 공용 core/ports crate로 순환 참조를 숨기지 않는다.

## 의존 및 공개 API 계약

- root는 기능 crate를 소비한다. 기능 crate는 root를 normal/dev/build dependency로
  되가져오지 않는다. optional/rename dependency에도 같은 제약이 적용된다.
- `check_crate_boundaries.mjs`는 Cargo의 실제 member/manifest/선언 의존성과
  registry를 대조한다. 등록되지 않은 member와 허용되지 않은 의존 방향은 실패한다.
- `IssueValidationPassed.shared_copy`는 private이고 생성자는
  `validate_issue`뿐이다. Deserialize를 추가하지 않는다. 새 package의 외부 컴파일
  실패 시험으로 생성·역직렬화 거절을 확인한다.
- Markdown parser 내부 타입/함수와 내부 helper를 외부 재노출하지 않는다.
  root의 한시적 module 재노출은 기존 소비자 경로 호환용이며 구현은 복제하지 않는다.
- Vault 원문·키를 issue-core로 옮기지 않는다. confirmation/token의 제한된 생성,
  Debug 가림, 비직렬화, zeroize 계약은 추출과 관계없이 보존한다.
- issue hash 입력/공유본 JSON/이벤트 tag·revision, receipt와 저장 상태는 기존
  fixture를 함께 옮기고 변경 전후를 대조한다.

## 파일·시험·외부 입력 소유권

기계 판독 registry에 첫 추출의 파일별 원래/새 경로와 libtest prefix가 있다.
`issue_masking/tests.rs`도 코드와 함께 이동한다. package/target/OS/features/
ignored 상태가 시험 식별의 일부이다. 이름만 같아도 다른 target은 다른 시험이다.
move 전후 목록의 합집합을 비교하고 삭제·중복·ignored 변화는 허용하지 않는다.

현재 root의 consumer 계약 소유권은 **full**이다. Cargo 컴파일 역의존이나
public signature가 같다는 사실만으로 root 행동 시험을 생략하지 않는다.
미등록 경로·생성 입력·공유 fixture·불완전 base 그래프는 full로 처리한다.
templates/admin은 Rust와 Node 양쪽 입력이다. schema/IPC/플러그인 변경도
root 소비자 검증을 유지한다.

## 고비용 root 통합시험

| 그룹 | 유지 이유 | 2026-11-27 Windows 완료 시간 |
| --- | --- | ---: |
| distributor::evidence_gate | parallel admission, issue binding, SQLite/Git 결합 | 1,016.41초 |
| post_turn automatic_verification | 후속 턴과 검증 권한 연결 | 531.61초 |
| orchestrator automatic_verification | orchestrator 소유권·후속 실행 | 364.72초 |
| pool lease task_verification | live lease와 작업 검증 | 224.68초 |

근거: [실행 45936932223](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45936932223/job/140302241610).
126필터 중 80개를 시작하고 79개를 완료한 뒤 60분 제한으로 취소됐다.
네 그룹 전체를 native adapter 단위시험으로 간주하지 않는다. 실제 이전의 위치와
root 잔여를 #2910에서 갱신하고, 개별 지연·OS 편차는 #2864에 남긴다.

## 적용 순서와 검증

1. #2903: issue-core 코드·단위시험과 Workspace, 전체 `--workspace` 검증.
2. #2904/#2905: 동일 registry/plan을 사용하는 영향 계획과 package/target 실행기.
3. #2906/#2907: 로컬 및 Actions 연결. 새 member가 full 경로에서 누락되면 실패.
4. #2908/#2909/#2910: native/verification 실제 추출과 계약 분류.
5. #2911: 같은 편집·runner·OS·feature 조건의 compile/test/wall 인수.

첫 issue-core 파일럿은 후속 native 전체 추출 완료를 기다리지 않는다. 단, 아직
계약 소유권이 완전하지 않은 root는 full 검증을 유지한다. #2912는 별도 경계 준비이며
#2569의 실제 두 번째 production 실행기 인수 조건을 대신하지 않는다.

경계 검사: `node scripts/check_crate_boundaries.mjs`.
정책 반례: `node --test scripts/crate-boundaries.test.mjs`.
