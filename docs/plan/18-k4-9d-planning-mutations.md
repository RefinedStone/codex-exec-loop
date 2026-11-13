# 18-K4-9d. planning 의 변경을 커널 명령으로 옮긴다

## 문서 상태

- 상태: **구현 중**. 9d-1 을 구현했다 (2026-11-13, §8). 나머지 조각은 제안이다. 현재 제품 동작의 기준은 `docs/reference/` 이다.
- 상위 문서: [K4 명령 스펙](18-k4-command-spec.md) §2.2, §3.7 의 순서 4, §4 의 K4-9d. 같은 방법의 앞 조각은 §8 의
  K4-9a (planning 초기화)이다.
- 이 문서를 따로 둔 이유: K4 설계는 K4-9d 를 한 줄로 적었다. 2026-11-13 의 코드 조사와 독립 리뷰(§7)에서 그 한 줄에 새
  구조 세 개가 들어 있다는 것을 알았다: Gateway 의 커널이 저장 명령을 실행하는 길, 명령의 결과를 답에 싣는 방법, 낡은
  revision 의 거절. 그래서 구현 전에 조각과 결정을 적는다.
- 병렬 제어([18-K4-9b](18-k4-9b-parallel-control.md))와 달리 이 조각은 K6 을 기다리지 않는다. 저장 명령은 커널이 떠
  있지 않아도 실행된다.
- K4 설계와 다르게 정한 것: 명령의 이름과 수 (§3.3), 초안의 범위 (§3.4), `planning-tool` 의 범위 (§3.5).

## 1. 목표

| 번호 | 목표 | 확인하는 방법 |
| --- | --- | --- |
| D1 | admin 의 작업·방향·파일 변경과 초안의 승격이 커널 명령이다. 기록에 남는다 | 그 변경마다 `command_received` 에 `caller: akra-admin` 이 남는다 (입구 시험) |
| D2 | 고치기와 지우기는 `expected_planning_revision` 이 필수이다. 낡은 값은 이유 코드로 거절한다 | 낡은 revision 의 명령이 아무것도 바꾸지 않고 `planning_revision_stale` 로 거절된다 |
| D3 | 사용자는 낡은 화면에서 낸 변경의 이유를 본다 | admin 이 500 대신 409 와 "다시 읽고 다시 하라"를 답한다 |
| D4 | 입구의 adapter 가 변경 서비스를 직접 부르지 않는다 (K4 의 G3) | 입구 경계 시험. 초안의 편집과 `planning-tool` 은 예외로 적는다 (§3.4, §3.5) |

## 2. 현재 상태 (2026-11-13 코드 조사, 독립 리뷰로 확인)

### 2.1 입구의 변경

| 변경 | 입구 | 부르는 곳 | revision | 사용자에게 보이는 것 |
| --- | --- | --- | --- | --- |
| 작업 만들기·고치기 | admin (JSON, HTML) | `PlanningAdminPort::upsert_task` | 고치기만 `expected_planning_revision` 을 받는다 | 알림 글과 관리 화면의 값 |
| 작업 지우기 | admin | `delete_task` | 없다 | 같다 |
| 방향 만들기·고치기·지우기 | admin | `upsert_direction`, `delete_direction` | 없다 | 같다 |
| 초안 만들기·저장 | admin | `create_draft_session`, `save_draft_session` (검사 `/validate` 도 저장한다) | 없다 | 초안 화면의 값 |
| 초안 승격 | admin | `promote_draft_session` | admin 프로세스의 메모리에 둔 값으로 검사한다 | 파일 수와 유효한가 |
| 파일 내보내기·적용 | admin | `export_active_files_for_edit`, `apply_exported_files` | 적용만. 내보낼 때 저장소에 둔 기준(baseline)으로 검사한다. 입력은 없다 | 알림 글과 경로 |
| 작업 만들기·고치기 | CLI `akra planning-tool run` | `PlanningTaskToolPort` | 없다 | JSON 한 줄 (만든 작업 id, 새 revision, 큐의 머리 등) |
| 초안과 큐의 일부 | TUI | core 명령 (`StagePlanningEditor`, `MutatePlanningEditor`, `SubmitQueueMutation` 등) | 큐의 취소는 받는다 | 화면 |

- Telegram 은 작업, 방향, 초안을 바꾸지 않는다.
- admin 의 처리기는 이 변경들의 서비스 오류를 모두 HTTP 500 으로 답한다. 원인("다시 읽고 다시 하라" 포함)은 서버
  로그에만 남는다. 처리기의 오류 타입이 상태 코드뿐이라서 본문이 없다. (다른 경로에는 400, 403, 409, 428 이 있다.)
- admin 의 변경은 `PlanningAdminFacadeService` 의 메서드이다. K4-9a 가 쓴 자리(`PlanningServices` 의 use case)를 지나지
  않는다. 커널의 조립이 만드는 서비스 묶음에는 admin 의 변경이 없다.
- 초안의 승격은 admin 의 메모리(`draft_stage_metadata`)를 쓴다. 그 값은 초안을 만든 때의 revision, 초안의 종류, 방향
  id, 고칠 수 있는 경로이다. 메모리에 그 값이 없으면 승격을 거절한다.
- 서비스의 오류는 글자이다 (`bail!`). 낡은 revision 을 타입으로 가릴 수 없다. 변경 guard 의 거절(slot 의 lease, 배분
  항목, 세션, worker 의 `official-refresh`, "이미 고치는 중")도 같은 글자 오류이다.
- 파일 내보내기·적용은 Windows 에서 되지 않는다.

### 2.2 `planning-tool` 과 worker

- 연산은 `list_tasks`, `create_task`, `update_task` 이다. 지우기는 없다. revision 을 검사하지 않는다.
- admin 의 고치기와 뜻이 다르다:

  | | admin | `planning-tool` |
  | --- | --- | --- |
  | 고치기 | 작업 전체를 바꾼다 | 준 필드만 바꾼다 |
  | 출처 | 사용자 | worker, 부모 thread 와 턴의 id |
  | 변경 guard | 잡는다. worker 의 `official-refresh` claim 이 있으면 거절된다 | 잡지 않는다. worker 는 그 claim 아래에서 돈다 |

- worker 에는 둘째 길이 있다: 커널이 worker 의 마지막 답에 든 작업 명령을 적용한다.
- worker 는 app-server 의 숨은 thread 이고, 그 셸이 `akra planning-tool run .` 을 부른다. 기본 sandbox 는 전체 접근이다.
- app-server 프로세스의 환경 변수는 허용 목록으로 줄이고 `AKRA_HOME` 은 그 목록에 없다. 숨은 thread 는 셸에 변수를 따로
  줄 수 있다 (`shell_environment_policy`). 같은 방법으로 `AKRA_HOME` 을 줄 수 있다.
- 커널의 효과는 작업 스레드에서 돈다. 호스트는 그 사이에 명령을 받는다 (코드로 읽었다. 실행해 보지 않았다).

### 2.3 커널

- Gateway 의 커널에는 이 변경들을 실행할 core 길이 없다. TUI 의 초안 명령은 다른 식별자(편집기 세션)를 쓴다.
- 명령 전용 조립(`composition/kernel_command_execution.rs`)은 `reset_planning` 하나를 코드에 적었다. 살아 있는 상태의
  조회(`planning` 포함)를 거절한다.
- `planning` 조회의 답에 `revision` 이 있다. admin 이 견주는 값과 같은 숫자이다. 그러나 core 가 기억한 값이고 없을 수
  있다. 커널 밖의 쓰기(TUI, worker, 직접 경로) 뒤에는 낡은 값이다.
- Gateway 는 커널의 답을 10초 기다린다. 저장소의 busy 대기는 연결마다 5초이고, 변경 하나가 트랜잭션을 여러 번 연다.
- 클라이언트는 옛 Gateway 를 attach 에서만 가린다 (K4-9a). K4-9a 판의 Gateway 는 attach 를 받고, 모르는 명령에는
  `-32602` 로 답한다.

### 2.4 결과

명령의 답(`processed`, `rejection`, `operation`)은 실행의 결과를 싣지 않는다. K4-9a 는 결과가 대상마다 정해져 있어서
답 없이 풀었다. 이번에는 실행에 따라 달라지는 결과가 있다.

| 결과가 정해져 있다 | 결과가 실행에 따라 다르다 |
| --- | --- |
| 파일 내보내기·적용 (경로 하나), 작업 지우기의 알림 | 만든 작업의 id, 만든 방향의 id, 방향을 지울 때 함께 지운 작업 수와 기본 방향을 남겼는가, 승격의 파일 수와 유효한가, 새 revision |

관리 화면의 값은 명령 뒤에 다시 읽으면 된다. 읽기는 입구가 직접 한다.

## 3. 설계

### 3.1 저장 명령은 잠금을 쥔 쪽에서 바로 실행한다

저장 명령(`state: store`)은 저장된 planning 데이터만 바꾼다. 도는 커널의 상태가 필요 없다. 그래서 실행기 하나를
만들고, 저장소 잠금을 쥔 쪽이 그 실행기를 부른다.

| 상황 | 실행하는 곳 | 기록 |
| --- | --- | --- |
| Gateway 의 커널이 돈다 | 그 커널의 런타임이 명령을 받고 실행기를 부른다. 끝난 뒤(성공이든 거절이든) planning 을 다시 읽는다 | 남는다 |
| Gateway 가 없다 | 명령 전용 조립이 잠금을 `command` 로 잡고 실행기를 부른다 | 남는다 |
| Gateway 가 이 명령을 모른다 (옛 판) | 명령 전용 조립. 그 Gateway 가 그 저장소의 커널을 쥐고 있으면 잠금에서 멈추고 "Gateway 를 다시 시작하라"를 답한다 | 남는다 |
| TUI 가 커널을 쥐고 있다 | K6 까지 직접 실행한다 (실행기를 잠금 없이 부른다) | 남지 않는다 |
| 저장소 커널이 있을 수 없는 폴더 (병렬 slot worktree, 대표 checkout 이 없는 저장소) | 직접 실행한다 (K4-9a 와 같다) | 남지 않는다 |

- 실행기는 application 의 port 이고 구현은 composition 에 둔다. 그래야 커널 시뮬레이션이 가짜 실행기를 끼울 수 있다.
  시뮬레이션은 효과를 가짜로 바꾸는데, 호스트 스레드에서 도는 실행기는 그 가짜를 지나지 않기 때문이다.
- 실행기가 부르는 서비스: 커널의 조립에는 admin 의 변경 서비스가 없다. 실행기는 `PlanningAdminFacadeService` 를 커널의
  조립(직접 경로)으로 만들어 쓴다. 입구의 조립을 쓰면 명령이 자기에게 돌아온다.
- 작업 공간: 실행기는 커널의 대표 checkout 을 쓴다. 클라이언트가 붙은 폴더가 아니다. 파일 내보내기는 그 폴더에 파일을
  쓴다. admin 의 작업 공간이 대표 checkout 과 다른 경우를 시험한다.
- 옛 Gateway: 입구는 자기 스펙으로 입력을 먼저 검사한다. 그래서 Gateway 가 그 명령에 `-32602` 로 답하면 옛 판으로
  본다 (attach 의 규칙을 명령으로 넓힌다). 옛 Gateway 는 실행하기 전에 거절하므로 두 번 실행되지 않는다.
- 실행은 동기이다. 커널의 호스트 스레드가 저장소에 쓰는 동안 막힌다. 한도는 §6 에 있다.
- Gateway 커널의 `reset_planning` 은 지금의 core 길(비동기)에 둔다.
- 커널이 명령으로 받는 입력은 지금 core 명령 하나이다. 이것을 "core 명령 또는 저장 명령"으로 넓힌다. 넓힐 때 풀 것은
  K4-9b 문서의 §3.2 에 있다 (`into_core` 를 부르는 세 곳, 런타임이 직접 만드는 거절과 그 기록, 봉인된 런타임 API).

### 3.2 명령의 결과는 답에 싣는다

- 명령의 답에 선택 필드 `result` 를 더한다. 저장 명령이 성공하면 그 명령의 결과가 들어간다. 타입은 명령마다 모양이
  정해진 값이다 (`type` 으로 가리는 enum). 스펙의 명령 항목에 그 모양(`result` 의 JSON Schema)을 더한다. Desktop 타입
  생성기는 타입 없는 값을 거절하므로 열린 값을 쓰지 않는다. `runtime` 명령과 `reset_planning` 은 `result` 가 없다.
- 저장 명령은 실행이 끝난 뒤에 답한다. 그래서 답이 올 때 그 연산은 이미 끝났다 (`command_completed` 가 기록되어 있다).
  입구는 `operation` 조회로 기다릴 필요가 없다.
- 결과는 기록에 싣지 않는다. 기록은 받음, 끝남, 실패의 원인이다. 그래서 같은 멱등 키로 다시 보낸 명령의 답에는
  `result` 가 없다. 기록은 명령의 지문만 가지므로 어느 작업을 바꿨는지도 기록에 없다. 이 한계는 그대로 둔다.
- 실패의 원인은 K4-9a 와 같다: `command_completed.detail`.

### 3.3 명령

K4 설계는 `upsert_task` 처럼 "만들기와 고치기"를 한 명령으로 적었다. 나눈다. "고치기만 revision 이 필수"는 입력
스키마로 적을 수 없기 때문이다.

| 명령 | 입력의 요점 | `effect` | `idempotency` | `result` | 바로 거절 |
| --- | --- | --- | --- | --- | --- |
| `create_task` | 작업의 필드 | write | keyed | 작업 id, 새 revision | `planning_locked` |
| `update_task` | 작업 id, 작업의 필드 전체, `expected_planning_revision` | write | natural | 새 revision | `planning_revision_stale`, `planning_locked` |
| `delete_task` | 작업 id, `expected_planning_revision`, `confirm` | destructive | natural | 새 revision | 같다 |
| `create_direction` | 방향의 필드 | write | keyed | 방향 id, 새 revision | `planning_locked` |
| `update_direction` | 방향 id, 방향의 필드 전체, `expected_planning_revision` | write | natural | 새 revision | `planning_revision_stale`, `planning_locked` |
| `delete_direction` | 방향 id, `expected_planning_revision`, `confirm` | destructive | natural | 함께 지운 작업 수, 기본 방향을 남겼는가, 새 revision | 같다 |
| `promote_draft` | 초안 이름, 초안의 종류, 방향 id (있으면), `expected_planning_revision` | write | natural | 승격한 파일 수, 유효한가, 새 revision (승격했을 때만) | `planning_revision_stale`, `planning_locked` |
| `export_planning_files` | 없음 | write | natural | 경로 | `planning_locked` |
| `apply_planning_files` | 없음 | write | natural | 경로, 새 revision | `planning_revision_stale`, `planning_locked` |

- `scope` 는 모두 `planning` 이고 `state` 는 모두 `store` 이다.
- 뜻은 admin 의 것이다. `update_task` 는 작업 전체를 바꾸고, 출처는 사용자이고, 변경 guard 를 잡는다. worker 의 부분
  고치기는 이 명령이 아니다 (§3.5).
- `planning_revision_stale`: "보낸 revision 이 지금의 revision 과 다르다. 아무것도 바꾸지 않았다." `next` 는 `planning`
  조회, 그 다음 사람에게 묻기이다. 그 `next` 를 따를 수 있게 두 가지를 한다:
  - 명령 전용 조립이 `planning` 조회를 저장소에서 답한다. 지금은 거절한다.
  - Gateway 의 커널은 저장 명령을 거절한 뒤에도 planning 을 다시 읽는다. 그래서 다음 조회가 낡은 값을 답하지 않는다.
- `planning_locked`: "다른 변경이나 병렬 작업이 planning 데이터를 쥐고 있다. 아무것도 바꾸지 않았다." 변경 guard 의
  거절이다. `next` 는 잠깐 기다림, 다시 보냄이다. 이것을 실패(`failed`)로 두면 다시 해도 되는 조건이 500 이 된다.
- 두 이유를 가리려면 서비스의 글자 오류를 타입 오류로 바꿔야 한다. 실행기가 그 타입으로 거절을 만든다. 그 밖의 오류는
  실패이다.
- 낡은 revision 과 guard 의 거절은 실패가 아니라 거절이다. 저장 명령은 바로 실행하므로 거절을 답에 실을 수 있다
  (`rejection` 과 `command_rejected` 기록).
- 지우는 명령은 `confirm: true` 가 필요하다 (K4-5 의 규칙). 입구의 확인 화면이 그 값을 붙인다.
- `apply_planning_files` 는 revision 입력이 없다. 내보낼 때 저장소에 둔 기준이 이미 검사한다. 그 검사의 실패를
  `planning_revision_stale` 로 답한다.

### 3.4 초안

- 초안 만들기와 저장은 옮기지 않는다. 초안은 admin 의 편집 중인 파일이고 planning 의 기준 데이터가 아니다. 검사
  (`/validate`)도 저장하므로, 옮기면 글자를 칠 때마다 기록이 두 개씩 생긴다.
- 승격만 명령이다. 승격이 기준 데이터를 바꾸기 때문이다.
- 지금 승격은 admin 의 메모리를 믿고 클라이언트의 값을 믿지 않는다. 실행기는 그 메모리를 볼 수 없다. 그래서 admin 이
  메모리의 값(초안을 만든 때의 revision, 종류, 방향 id)을 명령의 입력으로 보내고, 실행기는 입력을 받는 새 승격 함수를
  쓴다. 믿는 쪽이 "서버의 메모리"에서 "명령을 보낸 입구"로 바뀐다. 입구는 같은 사용자의 로컬 프로세스이다 (§5 의
  Q-9d-3).

### 3.5 입구

- admin: `PlanningAdminPort` 를 감싸는 조립의 구현을 둔다. 읽기와 초안의 편집은 지금의 facade 로 보내고, §3.3 의 변경은
  커널 명령으로 보낸다. 답의 `result` 와 다시 읽은 값으로 지금의 결과 타입을 만든다.
- admin 의 처리기는 바뀐다. 거절을 409 와 글로 답하려면 처리기의 오류 타입이 본문을 가져야 한다. JSON 은 이유 코드와
  글을 답하고, HTML 은 알림(notice)으로 보이고, HTMX 조각도 같은 글을 보인다. 그 밖의 실패는 500 이지만 원인의 글을
  답에 싣는다.
- admin 의 화면: 작업 지우기, 방향 고치기·지우기 폼에 revision 을 더한다. 지금은 작업 고치기 폼에만 있다.
- `planning-tool`: 이 조각에서 커널 명령으로 옮기지 않는다. 뜻이 달라서 §3.3 의 명령으로 받을 수 없다 (§2.2). 하는
  일은 하나이다: 요청에 `expected_planning_revision` (선택)을 더하고, worker 의 skill 이 `list_tasks` 가 준 값을
  `update_task` 에 붙이게 한다. 커널 명령으로 옮기는 일(worker 용 명령, claim 아래의 guard, `AKRA_HOME` 의 전달)은
  따로 설계한다.
- TUI: 바꾸지 않는다. TUI 의 초안과 큐 명령은 이미 core 명령이다.

### 3.6 하지 않는 것

- 읽기는 옮기지 않는다 (K4 설계 §3.7).
- TUI 의 편집기 세션과 admin 의 초안을 하나로 합치지 않는다.
- Gateway 커널의 `reset_planning` 을 동기 실행으로 바꾸지 않는다.
- worker 의 둘째 길(커널이 마지막 답의 작업 명령을 적용한다)은 이미 커널 안의 일이다. 바꾸지 않는다.

## 4. 조각 (PR 단위)

| 조각 | 내용 | 동작 | 확인 |
| --- | --- | --- | --- |
| 9d-1 (구현 2026-11-13, §8) | 저장 명령의 실행기 (port 와 구현). 커널 입력을 "core 명령 또는 저장 명령"으로 넓힌다. 답의 `result` 와 스펙의 모양. 이유 코드 `planning_revision_stale`, `planning_locked` 와 서비스의 타입 오류. 명령 전용 조립의 `planning` 조회. 옛 Gateway 의 판별을 명령으로 넓힌다. 첫 명령 셋: `create_task`, `update_task`, `delete_task` | 확장 (`akra kernel planning create_task …` 가 된다. 입구는 그대로이다) | 실행기 시험 (다섯 갈래, 낡은 revision, guard, 결과), 프로토콜 시험, 명령 전용 조립 시험, Gateway 커널 시험 (실행 뒤와 거절 뒤의 `planning` 이벤트), 커널 시뮬레이션 (가짜 실행기. 기록과 거절의 불변식), 스펙·Desktop 타입·skill 일치 |
| 9d-2 | admin 의 작업 변경이 커널 명령으로 간다. 지우기 폼에 revision. 처리기의 오류 타입, 409 와 원인의 글 | 전환 (지우기에 revision 이 필수가 된다) | 입구 시험 (기록의 호출자, 낡은 폼의 409), admin 시험 (JSON, HTML, HTMX) |
| 9d-3 | 방향: 명령 셋과 admin | 전환 (고치기·지우기에 revision 이 필수가 된다) | 같다 |
| 9d-4 | 파일 내보내기·적용: 명령 둘과 admin | 유지 | 같다. 작업 공간이 대표 checkout 과 다른 경우 |
| 9d-5 | 초안의 승격: 명령 하나, 입력을 받는 승격 함수, admin | 유지 | 같다. 명령 전용 조립에서 승격이 되는지 본다 |
| 9d-6 | `planning-tool` 의 선택 revision 과 worker 의 skill | 확장 | skill 파일 시험, CLI 시험 (모델 턴 없음) |
| 9d-7 | 읽기 경로에서 프롬프트 기록 정리를 뺀다. 입구 경계 시험 | 유지 | 구조 시험 |

- 9d-1 이 나머지의 전제이다. 9d-2 ~ 9d-5 는 서로 독립이다.

## 5. 사용자가 정할 것

| 번호 | 질문 | 기본값 | 다른 선택 |
| --- | --- | --- | --- |
| Q-9d-1 | admin 에서 작업 지우기와 방향 고치기·지우기에 revision 이 필수가 된다. 다른 창이나 worker 가 planning 을 바꾼 뒤의 낡은 화면에서는 그 변경이 거절되고 "다시 읽고 다시 하라"가 보인다. 받아들이는가 | 받아들인다 (K4 설계 §3.7 이 정했다) | 지우기는 revision 없이 둔다 |
| Q-9d-2 | `planning-tool` 의 `update_task` 에 revision 을 언제 필수로 하는가 | 선택으로 시작한다. worker 의 skill 이 값을 붙이는 것을 본 뒤에 필수로 바꾼다 | 처음부터 필수로 한다 (옛 skill 을 쓰는 worker 의 고치기가 실패한다) |
| Q-9d-3 | 초안의 승격에서 믿는 쪽이 admin 의 메모리에서 명령의 입력으로 바뀐다 (§3.4). 받아들이는가 | 받아들인다. 입구는 같은 사용자의 로컬 프로세스이다 | 초안의 메타데이터를 저장소에 저장하고 실행기가 읽는다 (새 표가 필요하다) |
| Q-9d-4 | 9d-2 뒤에는, 프로그램을 올린 뒤 옛 Gateway 가 그 저장소의 커널을 쥐고 있는 동안 admin 의 편집이 "Gateway 를 다시 시작하라"로 실패한다. 받아들이는가 | 받아들인다. 올린 뒤에 Gateway 를 다시 시작하는 것은 Desktop 의 새 기능에도 필요하다 | 프로그램을 올릴 때 Gateway 를 스스로 다시 시작하게 한다 (별도 작업) |

답이 없으면 기본값으로 간다.

## 6. 위험

| 위험 | 대응 |
| --- | --- |
| 커널의 호스트 스레드가 저장 명령을 실행하는 동안 막힌다 | 저장 명령은 저장소 쓰기이다. 파일 적용과 승격의 시간을 시험에서 본다. 길면 그 둘만 효과로 옮긴다 |
| Gateway 는 커널의 답을 10초만 기다린다. 저장소가 바쁘면 시간이 넘고, 호출자는 오류를 받지만 변경은 반영될 수 있다 | 만들기 명령은 멱등 키가 있다. 같은 키로 다시 보내면 두 번 실행되지 않는다. 그 답에는 `result` 가 없으므로 입구는 상태를 다시 읽는다. 고치기와 지우기는 revision 이 두 번째 실행을 거절한다 |
| 실제 조립을 지나는 admin 시험이 명령 전용 조립으로 간다 | K4-9a 와 같다. 시험 빌드는 실제 Gateway 에 붙지 않는다. 그 시험에 기록의 확인을 더한다 |
| 지우기에 revision 이 없는 기존 시험과 폼이 깨진다 | 9d-2, 9d-3 에서 폼과 시험을 함께 고친다 |
| Gateway 가 돌고 그 저장소의 커널이 없으면, admin 의 변경 하나가 커널을 조립하게 하고 남아 있던 프롬프트의 턴을 시작할 수 있다 | K4-9a 의 남긴 것과 같다. K4-8 의 잠깐 붙는 접속의 성질이다. 여기서 풀지 않는다 |
| `AKRA_HOME` 이 worker 의 셸에 전달되지 않는다 | 지금도 있는 어긋남이다. `planning-tool` 을 커널 명령으로 옮기는 설계에서 정한다 |

## 7. 리뷰 반영 (2026-11-13)

독립 리뷰가 첫 판을 코드와 견주었다. §2 의 사실은 대부분 맞았다. 설계에서 고친 것:

- 낡은 revision 의 `next` (`planning` 조회)를 따를 수 없었다: 명령 전용 조립은 그 조회를 거절하고, Gateway 커널의 값은
  낡을 수 있다. 두 가지 대응을 §3.3 에 적었다.
- "Gateway 가 옛 판이면 명령 전용 조립"은 틀렸다. 옛 판은 attach 에서만 가렸다. 판별을 명령으로 넓힌다 (§3.1).
- 낡은 revision 을 타입으로 가릴 수 없다. 서비스의 타입 오류를 9d-1 에 넣었다.
- "입구의 코드는 그대로"는 409 와 맞지 않는다. 처리기의 변경을 9d-2 에 적었다.
- `upsert_task` 하나로 admin 과 worker 를 받을 수 없다 (전체 교체와 부분 고치기, 출처, guard). 명령의 뜻을 admin 의
  것으로 정하고 `planning-tool` 을 범위에서 뺐다.
- 승격은 admin 의 메모리가 필요하다. 초안 만들기와 저장을 범위에서 빼고, 승격은 입력을 받게 했다.
- 만들기와 고치기를 나눴다. "고치기만 revision 필수"는 스키마로 적을 수 없다.
- `apply_planning_files` 의 revision 입력을 지웠다. 저장된 기준이 검사한다.
- 변경 guard 의 거절을 이유 코드(`planning_locked`)로 더했다.
- 답의 필드 이름을 `output` 에서 `result` 로 바꾸고 타입이 있는 값으로 했다. 스펙의 `output` 은 이미 답 전체이다.
- 실행기를 port 로 두었다. 시뮬레이션이 가짜를 끼운다.
- 상황의 표에 "저장소 커널이 있을 수 없는 폴더"와 "Gateway 가 이 명령을 모른다"를 더했다.
- 호스트를 막는 시간의 한도(10초, 5초)를 위험에 적었다.

## 8. 조각의 결과

### 9d-1 (2026-11-13)

만든 것:

- 거절의 타입: domain `PlanningMutationRefusal` (낡은 revision, 잠김). 작업 서비스의 revision 검사, admin 의 문서
  commit, 저장소의 두 변경 guard 가 글자 오류 대신 이 타입을 돌려준다. 글은 이전과 같다.
- 실행기: inbound port `StoredCommandPort` 와 `StoredCommandService`. 사용자의 작업 만들기, 고치기, 지우기를 실행하고
  결과(작업 id, 새 revision)를 돌려준다. 거절 타입은 이유 코드가 있는 거절이 되고, 그 밖의 오류는 실패이다. admin
  facade 에 폼과 저장 명령이 함께 쓰는 함수를 두었다 (`apply_operator_task_command`, `delete_operator_task`).
- 스펙: 명령 `create_task`, `update_task`, `delete_task` (`state: store`), 이유 코드 `planning_revision_stale`,
  `planning_locked` 와 그 `next`. 명령 항목에 `result` (결과의 JSON Schema)를 더했다.
- 답: `result` (타입이 있는 값)와 `failure` (실패의 원인 글). 저장 명령의 답은 `result`, `rejection`, `failure` 중
  하나를 가진다.
- 커널의 입력: 프로토콜의 `KernelCommand::into_input` 이 "core 명령 또는 저장 명령"을 돌려준다. 런타임의
  `dispatch_client_command` 가 저장 명령을 받으면 실행기를 바로 부르고, 끝을 기록하고 (`command_completed`
  또는 `command_rejected`), planning 을 다시 읽는다. Gateway 의 커널이 실행기를 붙인다.
- 명령 전용 조립: 초기화와 저장 명령을 같은 길로 실행한다 (잠금, 받음 기록, 실행, 끝 기록). TUI 가 커널을 쥐
  저장소에서는 직접 실행하고 기록하지 않는다. `planning` 조회를 저장소에서 답한다.
- 옛 Gateway: 클라이언트는 요청에 대한 `-32602`, `-32601` 도 옛 판으로 본다 (attach 뿐이 아니다).
- CLI: 실패한 저장 명령의 종료 코드는 1 이다. 객체 필드는 JSON 글로 준다
  (`--task '{"title":"..."}'`).

설계와 다른 점:

- 답에 `failure` 를 더했다. 설계는 실패의 원인을 `command_completed.detail` 에만 두었다. 영수증의 `state` 는 받을
  때의 결과(`accepted`, `rejected`)만 적는다. 그래서 답만 보고 실패를 알 수 없었다.
- 실행기의 port 는 inbound port 이다. 쓰는 쪽(커널 런타임, 명령 전용 조립)이 부르는 use case 이기 때문이다.
- 고치기의 "필드 전체"는 admin 의 폼과 같다: 이유와 참조는 없으면 비우고, 방향·설명·상태·우선순위는
  없으면 그대로 둔다. 서비스의 고치기 입력이 그 필드들에서 "없음"을 "그대로"로 읽는다.
- 커널 시뮬레이션에 저장 명령을 넣지 않았다. 런타임 시험이 가짜 실행기로 기록과 거절을 본다.

확인: domain 시험 (거절을 오류 사슬에서 찾음, 이유 이름 고정), 프로토콜 시험 (명령의 왕복, 저장 명령의 종류와
결과 모양, 지우는 명령의 확인 규칙, 스펙 파일 일치), 런타임 시험 (가짜 실행기: 끝남·거절·실패의 기록과 영수증,
실행기가 없는 커널), 명령 전용 조립 시험 (실제 저장소: 만들기·고치기·지우기와 기록, 낡은 revision 의 거절,
실패의 원인, TUI 가 쥐 때의 직접 실행, 저장소에서 읽는 `planning` 조회), Gateway 클라이언트 시험 (명령을 모르는
Gateway), 끝에서 끝까지 (`tests/kernel_cli.rs`, unix: 실제 `akra kernel planning create_task`, 조회, 낡은 revision 의
종료 코드 2, 확인 없는 지우기의 `-32010`), 구조 시험, 스펙·Desktop 타입·skill 일치. 모델 턴은 없다.

남긴 것:

- 실제 Gateway 커널에서 저장 명령을 보내는 시험은 없다. 런타임 시험은 가짜 실행기를 쓴다. 실행 뒤에 planning 을
  다시 읽는 것(`planning` 이벤트)도 시험이 보지 않는다.
- 입구는 아직 이 명령을 쓰지 않는다 (9d-2).
- 옛 Gateway 가 그 저장소의 커널을 쥐고 있으면 명령 전용 조립은 잠금에서 멈추고 잠금의 오류를 돌려준다. "Gateway 를
  다시 시작하라"는 글을 더하지 않았다.
- 같은 멱등 키의 재전송은 `result` 를 받지 못한다 (설계 §3.2).
