# Task 증거 기록과 현재 상태

관련: [완료 기준 계약](task-acceptance.md), [상위 #2481](https://github.com/RefinedStone/codex-exec-loop/issues/2481),
[자동 실행 선행 조건 #2560](https://github.com/RefinedStone/codex-exec-loop/issues/2560).

## 제공하는 기능

E2는 작업자가 제출한 `pass`, `fail`, `blocked`와 호스트가 관찰한 파일·Git tree를 함께 기록한다.
호스트는 파일의 존재, 크기, SHA-256, 현재 작업 tree와 실행 lease를 확인한다. 파일 내용이 실제로
시나리오의 성공을 증명하는지 독립적으로 실행하거나 판정하지는 않는다. `pass`는 작업자의 보고이며,
이 기록만으로 task 완료·push·PR·통합 권한이 생기지 않는다.

기존 기준 없는 작업과 실행·완료 정책은 유지한다. [전달 게이트](task-delivery-evidence.md)는 기본 `observe`에서
판정을 기록하고, 명시적인 `enforce`에서 현재 호스트 검증 또는 Human 면제를 요구한다. 이 작업자 보고만으로
강제 게이트를 충족하지 않는다. 필수 기준을 전달 관문으로 강제하는 E3와 증거 탐색·면제 버튼을 제공하는
Desktop 화면 E4는 별도 범위다. Desktop은 활성 작업의 기준과 작업자 보고를 읽을 수 있으며, 기준 편집·검증
실행·파일 열기·면제 실행은 후속 화면 범위다.
사용자가 선택한 고정 recipe의 실제 실행은 [호스트 검증](task-verification.md) 계약이며,
아래 작업자 보고와 별도의 `host_verification`으로 조회한다. 일반 worker가 사용자 완료 기준을
수정할 수 있게 하지는 않는다.

## 기록 순서

1. `task_evidence` 조회로 현재 `task_id`, `criterion_id`, `scope_id`,
   `acceptance_revision`을 읽는다. 전체 task를 읽을 때에는 `next_task_id`로 다음 페이지를 읽는다.
2. `prepare_evidence`에 위 좌표와 새로운 `request_id`를 보낸다. 호스트는 해당 task의 유일한
   현재 `Running` lease와 고정 delivery base를 확인하고 증거 attempt를 예약한다.
3. 반환된 `artifact_path`에 실행 결과를 쓴다. 파일은 호스트가 미리 만든 비어 있는 일반 파일이다.
   기존 파일을 링크로 교체하거나 다른 경로를 명령에 지정하지 않는다.
4. `record_evidence`에 반환된 `attempt_id`, 보고할 `status`, 선택적인 `note`만 보낸다.
   경로, 파일 hash, Git tree, actor 또는 lease는 클라이언트 입력으로 받지 않는다.
5. 현재 유효 여부는 `task_evidence` 또는 `planning.task_evidence`로 다시 읽는다.

예를 들어 현재 저장소에서 다음 조회를 사용할 수 있다.

```sh
akra kernel query task_evidence --limit 64 --json
akra describe --json
```

실제 명령 필드와 결과 형식은 생성된 Kernel 스펙을 기준으로 한다.
`prepare_evidence`, `record_evidence`, `waive_criterion`은 `planning` 범위의 저장 명령이다.
명령의 `--idempotency-key`와 함께 업무 요청의 `request_id`도 재시도 동안 유지한다.

같은 prepare 요청은 같은 예약을 반환한다. 기준·lease·tree가 달라지면 이전 예약을 새 실행으로
재사용하지 않는다. 기록 성공 뒤 같은 attempt와 같은 status/note를 재전송하면 최초 영수증을
그대로 반환한다. 나중에 caller, 시각 또는 코드가 바뀌어도 영수증을 새로 만들지 않는다.
이 과거 사실을 반환하는 것과 현재 `Pass`로 인정하는 것은 별개다.

## 소스와 파일을 확인하는 방식

호스트의 worker 완료 commit과 같은 Git 보안 검사와 staging 규칙을 사용한다. 실제 index를
복사한 전용 임시 index에서 tree를 계산하므로 staged ignored 파일도 관찰에서 빠지지 않는다.
기록 과정은 실제 index, HEAD, branch ref와 작업 파일을 바꾸지 않는다. 변경이 없는 기준 tree도
증거로 기록할 수 있다. 지원하지 않는 index 형식, submodule·숨긴 index 변경, 실행 가능한 Git 설정,
읽기 실패 또는 관찰 도중 변화는 유효한 tree로 취급하지 않는다.

prepare와 record는 기존 pool 변경 잠금을 공유한다. 잠금이 사용 중이면 오래 기다리거나 불완전한
관찰로 진행하지 않는다. 파일 capture 전후에 tree와 lease를 다시 확인하고, SQLite transaction에서
현재 task·기준 scope·lease·작업 제어와 실행 permit을 재검사한다. 사람이 보류하거나 인계받은
작업, 대기 중인 사용자 질문, 취소된 실행 및 교체된 lease의 새 기록은 거절한다.

파일은 저장소 private runtime 아래 고정된 두 영역에 있다.

```text
evidence/incoming/<lease_generation>/<attempt_id>/artifact
evidence/sealed/<attempt_id>/artifact
```

작업자에게 추가로 전달하는 쓰기 root는 해당 lease의 incoming 영역이다. sealed 영역이나 전체
runtime을 추가 쓰기 root로 주지 않는다. 기존 read-only 설정을 workspace-write로 올리지 않는다.
이 설정이 기존 danger-full-access 실행을 OS 계정 수준으로 격리한다고 보장하지 않는다.

capture는 owner/private 권한, 일반 파일, 단일 hard link 및 고정한 부모 디렉터리를 확인한다.
symlink/reparse point, 빈 파일, 크기 초과, 파일 교체와 읽는 동안의 변경은 거절한다. 안정된 bytes를
새 sealed 파일로 복사하고 원자적인 no-replace 방식으로 게시한다. 기존 sealed 파일을 덮어쓰지
않으며, 현재 상태를 조회할 때에도 다시 파일 hash를 확인한다. 동일 OS 계정의 별도 프로세스가
파일을 수정할 수 없다는 보장은 하지 않는다.

파일 한 개는 최대 8 MiB, 기준·scope별 예약은 최대 64개, 저장소 전체 예약/면제 기록은 최대
4,096개다. 파일 예약은 먼저 최대 8 MiB를 차지하고 기록 성공 시 실제 byte 수로 정산한다.
장부의 저장 bytes와 미완료 예약 합계는 512 MiB를 넘지 않는다. 실패하거나 중단된 예약은 아래
한도 복구 정책에서 명시한 경우에만 환불한다. 이 한도는 작업자가 incoming에 만드는 임의 파일을 포함하는 OS
디스크 quota가 아니다. 자동 GC나 사용자 artifact 삭제도 이 변경에 포함되지 않는다.

## 현재 상태

`data`, `http`, `screen` 기준에서 현재 Running lease가 제출한 증거는 정제된 사본을 확인하고
사람이 통과 또는 실패로 기록할 수 있다. 버튼이나 helper 요청은 판정이 아니다. 기본 취소인 별도
native 확인 창에서 정확한 task, 기준, scope, tree 및 artifact를 승인한 경우에만 장부를 쓴다.
확인 후 같은 lease와 tree, sealed bytes를 재검사한다. 공개 상태 `human_passed`/`human_failed`는
작업자 보고 및 `waived`와 구분된다. tree 변경은 `stale`이며 면제의 지속성을 물려받지 않는다.
기존 작업자의 명시적 실패나 막힘은 전달을 차단한다. 사람 통과는 Auto의 기계 검증 조건을 대신하지 않는다.
독립 helper가 호출자 지정 X/Wayland 서버를 사람 확인으로 믿지 않도록 Linux는 열람만 허용한다.

Desktop에서 기준의 **정제된 증거 열기**를 누르면 main이 해당 scope·revision·criterion 및
receipt event ID와 hash로 전용 로컬 helper에 요청한다. 읽을 수 있는 대상은 16 KiB 이하의
sealed UTF-8 텍스트이며, 호스트 capture는 같은 한도 안에서 stdout/stderr를 풀어 보여 준다.
같은 파일 핸들에서 읽은 bytes의 hash와 크기가 일치해야 한다. 일반 경로·incoming 파일·
binary나 잘린 부분은 열지 않는다. 기존 공유본 정책의 알려진 비밀 형식을 가린 뒤 별도 OS
대화상자에 표시한다. 사용자가 닫으면 본문은 유지하지 않는다. 일반 renderer, 이벤트 장부,
예외에는 본문이 없다. 알려지지 않은 비밀 형식까지 탐지한다는 보장은 아니다.

| 상태 | 의미 |
| --- | --- |
| `pending` | 이 기준에 적용할 완료된 증거 기록이나 현재 면제가 없음 |
| `pass`, `fail`, `blocked` | 현재 기준 scope·실행 session·소스 tree와 sealed 파일에 맞는 작업자 보고 |
| `stale` | 과거 기록은 있지만 기준 scope, 소스 tree 또는 실행 session이 달라짐 |
| `waived` | 현재 기준 scope의 네이티브 Human 확인으로 면제됨 |
| 상태 생략 + `unavailable` | 현재 소스나 파일을 확실하게 관찰할 수 없음 |

실행 중이면 유일한 현재 Running lease의 작업 tree를 읽는다. 실행이 끝난 뒤에는 같은 task/session의
유일한 queue 항목에 저장된 `source_commit_sha`를 사용한다. mutable branch나 legacy `commit_sha`로
대체하지 않고, 조회를 위해 원격 fetch도 하지 않는다. 작업자의 dirty tree가 이후 호스트 commit의
tree와 같으면 HEAD commit이 달라졌다는 이유만으로 무효로 만들지 않는다.
실행 중인 소스는 sealed 파일 확인 뒤 tree와 session을 다시 관찰한다. 파일을 읽는 사이 코드가
바뀌면 이전 Pass를 현재 상태로 반환하지 않고 unavailable로 내린다.

기준별로 현재 scope에서 가장 나중에 예약되어 **기록까지 완료된** attempt를 읽는다. 아직 기록하지
않은 새 예약만으로 이전 완료 기록을 지우지 않는다. 해당 tree/session이나 기준이 바뀌면 이전 기록은
별도로 무효가 된다. 파일 한 개라도 현재 검증에 실패하면 같은 task의 관찰된 결과를 unavailable로
내려 일부 기준의 Pass와 불확실한 관찰을 섞지 않는다.

scope에는 task ID 외에 별도의 무작위 식별자, acceptance revision 및 정규화한 기준 hash가 포함된다.
task나 기준을 지웠다가 같은 ID·같은 내용으로 다시 만들어도 과거 증거와 면제가 되살아나지 않는다.
제목이나 기준 순서만 바뀐 no-op은 scope를 바꾸지 않는다. 과거 receipt와 이벤트는 감사용으로 남긴다.

호스트 recipe가 있는 기준은 선택적인 `host_verification`을 함께 반환한다. 작업자 보고의
`state`와 호스트 실행의 `state`는 별개이며, 작업자 `pass`를 호스트 `passed`로 승격하지 않는다.
현재 호스트 성공은 실행 receipt, 원래 기준 scope·recipe, base/source·환경과 sealed capture를
다시 확인한 결과다. 공개 metadata에는 private 실행 token이나 출력 원문을 포함하지 않는다.

`planning`은 기존 큐 응답에 선택적인 첫 증거 페이지를 추가한다. 페이지 크기는 최대 256 task이며,
완료·제외된 task도 증거 조회 대상이다. 상세 `task_evidence` 조회는 명시적 task 목록 또는
`after_task_id`를 받는다. 두 방식을 함께 보내거나 중복 ID·한도 초과를 보내면 거절한다.
페이지를 합칠 때에는 `planning_revision`과 `evidence_revision`을 모두 비교한다. 조회 도중 기준,
queue, lease 또는 기록이 달라지면 결과를 unavailable로 반환한다.

## 사람의 면제와 이벤트

`waive_criterion`은 Desktop main이 발급하고 Gateway가 검증한 확인 증거가 필요하다. 확인 대상에는
canonical workspace, task, criterion, scope, acceptance revision과 정규화한 이유의 digest가 들어간다.
caller를 `human`으로 쓰거나 `--confirm true`를 보내는 것으로 이 증거를 대신할 수 없다.
면제는 현재 기준 판에 적용하므로 코드만 바뀌면 유지되고 기준 scope가 바뀌면 무효가 된다.
명세, Delivery 및 Integrate 승인은 별도로 필요하다.

`criterion_evidence_recorded`와 `criterion_waived`는 원래 receipt와 같은 transaction의 outbox에 쓴다.
커널 기록에 저장이 확인된 뒤 ACK하고, 재시작 또는 재전달에도 event ID와 payload가 모두 같은
최초 사실만 사용한다. 공개 이벤트와 조회에는 metadata만 담는다. artifact 경로·내용, note, 면제 이유
원문, lease token과 네이티브 확인 capability는 넣지 않는다.
기존 비동기 기록기가 앞선 sequence를 저장 중이면 증거 outbox는 커널을 기다리게 하지 않고
다음 tick으로 미룬다. 앞선 명령보다 증거 사건이 먼저 저장되어 history 조회에서 명령이 빠지는
순서 역전을 막는다.

Desktop은 이 선택적 metadata를 검증하며 malformed 응답을 받은 뒤 과거 Pass를 캐시에 남기지 않는다.

## Desktop에서 기준과 보고 읽기

커널 패널의 planning 큐에서 활성 작업의 완료 기준·증거 영역을 펼치면 기준 문장, 필수 여부,
검증 채널과 서버가 판정한 상태를 읽을 수 있다. `pass`는 **작업자 보고: 통과**로 표시한다.
면제는 사람 면제로 구분한다. 보고 상세에는 작업자·기록 시각·시도와 artifact 해시·크기,
기록 당시 tree를 표시한다. 조회 기준과 보고 당시 기준은 각각의 metadata이며 클라이언트가
둘을 다시 비교해 상태를 판정하지 않는다.

면제 사유는 원문 대신 digest만 제공되므로 화면에서도 사유 해시로 표시한다. 이 화면은
기준 편집, 증거 기록, 검증 실행, 파일 열기 또는 면제 명령을 보내지 않는다.

- `criteria=[]`를 받은 작업에만 등록된 완료 기준이 없다고 표시한다.
- 첫 증거 페이지에 작업이 없으면 조회 범위에 포함되지 않았거나 항목을 받지 못했다고 표시한다.
  페이지는 완료·제외 작업도 포함하므로, 활성 작업이 없다는 이유로 기준이 없다고 추정하지 않는다.
  추가 페이지나 task별 조회는 이 화면의 범위에 포함되지 않는다.
- 이전 Gateway가 선택적 `task_evidence`를 보내지 않으면 정보가 제공되지 않았다고 표시한다.
- snapshot이나 task가 unavailable이면 과거 Pass를 현재 통과로 표시하지 않는다.
- 재조회·재연결 중에는 마지막 조회 정보임을 표시하고 통과 강조를 제거한다. 현재성 표시는
  최근 수락한 Kernel 조회를 의미하며, 조회 이후 코드 tree가 계속 같다는 보장은 아니다.

`criterion_evidence_recorded`와 `criterion_waived` 이벤트는 패널을 보는 동안 재조회 신호로
사용한다. 이벤트를 받자마자 이전 planning 응답을 무효화하고, 같은 처리 구간의 신호는 묶어서
읽는다. receipt를 현재 상태에 직접 합치거나 `refresh_planning` 명령을 보내지 않는다.
패널 닫기·저장소 변경·재연결 이전의 늦은 응답과 오류는 새 조회를 덮지 않는다.
planning 조회 자체가 실패하면 현재 상태를 확인할 수 없다고 표시하며, 다른 overview 조회의
실패만으로 이미 수락한 planning 결과를 과거 정보로 바꾸지 않는다.
일반 planning 알림은 query 전용 증거 metadata를 싣지 않으므로 같은 재조회 신호로 처리한다.
알림의 필드 생략만으로 구 Gateway의 미지원 상태로 바꾸지 않고, query 응답의 생략과 구분한다.

## 검증

회귀는 합성 task, fake runtime, 임시 Git/SQLite/파일 및 fake app-server 프로세스를 사용한다.
실제 모델 turn이나 사용자 저장소의 task·증거 데이터를 시험 입력으로 사용하지 않는다.
범위는 scope 재생성, 동시 예약/한도, 처음 확정된 receipt, 변경된 tree/lease, 파일 교체·크기·링크,
네이티브 확인의 정확한 대상, 이벤트 재전달, 현재/과거 상태 투영 및 초기·재개 worker의 root 전달이다.
Desktop 회귀는 표시 상태·페이지 누락·과거 조회의 구분, 증거 이벤트 재조회와 지연 응답 차단을
확인한다. fake Gateway Electron 화면에서 키보드 펼침, Pass→Stale→Unavailable 전환, receipt와
재조회 결과가 다른 경우의 표시, 패널을 닫은 뒤 조회 중단을 검증한다.
실행 SHA와 검증 결과는 구현 PR에 기록한다.

## 한도 복구: 실제 파일이 없는 종료 예약

`prepare_evidence`와 호스트 검증 예약은 저장소 한도에 도달하면 같은 transaction 안에서
미사용 charge 하나를 회수한 후 재시도한다. 별도의 파일 삭제 명령은 없다.
대상 task는 `done` 또는 `cancelled`이고, 현재 lease와 전달 queue가 없어야 한다.
task 최종 수정과 검증 종료 뒤 모두 30일이 지나고 종료 event의 ACK도 있어야 한다.

- manifest를 저장하기 전에 중단된 호스트 실행은 capture를 만들 수 없으므로 미사용 예약
  8 MiB와 entry 한 개를 회수할 수 있다.
- 명시적 `not_run`은 bytes를 이미 환불했으므로 entry 한 개만 회수할 수 있다.
- `task_evidence_retired_charges`에 기존 event ID, 반환한 bytes와 시각을 영구 기록한다.
  원 receipt, outbox, ordinal, worker 예약, 실제 sealed 파일은 삭제하거나 수정하지 않는다.
  같은 charge를 두 번 사용하지 않는다. 새 예약이 실패하면 회수도 되돌린다.
- 현재 증거나 불확실한 capture만 남으면 `limit`을 유지한다. 한도 복구를 위해 사용자 파일을
  지우거나 receipt가 가리키는 bytes를 지운 것으로 간주하지 않는다. 기준별 64회 제한은 그대로다.
