# 현재 품질 기준선

이 문서는 강의 시작 전에 저장소 상태를 어떻게 읽을지 정리한 기준선이다.
매 회차 시작 전에 다시 실행해 결과가 바뀌었는지 먼저 확인한다.

## 스냅샷 시점

- 작성 시점: `2026-04-21`
- 작업 브랜치 기준: `origin/prerelease`에서 분기한 강의 문서 브랜치

## 실행 명령

```bash
. "$HOME/.cargo/env"
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

## 요약

- `cargo test`
  - 전체 결과는 실패다.
  - 확인 시점 기준 `551 passed / 23 failed`.
  - 실패는 대부분 `application::service::parallel_mode::tests::*`와 `adapter::inbound::tui::app::tests::parallel_mode_runtime_tests::*`에 몰려 있다.
- `cargo clippy --all-targets --all-features -- -D warnings`
  - 전체 결과는 실패다.
  - unused import, `collapsible_if`, `too_many_arguments`, `needless_question_mark`, `new_without_default`, `iter_overeager_cloned`, `needless_borrow`가 대표적이다.

## 핵심 이슈 묶음

### 1. 테스트 코드 정리 부족

- 대표 파일: [src/adapter/inbound/tui/app/app_tests.rs](../../src/adapter/inbound/tui/app/app_tests.rs)
- 증상:
  - unused import 때문에 clippy가 즉시 실패한다.
- 강의 포인트:
  - Rust는 테스트 코드도 동일한 품질 게이트를 통과해야 한다.
  - 큰 테스트 허브 파일은 import fan-in이 쉽게 무너진다.

### 2. Parallel Mode 런타임 불안정

- 대표 파일: [src/application/service/parallel_mode/mod.rs](../../src/application/service/parallel_mode/mod.rs)
- 대표 실패:
  - `acquire_slot_lease_persists_metadata_and_marks_slot_leased`
  - `reconcile_provisions_missing_slots_into_idle_baselines`
  - `mark_slot_running_updates_persisted_lease_and_pool_state`
  - `build_supervisor_snapshot_reads_store_backed_runtime_projections_after_mirror_loss`
- 공통 증상:
  - slot lease 파일이 생성되지 않았다고 본다.
  - pool slot worktree가 존재하지 않는다고 본다.
  - distributor queue record가 있어야 하는데 없다.
  - `Missing`으로 잘못 분류된다.
- 강의 포인트:
  - 복합 상태를 파일시스템, Git, SQLite, 메모리 스냅샷에 동시에 걸쳐 운영할 때 경계가 조금만 흐려져도 테스트가 무너진다.

### 3. TUI 상태 전이 불일치

- 대표 파일:
  - [src/adapter/inbound/tui/app/conversation_runtime.rs](../../src/adapter/inbound/tui/app/conversation_runtime.rs)
  - [src/adapter/inbound/tui/app/shell_runtime.rs](../../src/adapter/inbound/tui/app/shell_runtime.rs)
- 대표 실패:
  - `stream_worker_forces_failure_when_service_exits_without_terminal_event`
  - `leased_slot_success_completion_waits_for_official_refresh_before_cleanup`
  - `parallel_mode_runtime_keeps_cleaned_session_detail_after_slot_return`
- 강의 포인트:
  - 이벤트 기반 UI는 함수 호출보다 상태 전이와 타이밍 가정을 먼저 읽어야 한다.

### 4. Clippy가 드러내는 설계 냄새

- 대표 파일:
  - [src/domain/parallel_mode.rs](../../src/domain/parallel_mode.rs)
  - [src/application/service/parallel_mode/distributor.rs](../../src/application/service/parallel_mode/distributor.rs)
  - [src/adapter/inbound/tui/app/shell_presentation.rs](../../src/adapter/inbound/tui/app/shell_presentation.rs)
- 대표 징후:
  - 함수 인자 과다
  - 중첩 `if`
  - 불필요한 clone/borrow/question mark
  - `new()`만 있고 `Default`가 없음
- 강의 포인트:
  - lint는 문법 잔소리가 아니라 API 모양과 책임 분산 상태를 보여주는 신호다.

## 강의 진행 규칙

- 매 회차 시작 10분은 반드시 기준선 재측정에 쓴다.
- 기준선이 바뀌면 교안의 “이번 회차 이슈”도 갱신한다.
- 모든 수정 과제는 다음 세 가지 중 하나로 분류한다.
  - 즉시 수정 가능한 작은 경고
  - 실패 테스트를 재현하고 원인만 설명할 중간 이슈
  - 구조적 리팩터링이 필요한 큰 이슈
