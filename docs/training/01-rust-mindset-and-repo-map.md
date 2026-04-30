# 1회차: Rust 사고방식과 저장소 지도

## 세션 목표

- 이 저장소를 `Spring Boot` 애플리케이션처럼 읽지 않고, Rust의 경계 중심 구조로 읽는 법을 익힌다.
- `adapter -> application -> domain` 의존 방향이 실제 파일 배치에서 어떻게 드러나는지 확인한다.
- 첫 품질 게이트로 `cargo test`와 `clippy`를 재측정한다.

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

## 이번 회차 관찰 지점

- 대상 파일:
  - [../../src/adapter/inbound/tui/app/shell_rendering_tests.rs](../../src/adapter/inbound/tui/app/shell_rendering_tests.rs)
  - [../../src/adapter/inbound/tui/app/shell_rendering_contract_tests.rs](../../src/adapter/inbound/tui/app/shell_rendering_contract_tests.rs)
- 현재 증상:
  - 현재 기준선에서는 `cargo test`와 `cargo clippy --all-targets --all-features -- -D warnings`가 통과한다.
- 수업에서 볼 질문:
  - 테스트가 operator journey와 subsystem contract를 얼마나 잘 드러내는가?
  - 테스트 파일 경계가 production 구조를 흐리게 만들고 있지는 않은가?
  - 통과하는 품질 게이트를 다음 리팩터링의 기준선으로 어떻게 사용할 것인가?

## 실습

```bash
. "$HOME/.cargo/env"
cargo clippy --all-targets --all-features -- -D warnings
```

- `cargo test`와 `cargo clippy` 결과를 기준선 문서와 대조한다.
- shell rendering test가 어떤 operator-facing 화면 계약을 보호하는지 설명한다.
- 수정 과제:
  - 오래된 실패 설명이나 사라진 파일 경로가 문서에 남아 있는지 찾기
  - test helper import가 concern별 테스트 파일에 머무르는지 확인하기

## 수강생이 가져가야 할 판단 기준

- Rust 프로젝트는 “문법”보다 “경계와 fan-in”으로 먼저 읽는다.
- 테스트 파일도 구조 부채를 그대로 드러낸다.
- 통과하는 lint와 테스트는 다음 refactor slice를 작게 잡기 위한 기준선이다.
