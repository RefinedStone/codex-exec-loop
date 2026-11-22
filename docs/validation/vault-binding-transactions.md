# Vault binding transaction 검증 기록

관련 이슈: #2414, #2654. 기준: `prerelease@42fd9ef85ac511e8620ffbe59529a1ad74f07fe9`.
작업 branch: `codex/vault-binding-transactions`.

## 수정 전 실패 재현

Windows x86_64 MSVC, Rust 1.95.0에서 기존 생산 코드에 회귀 테스트만 추가해 실행했습니다.

```bash
cargo test --all-features --target x86_64-pc-windows-msvc --lib \
  application::service::evidence_vault_restore::tests::a_reconnect_rejects_different_mappings_from_the_same_review \
  -- --exact --nocapture --test-threads=1
```

결과: **1 실패**, 프로세스 종료 코드 101. 제목만 연결한 동일 승인 재시도는 성공했지만,
같은 검토에서 본문만 선택한 다른 승인은 `Err(ChangedDuringReview)` 대신 `Ok(())`를
반환했습니다. 실행 session: `2cfc7f74-5df1-42b6-b3e0-c75ff695ae3d`.
수정 전 실제 실패와 이후 통과를 구분합니다.

## Windows 집중 검증

환경은 Windows x86_64 MSVC, Rust 1.95.0입니다. 모든 시험은 합성 이슈·키·평문과 fake 공급자를
사용했습니다. 실제 모델 turn, 실제 사용자 Vault, 실제 native 승인·키 저장소는 사용하지 않았습니다.
공유 빌드 캐시는 한 프로세스만 사용했고 `TEMP/TMP`는 저장소 밖의 작업 전용 경로로 지정했습니다.

공통 명령:

```bash
cargo test --all-features --target x86_64-pc-windows-msvc --lib <filter> -- --test-threads=1
```

| filter / 범위 | 결과 |
| --- | --- |
| `evidence_vault` | 48 통과 |
| `issue_publish` — 아래 기존 파일 시험 모듈 제외 | 36 통과 |
| `application::service::issue_creation` | 24 통과 |
| `application::service::issue_draft` | 30 통과 |
| `adapter::outbound::jira::publish_flow_tests` | 5 통과 |
| `desktop_restore` | 21 통과 |

실행 session: `3bf1e741-7426-45b8-832f-250f6b2ba468`,
`0991a38d-f75f-4bea-96b5-12ca1fe93b62`.

처음 넓은 `issue_publish` filter는 37 통과·2 실패였습니다. 두 실패는 기존
`adapter::outbound::filesystem::issue_publish_intent_journal::tests`의
`a_broken_line_is_an_error_and_is_not_overwritten`,
`records_survive_a_new_adapter_and_a_later_save_replaces_the_intent`입니다.
기존 secure filesystem이 Windows에서 지원하지 않는 직접 파일 읽기를 호출해 실패했습니다.
저장소의 플랫폼 경계를 바꾸거나 시험을 삭제하지 않았습니다. Windows 후속 명령은
`--skip adapter::outbound::filesystem::issue_publish_intent_journal::tests`로 서비스·도메인을
분리했고, 실제 파일 시험은 Unix CI 대상에 유지했습니다.

## 저장소 필수 로컬 gate

planner의 선택은 `rust`였습니다. 다음을 그대로 실행했습니다.

```bash
cargo fmt --all
git diff --check
node scripts/agent-plan.mjs
powershell -File scripts/check_native_pr.ps1 -Mode Rust
```

**종료 코드 0**. session: `900ad61e-8c6a-4bc6-a87c-a2b95c90a887`.

- CI policy 30개 통과
- TUI layering 및 Rust formatting 통과
- Windows native target의 전체 feature lib/bins 검사 통과
- Windows portable/security 계약 47개 통과
- 전체 target/feature clippy, 경고 오류 처리 통과

Desktop TypeScript·renderer 코드는 바꾸지 않았습니다. 이 작업의 로컬 검증 결과를
Desktop UI 또는 실제 native 사용자 확인 성공으로 확대하지 않습니다.

## 회귀가 구분하는 결과

### 서비스와 메모리 저장소

- 한 검토의 동일 승인 재시도는 쓰기 없이 성공하고, 같은 해시의 다른 mapping은 충돌합니다.
- 일부 표식을 제외한 첫 확정 뒤에도 재시도는 원래 source version에서 같은 bundle을 구성합니다.
- 포인터 교체 뒤 결과 불명과 완료 일지 저장 실패에서 같은 operation을 재확인합니다.
- 원격 게시 전에 저장한 expected token으로 늦은 finalize를 막습니다.
- 신규 restorable 게시의 unreadable pointer는 원격 읽기·쓰기 전에 Held이며, 명시한 공유본 전용
  게시에는 영향을 주지 않습니다.
- A approval과 B의 유효 grant를 섞는 시험은 B에도 실제 readable binding을 두고 B 복원 성공을
  먼저 확인합니다. 프로젝트·서버·저장소·이슈별 거절과 A/B 전체 레코드 보존을 확인합니다.
- 최신 조회 중 세션을 종료하면 쓰기 없이 Locked입니다.
- 백업 인증·빈 pointer·duplicate/alias scope 검증은 첫 로컬 쓰기보다 앞섭니다.
  이슈별 일부 성공과 재시도, unreadable 복구 승인, readable local 보존을 구분합니다.
- 공용 fake는 실제 MutexGuard, 관측 token, 전체 field 비교와 CAS 조건을 구현합니다.
  성공만 반환하는 transaction stub을 사용하지 않습니다.

### 실제 Unix 파일·프로세스 시험

다음은 Windows 로컬에서 실행하지 않는 CI 검증 범위입니다. 이 목록을 로컬 실행 성공으로
기록하지 않습니다. 최종 head의 실행 결과와 Codex 리뷰 결과는
[#2654 진행 기록](https://github.com/RefinedStone/codex-exec-loop/issues/2654)과 연결 PR에 기록합니다.

- `evidence_vault_store::binding::tests`: 18개 시나리오와 합성 child entry 1개.
  실제 암호문, 동일 token의 독립 프로세스 경쟁, fullscope별 잠금, 부분 staging 재사용,
  포인터 CAS, alias/inode/root 안전 경계, 포인터 이전·이후 오류 의미를 검사합니다.
- 실제 자식 프로세스를 제목 staging 뒤와 포인터 직전에 종료하고, 이전 pointer·두 필드의
  읽기 가능 상태와 lease 해제·동일 operation 재시도를 확인합니다.
  부모의 kill/wait와 별도로 자식 자체의 최대 대기 시간도 둡니다.
- 백업의 실제 파일 export/import는 중복 issue 선택의 왕복, v1 archive와 로컬 키 재봉인을
  확인합니다. root는 기존 계약대로 준비된 0700 실제 디렉터리입니다.
- Gateway issue flow는 실제 파일 store/journal과 fake GitHub를 사용합니다.
  동일 승인 재시도·다른 승인 거절·서비스 재생성 후 새 grant에서도 암호문 bytes가 유지되고
  원격 쓰기가 늘지 않는지 확인합니다.

Gateway의 서비스 재생성은 실제 OS 프로세스 종료와 다른 시험입니다. 두 프로세스 경쟁은
공통 filesystem transaction 경계를 직접 실행하며 native helper나 Desktop UI를 실행하지 않습니다.
전체 archive를 하나의 원자적 transaction으로 처리한다고 주장하지 않습니다.

## 독립 검토와 반영

서비스, 게시·백업, 저장소·프로세스 시험을 교차 읽기 검토했습니다. 검토에서 발견한
늦은 finalize의 기대값 시점, 중복 export scope, 빈 current version의 선행 검증,
unreadable 게시 보류, 이슈별 lease 분리와 child 대기 상한을 반영했습니다.

새 root를 자동 생성하지 못한다는 초기 관찰은 기존 adapter 계약을 확인한 뒤 시험 준비 문제로
정정했습니다. 제품 경계를 넓히지 않고 fixture만 올바르게 준비했습니다.

최종 리뷰·CI와 rebase merge는 이 로컬 기록 이후 연결 PR에서 확인합니다. 상위 #2414의
native 재연결 UI 및 실제 OS 확인은 후속 범위로 유지합니다.
