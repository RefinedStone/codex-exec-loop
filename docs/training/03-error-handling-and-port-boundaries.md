# 3회차: 에러 처리와 포트 경계

## 세션 목표

- Spring의 예외 전파와 Rust의 `Result` 중심 실패 모델 차이를 실제 코드에서 본다.
- 포트와 어댑터 경계가 없으면 작은 helper도 왜 설계 문제로 커지는지 이해한다.
- boundary hygiene 관점에서 현재 통과하는 코드를 읽고 다음 수정 우선순위를 정한다.

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

## 이번 회차 관찰 지점

- 대상 경계 묶음:
  - [GithubAutomationAdapter](../../src/adapter/outbound/github/automation.rs)의 process execution boundary
  - [parallel_mode/distributor.rs](../../src/application/service/parallel_mode/distributor.rs)의 GitHub/merge/delivery orchestration
  - [planning/worker/orchestration.rs](../../src/application/service/planning/worker/orchestration.rs)의 worker prompt and result boundary
- 수업에서 볼 질문:
  - adapter helper가 application service에 너무 많은 infrastructure detail을 노출하고 있지는 않은가?
  - 생성 API는 객체 생성 의도를 분명히 드러내는가?
  - `Result` 포장과 해제가 반복될 때 실패 경계가 오히려 흐려지지 않는가?

## 실습

```bash
. "$HOME/.cargo/env"
cargo clippy --all-targets --all-features -- -D warnings
```

- 위 세 파일에서 I/O 호출, domain 판단, prompt/copy assembly를 구분한다.
- 각 helper를 “adapter detail”, “application orchestration”, “domain candidate”로 분류한다.
- 수정 과제:
  - 생성 API가 실제 사용성을 높이는지 검토
  - helper 함수 시그니처가 호출자에게 infrastructure detail을 새게 하는지 재검토
  - 실패 메시지가 port boundary를 기준으로 설명되는지 정리

## 수강생이 가져가야 할 판단 기준

- Rust에서 에러 처리는 예외 처리 대체재가 아니라 API 설계의 일부다.
- 포트 경계가 잘 서 있으면 lint 수정도 안전하고 작아진다.
- clippy와 테스트를 “자동 정리기”가 아니라 boundary review 기준선으로 사용해야 한다.
