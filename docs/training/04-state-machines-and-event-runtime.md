# 4회차: 상태 머신과 이벤트 기반 런타임

## 세션 목표

- 요청-응답형 서버 사고에서 벗어나, 이벤트 기반 TUI 런타임을 상태 전이로 읽는 법을 배운다.
- “왜 이 상태가 남아 있었는가”를 테스트 이름과 reducer-style 흐름으로 추적한다.
- 실패 테스트 하나를 끝까지 따라가며 원인 후보를 세운다.

## Spring Boot/Kotlin 비교

| 서버 개발에서 익숙한 흐름 | 이 저장소에서 대응되는 흐름 |
| --- | --- |
| HTTP request lifecycle | turn lifecycle, background message, shell redraw |
| controller state는 짧게 생존 | TUI state는 세션 내내 유지됨 |
| 실패 응답 한 번으로 종료 | 상태 누수는 다음 이벤트까지 이어질 수 있음 |
| integration test는 요청/응답 검증 | runtime test는 이벤트 순서와 최종 상태를 같이 검증 |

## 읽기 순서

1. [../../src/adapter/inbound/tui/app/conversation_runtime.rs](../../src/adapter/inbound/tui/app/conversation_runtime.rs)
2. [../../src/adapter/inbound/tui/app/shell_runtime.rs](../../src/adapter/inbound/tui/app/shell_runtime.rs)
3. [../../src/adapter/inbound/tui/app/app_tests/shell_surface_tests.rs](../../src/adapter/inbound/tui/app/app_tests/shell_surface_tests.rs)
4. [../../src/adapter/inbound/tui/app/app_tests/parallel_mode_runtime_tests.rs](../../src/adapter/inbound/tui/app/app_tests/parallel_mode_runtime_tests.rs)

## 이번 회차 이슈

- 대표 실패:
  - `stream_worker_forces_failure_when_service_exits_without_terminal_event`
- 보조 실패:
  - `leased_slot_success_completion_waits_for_official_refresh_before_cleanup`
  - `parallel_mode_runtime_keeps_cleaned_session_detail_after_slot_return`
- 수업에서 볼 질문:
  - terminal event가 오지 않았을 때 상태를 `ReadyToContinue`로 되돌려야 하는가, 아니면 `SubmittingTurn` 유지가 맞는가?
  - background worker 종료와 UI 상태 전이는 같은 이벤트에서 처리되어야 하는가?
  - refresh 타이밍이 늦어질 때 테스트는 어떤 상태를 기대해야 하는가?

## 실습

```bash
. "$HOME/.cargo/env"
cargo test adapter::inbound::tui::app::tests::shell_surface_tests::stream_worker_forces_failure_when_service_exits_without_terminal_event -- --nocapture
```

- 실패 전후 상태를 표로 정리한다.
- 이벤트가 실제로 몇 번 발생했는지 추적한다.
- 수정 과제:
  - 종료 이벤트 누락 시 fallback 상태 전이 명시
  - runtime notice와 상태 label 갱신 순서 정리

## 수강생이 가져가야 할 판단 기준

- Rust UI 코드는 async 문법보다 상태 전이 계약을 먼저 읽어야 한다.
- 테스트 이름 하나가 상태 머신의 요구사항을 거의 그대로 담고 있다.
- 상태 누수는 panic보다 더 느리고 비싸게 문제를 만든다.
