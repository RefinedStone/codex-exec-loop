# 작업 지시의 1회 등록과 task 계보

관련 이슈: #2546 (IA3-3). 후속 #2547의 허가 집행과 #2548의 이슈 자동화 호출은 이 문서의 구현 범위에 포함하지 않는다.

## 보장하는 계약

`PlanningWorkOrderService`는 이슈 자동화 타입을 모르는 중립 planning 서비스다. 기존 task 생성 정책, 방향 검증, 큐 투영을 재사용하며, `PlanningWorkOrderRepositoryPort`를 통해 task와 작업 지시를 같은 planning authority에 저장한다. 포트는 `PlanningTaskRepositoryPort`를 확장한다. 서비스가 받는 저장소 Arc 하나를 task 서비스와 공유하므로 서로 다른 저장소에 task와 계보를 나누어 쓸 수 없다.

기록은 `{ work_order_id, origin, task_ids, permits }`다. `work_order_id`와 `origin`은 호출자가 정한 불변 문자열이고, 첫 번째 task ID가 최초 task다. 이후 ID는 보완 task의 연결 순서다. 등록 입력에는 permits가 없으며 최초 기록은 빈 배열로 시작한다. 이번 구현은 허가를 부여하거나 해석하지 않는다.

작업 지시 ID와 origin은 입력 바이트를 보존한다. 빈 값, 제어문자, 길이 한도 초과를 거절한다. task ID는 실제 planning key와 일치하도록 주변 공백도 거절한다. 이슈 번호·필드 해시·시도 횟수로 ID를 만드는 규칙은 호출자의 책임이다.

## 등록과 재전송

서비스는 task 입력 검증이나 authority 로드보다 먼저 작업 지시를 조회한다. SQLite는 별도로 같은 검사를 쓰기 transaction 안에서 다시 수행한다.

| 상황 | 결과 |
|---|---|
| 같은 작업 지시 ID와 origin의 재등록 | 원래 기록과 최초 task ID, `created: false` 반환 |
| 재등록 입력의 제목·본문 등이 달라짐 | 기존 task를 덮어쓰지 않고 최초 결과 반환 |
| 같은 ID와 다른 origin | 명시적 `OriginConflict`, 기존 기록 보존 |
| task provenance가 편집됨 | 영구 작업 지시 기록으로 최초 task 조회 |
| task 삭제 또는 authority 초기화 뒤 재등록 | 원래 task ID 반환, task를 복구하거나 새로 만들지 않음 |
| kernel event·명령 영수증 정리 | 작업 지시의 중복 판정에 영향 없음 |
| planning revision 경합 | 최신 authority를 다시 읽어 제한된 횟수로 재시도 |
| 새 task ID가 현재 task 또는 다른 영구 계보와 충돌 | 기존 task ID suffix 정책으로 다른 ID 생성 |

CAS와 ID 충돌 재시도 동안 task 생성 시각을 고정한다. task를 삭제한 후 반환되는 ID는 과거의 정체성이다. 현재 실행 가능한 task가 존재한다는 뜻이 아니며, 재등록은 operator의 삭제·reset을 취소하지 않는다.

## SQLite transaction

신규 등록은 `IMMEDIATE` transaction 안에서 다음 순서로 진행한다.

1. 작업 지시를 다시 조회한다. 기존 기록이면 origin을 검사하고 반환한다.
2. 신규 등록에만 기존 authority mutation guard와 planning revision 조건을 적용한다.
3. 기존 task를 바꾸지 않고 최초 task 하나를 추가하는 candidate인지 검사한다.
4. 기존 저장 helper로 task authority·queue·planning revision·task mutation audit를 갱신한다.
5. 작업 지시 기록과 최초 task 연결을 저장한 뒤 commit한다.

후반의 기록 쓰기가 실패하면 task, queue, revision, audit도 함께 rollback된다. 이미 등록된 작업 지시는 stale candidate나 현재 guard 때문에 새 task로 바뀌지 않는다. 별도의 명령 영수증 보존 기간에 의존하지 않는다.

기존 task 저장 함수는 transaction 내부 helper를 공유하지만 guard, revision 비교, no-op, audit의 순서를 유지한다. 작업 지시가 없는 기존 task에는 새로운 실행 조건을 적용하지 않는다.

## 보완 task와 양방향 조회

`load_work_order`는 작업 지시 ID로, `load_work_order_for_task`는 최초·보완 task ID로 같은 전체 기록을 반환한다. 기록과 연결 목록은 한 read transaction에서 읽고, 순번 0부터 이어지는 계보를 검사한다.

`link_supplemental_task`는 현재 존재하는 task를 기존 작업 지시에 연결한다. 동일 연결 재시도는 no-op이며, 그 task가 이후 삭제됐어도 원래 연결을 반환한다. 다른 작업 지시에 이미 속한 task는 `TaskAlreadyLinked`로 거절한다. 새 연결은 task 존재 확인·중복 소속 검사·계보 순번 추가·planning revision 증가를 같은 `IMMEDIATE` transaction에 묶는다.

작업 지시 ID, task ID 소속, 작업 지시 내 순번은 DB 제약으로 보호한다. `planning_tasks`에 대한 cascade foreign key를 두지 않으므로 task 삭제와 기존 authority 초기화가 계보를 지우지 않는다. 이 구현에는 작업 지시 기록을 정리하는 API가 없다.

## 기존 DB와 이전 바이너리

planning schema version은 **19 그대로**다. 이미 v19인 DB에서는 기존 schema 초기화가 일찍 반환하므로, 작업 지시 진입점에서 부가 표를 별도로 보장한다. 기존 표·열·schema marker의 의미를 바꾸지 않는다.

호환성 검증은 새 부가 표와 실제 등록 결과가 있는 DB를 준비한 뒤, #2546 구현 전에 보존한 이전 `akra` 바이너리로 `kernel query planning --workspace <fixture> --json`을 실행한다. 합성 fixture와 격리된 `AKRA_HOME`만 사용하고, 조회 전후 작업 지시·계보·planning revision·schema marker를 대조한다. version 숫자 assertion만으로 이전 바이너리 실행을 대신하지 않는다.

## 회귀 검증

계약·서비스 시험은 식별자 검증, 입력이 달라진 재전송, provenance 변경, authority 삭제, CAS 재시도, 고정 생성 시각, task ID 충돌, 기존 planning validation을 다룬다.

실제 SQLite 시험은 서로 다른 연결의 동시 등록, task/queue 저장 뒤 SQL trigger로 주입한 기록 실패의 rollback, kernel event 정리, 재시작, 보완 계보의 양방향 조회와 이중 소속 거절, 기존 v19 DB의 부가 표 생성과 기존 task 저장 동작을 확인한다.

최종 실행 명령과 결과는 이 변경의 PR 검증 항목에 기록한다. 모델 턴, 원격 push, PR 생성, 통합을 실행하는 기능은 이 서비스에 추가하지 않는다.
