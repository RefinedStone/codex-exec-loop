# Task 완료 기준 계약

관련: [상위 #2481](https://github.com/RefinedStone/codex-exec-loop/issues/2481) ·
[A01-1 #2651](https://github.com/RefinedStone/codex-exec-loop/issues/2651) ·
[증거 검증의 후속 계획](../plan/24-akra-evidence-harness.md)

## 현재 제공하는 기능

사용자는 기존 Kernel `create_task` / `update_task`와 관리자 JSON 경로에서 task의 완료 기준을
저장하고 수정할 수 있다. 기존 `akra planning-tool`의 `list_tasks`는 전체 task와 같은 snapshot의
`committed_planning_revision`을 반환하므로 기준과 기준 revision을 함께 읽을 수 있다.

이 계약은 기준의 내용과 변경 이력을 저장한다. channel을 지정해도 명령이나 브라우저를 실행하지
않고, 기준을 저장했다는 사실을 pass로 판정하지 않는다. 실제 검증 실행·증거·수동 면제와
observe/enforce 전달 게이트는 후속 범위다. 기준이 없는 기존 task도 계속 실행할 수 있다.

Desktop의 일반 Kernel `planning` 조회는 여전히 큐 요약이다. 이 변경에는 Desktop 기준 편집이나
증거 탐색 화면이 포함되지 않는다. 기존 관리자 HTML 편집은 기준을 생략하여 그대로 보존한다.

## 기준과 호스트 버전

| 필드 | 의미 |
| --- | --- |
| `id` | task 안에서 유일한 비어 있지 않은 식별자 |
| `scenario` | 확인할 동작과 기대 결과를 설명하는 비어 있지 않은 문자열 |
| `channel` | `command`, `data`, `http`, `screen` 중 하나 |
| `essential` | 후속 증거 정책에서 필수 기준인지 구분하는 boolean |
| task의 `acceptance_revision` | 호스트가 관리하는 unsigned 64-bit 기준 버전 |

기준의 네 필드는 모두 필수다. 알 수 없는 필드, channel, 빈 id/scenario와 중복 id는 거절한다.
id와 scenario의 앞뒤 공백을 제거하고 id 순서로 정규화한다. 기준 순서만 바뀐 요청은 내용 변경으로
세지 않는다. 이 검사는 시나리오가 요구사항을 충분히 설명하는지 판정하지 않는다.

`acceptance_revision`은 입력 DTO에 없다. 기준 없는 새 task는 0, 기준 있는 새 task는 1로 시작한다.
기준 내용이 실제로 바뀌거나 있던 기준을 비우면 1 증가한다. 제목·상태 등 다른 필드의 변경,
정규화 뒤 같은 기준, 이미 빈 기준을 다시 비우는 요청에서는 유지한다. 증가가 넘치면 전체 요청을
거절한다. 이전 기준을 삭제한 task는 빈 목록과 0보다 큰 revision을 가질 수 있다.

## 사용자 수정과 worker 경계

| 요청의 `acceptance` | 생성 | 수정 |
| --- | --- | --- |
| 생략 또는 `null` | 빈 기준, revision 0 | 기존 기준과 revision 유지 |
| `[]` | 빈 기준, revision 0 | 기존 목록 비우기; 이미 비었으면 no-op |
| 기준 목록 | 정규화한 기준, revision 1 | 의미가 바뀐 경우에만 revision 증가 |

수정 요청이 기준을 명시하면 `expected_planning_revision`이 필요하다. 저장소의 현재 planning revision과
다르면 쓰지 않고 재조회하도록 거절한다. 이 값은 task의 `acceptance_revision`과 별개다. 첫 값은
동시 수정 방지를 위한 전체 planning snapshot 버전이고, 둘째 값은 해당 task 기준의 변화만 나타낸다.

Kernel/Stored/admin 변환은 생략과 빈 배열의 차이를 보존한다. 기존 활성 작업·lease·dispatcher
보호를 유지하고, 공용 typed mutation에서도 실행 중 task의 기준 변경을 거절한다. 같은 요청에서
상태를 바꿔 이 제한을 피할 수 없다.

등록된 worker task tool과 worker JSON 명령은 기준 수정 및 기준 revision 필드를 받지 않는다.
공용 mutation service도 Worker/System의 명시적 기준 입력을 거절한다. 사용자 생성 미리보기는
커밋 직전에 원 요청으로 계산한 기준과 초기 revision을 다시 비교하므로 공개 preview 객체의
기준과 기준 revision만 바꿔 저장하는 우회를 거절한다.

이 정책은 현재 trusted-local 제품 경로의 호출 구분에 기반한다. Kernel의 진단용 caller 문자열이
사용자 인증을 증명하지는 않으며, 같은 OS 사용자의 shell·파일·DB 접근을 격리하는 기능도 아니다.

## 기존 문서와 DB 호환

- directions의 `PLANNING_FORMAT_VERSION`은 1로 유지한다.
- task 문서는 별도 `TASK_AUTHORITY_FORMAT_VERSION = 2`를 사용한다. v1과 v2를 읽고 미래 버전은
  application 검증과 task 조회에서 거절한다.
- 이전 문서·SQLite `content_json`에 두 필드가 없으면 `acceptance: []`,
  `acceptance_revision: 0`으로 읽는다. 새 bootstrap과 의미 있는 task mutation은 v2로 쓴다.
- 읽기나 의미상 no-op은 기존 문서를 다시 저장하지 않는다. SQLite의 기존 JSON 저장을 사용하므로
  SQL column 또는 별도 데이터베이스 migration은 필요하지 않다.
- Worker의 일반 상태·제목 수정, 자동 proposal 승격, continuation과 audit snapshot은 기준을
  보존한다. 초안 승격은 validation 전에 읽은 authority를 보존하고 같은 revision으로 최종 CAS를
  수행한다. 초안에서 노출하지 않는 기존 task를 빈 목록으로 덮어쓰지 않는다.
- 명시적인 queue/all reset과 task 삭제는 기존 운영자 동작을 유지한다. directions reset은 유지할
  기존 task를 검증하며 미래 문서 버전을 현재 버전으로 바꿔 해석하지 않는다.

## 구현과 검증 위치

- Domain: `src/domain/planning/acceptance.rs`, `TaskDefinition`, semantic validation.
- 쓰기: `PlanningTaskMutationService`, 기존 Stored/admin/Kernel 경로.
- 조회: `PlanningTaskToolService::list_tasks`.
- Kernel 생성 스펙: `schema/akra-kernel-spec.json`; 기존 생성기로 Desktop 타입까지 동기화한다.
- 회귀: domain/mutation acceptance tests, Kernel wire와 실제 SQLite admin 테스트,
  legacy DB round-trip, worker task tool 거절, 초안·directions reset 보존 테스트.

검증은 합성 task와 fake 또는 임시 SQLite 저장소를 사용한다. 실제 모델 turn이나 사용자 저장소의
task 데이터를 시험 입력으로 사용하지 않는다. 완료된 실행 SHA·명령·결과는 구현 PR에 기록한다.
