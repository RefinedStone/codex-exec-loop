# Spring Boot/Kotlin 개발자를 위한 Rust 교본

이 문서는 `codex-exec-loop` 저장소를 실제 교본으로 사용해 Rust를 설명하는 6회 압축 커리큘럼의 인덱스다.
강의 목표는 Rust 문법 소개가 아니라, 기존 서버 개발 감각을 유지한 채 이 저장소를 읽고 고칠 수 있는 수준까지 끌어올리는 것이다.

## 대상

- Spring Boot/Kotlin 서버 개발 경험은 충분하지만 Rust 실무 경험은 거의 없는 개발자
- 계층 구조, 예외 처리, 직렬화 계약, 테스트 문화는 익숙하지만 ownership/borrowing은 낯선 개발자
- 실제 코드베이스를 읽으면서 언어와 설계를 같이 익히고 싶은 개발자

## 운영 원칙

- 매 회차는 반드시 `Spring Boot/Kotlin` 대응 개념부터 시작한다.
- 매 회차는 실제 저장소 파일을 읽는다.
- 매 회차는 현재 브랜치의 품질 문제 하나를 분석하고 수정 과제 또는 수정 후보로 연결한다.
- 설명보다 증거를 우선한다. 명령, 테스트 이름, 파일 경로를 근거로 삼는다.

## 현재 기준선

- 기준선 문서: [quality-baseline.md](./quality-baseline.md)
- `2026-04-30` 기준 `cargo test`와 `cargo clippy --all-targets --all-features -- -D warnings`는 통과한다.
- 따라서 이 강의는 과거 실패 재현보다 현재 경계가 왜 안정화됐는지, 그리고 다음 리팩터링을 어떻게 작게 잡을지에 맞춘다.

## 강의 흐름

| 회차 | 주제 | 저장소 진입점 | 이번 회차 관찰 지점 |
| --- | --- | --- | --- |
| 1 | Rust 사고방식과 저장소 지도 | [README.md](../../README.md), [docs/design/04-hexagonal-runtime-architecture.md](../design/04-hexagonal-runtime-architecture.md), [src/lib.rs](../../src/lib.rs) | 계층과 테스트 게이트를 함께 읽는 법 |
| 2 | 타입 모델링과 직렬화 계약 | [src/domain/planning](../../src/domain/planning), [src/domain/parallel_mode.rs](../../src/domain/parallel_mode.rs) | planning/parallel domain projection이 허용 상태를 닫는 방식 |
| 3 | 에러 처리와 포트 경계 | [src/application/service/planning/worker/orchestration.rs](../../src/application/service/planning/worker/orchestration.rs), [src/application/service/parallel_mode/distributor.rs](../../src/application/service/parallel_mode/distributor.rs), [src/adapter/outbound/github/automation.rs](../../src/adapter/outbound/github/automation.rs) | 실패 경계와 포트 책임 |
| 4 | 상태 머신과 이벤트 기반 런타임 | [src/adapter/inbound/tui/app/conversation_runtime.rs](../../src/adapter/inbound/tui/app/conversation_runtime.rs), [src/adapter/inbound/tui/app/shell_runtime.rs](../../src/adapter/inbound/tui/app/shell_runtime.rs) | 통과하는 상태 전이 테스트를 근거로 런타임 읽기 |
| 5 | 파일시스템, Git, SQLite를 명시적으로 다루기 | [src/application/service/parallel_mode/pool.rs](../../src/application/service/parallel_mode/pool.rs), [src/adapter/outbound/db/sqlite_planning_authority_adapter.rs](../../src/adapter/outbound/db/sqlite_planning_authority_adapter.rs) | authority store, worktree, projection의 진실 소스 분리 |
| 6 | 구조 부채 지도와 안전한 리팩터링 | [docs/plan/17-structure-and-architecture-debt-map.md](../plan/17-structure-and-architecture-debt-map.md), [src/application/service/parallel_mode/pool.rs](../../src/application/service/parallel_mode/pool.rs), [src/adapter/inbound/tui/app/shell_presentation.rs](../../src/adapter/inbound/tui/app/shell_presentation.rs) | 남은 mixed-responsibility hotspot 분해 계획 |

## 회차별 교안

1. [01-rust-mindset-and-repo-map.md](./01-rust-mindset-and-repo-map.md)
2. [02-type-modeling-and-serialization.md](./02-type-modeling-and-serialization.md)
3. [03-error-handling-and-port-boundaries.md](./03-error-handling-and-port-boundaries.md)
4. [04-state-machines-and-event-runtime.md](./04-state-machines-and-event-runtime.md)
5. [05-filesystem-git-sqlite-boundaries.md](./05-filesystem-git-sqlite-boundaries.md)
6. [06-debt-map-and-safe-refactoring.md](./06-debt-map-and-safe-refactoring.md)

## 공통 준비 명령

```bash
. "$HOME/.cargo/env"
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

## 기대 결과

- 저장소를 “Rust라서 어렵다”가 아니라 “경계와 상태가 복잡하다”로 읽게 된다.
- Kotlin에서 익숙한 설계 감각을 Rust 타입, 포트, 상태 전이 모델로 번역할 수 있게 된다.
- 강의가 끝나면 최소 한 개 이상의 실제 품질 문제를 독립적으로 재현하고 수정 방향을 제안할 수 있어야 한다.
