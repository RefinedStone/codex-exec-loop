# 실행기 지원 기능의 기준선 (A12)

`InteractiveTurnRuntimePort::runtime_capabilities()`와
`ConversationService::runtime_capabilities()`는 연결이나 모델 실행 없이 현재 adapter의
지원 선언을 반환한다. 계약 버전은 1이다. 이는 구현 기준선이며 현재 계정·정책·소유권의
승인 결과가 아니다. 실행 직전의 기존 admission과 오류 처리는 그대로 유지한다.

## Codex adapter

| 기능 | 지원 범위 | 해석할 때 지킬 경계 |
| --- | --- | --- |
| start | 검증한 workspace의 새 thread와 turn | 적용된 설정·정책과 서버 admission을 확인한다. |
| resume | 같은 host/runtime의 기존 thread | 조회·resume의 thread ID와 실제 적용 workspace를 검사한다. |
| interrupt | 이 프로세스의 stream이 소유한 thread/turn | 요청 접수는 중단 완료가 아니다. 먼저 도착한 완료 receipt가 최종 결과일 수 있다. |
| steer | expected turn ID가 일치하는 현재 main/worker stream | 오래된 binding·종료한 턴·시간 초과는 거절한다. |
| approval | 해당 stream의 미결 commandExecution/permissions 요청 | fileChange와 legacy 승인은 거절한다. 만료·소비·취소된 승인을 재사용하지 않는다. |
| usage | 바인딩된 thread/turn에서 관측한 토큰 통지 | 누락은 미관측이며 중복 가능하다. 금액이나 작업 전체 합계로 해석하지 않는다. |

정적 `protocol_baseline`은 저장된 v2 schema의 `codex-cli 0.144.0`이다.
`observed_runtime_version`은 이 조회에서 서버를 검사하지 않으므로 `null`이다.
서버가 같은 버전이거나 schema의 모든 기능을 지원한다고 추론하지 않는다. 서버 교체·재연결·
정책 변경 뒤에는 실제 연결의 버전과 적용 정책을 다시 확인해야 한다.

`runtime_namespace`는 엔진 계열이며 host 주소나 계정의 식별자가 아니다. 저장하거나 요청을
대조할 때는 기존 host identity와 함께 `(host, runtime_namespace, thread_id)`로 다룬다.
이 PR은 저장소의 기존 thread 식별자를 일괄 변경하지 않는다.

## 다른 adapter와 하위 호환

지원은 `supported`(scope/limitations), 미지원은 `unsupported`(reason), 선언 부재는
`not_declared`(reason)로 구분한다. 새 메서드의 기본값은 모든 기능이 `not_declared`다.
기존 fake나 adapter가 optional 메서드를 재정의했는지는 trait가 알 수 없으므로 선언 부재를
미지원이나 지원으로 추정하지 않는다. 기본 interrupt/steer/approval은 기존의 명시적인
unsupported 오류를 계속 반환한다. 새 adapter는 실제 동작과 일치하는 선언을 제공해야 한다.

## 검증 범위

`cargo test --locked --lib runtime_capabilities`는 정적 Codex 선언과 생성 schema 기준선,
non-Codex fake의 선언 부재와 optional 요청 거절, 서비스 위임 및 DTO round-trip을 확인한다.
모델을 호출하지 않는다. 생성 schema에 필드가 있다는 것만으로 실행기 적합성을 입증하지 않는다.

실제 제어 의미는 기존 `connection.rs` 회귀가 소유한다. 예를 들어
`terminal_before_steer_response_preserves_authoritative_terminal_and_fails_caller`,
`interrupt_racing_with_received_accept_is_rechecked_before_response`,
`acknowledged_interrupt_without_terminal_event_fails_closed`가 완료 경합과 승인 수명을 검사한다.
공통 stream/receipt 모델은 그대로 사용하며 이 테스트들을 새 프레임워크로 복제하지 않는다.

이번 범위는 #2481 A12-1과 최소 선언 계약 검증이다. 전체 non-Codex 실행 적합성 corpus,
실제 두 번째 엔진, 인증·운영 방식 및 #2569의 crate 분리는 별도 조건부 후속이다.
