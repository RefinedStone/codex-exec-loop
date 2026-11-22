# Legacy work-order publication reference 재접수 검증

- 기록일: 2026-11-22 UTC
- 기준: `prerelease`의 `bd85e3062e2bfeb74f901b61ff1272fd842c14b2`와 이 변경의 코드
- 원 지적: [PR #2643의 P2](https://github.com/RefinedStone/codex-exec-loop/pull/2643#issuecomment-7082302211)
- 범위: 기존 planning work order의 빠진 publication reference 복구. 새 작업 생성, 실행 승인, close 정책 변경은 포함하지 않는다.

## 실패 원인과 재현

예전 issue checkpoint는 URL을 `order.link_reference`에 보존했지만 planning work order에는
publication reference를 저장하지 않았다. 이후 도입된 `planning_work_order_links`에는 해당
기존 작업의 행이 없다. 최신 실행 adapter는 같은 checkpoint를 재접수할 때 URL을 전달하므로,
서비스의 빠른 existing admission과 SQLite registration의 existing 분기 모두
`None != Some(reference)`를 변경 요청으로 판단했다. 그 결과 새 작업을 만들 필요가 없는
재접수, 특히 frozen source에 대한 전달 허가 갱신 전 `submit`도 실패했다.

실제 SQLite fixture에 원본 작업과 보완 작업의 계보, dispatch/push permit, 더 최신 permit basis,
실행 한도 3과 이미 소비한 시도 1을 저장했다. 이어 예전 버전처럼 links 테이블을 제거했다.
기존 permit은 새 basis에 대해 의도적으로 stale 상태다.

생산 코드를 수정하기 전에 다음 두 회귀를 실행했으며 모두
`work order publication reference changed`로 실패했다. 컴파일 오류가 아닌 실제 실행 실패다.

| 원본 코드의 회귀 | 결과 |
| --- | --- |
| `legacy_publication_reference_recovers_atomic_registration_without_authority_changes` | 실패 |
| `legacy_publication_reference_recovers_fast_admission_before_replacement_validation` | 실패 |

명령: `cargo test --locked --lib legacy_publication_reference -- --nocapture`

원본 결과: 0 passed, 2 failed, exit 101.

## 복구 계약

`PlanningWorkOrderRepositoryPort::replay_work_order`를 필수 저장 경계로 추가했다.
기본 구현에서 복구를 했다고 가정하지 않는다. SQLite 구현과 테스트용 메모리 구현이 각각
자신이 소유한 저장소를 갱신한다.

서비스의 빠른 재접수와 SQLite registration의 existing 분기는 같은 SQLite helper를 사용한다.
`BEGIN IMMEDIATE` 안에서 현재 work-order identity, origin, 연속 task ordinal,
중복 없는 task 계보와 permit의 작업 신원·구조를 먼저 검증한다. 현재 basis에 대한 permit의
효력은 재접수 조건이 아니다. 기존 허가 갱신을 위해 필요한 재접수를 차단하지 않는다.

| 저장된 reference | 재접수 reference | 결과 |
| --- | --- | --- |
| `None` | `None` | 기존 작업 반환 |
| `None` | 유효한 `Some(close_on_merge=false)` | 링크 행을 한 번 삽입하고 기존 작업 반환 |
| `None` | `Some(close_on_merge=true)` | 거절 |
| `Some` | 완전히 같은 `Some` | 기존 작업 반환; 기존 `true`도 동일 요청이면 허용 |
| `Some` | `None`, 다른 URL, 다른 close 값 | 거절 |

복구는 링크 행 하나만 쓴다. 기존 task, queue, audit, planning revision, permit, permit basis,
control, 실행 한도와 소비 내역을 수정하지 않는다. 두 연결이 같은 URL을 보충하면 같은 작업을
반환하며, 서로 다른 URL로 경쟁하면 처음 저장된 URL만 남고 다른 요청은 거절된다.
INSERT 실패는 성공 admission으로 바꾸지 않고 transaction을 롤백한다.

## 실행한 회귀

새 회귀는 `work_orders/tests/publication_reference.rs`와 하위 `submission.rs`에 있다.

| 검증 | 결과 |
| --- | --- |
| DB registration의 existing 분기에서 빠진 링크 복구 | 통과 |
| 서비스 fast admission이 무효 replacement input 검증 전에 기존 작업 복구 | 통과 |
| origin 변경, invalid URL, 닫기 권한 추가, 기존 reference 변경·삭제 거절 | 통과 |
| 이미 저장된 closing=true의 정확한 replay 보존 | 통과 |
| 손상된 task ordinal 또는 다른 작업의 permit이면 삽입 전에 거절 | 통과 |
| 독립 연결의 같은 URL·충돌 URL 경쟁과 한 번만 저장 | 통과 |
| 실제 SQLite INSERT 실패 롤백 후 정상 재시도 | 통과 |
| 실제 composition `submit` 재접수 후 별도 delivery permit 갱신 | 통과 |

결과: **8 passed, 0 failed**, exit 0.

회귀는 links를 제외한 기존 SQLite authority의 모든 일반 테이블 행을 전후 비교한다.
따라서 반환된 task ID뿐 아니라 task 중복 없음, planning revision/audit/queue,
permit/basis/control, 실행 예산의 한도와 소비 내역이 그대로임을 확인한다.
새 adapter/service 인스턴스로 같은 SQLite를 다시 열어도 동일 결과가 유지된다.

실제 `AkraIssueWorkExecutionAdapter::submit` 회귀는 예전 JSON의 `providerLink` 누락이
`false`로 읽히는 경계도 통과한다. 복구 직후 기존 Push permit은 계속 stale이고,
합성 승인 basis를 사용하는 기존 `grant`를 별도로 호출한 뒤에만 frozen source의
Push/PullRequest/Integrate 허가가 갱신된다. Integrate의 human-review 조건은 유지된다.
새 execution adapter로 재접수해도 추가 저장 변경이 없다.

## 로컬 gate

환경: Windows MSVC, 저장소 pin Rust 1.95.0, `RUST_TEST_THREADS=1`,
`CARGO_BUILD_JOBS=4`. `TEMP`/`TMP`는 저장소 밖의 전용 임시 디렉터리다.
기존 target cache를 재사용했으며 삭제하지 않았다.

| 명령 | 결과 |
| --- | --- |
| `cargo fmt --all` | 통과 |
| `git diff --check` | 통과 |
| `node scripts/agent-plan.mjs` | Rust gate 선택 |
| `cargo test --locked --lib application::service::planning::task_mutation::work_order::tests::` | 10 passed |
| `cargo test --locked --lib adapter::outbound::db::sqlite_planning_authority_adapter::work_orders::tests::` | 73 passed (새 회귀 8개 포함) |
| `powershell -File scripts/check_native_pr.ps1 -Mode Rust` | 통과, exit 0 |

Rust gate의 세부 결과: CI 정책 30/30, TUI layering, rustfmt, Windows all-features lib/bins check,
Windows 계약 47/47, `cargo clippy --locked --all-targets --all-features -- -D warnings`가 모두 통과했다.

## 검증의 경계

이 문서의 성공은 실제 SQLite와 실제 application/composition 등록·허가 경로에 대한 회귀 결과다.
확인 basis와 공유 handoff는 합성 fixture다. 사용자 원문, 실제 native 확인 응답, 실제 모델 turn,
provider 쓰기, 시험 PR 생성·merge, 이슈 닫기를 수행하지 않았다.

재시작 검증은 adapter/service 재생성과 SQLite 재개방이다. OS 프로세스 강제 종료, 전원 장애,
전체 coordinator의 실제 모델 흐름을 검증했다는 의미가 아니다.
[#2555](https://github.com/RefinedStone/codex-exec-loop/issues/2555)와
[#2644](https://github.com/RefinedStone/codex-exec-loop/issues/2644)의 실제 모델 운영 검증은
이 변경으로 완료 처리하지 않는다. 전체 POSIX Rust suite와 최종 병합 가능 여부는 CI Gate가
별도로 확인한다.
