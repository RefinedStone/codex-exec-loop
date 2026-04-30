# 현재 품질 기준선

이 문서는 강의 시작 전에 저장소 상태를 어떻게 읽을지 정리한 기준선이다.
매 회차 시작 전에 다시 실행해 결과가 바뀌었는지 먼저 확인한다.

## 스냅샷 시점

- 갱신 시점: `2026-04-30`
- 작업 브랜치 기준: `origin/prerelease`

## 실행 명령

```bash
. "$HOME/.cargo/env"
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

## 요약

- `cargo test`
  - 전체 결과는 통과다.
  - 확인 시점 기준 `499 passed / 0 failed`.
  - parallel mode pool, distributor, supervisor, turn 테스트가 현재 회귀 안전망이다.
- `cargo clippy --all-targets --all-features -- -D warnings`
  - 전체 결과는 통과다.
  - clippy 실패 목록 대신 남은 구조적 hotspot을 읽고 다음 작은 리팩터링 후보를 잡는다.

## 핵심 이슈 묶음

### 1. 테스트 게이트가 현재 기준선이다

- 대표 파일:
  - [src/adapter/inbound/tui/app/shell_rendering_tests.rs](../../src/adapter/inbound/tui/app/shell_rendering_tests.rs)
  - [src/adapter/inbound/tui/app/shell_rendering_contract_tests.rs](../../src/adapter/inbound/tui/app/shell_rendering_contract_tests.rs)
- 증상:
  - shell rendering과 contract 테스트가 현재 TUI 회귀 기준선이다.
- 강의 포인트:
  - Rust는 테스트 코드도 동일한 품질 게이트를 통과해야 한다.
  - 통과하는 테스트도 어떤 operator journey를 보호하는지 이름과 파일 배치로 설명되어야 한다.

### 2. Parallel Mode 런타임 기준선

- 대표 파일:
  - [src/application/service/parallel_mode/pool.rs](../../src/application/service/parallel_mode/pool.rs)
  - [src/application/service/parallel_mode/distributor.rs](../../src/application/service/parallel_mode/distributor.rs)
  - [src/domain/parallel_mode.rs](../../src/domain/parallel_mode.rs)
- 대표 회귀 테스트:
  - `acquire_slot_lease_persists_metadata_and_marks_slot_leased`
  - `reconcile_provisions_missing_slots_into_idle_baselines`
  - `mark_slot_running_updates_persisted_lease_and_pool_state`
  - `build_supervisor_snapshot_reads_store_backed_runtime_projections_after_mirror_loss`
- 공통 증상:
  - 과거에는 slot lease, pool worktree, store-backed queue record의 진실 소스가 흔들렸다.
  - 현재는 service가 I/O와 recovery를 담당하고, domain이 readiness, roster, selected detail,
    pool slot state, cleanup decision 같은 순수 projection을 담당한다.
- 강의 포인트:
  - 복합 상태를 파일시스템, Git, SQLite, 메모리 스냅샷에 동시에 걸쳐 운영할 때 경계가 조금만 흐려져도 테스트가 무너진다.

### 3. TUI 상태 전이 기준선

- 대표 파일:
  - [src/adapter/inbound/tui/app/conversation_runtime.rs](../../src/adapter/inbound/tui/app/conversation_runtime.rs)
  - [src/adapter/inbound/tui/app/shell_runtime.rs](../../src/adapter/inbound/tui/app/shell_runtime.rs)
- 대표 회귀 테스트:
  - `missing_terminal_event_becomes_forced_failure_and_notice`
  - `successful_running_turn_is_cleanup_candidate`
  - `build_supervisor_snapshot_keeps_cleaned_session_detail_after_slot_return`
- 강의 포인트:
  - 이벤트 기반 UI는 함수 호출보다 상태 전이와 타이밍 가정을 먼저 읽어야 한다.

### 4. Domain Extraction이 드러내는 남은 설계 냄새

- 대표 파일:
  - [src/domain/parallel_mode.rs](../../src/domain/parallel_mode.rs)
  - [src/domain/planning](../../src/domain/planning)
  - [src/application/service/parallel_mode/distributor.rs](../../src/application/service/parallel_mode/distributor.rs)
  - [src/application/service/planning/repair/reconciliation.rs](../../src/application/service/planning/repair/reconciliation.rs)
- 대표 징후:
  - application service에 남은 순수 projection 후보
  - prompt/copy 조립과 domain fact의 경계
  - 큰 테스트 fixture가 특정 구현 파일에 묶이는 현상
- 강의 포인트:
  - lint 통과는 끝이 아니라 기준선이다.
  - 다음 리팩터링은 I/O 없는 판단을 domain으로 내릴 수 있는지부터 확인한다.

## 강의 진행 규칙

- 매 회차 시작 10분은 반드시 기준선 재측정에 쓴다.
- 기준선이 바뀌면 교안의 “이번 회차 관찰 지점”도 갱신한다.
- 모든 수정 과제는 다음 세 가지 중 하나로 분류한다.
  - 즉시 수정 가능한 작은 경고
  - 통과 테스트가 보호하는 계약을 더 명확히 설명할 중간 이슈
  - 구조적 리팩터링이 필요한 큰 이슈
