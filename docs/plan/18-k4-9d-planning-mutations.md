# 18-K4-9d. planning 의 변경을 커널 명령으로 옮긴다

## 문서 상태

- 상태: **제안**. 구현하지 않았다. 현재 제품 동작의 기준은 `docs/reference/` 이다.
- 상위 문서: [K4 명령 스펙](18-k4-command-spec.md) §2.2, §3.7 의 순서 4, §4 의 K4-9d. 같은 방법의 앞 조각은 §8 의
  K4-9a (planning 초기화)이다.
- 이 문서를 따로 둔 이유: K4 설계는 K4-9d 를 한 줄로 적었다. 2026-11-13 의 코드 조사에서 그 한 줄에 새 구조 세 개가
  들어 있다는 것을 알았다: Gateway 의 커널이 저장 명령을 실행하는 길, 명령의 결과를 답에 싣는 방법, 낡은 revision 의
  거절. 그래서 구현 전에 조각과 결정을 적는다.
- 병렬 제어([18-K4-9b](18-k4-9b-parallel-control.md))와 달리 이 조각은 K6 을 기다리지 않는다. 저장 명령은 커널이 떠
  있지 않아도 실행된다.

## 1. 목표

| 번호 | 목표 | 확인하는 방법 |
| --- | --- | --- |
| D1 | admin 의 작업·방향·초안·파일 변경이 커널 명령이다. 기록에 남는다 | 그 변경마다 `command_received` 에 `caller: akra-admin` 이 남는다 (입구 시험) |
| D2 | 고치기와 지우기는 `expected_planning_revision` 이 필수이다. 낡은 값은 이유 코드로 거절한다 | 낡은 revision 의 명령이 아무것도 바꾸지 않고 `planning_revision_stale` 로 거절된다 |
| D3 | 사용자는 낡은 화면에서 낸 변경의 이유를 본다 | admin 이 500 대신 "다시 읽고 다시 하라"를 답한다 |
| D4 | 입구의 adapter 가 변경 서비스를 직접 부르지 않는다 (K4 의 G3) | 입구 경계 시험 |

## 2. 현재 상태 (2026-11-13 코드 조사)

### 2.1 입구의 변경

| 변경 | 입구 | 부르는 곳 | revision | 사용자에게 보이는 것 |
| --- | --- | --- | --- | --- |
| 작업 만들기·고치기 | admin (JSON, HTML) | `PlanningAdminPort::upsert_task` | 고치기만 `expected_planning_revision` 을 받는다 | 알림 글과 관리 화면의 값 |
| 작업 지우기 | admin | `delete_task` | 없다 | 같다 |
| 방향 만들기·고치기·지우기 | admin | `upsert_direction`, `delete_direction` | 없다 | 같다 |
| 초안 만들기·저장·승격 | admin | `create_draft_session`, `save_draft_session`, `promote_draft_session` | 승격만. 초안을 만들 때의 revision 을 admin 프로세스의 메모리에 둔다 | 초안 화면의 값. 승격은 파일 수와 유효한가 |
| 파일 내보내기·적용 | admin | `export_active_files_for_edit`, `apply_exported_files` | 적용만. 내보낼 때 저장한 revision 을 쓴다 | 알림 글과 경로 |
| 작업 만들기·고치기 | CLI `akra planning-tool run` | `PlanningTaskToolPort` | 없다 | JSON 한 줄 (만든 작업 id, 새 revision, 큐의 머리 등) |
| 초안과 큐의 일부 | TUI | core 명령 (`StagePlanningEditor`, `MutatePlanningEditor`, `SubmitQueueMutation` 등) | 큐의 취소는 받는다 | 화면 |

- Telegram 은 작업, 방향, 초안을 바꾸지 않는다.
- admin 의 처리기는 모든 오류를 HTTP 500 으로 답한다. 원인("다시 읽고 다시 하라" 포함)은 서버 로그에만 남는다.
- admin 의 변경은 `PlanningAdminFacadeService` 의 메서드이다. K4-9a 가 쓴 자리(`PlanningServices` 의 use case)를 지나지
  않는다. 그래서 K4-9a 의 방법을 그대로 쓸 수 없다.
- `planning-tool` 은 커널이 띄운 숨은 planning worker 가 skill 로 부른다. 지우기는 없다.

### 2.2 커널

- Gateway 의 커널에는 아홉 변경 중 어느 것도 실행할 core 길이 없다. TUI 의 초안 명령은 다른 식별자(편집기 세션)를 쓴다.
- 명령 전용 조립(`composition/kernel_command_execution.rs`)은 `reset_planning` 하나를 코드에 적었다.
- 낡은 revision 의 이유 코드가 없다.
- `planning` 조회의 답에는 `revision` 이 이미 있다.

### 2.3 결과

명령의 답(`processed`, `rejection`, `operation`)은 실행의 결과를 싣지 않는다. K4-9a 는 결과가 대상마다 정해져 있어서
답 없이 풀었다. 이번에는 실행에 따라 달라지는 결과가 있다.

| 결과가 정해져 있다 | 결과가 실행에 따라 다르다 |
| --- | --- |
| 파일 내보내기·적용 (경로 하나), 작업 지우기의 알림 | 만든 작업의 id 와 "만들었다/고쳤다", 만든 방향의 id, 방향을 지울 때 함께 지운 작업 수, 초안의 이름 (시각에서 만든다), 승격의 파일 수와 유효한가, `planning-tool` 의 답 (만든 id, 새 revision, 큐의 머리) |

관리 화면의 값과 초안 화면의 값은 명령 뒤에 다시 읽으면 된다. 읽기는 입구가 직접 한다.

### 2.4 planning worker

- worker 는 app-server 의 숨은 thread 이고, 그 셸이 `akra planning-tool run .` 을 부른다. 기본 sandbox 는 전체 접근이다.
- app-server 프로세스의 환경 변수는 허용 목록으로 줄인다. `AKRA_HOME` 은 그 목록에 없다. 그래서 `AKRA_HOME` 을 바꾼
  사용자의 worker 는 기본 위치(`~/.akra`)의 소켓과 저장소를 본다. 지금도 있는 어긋남이다.
- Gateway 의 커널이 띄운 worker 가 `planning-tool` 을 커널 명령으로 보내면, 그 명령은 worker 를 기다리는 같은 커널로
  돌아온다. 커널이 그 사이에 명령을 받는지는 확인하지 않았다.

## 3. 설계

### 3.1 저장 명령은 잠금을 쥔 쪽에서 바로 실행한다

저장 명령(`state: store`)은 저장된 planning 데이터만 바꾼다. 도는 커널의 상태가 필요 없다. 그래서 실행기 하나를
만들고, 저장소 잠금을 쥔 쪽이 그 실행기를 부른다.

| 상황 | 실행하는 곳 | 기록 |
| --- | --- | --- |
| Gateway 의 커널이 돈다 | 그 커널의 런타임이 명령을 받고 실행기를 부른다. 끝난 뒤 planning 을 다시 읽는다 (`refresh_planning` 과 같은 일) | 남는다 |
| Gateway 가 없다, 또는 Gateway 가 옛 판이다 | 명령 전용 조립이 잠금을 `command` 로 잡고 실행기를 부른다 | 남는다 |
| TUI 가 커널을 쥐고 있다 | K6 까지 직접 실행한다 (실행기를 잠금 없이 부른다) | 남지 않는다 |

- 실행기는 composition 에 둔다. 입력은 명령의 종류와 입력 값이고, 답은 결과 값이거나 거절 또는 실패이다. 커널의 조립이
  만든 서비스(`planning_services_from_ports`)를 쓴다. 입구의 조립을 쓰면 명령이 자기에게 돌아온다.
- 실행은 동기이다. 커널의 호스트 스레드가 저장소에 쓰는 동안 막힌다. 쓰기는 짧다. admin 은 지금도 다른 프로세스에서
  같은 저장소에 쓴다. 저장소의 비교 후 쓰기가 겹침을 막는다.
- Gateway 커널의 `reset_planning` 은 지금의 core 길(비동기)에 둔다. 이 조각에서 바꾸지 않는다.
- 커널이 명령으로 받는 입력은 지금 core 명령 하나이다. 이것을 "core 명령 또는 저장 명령"으로 넓힌다. 병렬 명령
  (K4-9b)도 뒤에 같은 자리에 들어간다. 넓힐 때 풀 것은 K4-9b 문서의 §3.2 에 있다 (`into_core` 를 부르는 세 곳, 런타임이
  직접 만드는 거절, 봉인된 런타임 API).

### 3.2 명령의 결과는 답에 싣는다

- 명령의 답에 선택 필드 `output` 을 더한다. 저장 명령이 성공하면 그 명령의 결과가 들어간다. 스펙의 명령 항목에
  `result` (결과의 JSON Schema)를 더한다. `runtime` 명령과 `reset_planning` 은 `output` 이 없다.
- 저장 명령은 실행이 끝난 뒤에 답한다. 그래서 답이 올 때 그 연산은 이미 끝났다 (`command_completed` 가 기록되어 있다).
  입구는 `operation` 조회로 기다릴 필요가 없다.
- 결과는 기록에 싣지 않는다. 기록은 판단의 사실이다 (받음, 끝남, 실패의 원인). 그래서 같은 멱등 키로 다시 보낸
  명령의 답에는 `output` 이 없다. 그 클라이언트는 상태를 다시 읽는다.
- 실패의 원인은 K4-9a 와 같다: `command_completed.detail`.

### 3.3 명령

| 명령 | 입력의 요점 | `effect` | `idempotency` | 결과 (`output`) | 바로 거절 |
| --- | --- | --- | --- | --- | --- |
| `upsert_task` | 작업의 필드. 고치기는 `expected_planning_revision` 필수 | write | keyed | 작업 id, 만들었는가, 새 revision | `planning_revision_stale` |
| `delete_task` | 작업 id, `expected_planning_revision`, `confirm` | destructive | natural | 새 revision | `planning_revision_stale` |
| `upsert_direction` | 방향의 필드. 고치기는 `expected_planning_revision` 필수 | write | keyed | 방향 id, 만들었는가, 새 revision | `planning_revision_stale` |
| `delete_direction` | 방향 id, `expected_planning_revision`, `confirm` | destructive | natural | 함께 지운 작업 수, 기본 방향을 남겼는가, 새 revision | `planning_revision_stale` |
| `create_draft` | 초안의 종류 | write | keyed | 초안 이름, 초안을 만든 때의 revision | 없음 |
| `save_draft` | 초안 이름, 파일 내용 | write | natural | 유효한가 | 없음 |
| `promote_draft` | 초안 이름, `expected_planning_revision` | write | keyed | 승격한 파일 수, 유효한가, 새 revision | `planning_revision_stale` |
| `export_planning_files` | 없음 | write | natural | 경로, 내보낸 때의 revision | 없음 |
| `apply_planning_files` | `expected_planning_revision` | write | natural | 경로, 새 revision | `planning_revision_stale` |

- `scope` 는 모두 `planning` 이고 `state` 는 모두 `store` 이다.
- 이유 코드 `planning_revision_stale`: "보낸 revision 이 지금의 revision 과 다르다. 아무것도 바꾸지 않았다." `next` 는
  `planning` 조회, 그 다음 사람에게 묻기이다. 다시 보내도 되는지는 바뀐 내용을 본 사람이 정한다.
- 낡은 revision 은 실패가 아니라 거절이다. 저장 명령은 바로 실행하므로 거절을 답에 실을 수 있다 (`rejection` 과
  `command_rejected` 기록).
- 지우는 명령은 `confirm: true` 가 필요하다 (K4-5 의 규칙). 입구의 확인 화면이 그 값을 붙인다.
- 승격의 revision: admin 은 초안을 만든 때의 revision 을 자기 메모리에 둔다. 명령 전용 조립은 그 메모리를 볼 수 없다.
  그래서 `create_draft` 의 결과가 그 revision 을 돌려주고, 입구가 `promote_draft` 의 입력으로 다시 보낸다. 저장소에 새
  표를 만들지 않는다.
- 승격이 유효성 검사에 막히는 것은 오류가 아니다. 결과의 "유효한가"가 `false` 이다.

### 3.4 입구

- admin: `PlanningAdminPort` 를 감싸는 조립의 구현을 둔다. 읽기는 지금의 facade 로 보내고, 변경은 커널 명령으로 보낸다.
  답의 `output` 과 다시 읽은 값으로 지금의 결과 타입을 만든다. 입구의 코드는 그대로이다.
- admin 의 오류: `planning_revision_stale` 의 거절은 HTTP 409 와 "다시 읽고 다시 하라"로 답한다. 그 밖의 실패는 지금처럼
  500 이지만 원인의 글을 답에 싣는다.
- admin 의 화면: 작업 지우기, 방향 고치기·지우기 폼에 revision 을 더한다. 지금은 작업 고치기 폼에만 있다.
- `planning-tool`: 두 단계이다.
  1. 요청에 `expected_planning_revision` (선택)을 더한다. `list_tasks` 가 이미 revision 을 돌려준다. worker 의 skill 이
     그 값을 `update_task` 에 붙이게 한다.
  2. 커널 명령으로 보내는 일은 §2.4 의 두 가지를 확인한 뒤에 정한다. 확인이 안 되면 `planning-tool` 은 직접 경로에
     남긴다 (K4 설계 §5 의 대응과 같다).
- TUI: 바꾸지 않는다. TUI 의 초안과 큐 명령은 이미 core 명령이다.

### 3.5 하지 않는 것

- 읽기는 옮기지 않는다 (K4 설계 §3.7).
- TUI 의 편집기 세션과 admin 의 초안을 하나로 합치지 않는다.
- Gateway 커널의 `reset_planning` 을 동기 실행으로 바꾸지 않는다.

## 4. 조각 (PR 단위)

| 조각 | 내용 | 동작 | 확인 |
| --- | --- | --- | --- |
| 9d-1 | 저장 명령의 실행기. 커널 입력을 "core 명령 또는 저장 명령"으로 넓힌다. 답의 `output` 과 스펙의 `result`. 이유 코드 `planning_revision_stale`. 첫 명령 둘: `upsert_task`, `delete_task` | 확장 (`akra kernel planning upsert_task …` 가 된다. 입구는 그대로이다) | 실행기 시험 (세 갈래, 낡은 revision, 결과), 프로토콜 시험, 명령 전용 조립 시험, Gateway 커널 시험 (실행 뒤 `planning` 이벤트), 스펙·Desktop 타입·skill 일치 |
| 9d-2 | admin 의 작업 변경이 커널 명령으로 간다. 지우기 폼에 revision. 409 와 원인의 글 | 전환 (지우기에 revision 이 필수가 된다) | 입구 시험 (기록의 호출자, 낡은 폼의 409), admin 시험 |
| 9d-3 | 방향: 명령 둘과 admin | 전환 (고치기·지우기에 revision 이 필수가 된다) | 같다 |
| 9d-4 | 파일 내보내기·적용: 명령 둘과 admin | 유지 | 같다 |
| 9d-5 | 초안: 명령 셋과 admin. 승격의 revision 을 명령의 입력으로 | 유지 | 같다. 명령 전용 조립에서 승격이 되는지 본다 |
| 9d-6 | `planning-tool` 의 선택 revision 과 worker 의 skill | 확장 | skill 파일 시험, CLI 시험 (모델 턴 없음) |
| 9d-7 | 읽기 경로에서 프롬프트 기록 정리를 뺀다. 입구 경계 시험 | 유지 | 구조 시험 |

- 9d-1 이 나머지의 전제이다. 9d-2 ~ 9d-5 는 서로 독립이다. 9d-6 의 둘째 단계(커널 명령으로 보내기)는 조각으로 적지
  않았다. §2.4 의 확인이 먼저이다.

## 5. 사용자가 정할 것

| 번호 | 질문 | 기본값 | 다른 선택 |
| --- | --- | --- | --- |
| Q-9d-1 | admin 에서 작업 지우기와 방향 고치기·지우기에 revision 이 필수가 된다. 다른 창이나 worker 가 planning 을 바꾼 뒤의 낡은 화면에서는 그 변경이 거절되고 "다시 읽고 다시 하라"가 보인다. 받아들이는가 | 받아들인다 (K4 설계 §3.7 이 정했다) | 지우기는 revision 없이 둔다 |
| Q-9d-2 | `planning-tool` 의 `update_task` 에 revision 을 언제 필수로 하는가 | 선택으로 시작한다. worker 의 skill 이 값을 붙이는 것을 본 뒤에 필수로 바꾼다 | 처음부터 필수로 한다 (옛 skill 을 쓰는 worker 의 고치기가 실패한다) |

답이 없으면 기본값으로 간다.

## 6. 위험

| 위험 | 대응 |
| --- | --- |
| 커널의 호스트 스레드가 저장 명령을 실행하는 동안 막힌다 | 저장 명령은 저장소 쓰기 하나이다. 파일 적용과 승격의 시간을 시험에서 본다. 길면 그 둘만 효과로 옮긴다 |
| 실제 조립을 지나는 admin 시험이 명령 전용 조립으로 간다 | K4-9a 와 같다. 시험 빌드는 실제 Gateway 에 붙지 않는다. 그 시험에 기록의 확인을 더한다 |
| 지우기에 revision 이 없는 기존 시험과 폼이 깨진다 | 9d-2, 9d-3 에서 폼과 시험을 함께 고친다 |
| 같은 멱등 키의 재전송은 결과를 받지 못한다 | 문서와 skill 에 적는다. 입구는 키를 다시 쓰지 않는다 |
| Gateway 의 커널이 띄운 worker 의 `planning-tool` 이 같은 커널로 돌아온다 | 9d-6 의 둘째 단계 전에 확인한다. 그 전에는 직접 경로이다 |
| `AKRA_HOME` 이 worker 의 셸에 전달되지 않는다 | 지금도 있는 어긋남이다. 9d-6 의 둘째 단계와 함께 정한다 |
