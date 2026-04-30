# 2회차: 타입 모델링과 직렬화 계약

## 세션 목표

- Kotlin `data class`와 Rust `struct`/`enum`이 다르게 강제하는 지점을 이해한다.
- `serde` 기반 직렬화 계약과 상태 enum이 설계 안정성에 어떤 역할을 하는지 배운다.
- 고인자 생성자가 왜 모델 경계 냄새인지 설명할 수 있게 한다.

## Spring Boot/Kotlin 비교

| Kotlin/Spring 습관 | Rust에서 대체되는 방식 |
| --- | --- |
| named argument로 긴 생성자 완화 | 입력 전용 struct, builder, factory 메서드 분리 |
| Jackson annotation으로 계약 제어 | `serde` derive와 enum/field 기본값 설계 |
| nullable 필드 남발 | `Option<T>`를 의도적으로 노출 |
| sealed class로 상태 모델링 | Rust `enum`으로 폐쇄형 상태 표현 |

## 읽기 순서

1. [../../src/domain/planning](../../src/domain/planning)
2. [../../src/domain/parallel_mode.rs](../../src/domain/parallel_mode.rs)
3. [../../src/application/service/planning/runtime/validation.rs](../../src/application/service/planning/runtime/validation.rs)

## 강의 흐름

1. planning 도메인에서 `enum`, `serde`, semantic validation이 계약을 어떻게 닫는지 읽는다.
2. `PriorityQueueProjection`이 queue/proposal summary를 domain fact로 제공하는 이유를 설명한다.
3. parallel mode domain에서 readiness, roster, selected detail, cleanup decision이 service 밖으로 빠진 효과를 읽는다.
4. `Option`, default, validation 책임을 어디에 둘지 설명한다.
5. 타입이 풍부해질수록 함수 인자 개수가 왜 줄어야 하는지 설명한다.

## 이번 회차 이슈

- 대상 심볼:
  - [PriorityQueueProjection](../../src/domain/planning/mod.rs)
  - [ParallelModeAgentRosterSnapshot::project_from_leases](../../src/domain/parallel_mode.rs)
  - [ParallelModePoolSlotCleanupDecision](../../src/domain/parallel_mode.rs)
- 현재 증상:
  - planning과 parallel mode의 순수 projection이 application service에서 domain으로 내려와 있다.
- 수업에서 볼 질문:
  - queue summary와 proposal summary는 UI copy인가, domain fact인가?
  - lease state에서 pool slot state와 cleanup 가능 여부를 정하는 책임은 왜 domain에 있는가?
  - roster 정렬과 selected-detail 선택은 service orchestration인가, 순수 projection인가?

## 실습

- `PriorityQueueProjection::queue_summary`와 `proposal_summary` 호출부를 찾는다.
- `ParallelModeAgentRosterSnapshot::project_from_leases` 호출부를 찾는다.
- 호출부를 “I/O가 필요한 입력 수집”과 “순수 파생 값”으로 나눈다.
- 수정 과제:
  - application service에 남은 projection 후보 하나를 찾고 domain으로 옮겨도 되는지 판정한다.
  - 테스트 fixture 생성 시 domain factory를 쓸지 named-field struct literal을 쓸지 비교한다.

## 수강생이 가져가야 할 판단 기준

- Rust 타입 설계는 DTO 나열이 아니라 “어떤 상태를 허용하지 않을지”를 정하는 작업이다.
- 순수 projection은 application service보다 domain에 있을 때 테스트와 문서가 더 짧아진다.
- 직렬화 계약과 런타임 validation은 경쟁 관계가 아니라 상호 보완 관계다.
