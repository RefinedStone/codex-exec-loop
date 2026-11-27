# Issue Automation–Kernel 경계 준비 (#2912)

## 상태와 범위

이번 변경은 Automation의 상태 모델에서 Kernel journal을 읽는 역방향 의존을 제거한다. Automation은 여전히 root 패키지 `akra-ade` 안에 있다. 독립 Automation crate, 두 번째 production 실행기, 모델을 호출하는 실제 이슈 처리 흐름은 추가하지 않는다.

[#2569](https://github.com/RefinedStone/codex-exec-loop/issues/2569)의 실제 추출 착수 조건과 [#2555](https://github.com/RefinedStone/codex-exec-loop/issues/2555)의 선행 검증은 그대로 남는다. 2026-11-27 조사 시 두 이슈 모두 open이었다. #2912를 완료해도 두 이슈의 착수 또는 완료로 계산하지 않는다. fake adapter를 사용하는 회귀 시험은 두 번째 production 실행기 지원의 근거가 아니다.

## 조사한 참조와 소유권

| 참조 위치 | 기존 의존 | 변경 후 소유권과 허용 방향 |
| --- | --- | --- |
| `src/domain/issue_automation/timeline.rs` | Automation → `KernelEvent`, `RecordedKernelEvent`, issue journal record 타입 | 순수 `IssueTimelineProjection`, `IssueTimelineEntry`와 조회 상한만 유지한다. Kernel 참조는 없다. |
| `src/domain/issue_automation/timeline/tests.rs` | Automation 시험 → Kernel 이벤트 생성 및 투영 | 원래 6개 시험을 `src/application/service/issue_timeline/tests.rs`로 함께 이동한다. |
| `src/application/service/issue_timeline.rs` | 신규 소유 위치 | root application → Kernel journal + Automation 상태/표시 타입. 조회는 사건을 투영하며 실행이나 원격 재확인을 시작하지 않는다. |
| `src/domain/kernel_event/issue_scope.rs` | 기존 timeline의 이슈 좌표 비교 | Kernel 사건과 issue-core 좌표만 비교한다. application 투영과 SQLite adapter가 이 함수를 함께 사용한다. |
| `src/domain/kernel_event.rs` | Kernel → Automation collection/execution/arbiter | 기존 참조를 유지한다. 이벤트 tag, revision, 직렬화 형식은 바꾸지 않는다. |
| `src/domain/kernel_event/issue_decision.rs` | Kernel → Automation decision/delivery/policy/arbiter | 기존 journal 변환과 record DTO를 Kernel에 유지한다. |
| `src/domain/kernel_event/issue_feedback.rs` | Kernel → Automation feedback, 같은 방향의 시험 import | 기존 확인된 피드백 기록 및 시험을 유지한다. |
| `src/domain/kernel_event/issue_binding_pause.rs` | Kernel → Automation binding_pause | 기존 receipt 변환을 유지한다. |
| `src/composition/native_client_runtime/issue_timeline.rs` | runtime → timeline 투영 | application service에서 투영 함수를 가져온다. 표시 DTO와 상한은 Automation domain에서 가져온다. |
| `src/adapter/outbound/db/sqlite_planning_authority_adapter/kernel_events/issue_timeline.rs` | SQLite journal reader → 이슈 좌표 matcher | Kernel의 동일 matcher를 사용한다. adapter가 application service를 가져오지 않으며, 읽기 쿼리와 저장 계약은 그대로다. |
| `src/adapter/inbound/kernel_protocol/issue_automation/timeline.rs` | inbound protocol → 표시 DTO | 순수 Automation 표시 DTO만 사용하는 기존 방향을 유지한다. |

조사 범위는 `src/domain/issue_automation` 전체의 production 코드와 테스트 파일, inline `cfg(test)` 코드다. 이 범위의 Kernel 역참조는 timeline 구현과 그 시험에 있었다. 새 architecture 규칙은 이 범위를 모두 검사하고, alias·grouped use·상대 경로·`cfg(test)`를 통해 Kernel 또는 root application/composition/adapter/core로 돌아가는 import를 거부한다. 기존 다른 architecture 검사의 production 전용 동작은 유지한다.

허용 방향은 `root application → Kernel journal → Automation 상태 타입`과 `root application → Automation 표시 DTO`다. Automation에서 application 투영 함수를 재export하는 우회도 허용하지 않는다. 공용 crate로 Kernel/planning 전체를 옮기거나 root를 dev-dependency로 가져오는 방식은 사용하지 않았다.

## 실행 포트와 composition

실제 production 구현은 기존 `src/composition/issue_work_execution.rs`의 `AkraIssueWorkExecutionAdapter`이며, `src/application/port/outbound/issue_work_execution_port.rs`의 `IssueWorkExecutionPort`를 그대로 구현한다. 포트의 요청·영수증·Unknown 결과와 stable recovery command ID, adapter 조립에는 변경이 없다.

`IssueCoordinatorService`의 `FakeExecution`/`AnswerExecution`, kernel simulation의 `Execution`/`NoExecution`, restart fixture의 실행기는 모두 테스트 구현이다. 이번 변경으로 production 구현 수가 늘어나지 않는다.

## 시험 및 저장 형식 보존

timeline의 원래 6개 시험은 함수 이름과 본문을 유지한 채 소유 모듈만 바꾼다. 모든 좌표 비교, 저장/최근 기록 병합, 20개 상한과 older 표기, sequence 충돌의 Unavailable 처리, provider 메타데이터 비노출, 실행/검증 횟수 분리 계약을 유지한다.

`tests/fixtures/issue_automation_timeline_journal_v1.json`은 기존 tracked/permit-revoked/execution-progressed journal을 고정한다. 추가 시험은 다음을 확인한다.

- 저장 JSON의 event tag·revision·state·attempts·terminal identity가 역직렬화/재직렬화 뒤 동일하다.
- 저장된 기록만 복구한 결과와 일부 저장 기록에 최근 중복 기록을 합친 결과가 같다.
- 순서와 기존 한국어 표시 문구가 유지된다.

아래 기존 fixture는 책임 위치와 본문을 유지한다. 순수 투영 시험을 전체 실행 복구의 증거로 대신하지 않는다.

| 계약 | 유지하는 시험/위치 |
| --- | --- |
| Kernel journal 직렬화 | `domain::kernel_event::tests::execution_records_preserve_durable_identity_without_question_or_worker_payloads` 및 기존 event JSON roundtrip 시험 |
| 예전 실행 상태 JSON 호환 | `domain::issue_automation::execution::tests::legacy_execution_json_without_review_link_remains_readable` |
| 재시작·결과 불명·중복 전송 금지 | `application::service::issue_coordinator::tests::execution_questions::a_lost_answer_ack_is_polled_after_restart_without_a_second_send` |
| 잘못된 질문/세대의 영수증 거부 | `application::service::issue_coordinator::tests::execution_questions::a_receipt_for_the_wrong_question_or_generation_cannot_settle_the_claim` |
| 취소 이후 claim/전송 금지 | `application::service::issue_coordinator::tests::execution_questions::disabled_execution_and_cancelled_tracking_never_claim_or_send_a_reply` |
| 이전 사람 응답을 사용한 제어 우회 금지 | `application::service::issue_coordinator::tests::execution_questions::blocking_work_controls_cannot_use_an_old_human_reply` |
| runtime 읽기 전용/부분 이력 거부 | `composition::native_client_runtime::issue_timeline::tests::`의 기존 2개 시험 |
| SQLite bounded read와 좌표 검사 | 기존 `sqlite_planning_authority_adapter::kernel_events::issue_timeline::tests::` |

## 선택 실행과 시험 목록

`scripts/ci-crate-ownership.json`에 같은 root 패키지 안에서 이동한 6개 시험을 등록한다.

- 이전 prefix: `domain::issue_automation::timeline::tests::`
- 현재 prefix: `application::service::issue_timeline::tests::`
- package/target: `akra-ade` / `akra_ade` lib
- Linux/Windows/macOS, default/all features, 기존 ignored 상태를 유지한다.
- 새 고정 journal fixture 시험은 1개다. 삭제하거나 새로 ignored로 바꾼 시험은 없다.

root 내부 이동도 이전 이름과 현재 이름으로 조회할 수 있다. 위 timeline의 정확한 이전 prefix는 현재 application prefix에 대응하고, 실제 inventory에서 동일한 시험만 선택한다. 명시적인 package/target 선택은 현재 이름을 그대로 사용한다.

더 넓은 이전 필터가 여러 namespace로 나뉘어 기존 단일 libtest 필터로 같은 집합을 재현할 수 없으면 기존처럼 실행 전에 실패한다. 시험을 누락하거나 여러 프로세스로 나누어 통과시키지 않는다. OR positional filter 지원은 실제 Rust fixture로 한 프로세스/같은 시험 집합/ignored 보존을 확인하기 전까지 활성화하지 않았다. 현재 `check_native_pr.ps1`의 native 필터 목록에는 timeline의 이전 prefix나 `domain::issue_automation::` 전체를 선택하는 필터가 없다.

## 검증 기록

검토용 source 기준은 `6e7b44a50bbb6b829d79634efaddae3b8ad0ae07`이다. Windows tunnel 연결 중단 뒤 같은 GitHub source에서 변경을 준비했다. 아래 결과는 이 준비본의 결과이며, Windows worktree에 적용하거나 Rust 바이너리를 검증한 결과로 표시하지 않는다.

- `node --test scripts/ci-contract-targets.test.mjs`: 13개 통과, 실패/취소/skip 0개. 같은 root 패키지의 정확한 이전 이름 매핑, 명시적 선택, 중복 실행 거부와 재현 불가능한 넓은 필터의 실행 전 실패를 포함한다.
- architecture 새 경계/회귀 규칙: 아직 Rust로 컴파일하거나 실행하지 않음.
- timeline 6개 이동 시험 및 신규 저장 JSON 시험 1개: 아직 Rust로 컴파일하거나 실행하지 않음.
- 위 표의 기존 실행/재시작/취소/unknown 계약: 본문을 유지했으며, 최종 통합 root lib 검증에 합산 예정.
- Cargo workspace registry 검사, `cargo fmt --all`, agent planner: Windows 연결 복구 또는 CI에서 실행 필요.
- 실제 libtest OR fixture: 실행하지 않았고 OR 실행 경로도 활성화하지 않음.
- 실제 모델 턴: 실행하지 않음
- Automation 독립 crate 빌드/두 번째 production 실행기: 이번 범위에 없음
