# 6회차: 구조 경계와 안전한 리팩터링

## 세션 목표

- “Rust 코드가 길다”를 넘어서 “어떤 책임이 어디서 충돌하는가”로 구조 부채를 읽는다.
- 큰 파일을 나누는 기준을 문법이 아니라 경계와 운영 비용으로 설명한다.
- 현재 경계 규칙을 근거로 작은 리팩터링이 안전한지 판정한다.

## Spring Boot/Kotlin 비교

| Kotlin/Spring에서 흔한 문제 | 이 저장소에서 보이는 형태 |
| --- | --- |
| 서비스 클래스 비대화 | `parallel_mode/pool.rs`, `parallel_mode/distributor.rs` 같은 큰 service child module |
| presentation/service/repository 경계 침식 | TUI presentation과 상태 wording이 한 파일에 섞임 |
| 테스트가 구현 파일 구조를 그대로 따라감 | operator journey보다 현재 모듈 경계를 따라가는 테스트 클러스터 |

## 읽기 순서

1. [../design/04-hexagonal-runtime-architecture.md](../design/04-hexagonal-runtime-architecture.md)
2. [../design/05-parallel-control-plane-architecture.md](../design/05-parallel-control-plane-architecture.md)
3. [../design/07-tui-layered-architecture-and-aesthetic-contract.md](../design/07-tui-layered-architecture-and-aesthetic-contract.md)
4. [../../src/application/service/parallel_mode/control_plane](../../src/application/service/parallel_mode/control_plane)
5. [../../src/application/service/parallel_mode/pool.rs](../../src/application/service/parallel_mode/pool.rs)
6. [../../src/application/service/parallel_mode/distributor.rs](../../src/application/service/parallel_mode/distributor.rs)
7. [../../src/adapter/inbound/tui/app/shell_presentation.rs](../../src/adapter/inbound/tui/app/shell_presentation.rs)
8. [../../src/adapter/inbound/tui/app/shell_rendering.rs](../../src/adapter/inbound/tui/app/shell_rendering.rs)

## 이번 회차 관찰 지점

- 구조 관찰:
  - `src/application/service/parallel_mode/mod.rs`는 facade 수준으로 줄었고, 남은 hotspot은
    `pool.rs`, `distributor.rs`, `session_detail.rs` 같은 child module에 있다.
  - readiness, supervisor state, roster projection, selected detail, pool slot state, cleanup
    decision은 `src/domain/parallel_mode.rs`로 내려가 있다.
  - `src/adapter/inbound/tui/app/shell_presentation.rs`와 `shell_rendering.rs`도 presentation, wording, layout, overlay projection이 넓게 섞여 있다.
- boundary 단서:
  - control-plane이 mutation ordering을 소유하고 domain이 policy를 소유하는지 확인한다.
  - service child module이 I/O orchestration과 순수 판단을 다시 섞기 시작하는지 확인한다.
  - domain으로 내려간 projection이 application/TUI copy로 다시 중복되지 않는지 확인한다.
- 수업에서 볼 질문:
  - 어떤 기준으로 `readiness`, `slots`, `distributor`, `recovery`, `snapshot`을 service와 domain에 나눌 수 있는가?
  - 지금 domain에 있는 판단 중 application으로 되돌아가면 어떤 테스트가 길어지는가?
  - presentation에서는 layout과 copy projection을 왜 나눠야 하는가?

## 실습

- 현재 design 문서의 boundary rule을 실제 파일과 대조한다.
- 하나의 refactor 후보를 아래 형식으로 적는다.
  - 바꿀 경계
  - 바꾸지 않을 공개 계약
  - 이동할 책임
  - 같이 옮길 테스트
- 연습:
  - `parallel_mode/pool.rs`에서 domain으로 내려간 판단과 service에 남은 I/O orchestration을 구분한다.
  - `parallel_mode/control_plane`에서 effect ordering과 domain decision 호출의 경계를 설명한다.
  - `parallel_mode/distributor.rs`에서 Git/GitHub delivery side effect와 queue-state projection의 경계를 설명한다.
  - shell presentation에서 copy, view model, rendering 책임이 어떤 파일에 놓였는지 추적한다.

## 수강생이 가져가야 할 판단 기준

- Rust 리팩터링의 핵심은 borrow trick이 아니라 책임 재배치다.
- 큰 파일을 줄이는 목적은 미관이 아니라 review safety와 recovery clarity다.
- 테스트도 operator journey 중심으로 다시 묶어야 다음 수정이 쉬워진다.
