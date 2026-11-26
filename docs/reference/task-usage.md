# 작업 사용량 조회

`akra/kernel/query`의 `task_usage`는 작업 하나에 대해 실제 관측한 토큰의 하한과
사용량을 관측하지 못한 실행을 반환합니다. 비용 청구서나 작업 전체 비용의 확정값은 아닙니다.

```json
{"query":{"type":"task_usage","task_id":"task-123"}}
```

- `status`: `available`, `missing`, `unavailable`을 구분합니다.
- `observed_tokens`: 관측한 실행들의 토큰 합계입니다. 관측이 없으면 `null`이며 0으로 바꾸지 않습니다.
- `executions`: kernel turn, 아직 turn이 관측되지 않은 operation, parallel session,
  명시적으로 작업에 연결한 delegation attempt를 구분합니다. 각 실행에는 `thread_id`,
  `turn_id`, `op_id`, 관측치 및 `gaps`가 있습니다.
- `runtime_available`, `usage_available`, `delegations_available`: 읽지 못한 저장소를 빈 결과로
  가장하지 않습니다. `truncated`이면 최대 4096개 제한 때문에 전체 결과가 아닙니다.
- `unlinked_repository_delegations`: task 연결이 없는 기존 **저장소 전체** 위임 수입니다.
  요청한 task의 실행으로 추정해 합산하지 않습니다.

## 집계와 보존

실제 kernel이 수용한 progressive activity의 숫자만 기존 kernel event log에 기록합니다.
대화 본문, 모델 응답, callback token은 기록하지 않습니다. SQLite event append와 같은
transaction에서 턴별 최신 집계를 갱신하므로 history retention 이후에도 관측치는 남습니다.

누적 `total` snapshot을 매번 더하지 않습니다. 한 턴의 첫 관측에서는 `last`만 셉니다.
첫 `total`에 더 오래된 값이 포함되었으면 `prefix_not_observed`로 표시합니다. 이후 같은 턴에서
수신한 `total`의 증가분을 한 번씩 더합니다. 같은 값·같은 기록의 재처리는 더하지 않으며,
역행하거나 초기화된 counter는 `counter_discontinuity`로 표시합니다. 범위를 넘어선 합계는
`arithmetic_overflow`와 `null`로 반환합니다.
`last`가 `total`보다 큰 모순된 snapshot은 baseline과 합계에 반영하지 않으며,
아직 유효한 관측이 없으면 미관측 상태를 유지합니다.

준비된 사용자 task와 정확한 kernel operation의 연결로만 숫자를 귀속합니다. 기존 parallel
session은 실행 목록에 포함하지만, thread 이름이나 현재 task 연결만으로 과거 turn을
그 session에 귀속하지 않습니다. 수집 이전의 사용량, 여러 작업이 재사용한 thread,
외부 실행기에서 제공하지 않은 사용량은 미관측으로 남깁니다.
실제 model 호출을 새로 발생시키거나 기존 예산을 변경하지 않습니다.

## 위임의 명시적 작업 연결

기존 `submit_delegation`에 선택적인 `task_id`를 전달할 수 있습니다. 존재하는 task인지
확인하고 위임 장부와 같은 transaction에서 저장합니다. 같은 멱등 키를 다른 task로
재사용하면 거절하고, 안전한 재시도는 원래 task 연결을 이어받습니다.

```json
{"command":{"type":"submit_delegation","instruction":"테스트 결과를 검토해 주세요.",
"idempotency_key":"review-task-123","task_id":"task-123"}}
```

task_id 없는 기존 호출은 그대로 동작합니다. 현재 위임 backend는 usage counter를
제공하지 않으므로 연결한 실행도 `backend_usage_unavailable`이며 0 비용으로 표시하지 않습니다.

## 검증 범위

도메인 회귀는 중복 누적값, 재시작 후 재처리, 0과 미관측의 차이, counter 역행과 overflow를
검증합니다. SQLite 회귀는 같은 thread의 서로 다른 task 분리 및 history와 독립된 집계를,
core 회귀는 거절된 batch·다른 턴의 숫자를 기록하지 않는 경계를 검증합니다.
실제 모델 실행과 비용 청구 대조는 이 변경의 검증 범위에 포함하지 않습니다.
