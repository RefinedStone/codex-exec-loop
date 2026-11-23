# 바인딩별 이슈 자동화 Pause / Resume

관련: [#2560](https://github.com/RefinedStone/codex-exec-loop/issues/2560),
[이슈 Auto 전달](issue-auto-delivery.md), [전달 증거 게이트](task-delivery-evidence.md),
[호스트 검증](task-verification.md).

## 적용 범위

Pause는 한 저장소에 등록된 특정 이슈 바인딩의 새 자동 행동을 닫는다. 수집 설정이나 선택한
자율 수준을 바꾸지 않으며, 다른 바인딩과 일반 planning 작업은 기존 정책대로 진행한다.
작업별 Hold·Cancel·Takeover와 바인딩 Pause는 별도 상태다. Resume가 기존 작업별 사람 개입을
해제하거나 전역 병렬 pool을 켜지는 않는다.

`set_issue_binding_paused`는 실행 중인 저장소 커널이 처리하는 Runtime 명령이다. 공유 실행
경계가 없는 저장소 전용 명령 실행으로 대신 완료할 수 없다. 입력은 `command_id`,
`binding_id`, `paused`, `expected_revision`이며, 현재 바인딩 실행 revision과 일치해야 한다.
Pause에는 별도 사람 확인이 필요하지 않지만, 정지한 바인딩을 Resume하려면 아래의 Native
확인이 필요하다.

## 수락과 완료

명령 응답 `issue_binding_pause_requested`는 수락한 요청을 나타낸다. `receipt`가 없으면
정지 또는 재개가 아직 완료되지 않았다. 일반 `CommandCompleted`도 이 요청 처리의 종료를
뜻하므로, 별도 Pause 완료 영수증으로 해석하면 안 된다.

| 상태 또는 결과 | 의미 |
| --- | --- |
| `pending.paused=true` | Pause를 영속 수락하고 새 실행 진입을 닫았다. 허가 회수와 이미 진입한 요청의 정리가 남아 있다. |
| `pending.paused=false` | Resume를 영속 수락했다. 현재 정책·권한과 진행 중인 요청을 확인하는 동안 계속 닫혀 있다. |
| `paused` 영수증 | 해당 바인딩의 작업 권한을 회수하고 진행 중인 실행 진입이 모두 빠져나간 뒤 정지를 완료했다. |
| `resumed` 영수증 | 현재 확인 근거와 권한 검사를 마치고 새 실행 세대를 열었다. 이전 허가를 복원하지 않는다. |
| `resume_superseded` 영수증 | 재개를 확인하는 동안 근거가 바뀌어 정지를 유지했다. 새 현재 상태에서 다시 확인해야 한다. |
| `unchanged` 영수증 | 이미 같은 영속 상태여서 변경하지 않았다. 새 재개 권한이나 진행 중 요청의 정리 완료를 증명하지 않는다. |

실제 상태 변경을 수락할 때 revision을 한 번 올리며, 완료할 때 다시 올리지 않는다.
`paused`와 `resumed`만 `changed=true`이고, `resume_superseded`와 `unchanged`는
`changed=false`다. 같은 `command_id`의 동일 요청은 저장된 최초 명령과 영수증을 돌려준다.
다른 바인딩·목표 상태·기대 revision으로 같은 ID를 재사용하면 거절한다. 과거 요청의 응답을
현재 상태로 덮어쓰지 않으므로, 최신 상태는 이슈 자동화 조회의 `binding_pause`로 확인한다.

Desktop은 접수, 처리 중, 일시정지됨, 상태 확인 필요를 구분한다. 처리 중에는 새 전환 버튼을
비활성화하며, 응답을 확인하지 못한 명령을 임의로 자동 재전송하지 않는다. 완료 이벤트를
받으면 현재 상태를 다시 읽고, 오래된 조회가 더 최신 revision이나 같은 revision의 완료
상태를 되돌리지 않도록 한다.

## Resume의 Native 확인

정지한 바인딩의 재개 확인은 다음 전체 대상에 결속된다.

- canonical workspace, `command_id`, `binding_id`, 기대하는 실행 revision
- 바인딩 메타데이터 revision과 현재 자율 수준 선택 revision
- 현재 유효 정책 버전과 유효 자율 수준

일반 명령 payload의 `paused=false`나 호출자 이름으로 이 확인을 대신하지 않는다.
기존 Native 확인 경로에서 발급·검증한 정확한 대상만 받으며, 재개 요청 시 현재 대상과
비교한다. Pause는 정책 파일을 읽을 수 없어도 요청할 수 있지만, Resume는 현재 정책을
확인할 수 있어야 한다.

수락 이후에도 실행 진입을 닫아 둔 채 현재 정책을 다시 읽는다. 완료 transaction은 저장된
바인딩·자율 수준 revision과 소유한 작업 fence를 다시 확인한다. 어느 근거가 달라지거나
확인할 수 없으면 `resume_superseded`로 끝내고 정지를 유지한다. 새로 열린 세대에서도
기존 정책, 사람 필수 관문, 작업 control, 예산, 전달 permit과 증거 게이트를 다시 충족해야 한다.

## 실제 실행과 외부 쓰기 경계

같은 저장소 owner가 collection·coordinator·worker·distributor·feedback·호스트 검증에
하나의 `IssueBindingAdmission`을 주입한다. Pause의 영속 수락과 메모리 진입 차단은 같은
짧은 admission mutex 안에서 이뤄진다. 외부 요청을 기다리는 동안 그 mutex를 잡지 않는다.

작업은 바인딩 실행 generation과 planning `authorization_epoch`를 준비 시점에 보관한다.
이슈 작업의 계보는 실제 WorkOrder·Issue·등록된 binding으로 확인하며, URL이나 task ID 모양으로
추측하지 않는다. 이슈 출처인데 계보가 없거나 일치하지 않으면 실행을 허용하지 않는다.
외부 효과 직전에는 기존 source·lease·permit·claim 검사를 유지하고, 메모리 guard도 확인한다.

| 실행 경계 | 보관하는 진입 권한 |
| --- | --- |
| worker 새 thread / resume의 실제 `turn/start` | 준비를 마친 뒤 RPC 직전에 원래 세대의 guard를 얻고, 시작 응답 또는 Unknown·오류 처리가 끝날 때까지 유지한다. |
| arbiter의 지연 실행 | dispatch를 예약한 시점의 세대를 실제 isolated turn 시작 안쪽에서 확인한다. 예약만으로 뒤늦은 모델 호출을 허용하지 않는다. |
| source push, PR 생성, integration push | 기존 전달 경로에서 같은 guard를 보유하고 각 실제 쓰기 직전에 다시 확인한다. |
| PR 닫기, remote branch 삭제 | 통합이 끝났더라도 새 외부 쓰기이므로 원래 세대가 여전히 유효해야 한다. |
| 이슈 feedback | 영속 Sent claim부터 외부 응답·Unknown과 결과 정산까지 guard를 유지한다. |
| 호스트 검증과 공식 완료 후속 처리 | 예약된 검증은 영속 terminal receipt 정산까지 추적한다. 공식 refresh와 thread archive도 원래 실행 capture를 확인한다. |

Pause 완료는 해당 바인딩의 in-flight guard가 0이고 작업별 권한 회수가 끝났을 때만
기록한다. 같은 바인딩을 Resume해도 이전 callback은 새 generation으로 바꿔 끼울 수 없다.
Native 일반 turn의 자동 후속 처리도 launch 시점의 binding·work·epoch capture를 전달받는다.
수동 Human turn 자체는 이 capture 실패로 막지 않으며, capture가 없거나 오래된 이슈 작업의
자동 후속 처리만 닫는다.

## 보존하는 사실과 실행 중인 turn

이미 시작 응답을 받은 worker turn을 강제로 종료했다는 의미는 아니다. Pause는 새 자동
행동과 오래된 세대의 후속 권한을 닫는다. 이미 받은 실행 결과, 영속 검증 영수증, 통합 사실,
Human 답변과 작업별 사용자 대기는 계속 보존한다. 늦은 arbiter 결과가 새 Start 권한을
만들거나 오래된 완료 callback이 새 refresh·전달을 시작할 수는 없다.

이미 원격으로 보낸 요청의 Unknown은 원격 효과가 없었다는 증거가 아니다. Pause는 외부
효과를 되돌리거나 원격 작업이 모두 종료됐다고 보장하지 않는다. 저장된 사실과 읽기
reconcile로 결과를 확인하며, 새 쓰기로 자동 재시도하지 않는다. paused queue head는
기존 행을 보존한 채 건너뛰어 다른 바인딩이나 일반 작업의 전달을 막지 않는다.

TUI도 저장소 owner가 복구한 동일 admission을 자신의 실행 graph와 공유한다. 저장소 잠금과
기존 단일 owner 규칙을 우회하는 별도 허가는 만들지 않는다. 복구나 조립이 실패하면 이슈
자동화 진입을 닫아 두며, 전역 planning 또는 모든 바인딩을 Pause한 것으로 표시하지 않는다.

## 재시작과 완료 이벤트

이슈 저장소 schema v14는 바인딩별 상태, 명령, 소유한 작업 fence와 완료 outbox를 저장한다.
기존 등록 바인딩을 이전할 때 초기 상태를 명시하며, 필요한 상태가 없거나 일부 schema만
남았다고 임의의 실행 가능 상태로 복구하지 않는다.

저장소 owner는 자동화 graph가 동작하기 전에 영속 paused/pending 상태를 admission에
복구한다. pending 명령은 작은 페이지 단위로 작업의 authorization epoch를 갱신하고 남은
permit을 회수한 뒤 정리를 계속한다. 이미 끝난 fence는 동일한 소유 명령으로 재사용하며,
다른 사람이 만든 Hold·Cancel·Takeover 상태를 해제하지 않는다.

완료 transaction은 영수증과 outbox를 함께 저장한다. Native는
`issue_binding_pause_completed`를 kernel event log에 영속 기록한 뒤 정확한 영수증을 ACK한다.
응답·append·ACK를 잃으면 outbox에서 다시 읽으며, 같은 event ID의 내용까지 비교하여 중복
기록을 방지한다. 완료 저장 응답만 잃은 경우에도 동일한 pending 명령의 최초 완료 영수증으로만
메모리 상태를 복구한다. 오래된 open 조회나 `unchanged` 응답으로 닫힌 진입을 다시 열지 않는다.

커널은 원격 응답을 동기 대기하지 않고 poll과 waker로 복구를 진행한다. pending 명령,
outbox와 진행 중인 진입은 런타임 수명 판단에 포함된다. 런타임 종료는 공유 admission을 닫는다.
Gateway를 다시 켰을 때 client 없이 저장소 커널을 시작하고 이어 가는 기능은
[#2561](https://github.com/RefinedStone/codex-exec-loop/issues/2561) 범위이며 이 계약에 포함하지 않는다.

## 구현 근거와 검증 범위

- [상태와 영수증 타입](../../src/domain/issue_automation/binding_pause.rs),
  [Runtime 명령 분류](../../src/domain/kernel_spec.rs)
- [Pause 서비스](../../src/application/service/issue_binding_pause.rs),
  [복구와 권한 회수](../../src/application/service/issue_binding_pause/recovery.rs),
  [공유 admission](../../src/application/service/issue_binding_admission.rs)
- [영속 명령과 완료](../../src/adapter/outbound/db/sqlite_issue_automation_store/binding_pause/commands.rs),
  [Native outbox](../../src/composition/native_client_runtime/binding_pause.rs)
- [Native 시작 capture](../../src/composition/core_turn_submission.rs),
  [작업 계보와 generation](../../src/application/service/parallel_mode/binding_action.rs)

회귀 소스는 SQLite 재오픈·재전송·정책 변경, fake-process worker/arbiter RPC,
feedback ACK/Unknown, 실제 임시 Git 원격의 전달·cleanup 경계, 바인딩 격리와 Native 후속
처리를 다룬다. 실제 모델 턴은 이 검증에 필요하지 않다. 이 문서는 시험 통과나 플랫폼별
검증 완료의 기록이 아니며, 실제 실행 결과는 구현 PR의 검증 기록을 따른다.
