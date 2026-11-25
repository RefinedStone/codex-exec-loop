# 이슈/Vault 장애 및 다중 창 통합 회귀 (#2705)

상위 #2414. 이미 구현된 프로세스/lease/key 초기화/재연결 경계를 재구현하지 않는다. 자동 회귀의 주장은 실제 실행한 범위에 한정한다.

| 시나리오 | 기존/추가 자동 검증 위치 | 남은 범위 |
| --- | --- | --- |
| 둘이 같은 편집 기준으로 서로 다른 공유본 게시 | 추가: issue_shared_copy_flow::independently_opened_editors_cannot_overwrite_a_newer_confirmed_copy (독립 Gateway, production journal, 합성 원격) | 실제 원격의 조건 없는 쓰기 race는 남음 |
| 두 편집기, 원격 응답 유실, 중복 클릭 및 재조립 | 추가: issue_shared_copy_flow::two_editors_with_an_uncertain_publish_share_one_durable_intent | 외부 API의 실제 장애는 시험하지 않음 |
| 같은 이슈의 두 전용 원문 창에서 한 창만 닫기 | 추가: desktop_restore_bridge::tests::closing_one_private_window_during_key_lookup_does_not_revive_it_or_cancel_another | 실제 OS 잠금과 Electron 창 이벤트는 #2704 |
| 최초 키 생성 경쟁/late key read | 기존 vault-key-initialization, desktop_restore_bridge::tests::eof_during_key_lookup_invalidates_grant_and_drops_late_plaintext | 실제 OS keyring은 #2704 |
| 게시 Sent/Confirmed commit 실패 | 기존 issue_shared_copy_flow/commit_failures.rs, 실제 SQLite busy rollback | 디스크 full 및 실제 I/O 오류 별도 주입 필요 |
| remote write 중 worker kill/응답 유실 | 기존 issue_shared_copy_flow/delayed_workers.rs | 전원 단절, 원격 exactly-once를 주장하지 않음 |
| binding CAS/중단 및 기존 데이터 보존 | 기존 vault-binding-transactions, reconnect/cancellation.rs | 실제 storage failure injection 보강 필요 |
| A→B→A, OS lock, 화면 재로드/agent 시작 | Desktop vaultRestoreWindow 및 native restore bridge 기존 취소 테스트 | OS 실제 전환 검증 및 전체 통합 반복 |
| Windows 신규 생성 후 장애 | #2702 real Windows creation journal + 합성 원격 | 선행 PR 병합 후 통합 |
| 외부 수정 뒤 AI 인계 | #2701 scoped handoff | 선행 PR 병합 후 통합 |

추가 시험은 두 개 독립 Gateway/전용 pipe 세션을 열어 동시 요청 또는 늦은 응답을 검증한다. 원문 창 취소 회귀는 pipe 종료와 별개로 취소된 blocking restore 전체의 반환 신호를 기다린 뒤 원격 읽기 횟수를 검사한다. 실제 GitHub, OS 키, 원문, 모델을 사용하지 않는다. 안전한 fixture에서 기존 회귀를 반복하되, 디스크를 가득 채우거나 사용자 프로세스를 종료하지 않는다. physical power-loss / ENOSPC / real keyring을 실행하지 않았다면 완료로 표시하지 않는다.

검증 명령:

```sh
cargo test --locked --lib adapter::inbound::gateway::tests::issue_shared_copy_flow::
cargo test --locked --lib adapter::inbound::desktop_restore_bridge::tests::
```

Windows 지정 portable inventory와 check_native_pr Rust gate는 두 suite를 이미 포함한다. PR에 실제 실행 결과와 나머지 한계를 기록한다.
