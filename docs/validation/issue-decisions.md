# 이슈 결정 요청과 답변 검증

대상: #2544 · plan 28 §10.1. 이 문서는 현재 구현과 검증 경계를 설명한다.

## 결정 객체

결정 요청은 종류를 가진 대상을 보관하며 첫 대상은 이슈다. 관문, 닫힌 질문 종류,
공유 질문, 선택지의 ID·이름·효과 설명, 권고, timeout 정책과 마감 시각을 가진다.
기준은 자동화 정책 판, 이슈 필드 해시, 선택적인 작업 지시 ID, 관문 판이다.

질문 종류는 시작 승인, 기준 확정, 계획 승인, 위험 예외, 이슈 변경,
실패 뒤 처리, 에이전트 질문으로 고정한다. 상태는 open, answered, expired,
withdrawn, superseded다. 이번 단계는 마감 타이머나 중재자를 실행하지 않는다.

불변 정의 전체가 같으면 새 요청 ID를 주더라도 기존 요청을 재사용한다.
질문·선택지·권고 등이 다른 요청은 같은 기준에서도 독립적이다.
호출자는 반복 생성 시 같은 due_at과 근거 참조를 유지해야 한다.

## 생성과 응답 경계

생성은 내부 `IssueDecisionService::ensure_request` API를 제공한다.
현재 순수 평가와 같은 영속 triage가 있어야 새 요청을 저장할 수 있다.
답변 dispatch는 `changed`가 참인 commit만 런타임 내부의 private 기록 함수에 전달한다.
요청 생성 기록·이벤트는 실제 저장 결과를 전달하는 테스트 전용 fixture로 검증한다.
자동 수집의 요청 생성 hook과 새 질문 명령은 추가하지 않았다. propose 자동 질문은
#2548에서 기존 커널의 typed 실행 경로에 연결하며, 별도 공개 변경 진입점을 만들지 않는다.

외부 명령은 다음 형태다.

```json
{
  "command": {
    "type": "answer_issue_decision",
    "decision_id": "decision-1",
    "option": "continue",
    "note": "선택의 이유"
  },
  "idempotency_key": "a-unique-answer-key"
}
```

`by`, `confirmation`, `caller`, `channel`, 현재 기준은 명령 입력으로 받지 않는다.
서버가 경로와 시각을 정하고, attach 때 받은 caller는 표시 정보로만 쓴다.
이 단계의 답은 항상 `by: agent`, `confirmation: null`이다.
Desktop이라는 이름도 사람 확인 증명이 아니다. 실제 사람 증명은 #2545에서 다룬다.

답변 직전에 현재 관찰과 현재 정책을 기존 순수 triage 함수로 다시 평가한다.
본문 변경 뒤 아직 새 triage가 저장되지 않았거나, 라벨·담당자·PR 근거·정책이
변경된 경우도 기준 비교에 반영한다. 기준이 다르면 답을 저장하지 않고
`superseded`로 닫으며 `decision_superseded` 이유를 반환한다.
현재 정책을 읽을 수 없으면 오류를 반환하고 열린 요청을 보존한다.

SQLite는 관찰·tracked·triage·결정 행의 revision과 수집 epoch를 같은 쓰기
transaction에서 확인한다. 첫 terminal 결과 이후 writer는 기존 행을 돌려받고
추가 catalog 증가나 resolved 이벤트를 만들지 않는다. 이미 닫힌 요청의 반환은
새 정책 읽기나 둘째 답의 메모 처리에 의존하지 않는다.

같은 멱등 키의 재전송은 기존 CommandMemory의 영수증을 재사용한다.
같은 키의 다른 입력은 거부한다. 별도 멱등 장부는 만들지 않았다.
TUI가 저장소 잠금을 쥔 경우 command-only 경로의 기록 없는 직접 실행 예외를
결정 답에 적용하지 않는다.

## 조회·기록·실시간 이벤트

`issue` 상세의 `open_decisions`는 이슈와 같은 SQLite 읽기 snapshot에서 얻는다.
열린 질문과 선택지는 공유 정책을 지난 값만 보여 준다. 닫힌 답·메모·caller·
confirmation·요청자의 근거 원문은 이 조회에 포함하지 않는다.

`issue_decision_requested`와 `issue_decision_resolved` 기록에는 안정 식별자,
기준, 상태, 답변 주체와 선택지 ID만 있다. 질문과 메모를 복사하지 않는다.
커널 호스트는 같은 commit fact와 catalog revision에서 `issue_decision`
실시간 이벤트를 만들며 기존 `recorded` 이벤트도 보존한다.
동시에 후속 변경이 생기면 조회의 revision은 해당 이벤트보다 높을 수 있다.

서비스는 기존 secret-span 탐지와 AiHandoff 마스킹 경계를 재사용한다.
결정 답의 가리기 전 입력은 Gateway 세션 녹화에서 제외한다.
해당 명령의 decoder 오류는 고정 문구로 반환하여 잘못된 필드명이나 값도
응답·녹화로 복사하지 않는다. 커널 명령 기록에는 입력 지문만 남는다.

## 저장과 호환성

장부 스키마는 4판이다. 정확한 기존 1·2·3판 스키마를 확인한 뒤 결정 요청
테이블을 추가한다. 기존 바인딩·관찰·작업 상태·커서·catalog 판을 보존한다.
저장된 JSON은 복원 시 상태 전이 규칙을 검사하고 SQL 좌표·상태·중복 키와 대조한다.
현재 판에서 위조된 human/default/arbiter 답을 복원하는 우회도 거부한다.

이 구현은 작업, 실행 허가, 모델 턴, provider 쓰기를 만들지 않는다.
관문의 통과나 자동 실행은 후속 이슈의 책임이다.
커널 감사 기록과 SQLite 장부 사이의 강제 종료를 가로지르는 durable outbox는
이번 변경의 보장에 포함하지 않는다. 재시작 상태의 기준은 SQLite 장부다.

## 검증

회귀는 실제 SQLite 연결과 실제 커널·저장 명령 서비스를 사용한다.
외부 이슈와 정책은 통제 가능한 대역이며 커널 시뮬레이션의 주기는 가짜 시계를 쓴다.
답변의 서버 기록 시각은 형식만 검사하고 고정 벽시계 값에 기대지 않는다.

- 두 SQLite 연결의 동시 답, 불변 정의 중복, 다른 질문의 독립성, 재시작과 migration.
- 본문·메타데이터·정책 변경, 정책 실패, 첫 답 뒤 잘못된 후속 입력.
- kernel의 동일 영수증 재전송과 다른 payload 거부, 실시간 이벤트 횟수·판 일치.
- command-only 재시작 영수증, TUI 점유 거절, planning 상태 보존.
- 실제 Gateway WebSocket과 녹화 파일의 정상·거절·decoder 오류 원문 비노출.

실행 결과는 PR의 최종 검증 기록에 남긴다.

### Windows에서 별도 확인한 기존 제한

넓은 `decision` 이름 필터는 기존
`queue_refresh_resolved_repair_uses_repaired_projection_for_final_decision` 시험도 포함한다.
이 시험은 Windows에서 지원하지 않는 direct planning filesystem fixture를 사용하여
`Idle / RepairSucceeded` 차이로 실패했다. #2543 최종 검증에서 보존한 실행 파일에서도
같은 시험을 단독 실행해 동일 실패를 확인했다. 해당 제품 경로와 fixture는 이번 변경에서
수정하지 않았다. Linux에서는 같은 시험이 통과했다. 이번 변경의 직접 회귀와
필수 Windows Full 결과는 이 추가 필터 결과와 구분해 보고한다.
