# 3회차: 에러 처리와 포트 경계

## 세션 목표

- Spring의 예외 전파와 Rust의 `Result` 중심 실패 모델 차이를 실제 코드에서 본다.
- 포트와 어댑터 경계가 없으면 작은 lint도 왜 설계 문제로 커지는지 이해한다.
- boundary hygiene 성격의 clippy 경고를 읽고 수정 우선순위를 정한다.

## Spring Boot/Kotlin 비교

| Kotlin/Spring 습관 | Rust에서 보게 되는 모습 |
| --- | --- |
| unchecked exception 전파 | `Result<T, E>`로 실패를 명시적으로 반환 |
| 구현체가 서비스에 바로 주입됨 | application layer가 port를 소유하고 adapter가 구현 |
| `@Transactional`로 범위 암묵화 | 함수 경계와 반환 타입으로 실패 범위를 드러냄 |
| convenience method 축적 | helper 함수가 API 냄새를 빠르게 드러냄 |

## 읽기 순서

1. [../../src/application/port/outbound/github_automation_port.rs](../../src/application/port/outbound/github_automation_port.rs)
2. [../../src/adapter/outbound/github/automation.rs](../../src/adapter/outbound/github/automation.rs)
3. [../../src/application/service/parallel_mode/distributor.rs](../../src/application/service/parallel_mode/distributor.rs)
4. [../../src/application/service/planning/worker/orchestration.rs](../../src/application/service/planning/worker/orchestration.rs)

## 이번 회차 이슈

- 대상 이슈 묶음:
  - [GithubAutomationAdapter::new](../../src/adapter/outbound/github/automation.rs)의 `new_without_default`
  - [parallel_mode/distributor.rs](../../src/application/service/parallel_mode/distributor.rs)의 `needless_question_mark`, `iter_overeager_cloned`
  - [planning/worker/orchestration.rs](../../src/application/service/planning/worker/orchestration.rs)의 `needless_borrow`
- 수업에서 볼 질문:
  - lint가 지적하는 불필요한 borrow와 clone은 단순 문법 문제인가, 아니면 API가 호출자에게 불필요한 부담을 준 결과인가?
  - `new()`와 `Default` 중 무엇을 지원해야 객체 생성 의도가 더 분명한가?
  - `Result` 포장과 해제가 반복될 때 실패 경계가 오히려 흐려지지 않는가?

## 실습

```bash
. "$HOME/.cargo/env"
cargo clippy --all-targets --all-features -- -D warnings
```

- 위 세 파일의 clippy 메시지만 따로 정리한다.
- 각 메시지를 “즉시 수정 가능”, “API 재설계 필요”, “보류 가능”으로 분류한다.
- 수정 과제:
  - `Default` 구현 추가가 실제 사용성을 높이는지 검토
  - clone과 borrow가 사라지도록 helper 함수 시그니처를 재검토
  - `Ok(...?)` 형태를 실패 경계가 더 분명한 형태로 정리

## 수강생이 가져가야 할 판단 기준

- Rust에서 에러 처리는 예외 처리 대체재가 아니라 API 설계의 일부다.
- 포트 경계가 잘 서 있으면 lint 수정도 안전하고 작아진다.
- clippy를 “자동 정리기”가 아니라 boundary review 도구로 사용해야 한다.
