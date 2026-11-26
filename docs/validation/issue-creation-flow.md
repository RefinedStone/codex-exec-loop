# 새 GitHub 이슈 생성과 결과 확인 (#2436, #2702)

새 이슈는 아직 안정 이슈 ID가 없으므로 기존 이슈 수정과 별도의 생성 의도를 사용한다. 저장소의 안정 ID와 로컬 의도 ID를 먼저 기록하고, 실제 이슈가 확인된 뒤 기존 게시·복원 서비스에 연결한다.

## 사용자 흐름

1. GitHub 이슈 목록의 **새 이슈**에서 초안을 연다. linked worktree와 경로 별칭을 대표 checkout으로 정규화하므로 같은 프로젝트의 다른 창이나 앱 재시작에서도 같은 저장소의 마지막 생성 의도를 연다.
2. 프로젝트 정책과 템플릿의 필수 입력을 확인하고, 로컬 원문에서 민감 구간을 선택한다. 기존 초안 검증·마스킹을 재사용한다.
3. 저장소·의도·공유본 해시·정책 판·템플릿 판이 묶인 미리보기를 승인한다. 코드 검증기는 게시 직전에 실행한다.
4. 승인 공유본을 먼저 동결하고, 복원할 원문은 저장소/의도/공유본/필드 범위의 Vault 레코드로 암호화한다. 준비가 끝나면 전송 의도를 영속 기록한 뒤 GitHub 생성 요청을 한 번 보낸다.
5. 응답의 안정 ID와 재조회의 안정 ID·공유본이 모두 맞으면 생성 완료이다. 복원 연결이 실패해도 생성 완료 상태를 유지하고 **복원 연결 대기**를 표시한다.
6. 전송 이후 화면은 현재 정책 파일이 손상돼도 기록된 결과를 보여준다. 새 게시의 정책 검증과 결과 확인을 분리한다. 결과를 모르면 자동으로 생성 요청을 다시 보내지 않는다. 알려진 이슈를 재조회하거나 사용자가 GitHub 이슈 URL을 선택한다. 선택한 URL도 저장소, 안정 ID, 승인 공유본을 다시 검증한다.
7. 별도 이슈가 필요하면 명시적으로 새 초안을 시작한다. 이전 생성 결과가 불확실하면 중복될 수 있음을 확인해야 하며 새 의도에 이전 의도 ID를 남긴다.

## 상태와 영속 경계

| 상태 | 의미 | 다음 동작 |
| --- | --- | --- |
| `Prepared` | 로컬 의도만 준비됨 | 편집·검증·승인 |
| `Held` | 승인 공유본을 동결했고 원문 저장이 완료되지 않았거나 중단됨 | 같은 의도에서 원문을 덮어쓰지 않음. 공유본만 게시를 명시 승인하거나 새 의도 시작 |
| `Recorded` | 공유본과 필요한 복원 준비를 기록함. 요청은 아직 전송 전 | 동일 승인에 한해 `Sent`를 영속 저장한 뒤 전송 |
| `Sent` | 전송 직전 기록. 실제 원격 결과는 모를 수 있음 | 읽기와 수동 확인만 허용 |
| `NeedsConfirmation` | 응답 유실, 재조회 실패 또는 모순된 응답 | 자동 재전송 없이 결과 확인 |
| `Confirmed` | 안정 ID와 승인 공유본 확인됨 | 필요한 복원 연결만 재시도 |
| `Abandoned` | 전송 전의 이전 의도를 명시적으로 종료함 | 이전 의도에서는 전송·URL 확정 금지 |

전송 여부가 불확실한 이전 의도는 상태를 지우지 않고 새 의도에서 참조한다. 새 의도 저장이 실패해도 다음 재시작에서 중복 위험 확인을 다시 요구할 수 있다.

파일 journal의 프로세스 간 lease를 조회부터 원격 요청 종료까지 유지한다. 각 상태 저장은 별도의 원자 교체·동기화로 완료된다. 전송을 포함한 하나의 미완료 DB 트랜잭션에 기대지 않는다.

GitHub의 `clientMutationId`는 상관관계를 확인하는 값으로 사용한다. [공식 입력 계약](https://docs.github.com/en/graphql/reference/issues#createissueinput)은 이 값을 멱등 재시도 키로 보장하지 않는다. 자동 검색의 0건 결과도 미생성 증거로 취급하지 않는다. 이 구현은 검색 후보를 자동 확정하지 않고 명시적인 URL 선택을 제공한다.

## 복원 연결의 소유권

생성 전 Vault 범위에는 가짜 `IssueReference`를 만들지 않는다. 확정 뒤 저장소+의도 범위에서 원문을 열고 실제 이슈 ID의 게시 의도 범위로 **다시 암호화**한다. 암호문 이름만 바꾸지 않는다.

기존 `IssueJournaledPublishService`를 생성·수정 흐름이 공유한다. 일반 수정과 생성 연결·복원 확정은 같은 별도 publication lease에 참여하므로 다른 Gateway 프로세스와도 게시 순서를 직렬화한다. 같은 순서 번호가 발견되면 복원 확정을 보수적으로 거절한다. 생성 연결의 ID는 `create:<creation-intent>`로 고정해 중단 뒤 재개를 멱등 처리한다. 같은 runtime journal에서 원격 안정 ID가 같은 다른 게시 의도는 과거 workspace 범위가 달라도 충돌로 본다. 해당 이슈에 다른 게시 의도나 기존 CurrentVersion이 있으면 늦은 생성 연결을 거절한다. 따라서 같은 공유본으로 가려지는 다른 원문이 새 게시의 복원 정보를 덮어쓸 수 없다. 확정된 필드를 모두 저장한 뒤 CurrentVersion을 마지막에 갱신한다.

## 정보 경계

- 원문 요청 타입은 Debug와 Serialize를 구현하지 않는다.
- Gateway의 6개 `akra/issue/create/*` 경로는 기존 초안과 같은 로컬 전용·세션 비녹화 분류를 사용한다.
- 생성 journal에는 승인된 공유본, ID와 해시만 저장한다. 가린 원문은 Vault의 암호화된 레코드에만 저장한다.
- Desktop 원문은 편집 컴포넌트의 메모리에만 둔다. 브라우저 저장소, 대화 입력, 세션 녹화에 넣지 않는다.
- 생성 응답이 승인 공유본과 다르면 원격의 다른 본문을 journal이나 UI 공유본에 복사하지 않는다.

## 검증 범위

| 대상 | 검증 |
| --- | --- |
| 생성 서비스 | 메모리 journal·원격·Vault 대역으로 승인 결합, 응답 유실, 중복 클릭·재시작, 수동 URL 검증, 원문 동결, 복원 재시도 |
| GitHub 어댑터 | 실제 GitHub 호출 없이 GraphQL 인자, 대상 검증, 오류 재전송 금지 |
| 파일 journal | Unix 파일 journal과 Windows private SQLite, 별도 writer reservation, 조회 전용 provenance, 프로세스 종료 이후 재개, 손상·별칭·변조 경계 |
| Vault staging | Unix에서 scope 분리, 키·인증 검증, 기존 이슈 레코드와의 호환 |
| Gateway | 생성 6개 메서드의 원격 클라이언트 차단, 민감 요청·오류응답 비녹화 |
| Desktop | 타입 검사, 단위 테스트, fake Gateway를 사용하는 Electron E2E |

모델 turn과 실제 원격 이슈 생성은 검증에 사용하지 않는다. 플랫폼과 실행한 검증 결과는 PR에 기록한다.

### Windows 복구 증거의 범위

- `windows::tests::forced_process_exit_releases_lease_but_preserves_sent`는 Sent 커밋 이후 자식 종료와 reservation 해제를 검사하는 저장소 단위 회귀다.
- `windows::tests::lost_response_recovers_by_stable_id_with_real_windows_journal_and_vault`는 같은 프로세스의 서비스 재조립으로 응답 유실·안정 ID 확인·실제 Windows Vault 연결을 검사한다.
- `windows::tests::processes::remote_applied_creation_survives_worker_termination_without_duplicate_posts`는 부모 프로세스의 TCP 합성 공급자가 생성 적용을 끝내고 응답을 보류한 지점에서 실제 생성 worker를 강제 종료한다. journal의 durable `Sent`와 안정 ID 부재를 확인한 뒤 두 새 worker가 같은 의도를 다시 열고 기존 승인 클릭·재조회·명시적 URL 확인을 수행한다. 실제 Windows journal/Vault와 재암호화된 복원 연결, provenance 확정을 확인하며 공급자의 생성 횟수는 한 번이다. 원문은 journal에 남지 않는다.
- `windows::tests::processes::concurrent_update_retains_its_original_when_creation_returns_late`는 생성 응답을 기다리는 worker와 독립 수정 worker가 같은 Windows 저장소를 사용한다. 수정 worker가 같은 공개문에 대한 **다른 원문**을 먼저 확정한 뒤 생성 응답을 돌려준다. 늦은 생성 연결과 또 다른 프로세스의 재확인은 새 원문을 덮지 않으며 publication 순서도 추가하지 않는다. 생성 1회·수정 1회만 관측한다.

프로세스 통합 fixture는 기존 게시의 `issue_shared_copy_flow/delayed_workers.rs`와 같은 부모 소유 TCP 공급자·독립 worker 수명 패턴을 사용한다. 현재 Windows native/portable inventory의 `filesystem::issue_creation_journal::` 필터가 부모 회귀를 실행하고, ignored worker entry는 부모가 명시적으로 호출한다. 테스트 코드 존재와 실제 실행 결과는 구분해 PR에 기록한다.

이 증거는 원격 적용 직후/응답 전의 프로세스 종료와 실제 Windows 생성·수정 경합을 다룬다. SQLite payload 커밋 도중 종료, 전원 단절, 실제 GitHub 네트워크 장애, OS 키 저장소 검증을 증명하지 않는다. 키·이슈·원문은 합성 데이터이며 실제 사용자 계정이나 모델을 호출하지 않는다.

### #2702 완료 조건 대조 — 2026-11-26

검토 기준은 `prerelease@d960d7d62e810f8cd94c7aa78918b0ba0104b6d6`이다. 이 절은 현재 소스와
머지된 PR의 기존 실행 기록을 대조한 인수 기록이며, 이번에 Windows 시험이나 GitHub CI를
다시 실행한 결과가 아니다.

[#2702](https://github.com/RefinedStone/codex-exec-loop/issues/2702)의 배경에 적힌
“`IssueCreationJournalAdapter::begin`은 non-Unix에서 거절한다”는 설명은
[#2710](https://github.com/RefinedStone/codex-exec-loop/pull/2710) 이전 상태이다.
현재 [플랫폼 분기](../../src/adapter/outbound/filesystem/issue_creation_journal.rs)는 Unix 파일 journal과
Windows private SQLite journal을 각각 사용하고, 그 외 플랫폼만 거절한다.

| #2702 완료 조건 | 현재 구현·회귀와 기존 실행 근거 | 판정 |
| --- | --- | --- |
| 실제 Windows에서 journal 저장·재시작·lease·provenance 동작 | [Windows journal](../../src/adapter/outbound/filesystem/issue_creation_journal/windows.rs)은 reservation DB와 payload DB를 분리하고 원격 생성 전에 `Sent`를 커밋한다. [저장소 회귀](../../src/adapter/outbound/filesystem/issue_creation_journal/windows/tests.rs)의 `absent_provenance_is_read_only_and_sent_is_visible_under_reservation`, `forced_process_exit_releases_lease_but_preserves_sent`가 읽기 전용 provenance, 동시 writer 거절, 새 adapter 재개와 자식 종료 뒤 lease 해제를 다룬다. #2710 및 #2725에 Windows 실행 기록이 있다. | 충족 |
| 합성 GitHub로 승인 공유본 생성·안정 ID 재조회·Vault 재암호화/복원 연결·재시도 | 같은 저장소 회귀의 `lost_response_recovers_by_stable_id_with_real_windows_journal_and_vault`와 [프로세스 회귀](../../src/adapter/outbound/filesystem/issue_creation_journal/windows/tests/processes.rs)의 `remote_applied_creation_survives_worker_termination_without_duplicate_posts`가 실제 Windows journal/Vault, 합성 원격과 합성 키를 사용한다. 두 복구 worker가 같은 의도를 확인한 뒤 생성 횟수 1회, 안정 ID/provenance, 새 범위로 연결한 원문 복원을 확인한다. | 충족; 실제 GitHub·OS 키 저장소의 실행 증거는 아님 |
| 응답 유실·강제 종료·동시 요청·오래된 승인·외부 수정·journal/ACL 변조에서 자동 중복 전송·평문 노출 방지 | #2725의 두 프로세스 회귀가 원격 적용 뒤 종료와 독립 수정 worker의 새 원문을 늦은 생성이 덮지 않는 경계를 다룬다. Windows 저장소의 `changed_approval_cannot_replace_durable_intent`, `corrupt_payload_schema_legacy_and_hardlink_fail_closed`, [공통 private SQLite 회귀](../../src/private_sqlite/tests.rs)의 ACL·junction·경로 고정 검사, [생성 서비스 회귀](../../src/application/service/issue_creation/tests.rs)와 [초안 facade 회귀](../../src/application/service/issue_draft/tests/creation.rs)의 승인/공유본 불일치 거절을 함께 대조했다. journal에 원문을 남기지 않는 검사와 화면 저장소·콘솔 비노출 검사는 아래 UI 근거에 있다. | 명시한 장애·경합 경계에서 충족; 모든 장애 조합이나 전원 단절의 보장은 아님 |
| 결과 불명 상태의 명시적 복구 UX 및 실패·취소 정책 | [Desktop 화면 회귀](../../desktop/test/e2e/issue-create.mjs)는 잘못된 URL 거절, 정확한 URL 확인, 재조회 시 생성 횟수 유지, 응답 유실 뒤 기록 재개, 복원 연결만 재시도, 새 초안의 중복 위험 확인, 편집 시 승인 취소를 검증한다. #2572에 Windows 실행 기록이 있으며 현재 소스에 해당 검사가 유지된다. 아래 실패·취소 계약을 함께 적용한다. | 대역 Gateway를 사용하는 UI와 서비스 계약에서 충족 |
| Windows 지정 CI에 회귀 포함, Unix 기존 생성 계약 유지 | [Windows native gate](../../scripts/check_native_pr.ps1)와 [Windows portable gate](../../scripts/check_windows_portable.sh)에 `filesystem::issue_creation_journal::`가 포함된다. [inventory 회귀](../../scripts/native-rust-contracts.test.mjs)는 portable 필터 누락을 거절한다. [PR workflow](../../.github/workflows/native-pr-checks.yml)는 portable gate와 Linux `cargo test --locked --timings`를 유지하고, [Unix journal 회귀](../../src/adapter/outbound/filesystem/issue_creation_journal/tests.rs)도 유지된다. | 소스상 포함·유지 확인; 이번 작업에서 CI 실행 상태는 조회하지 않음 |

실행 기록의 출처와 범위는 다음과 같다. 과거 실행 성공을 현재 커밋에서 새로 실행한 성공으로
바꾸어 기록하지 않는다.

- [#2572](https://github.com/RefinedStone/codex-exec-loop/pull/2572), head `f7c7808e19438ac6655cec127890c3fbade9cfdf`:
  Windows에서 신규 생성 Electron E2E 재실행 및 Full gate 통과를 기록했다. 이때 실제 생성
  journal의 Windows 지원은 없었으므로, 이 기록은 복구 화면·공통 서비스의 근거로만 사용한다.
- [#2710 통합 기록](https://github.com/RefinedStone/codex-exec-loop/pull/2710#issuecomment-7156950827),
  head `53cd57cf01571ee6fe31dbc9e5806bcd516bc068`: Windows all-features 라이브러리 시험 빌드와
  관련 회귀 39개 통과를 기록했다. 생성 서비스 28개, 인계 1개, journal 6개, 동시 편집·복원
  취소·재연결 4개를 합친 수치이며, journal 자식 진입점 1개는 부모가 사용하는 fixture다.
- [#2725](https://github.com/RefinedStone/codex-exec-loop/pull/2725),
  head `fe872bed353da64201c8c0d601937f36ec624c49`: Windows x86_64 / Rust 1.95.0에서 Rust gate
  통과와 journal 필터 `8 passed, 2 ignored`를 기록했다. 두 ignored 항목은 부모 회귀가
  명시적으로 실행하는 worker 진입점이며, 새 부모 회귀 두 개 모두 통과한 기록이다.

**인수 판정:** #2702가 정의한 Windows 신규 생성 journal·복구 연결 범위는 위 근거로 종료
가능하다. 실제 OS 승인·Credential Manager와 원문 창의 제품 통합 검증
([#2704](https://github.com/RefinedStone/codex-exec-loop/issues/2704),
[#2724](https://github.com/RefinedStone/codex-exec-loop/issues/2724))이나 Issue/Vault 전체 장애 행렬
([#2705](https://github.com/RefinedStone/codex-exec-loop/issues/2705))의 완료를 대신하지 않는다.

### 실패·취소 계약

- 전송 전 원문·정책·템플릿 변경은 승인을 무효화하며, 미리보기와 새 승인 없이 생성하지 않는다.
  `Prepared`/`Held`/`Recorded`에서 명시적으로 새 초안을 시작하면 이전 의도를 `Abandoned`로
  기록하고 이전 의도의 전송·URL 확정을 거절한다.
- 패널의 **닫기**는 화면과 원문 메모리의 수명 종료이다. 이미 보낸 원격 요청을 취소하거나
  생성된 이슈를 삭제하는 조작이 아니다. 다시 열면 저장된 의도로 돌아간다.
- `Sent`/`NeedsConfirmation` 이후에는 실패·응답 유실을 미생성으로 간주하지 않는다.
  재조회·정확한 URL 확인만 허용하고 자동 재전송하지 않는다. 별도 새 초안은 중복 위험의
  명시적 확인과 새 승인이 필요하며, 이전 의도와의 관계를 보존한다.
- 생성 확정 뒤 복원 연결에 실패하면 `Confirmed`를 유지하고 연결만 재시도한다.
  지원하지 않는 필수 외부 검증기를 건너뛰지 않는다. 검증기 실행 중 사용자 취소는
  [#2723](https://github.com/RefinedStone/codex-exec-loop/issues/2723)의 별도 계약이며,
  여기서 원격 exactly-once나 이미 전송된 요청의 취소를 보장하지 않는다.

## 현재 한계

- Windows에서는 #2702의 private SQLite journal과 #2688의 Vault 저장소를 사용한다. 예약 DB와 본문 DB를 분리해 Sent를 먼저 커밋하며, 기존 JSONL/lock 또는 지원하지 않는 schema는 자동 이전하지 않고 거절한다. 실제 원격 생성·전원 단절·OS 키 저장소를 포함한 검증은 별도로 필요하다.
- 기존 Desktop 복원 창의 `VaultRestoreSource` 연결은 이 변경의 범위에 포함되지 않는다. 생성 결과의 복원 연결 완료는 Vault 링크 상태이며 원문 열람 창 구현 완료를 뜻하지 않는다.
- 생성 의도 사이에 의도적으로 새 시도를 시작하면 이전 원격 생성과 중복될 수 있다. 이 선택을 자동화하지 않는다.
- 전원 단절, 실제 OS 키 저장소, 실제 GitHub 네트워크 장애는 대역 기반 테스트의 보장 범위를 벗어난다.

기존 이슈 편집이 과거에 linked worktree 경로로 만든 Vault 범위는 이 변경에서 자동 이동하지 않는다. 생성과 현재 이슈 수정 요청은 모두 커널의 대표 checkout 범위를 사용하며, 과거 AEAD 범위의 일괄 변경은 별도의 명시적인 복원·이전 설계가 필요하다.
