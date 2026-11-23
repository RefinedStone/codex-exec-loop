# 결정 요청의 기한·재알림·기본 응답 (#2557)

## 동작

새로 생성한 결정 요청은 최초 저장 트랜잭션에서 응답 기한을 고정한다.
현재 정책의 `timeouts.start_seconds`를 요청의 공통 표시 기한으로 사용하며 기본값은 24시간이다.
`timeouts.remind_before_seconds`의 기본값은 2시간이고, 0이면 재알림을 만들지 않는다.
동일 ID를 반복 생성하거나 커널을 재조립해도 최초 기한은 연장되지 않는다.
기한이 없던 기존 요청은 생성 시각을 추측해서 변경하지 않는다.

| 요청과 현재 관문 | 기한 도달 후 동작 |
| --- | --- |
| 명시적으로 선택한 Gated의 Start `timeout_default` | 현재 기준을 다시 확인하고 권고 선택을 `by: default`로 한 번 확정 |
| Propose Start, Human 관문 | 열린 요청을 유지하고 사람 답변 대기 |
| 작업 중 질문·실패 복구·외부 이슈 닫기 | 표시 기한과 재알림만 적용하며 사람 답변 대기 |
| 기준이 달라진 요청 | `superseded`로 종료; 이전 권고로 시작하지 않음 |
| 먼저 답변이 확정된 요청 | 저장된 첫 답변을 유지 |

Default 답변을 확정할 때 모델을 호출하지 않는다. 그 뒤의 작업 시작은 별도 단계이며
기존 원격 재확인·예산 admission·실행 permit 경계를 그대로 거친다.
Human, Policy, TimeoutDefault 시작은 같은 동시 실행·일일 예산을 공유한다.
명시적인 Gated 선택이나 유효한 실행 상태가 없으면 Default로 새 작업을 시작하지 않는다.
일반 Agent 답변이나 일반 저장 API로 Default를 대신 기록할 수 없다.

## 시간과 재시작

커널의 로컬 추적 worker가 전달받은 UTC 시각으로 기한과 재알림을 평가한다.
worker가 반환한 다음 예정 시각을 호스트가 캐시하고, 기존 5초 추적 주기와 비교해 더 이른
시각에 깨어난다. 새 요청이 생기면 즉시 이 일정을 다시 준비하므로 4초 기한·2초 전 알림처럼
짧은 유효 설정도 재알림 구간을 놓치지 않는다.
호스트의 `next_deadline` 조회에서 DB I/O나 모델 호출을 추가하지 않는다.
기한을 지난 Wait 요청 때문에 즉시 다시 깨어나는 반복도 만들지 않는다.
처리는 커널이 동작하는 동안 이루어지며, 다시 조립하면 저장된 기한부터 재평가한다.

재알림은 `remind_at <= now < due_at` 구간에 한 번 확정한다.
기한이 지난 뒤 재시작한 경우에는 늦은 재알림을 만들지 않고 기한 처리를 먼저 한다.
사람 답변과 Default가 경합하면 SQLite의 같은 판 비교와 트랜잭션이 첫 확정 하나를 보존한다.
Default 저장 후 작업 등록 전에 중단되어도 다음 추적에서 기존 결정과 같은 work order를 사용한다.
이미 Default로 시작한 작업은 현재 관문·기준이 달라졌을 때 정상 조정 경로에서 허가를 회수한다.

## 재알림 기록

SQLite schema 11의 `issue_decision_reminders`는 일정·최초 처리 영수증·미전달 이벤트를 보관한다.
결정당 unique 제약과 안정적인 `issue-decision-reminder:` event ID를 사용한다.
커널 이벤트 로그에 append한 뒤 ACK하며, append 뒤 ACK 전에 재시작해도 같은 ID로 중복을 막는다.
ACK는 영수증을 지우지 않는다. 기한 조회와 미전달 조회는 최대 256개씩 읽는다.

재알림은 catalog/event revision만 올린다. 결정 자체의 revision, Human 확인 대상,
작성 중인 선택·메모는 유지한다. 기록은 ID·기준·기한만 담고 질문이나 답변 본문을 복사하지 않는다.
Desktop은 `issue_decision_reminded`로 재조회와 고정 문구의 알림을 요청하며,
중복 event ID와 이미 종료된 결정의 늦은 알림을 적용하지 않는다.

## Desktop 조회 기준

Issue query의 optional `as_of_unix_ms`는 서버가 응답을 만든 UTC 시각이다.
기한과 남은 시간은 이 조회 시각을 기준으로 표시하며 클라이언트 시계로 결정을 진행하지 않는다.
조회 시각이 없는 이전 응답은 기한만 표시한다.
기한이 지난 Wait 요청은 사람 답변 대기 상태를 유지한다.
Default 요청에는 서버가 제공한 권고 선택과 기한 후 동작을 표시한다.
새 조회 시각이나 재알림은 결정 draft identity에 포함되지 않는다.

## 회귀 검증

- Domain: 기한 직전·정확한 기한·재평가, Human 유지, 권고 검증, stale basis,
  Default의 출처·시각 검증, 미확인 Agent의 선점 거절.
- SQLite: 동일 ID의 서로 다른 생성 시각 경합, 최초 기한 보존, 사람 답변과 기한 경합,
  재알림 경계·ACK·재시작·paging, 10→11 이관, 세 시작 권한의 공통 예산.
- Application: 실제 coordinator tracking에서 재알림·Default·작업 등록, 재조립 후 중복 0,
  예산 대기, current policy와 Off 전환에 따른 허가 회수.
- Kernel simulation: 가짜 Instant와 UTC를 주입한 짧은 기한의 실제 host scheduling, durable event 기록,
  append 후 ACK 유실과 재조립. production runtime의 실제 UTC 수집을 흉내 내는 시험이다.
- Desktop: 서버와 장치 시계 불일치, 기한 경과 Wait, Default 안내, legacy 응답,
  중복·늦은 재알림, 선택·메모·개입 초안 보존, 추가 command/native dialog 0회.

검증 실행 결과와 실제 head는 PR의 검증 기록에 남긴다.
이 회귀는 실제 모델이나 GitHub 외부 변경을 호출하지 않는다.
