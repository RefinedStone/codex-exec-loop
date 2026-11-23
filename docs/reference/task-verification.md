# 호스트가 실행한 Task 검증

관련: [Task 완료 기준](task-acceptance.md), [작업자 증거 기록](task-evidence.md),
[자동 실행 선행 조건 #2560](https://github.com/RefinedStone/codex-exec-loop/issues/2560).

## 검증 범위

사용자가 `command` 완료 기준에 `verification_recipe: "git_diff_check_v1"`을 선택하면,
실행 중인 task에 대해 호스트의 고정 검증을 요청할 수 있다. 기준을 저장하는 것만으로 검증을
시작하지 않는다. 기존 기준의 recipe 생략 또는 `null`은 검증 미설정이며 이전 JSON과 기준 hash를
유지한다. `data`, `http`, `screen` 기준에 이 recipe를 설정하는 요청은 거절한다.

`GitDiffCheckV1`은 호스트가 정한 delivery base tree와 관찰한 source tree 사이에서 Git의
`diff --check`를 실행한다. 성공은 해당 범위의 새 공백 오류와 충돌 표식 검사 결과다. 빌드,
단위·기능 테스트 또는 scenario 전체의 성공을 의미하지 않는다. 차이가 없는 두 tree도 성공할
수 있다. 이 실행은 모델을 호출하지 않는다.

검사 대상은 이 recipe가 해석할 수 있는 UTF-8 텍스트 변경이다. NUL이나 비 UTF-8 bytes를 포함한 blob, symlink,
gitlink, 불완전한 Git 출력, 변경 경로 4,096개 초과, 개별 blob 8 MiB 또는 읽을 blob 합계
64 MiB 초과는 실행 가능한 검증 대상으로 인정하지 않는다. 합계는 중복을 제외한 이전·새 blob
전체를 센다. 실행 파일·설정·입력을 안전하게
고정할 수 없는 경우에도 성공을 만들지 않는다.

## 요청과 결과 조회

`request_task_verification`은 `planning` 범위의 **Runtime 명령**이다. 현재 커널이 없는
command-only 경로에서는 `kernel_unreachable`로 거절하며 예약을 생성하지 않는다.
입력은 다음 다섯 필드뿐이다.

| 필드 | 값의 출처 |
| --- | --- |
| `request_id` | 호출자가 재시도 동안 유지하는 요청 식별자 |
| `task_id` | 현재 task ID |
| `criterion_id` | 해당 task의 완료 기준 ID |
| `scope_id` | `task_evidence` 조회의 현재 64자리 scope 식별자 |
| `expected_acceptance_revision` | 같은 조회에서 읽은 기준 revision |

호스트는 현재 기준, recipe, 유일한 Running lease와 실행 권한에 예약을 결속한다. workspace는
기존 커널 context에서 정하며 명령 입력이 아니다. 호출자는 recipe, 명령·인수, 환경, 경로,
Git tree, 결과 status나 출력 hash를 지정하지 않는다. 진단용 caller 문자열도 실행 권한을
대신하지 않는다.

명령 결과 `task_verification_requested`는 `attempt_id`와 `changed`만 반환한다. 이 결과는
예약 사실이며 검증 성공이 아니다. 같은 `request_id`와 같은 좌표의 재전송은 원래 예약을
반환한다. 명령 자체의 `--idempotency-key`도 재시도 동안 유지한다.

tree snapshot, runner 준비와 실제 Git 프로세스는 호스트 worker에서 수행한다. 커널의 요청
경로에는 기존 저장소 권한 조회와 예약 transaction이 남는다. 이 권한 조회는 저장소 연결을
재검증하는 동기 Git metadata 조회를 포함할 수 있으므로 모든 I/O가 비동기라는 보장은 하지
않는다. 작업 중 커널은 살아 있고, 완료 사실의 저장 재시도나 outbox ACK 재시도도
실제 Git 프로세스를 다시 실행하지 않는다.

현재 상태는 다음 조회로 읽는다.

```sh
akra kernel query task_evidence --limit 64 --json
akra kernel query planning --json
akra kernel history --wait --json
```

`planning.task_evidence`는 첫 페이지만 포함한다. 전체 목록은 `task_evidence` 응답의
`next_task_id`로 이어 읽고, `planning_revision`과 `evidence_revision`을 함께 확인한다.
완료 사실은 `task_verification_finished` 이벤트로도 전달된다. 이벤트는 과거 실행 사실이므로
수신 자체로 현재 `passed`를 합성하지 않고 현재 조회를 사용한다.

## 작업자 보고와 호스트 실행의 구분

기존 기준의 `state`와 `evidence`는 작업자가 보고한 `pass`·`fail`·`blocked` 및 Human 면제를
그대로 담는다. 호스트 실행은 별도의 선택적 `host_verification`에 담는다. 두 결과는 서로
덮어쓰지 않으며 작업자의 `pass`나 비어 있지 않은 파일만으로 호스트 성공을 만들지 않는다.

| `host_verification.state` | 의미 |
| --- | --- |
| `pending` | recipe가 설정되었지만 실행 사실이 없거나 예약 후 준비 중 |
| `running` | 해당 예약의 고정 실행 준비가 저장됨; 아직 완료 사실 없음 |
| `passed` | 현재 기준·소스·검증 환경에 맞고, 완전한 capture와 exit code 0이 확인됨 |
| `failed` | 현재 기준·소스·검증 환경에 맞는 실행 사실이 성공 조건을 충족하지 못함 |
| `not_run` | 실행하지 못한 이유 또는 재시작으로 중단된 사실이 있음 |
| `stale` | 과거 예약·실행 사실은 있으나 현재 기준·소스·실행 권한과 맞지 않음 |
| `unavailable` | 현재 소스, 검증 환경 또는 sealed capture를 확실히 확인할 수 없음 |

`phase`는 저장된 예약의 `pending`, `running`, `finished` 단계이고 `state`는 현재 조회의
판정이다. recipe가 있지만 예약이 없으면 attempt와 receipt 없이 `pending`을 반환한다.
과거 receipt의 scope·source·결과는 현재 기준으로 다시 쓰지 않는다. dirty 작업 tree가 이후
호스트 commit과 같은 tree가 되었다면 HEAD 변경만으로 무효화하지 않는다. 파일이나 환경을
확인할 수 없으면 과거 exit code 0을 현재 성공으로 승격하지 않는다.

공개 receipt에는 원래 기준 scope, attempt 번호, recipe, base/source 식별자, 실행 파일·명령·환경·
설정 fingerprint, 종료 종류, 시각·경과 시간과 stream/capture hash·크기만 담는다. `exited`,
`timed_out`, `cancelled`, `output_limit`, `signalled`, `io_failure`를 구분하고, 시작 전 거절은
`not_run`, 재시작 복구는 `interrupted`로 보존한다. 실제 실행 뒤 capture 저장에 실패해도 실행
사실은 남을 수 있지만 현재 `passed`로 인정하지 않는다. UTC 시각은 역행할 수 있으므로
`duration_ms`는 별도의 monotonic 관측값이다.

## 실행·출력 보존 경계

runner 준비 중의 Git 명령과 최종 recipe 실행은 공통 30초 deadline을 사용하고, stdout과 stderr는
각각 1 MiB로 제한한다. 실행 파일 hash, 파일 I/O, 사전 source snapshot과 프로세스 정리까지
포함한 전체 요청이 30초 안에 종료된다는 보장은 아니다. 취소나 출력 한도에 도달하면 기존
관리 프로세스의 정리 경로로 종료한다. 출력이 일부만 수집되었으면 그 사실을
stream의 `complete`에 남기며 완전한 성공으로 판정하지 않는다.

호스트 capture는 private runtime의 다음 영역에 새 파일로 게시한다.

```text
evidence/sealed/verification/<attempt_id>/artifact
```

작업자의 incoming 파일을 읽어 호스트 실행 결과로 사용하지 않는다. 최대 8 MiB의 capture를
기존 private 권한·일반 파일·고정 부모 검사와 no-replace 게시 규칙으로 보존한다. 조회와 공개
이벤트에는 이 경로, stdout/stderr 원문, 실행 인수, session/lease·owner/run token을 노출하지
않는다. 같은 OS 사용자의 모든 프로세스를 격리하는 보안 경계라는 뜻은 아니다.

완료 receipt와 outbox 이벤트는 같은 transaction에 저장한다. 커널은 durable event ID와 정확한
payload를 비교해 append 후 ACK한다. ACK 유실과 history retention 뒤 재전달에서도 같은 사실을
다시 게시하지 않는다. 같은 event ID에 다른 결과가 오면 기존 기록을 덮어쓰거나 ACK하지 않는다.

자동 재실행은 없다. 재시작 때 미완료 예약은 `interrupted`로 마무리하고 새 프로세스를 시작하지
않는다. 사용자가 다시 검증하려면 현재 scope를 읽고 새로운 요청을 보낸다. 실패·중단된 예약도
기존 증거 저장소의 한도와 보존 규칙을 따른다.

## 기존 권한과 후속 화면

이 기능은 Done, dispatch, push, PR 생성, Integrate 또는 Auto의 권한을 바꾸지 않는다. 기준별
Human 면제는 기존 정확한 native 확인 대상으로만 허용하며 Delivery·Spec·Integrate 승인을
대신하지 않는다. 전달 전 강제 검증 정책 E3와 Desktop 기준 카드·증거 열기 E4는 후속 범위다.

회귀는 domain/SQLite 예약·finish 계약, fake worker 비동기 완료와 복구, 실제 임시 Git·파일,
공개 DTO·생성 타입·renderer 검증 및 이벤트 ACK 재전달을 다룬다. 실제 모델이나 사용자
저장소의 실행·증거 데이터를 시험 입력으로 사용하지 않는다. 실행한 SHA와 결과는 구현 PR에
기록한다.
