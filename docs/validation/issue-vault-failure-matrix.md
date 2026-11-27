# 이슈/Vault 장애 및 다중 창 통합 회귀 (#2705)

상위 #2414. 이미 구현된 프로세스/lease/key 초기화/재연결 경계를 재구현하지 않는다. 자동 회귀의 주장은 실제 실행한 범위에 한정한다.

| 시나리오 | 기존/추가 자동 검증 위치 | 남은 범위 |
| --- | --- | --- |
| 둘이 같은 편집 기준으로 서로 다른 공유본 게시 | 추가: issue_shared_copy_flow::independently_opened_editors_cannot_overwrite_a_newer_confirmed_copy (독립 Gateway, production journal, 합성 원격) | 실제 원격의 조건 없는 쓰기 race는 남음 |
| 두 편집기, 원격 응답 유실, 중복 클릭 및 재조립 | 추가: issue_shared_copy_flow::two_editors_with_an_uncertain_publish_share_one_durable_intent | 외부 API의 실제 장애는 시험하지 않음 |
| 같은 이슈의 두 전용 원문 창에서 한 창만 닫기 | 추가: desktop_restore_bridge::tests::closing_one_private_window_during_key_lookup_does_not_revive_it_or_cancel_another | 실제 OS 잠금과 Electron 창 이벤트는 #2704 |
| 최초 키 생성 경쟁/late key read | 기존 vault-key-initialization, desktop_restore_bridge::tests::eof_during_key_lookup_invalidates_grant_and_drops_late_plaintext | 실제 OS keyring은 #2704 |
| 게시 Sent/Confirmed commit 실패 | 기존 issue_shared_copy_flow/commit_failures.rs, 실제 SQLite busy rollback | disk full / flush 실패는 별도 검증 |
| Windows journal 저장 open 실패 | 추가: issue_shared_copy_flow/storage_failures.rs, production private SQLite 파일에 실제 공유 잠금으로 ERROR_SHARING_VIOLATION(32) 발생 | commit 도중 실패, 디스크 full, 전원 단절과 구별 |
| remote write 중 worker kill/응답 유실 | 기존 issue_shared_copy_flow/delayed_workers.rs | 전원 단절, 원격 exactly-once를 주장하지 않음 |
| binding CAS/중단 및 기존 데이터 보존 | 기존 vault-binding-transactions, reconnect/cancellation.rs; 추가: evidence_vault_store/binding/tests/storage_failures.rs의 실제 Windows pointer 교체 거절 | replace 이후 flush 실패 및 전원 단절은 이 시험 범위 밖 |
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

## Windows 파일 공유 실패의 실제 저장소 경계

추가 회귀는 전용 임시 fixture 파일을 열린 handle로 고정한다. 운영 파일의 권한을 바꾸거나 디스크를 채우지 않으며, 저장소 어댑터가 반환할 오류를 가짜로 만드는 방식도 사용하지 않는다.

- `windows_write_open_failure_before_sent_keeps_recorded_and_sends_nothing`: 실제 journal DB의 쓰기 공유를 거절한다. production private SQLite의 save가 Windows 오류 32로 실패하며 마지막 durable 상태는 Recorded, 원격 쓰기는 0회다. 새 Gateway의 open/recheck도 원격을 쓰지 않는다.
- `windows_write_open_failure_after_remote_apply_recovers_without_resending`: 합성 원격 적용 후 Confirmed 저장 open을 같은 방식으로 거절한다. 디스크에는 Sent가 남는다. 새 Gateway는 원격 재조회로 같은 intent를 Confirmed로 복구하고, 이전 편집 기준의 승인 요청은 conflict로 끝나며 원격 쓰기는 총 1회다.
- `windows_denied_pointer_replace_keeps_original_and_allows_exact_retry`: 실제 암호화된 CurrentVersion 파일은 읽을 수 있지만 delete/replace를 허용하지 않는 handle을 유지한다. Windows delete-open probe는 오류 32, production native rename은 알려진 Storage 오류를 반환한다. 기존 pointer와 모든 기존 암호문은 byte 단위로 그대로이고 기존 원문을 읽을 수 있다. handle 해제 후 새 store가 동일 immutable operation을 명시적으로 재시도하면 새 판이 확정되며, 그 뒤 같은 재시도는 AlreadyCommitted다. 이전 판의 원문도 남는다.

Gateway 회귀의 원격/키/Vault API는 기존 합성 fixture이며, journal만 production Windows 저장소다. pointer 회귀는 별도로 실제 Windows Vault 어댑터와 합성 키·원문을 사용한다. 이 둘을 실제 OS keyring이나 Electron 전환까지 포함한 단일 종단 간 검증으로 표현하지 않는다.

Windows에서 세 추가 회귀만 재현하려면:

```powershell
cargo test --locked --all-features --lib --target x86_64-pc-windows-msvc storage_failures:: -- --nocapture
```

Windows native gate의 기존 `gateway::tests::issue_shared_copy_flow::`와 `filesystem::evidence_vault_store::` inventory가 이 시험을 포함한다. 다른 OS에서는 Windows 공유 잠금 fixture가 컴파일되지 않는다. 실제 실행 결과는 PR에 기록한다. 이번 증거는 저장 open과 pointer rename의 공유 거절이며 ENOSPC, fsync/flush, payload commit 중단 또는 물리적 전원 차단의 증거가 아니다.

## OS 전원 이벤트와 복원 창의 재열람 경계 (#2705)

화면 잠금 및 절전은 각각 독립적인 보류 상태로 기록한다. 이벤트 수신 시 모든 전용 창의 현재 열람 및 native channel을 폐기하고, 보류 상태가 유지되는 동안 뒤늦은 unlock IPC와 새 창의 ready도 helper를 시작하지 않는다. unlock-screen은 화면 잠금만, resume은 절전만 해제한다. 해제 자체는 열람을 시작하지 않는다. 일반적인 실행 프로필 검사 뒤 실제 OS idle state가 locked인지도 확인한다.

Windows x64, prerelease 1d538c2fe 기반:
- vaultRestoreWindow/vaultRestore/vaultRestoreSource 단위시험 114/114, Desktop typecheck 및 production build 통과.
- 실제 Electron + fake Gateway 화면 시험 통과. 폐기 프로필에서 합성 powerMonitor 이벤트와 synthetic restored state를 사용해 재가림, 보류 중 재열람, 겹친 화면 잠금과 절전, resume 후 자동 복원 금지를 확인했다.
- 다중 창 단위시험에서는 지연된 native prepare/restore 대역의 응답을 두 창에서 동시에 무효화하고 새 창도 보류하는 것을 검사했다.
- 실제 OS 잠금 이벤트, 대화형 OS 확인, 실제 Credential Manager/Vault, 외부 API는 이 시험에서 사용하지 않았다. 해당 native 인수는 #2704/#2724에 남긴다.
