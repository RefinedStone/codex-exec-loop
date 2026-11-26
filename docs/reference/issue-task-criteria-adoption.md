# 이슈 승인 완료 기준을 작업으로 연결하기 (#2719)

작업 시작 결정에서 사용자가 검증 채널, 필수 여부, 호스트 검증 방법을 확정하면 최초 작업에 구조화 완료 기준을 저장한다. 외부 이슈의 `## 완료 기준` 목록은 제안 문장으로만 사용한다. 체크 표시나 문장으로부터 채널, 검증 recipe, 통과 여부를 추측하지 않는다.

## 사용자 흐름

1. 현재 공유 명세와 동일한 field hash에 속한 작업 시작 결정을 연다.
2. 제안 문장의 검증 채널을 선택하여 작업 기준에 추가한다. 직접 기준을 추가하거나 문장과 필수 여부를 편집할 수 있다.
3. 명령 채널은 필요한 경우 `git_diff_check_v1` 호스트 검증을 선택할 수 있다. 이는 임의 셸 명령 입력을 허용하지 않는다.
4. 시스템 확인 창에서 실제 기준 전체를 확인하고 시작한다.
5. 생성된 작업은 최초 `acceptance_revision = 1`과 해당 기준을 가진다. 이후의 결과 수집과 필수 기준 검증은 기존 task evidence 경로를 따른다. 기준 채택 자체로 통과 증거를 만들지 않는다.

확인 창을 취소하면 입력 초안을 보존한다. 이슈 field hash, 승인 판, 결정 revision 또는 대상이 바뀌면 이전 초안을 폐기한다.

## 권한과 영속성

`answer_issue_decision.criteria`는 닫힌 neutral DTO 목록이다. native 확인 대상은 정규화된 전체 기준 digest를 포함한다. 기존 확인 capability로 기준을 추가, 제거 또는 변경할 수 없다. 기준이 있는 답변은 정책의 Start gate가 Auto여도 검증된 Human 확인이 필요하다.

답변과 기준은 기존 결정 CAS에서 함께 저장된다. coordinator는 저장된 Human/start 답변에서 승인 참조, decision revision, 원래 basis와 기준을 가져와 불변 WorkOrder checkpoint에 보존한다.

composition은 다음 항목을 모두 대조한 후 비직렬화 admission capability를 만든다.

- 영속된 Human/start 결정과 기준 영수증의 정확한 일치
- 승인 field hash와 이슈 명세의 일치
- 이슈 식별자, workspace, 결정, WorkOrder 식별자의 결합
- 저장된 불변 checkpoint 전체와 제출된 WorkOrder의 일치

일반 System task 생성·편집 경로는 계속 사용자 완료 기준을 받을 수 없다. 승인된 외부 작업을 위한 좁은 접수 경로가 최초 task candidate에만 기준을 채택하며, task의 System 출처를 User로 바꾸지 않는다.

SQLite는 최초 task, queue projection, planning revision, mutation audit, WorkOrder lineage, 승인 영수증 digest를 같은 트랜잭션에 저장한다. 영수증 저장이 실패하면 전체 생성이 롤백된다. `planning_work_order_acceptance_receipts`는 추가 테이블이며 기존 DB의 schema version을 바꾸지 않는다.

같은 영수증의 재접수는 원래 task ID만 반환한다. 사용자가 이후 수정한 기준과 revision은 덮어쓰지 않으며, 작업을 지우거나 authority를 초기화해도 새 작업을 만들지 않는다. 같은 WorkOrder에 다른 영수증을 붙이거나 이미 확정된 기준을 누락한 재접수는 거절한다.

## 호환성과 범위

구조화 기준이 없는 기존 저장 기록과 이전 클라이언트의 Start 답변은 계속 읽고 처리할 수 있다. 이 경로는 자동으로 사용자 기준을 만들지 않으므로 기존 evidence gate의 `criteria_missing` 판정도 그대로 유지된다. 기존 작업을 승인 후 소급 변경하는 기능은 포함하지 않는다.

이 연결은 최초 Human/start 결정에 한정한다. timeout default, arbiter, agent 답변 및 재승인/복구 결정에서 새 기준 소유권을 만들지 않는다. 작업을 시작한 뒤 기준을 바꾸려면 기존 사용자 task 편집 경로와 revision 검사를 사용한다.

## 검증

- native 확인 없이 기준 공급, 확인 후 기준 변경·누락, 이슈 명세 변경 거절
- 다른 scope, WorkOrder, field hash, decision revision, confirmation 참조, handoff 거절
- 실제 private SQLite task 생성 및 System 출처 보존
- 동일 영수증 재접수, 사용자 기준 편집 후 재접수, task 제거 후 원래 식별자 보존
- 동일 WorkOrder의 다른 영수증 충돌, receipt 저장 실패 원자적 롤백, 기존 DB 추가 테이블 호환
- Electron에서 명시 채널 선택, native 확인 취소, spec 변경 초안 폐기, 정확한 confirmed payload
