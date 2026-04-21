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

1. [../../src/domain/planning.rs](../../src/domain/planning.rs)
2. [../../src/domain/parallel_mode.rs](../../src/domain/parallel_mode.rs)
3. [../../src/application/service/planning/runtime/validation.rs](../../src/application/service/planning/runtime/validation.rs)

## 강의 흐름

1. planning 도메인에서 `enum`과 `serde`가 계약을 어떻게 닫는지 읽는다.
2. `Option`, default, validation 책임을 어디에 둘지 설명한다.
3. 타입이 풍부해질수록 함수 인자 개수가 왜 줄어야 하는지 설명한다.

## 이번 회차 이슈

- 대상 심볼: [ParallelModeSlotLeaseSnapshot::new](../../src/domain/parallel_mode.rs)
- 현재 증상:
  - `clippy::too_many_arguments`가 걸린다.
- 수업에서 볼 질문:
  - 생성자 인자가 많다는 것은 호출자가 너무 많은 내부 결정을 알고 있다는 뜻인가?
  - `slot_id`, `task_id`, `task_title`, `agent_id`, `branch_name`, `worktree_path`, `state`, `leased_at`, `running_started_at`를 한 번에 받는 이유가 정당한가?
  - `LeaseMetadata`, `LeaseLifecycle`, `NewLeaseSnapshot` 같은 중간 입력 모델이 더 읽기 쉬운가?

## 실습

- `ParallelModeSlotLeaseSnapshot` 호출부를 찾는다.
- 호출부를 “필수 입력”과 “파생 값”으로 나눈다.
- 수정 과제:
  - 고인자 생성자를 입력 struct 기반 팩토리로 바꾸는 설계안 작성
  - 테스트 fixture 생성 시 named-field struct literal을 쓸지 factory를 쓸지 비교

## 수강생이 가져가야 할 판단 기준

- Rust 타입 설계는 DTO 나열이 아니라 “어떤 상태를 허용하지 않을지”를 정하는 작업이다.
- clippy의 `too_many_arguments`는 스타일 지적이 아니라 모델 경계가 흐리다는 신호일 수 있다.
- 직렬화 계약과 런타임 validation은 경쟁 관계가 아니라 상호 보완 관계다.
