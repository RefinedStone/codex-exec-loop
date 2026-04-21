# 5회차: 파일시스템, Git, SQLite를 명시적으로 다루기

## 세션 목표

- Rust에서 인프라 경계를 얇게 유지하면서도 명시적으로 다루는 방법을 이해한다.
- 파일시스템, Git worktree, SQLite authority store가 동시에 개입할 때 어떤 테스트가 깨지는지 읽는다.
- `parallel_mode` 실패군을 통해 “상태의 단일 진실 소스”를 어떻게 지켜야 하는지 설명한다.

## Spring Boot/Kotlin 비교

| Kotlin/Spring 습관 | Rust에서 대응되는 방식 |
| --- | --- |
| DB 트랜잭션 중심 일관성 | 파일, Git, DB, 메모리 스냅샷을 수동으로 정렬 |
| repository abstraction에 숨김 | adapter가 I/O 디테일을 직접 노출 |
| 엔티티 저장 후 영속성 컨텍스트 신뢰 | write 후 파일 존재, worktree 상태, projection 재계산을 모두 확인 |

## 읽기 순서

1. [../../src/application/service/parallel_mode/mod.rs](../../src/application/service/parallel_mode/mod.rs)
2. [../../src/application/service/parallel_mode/tests.rs](../../src/application/service/parallel_mode/tests.rs)
3. [../../src/adapter/outbound/db/sqlite_planning_authority_adapter.rs](../../src/adapter/outbound/db/sqlite_planning_authority_adapter.rs)
4. [../../src/adapter/outbound/filesystem/planning_workspace.rs](../../src/adapter/outbound/filesystem/planning_workspace.rs)

## 이번 회차 이슈

- 대표 실패:
  - `acquire_slot_lease_persists_metadata_and_marks_slot_leased`
  - `reconcile_provisions_missing_slots_into_idle_baselines`
  - `mark_slot_running_updates_persisted_lease_and_pool_state`
  - `build_supervisor_snapshot_reads_store_backed_runtime_projections_after_mirror_loss`
  - `distributor_recovery_blocks_missing_worktree_from_store_backed_queue_record`
- 공통 증상:
  - slot lease가 저장되었다고 가정하지만 실제 파일을 못 찾는다.
  - pool root 아래 worktree slot이 만들어졌다고 가정하지만 존재하지 않는다.
  - store-backed queue record가 있어야 하지만 없다.
- 수업에서 볼 질문:
  - 진실 소스가 SQLite store인가, mirror 파일인가, worktree 상태인가?
  - reconcile 단계와 snapshot 단계가 같은 책임을 동시에 지고 있지는 않은가?
  - test fixture가 현재 권위 모델 변경을 제대로 따라가지 못하고 있는가?

## 실습

```bash
. "$HOME/.cargo/env"
cargo test application::service::parallel_mode::tests:: -- --nocapture
```

- 실패 테스트를 “lease persistence”, “pool provisioning”, “queue recovery” 세 묶음으로 분류한다.
- 각 실패가 읽는 경로와 쓰는 경로를 표로 정리한다.
- 수정 과제:
  - authority store와 file mirror의 책임 분리
  - slot lease write/read helper 정합성 재검토
  - reconcile과 snapshot read path 분리

## 수강생이 가져가야 할 판단 기준

- Rust는 인프라 세부사항을 숨기기보다 드러내는 쪽이 안전할 때가 많다.
- 대신 진실 소스와 projection을 명확히 나누지 않으면 테스트가 급격히 불안정해진다.
- 실패군을 묶어서 읽어야 구조적 원인이 보인다.
