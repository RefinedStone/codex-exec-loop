# 1회차: Rust 사고방식과 저장소 지도

## 세션 목표

- 이 저장소를 `Spring Boot` 애플리케이션처럼 읽지 않고, Rust의 경계 중심 구조로 읽는 법을 익힌다.
- `adapter -> application -> domain` 의존 방향이 실제 파일 배치에서 어떻게 드러나는지 확인한다.
- 첫 품질 게이트로 `clippy` 실패를 재현한다.

## Spring Boot/Kotlin 비교

| 익숙한 개념 | 이 저장소에서 대응되는 개념 |
| --- | --- |
| `Controller` | inbound adapter, 특히 [src/adapter/inbound/tui](../../src/adapter/inbound/tui) |
| `Service` | [src/application/service](../../src/application/service) |
| `Repository` interface | [src/application/port/outbound](../../src/application/port/outbound) |
| `JpaRepository` 구현체 | [src/adapter/outbound](../../src/adapter/outbound) |
| `data class` 기반 DTO | [src/domain](../../src/domain)와 일부 application snapshot 타입 |

## 읽기 순서

1. [../../README.md](../../README.md)
2. [../design/04-hexagonal-runtime-architecture.md](../design/04-hexagonal-runtime-architecture.md)
3. [../../src/lib.rs](../../src/lib.rs)
4. [../../src/adapter/inbound/cli.rs](../../src/adapter/inbound/cli.rs)
5. [../../src/adapter/inbound/tui/app.rs](../../src/adapter/inbound/tui/app.rs)

## 강의 흐름

1. `main -> lib -> inbound adapter` 흐름을 따라 실행 진입점을 설명한다.
2. `domain`이 왜 UI 타입과 파일시스템 타입을 몰라야 하는지 설명한다.
3. Kotlin에서 흔한 “service에서 다 해결” 습관이 Rust에서 왜 더 빨리 부채가 되는지 연결한다.
4. 문서가 말하는 구조와 실제 파일 크기, 테스트 배치를 대조한다.

## 이번 회차 이슈

- 대상 파일: [../../src/adapter/inbound/tui/app/app_tests.rs](../../src/adapter/inbound/tui/app/app_tests.rs)
- 현재 증상:
  - `cargo clippy --all-targets --all-features -- -D warnings`가 unused import로 실패한다.
- 수업에서 볼 질문:
  - 왜 테스트 허브 파일 하나에 너무 많은 import가 모였는가?
  - 테스트를 묶는 방식이 production 구조를 흐리고 있는가?
  - unused import를 지우는 것만으로 충분한가, 아니면 테스트 파일 경계도 다시 봐야 하는가?

## 실습

```bash
. "$HOME/.cargo/env"
cargo clippy --all-targets --all-features -- -D warnings
```

- 실패 메시지에서 `app_tests.rs` 관련 import만 먼저 추린다.
- 어떤 심볼이 실제로 쓰이는지와 왜 남았는지 설명한다.
- 수정 과제:
  - unused import 제거
  - test helper import를 concern별 테스트 파일로 이동

## 수강생이 가져가야 할 판단 기준

- Rust 프로젝트는 “문법”보다 “경계와 fan-in”으로 먼저 읽는다.
- 테스트 파일도 구조 부채를 그대로 드러낸다.
- 작은 lint 실패는 큰 hotspot을 찾는 출발점이 된다.
