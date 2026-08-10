# PR 검증 운영 전환 및 롤백 Runbook

[English](../../reference/pr-validation-rollout.md)

이 문서는 post-merge 검증을 수동 관찰에서 Planning Queue 자동 복구로 전환하는 실제 운영 계약입니다.
GitHub Ruleset 변경 권한을 부여하지 않습니다.

## Runtime mode

저장소 로컬 설정 `akra.prValidationMode`는 다음 값만 허용합니다.

| Mode | Provider 관찰 | 새 Queue admission | 영속 history |
| --- | --- | --- | --- |
| `off` | 중지 | 차단 | 보존 |
| `observe` | 활성 | 차단 | 보존·갱신 |
| `remediate` | 활성 | typed actionable finding만 허용 | 보존·갱신 |

값이 없으면 `observe`가 기본입니다. Canonical repository에서 다음처럼 설정합니다.

```text
git config --local akra.prValidationMode observe
git config --local akra.prValidationMode remediate
git config --local akra.prValidationMode off
```

Admin Validation Rail은 실제 mode를 `OFF`, `SHADOW`, `REMEDIATION`으로 표시하고 Queue admission
차단 여부를 별도로 보여 줍니다. 이 설정은 branch protection이나 GitHub required check를 바꾸지
않습니다.

## CI 계약

- `Fast Gate`는 CI scope, 선택된 Rust lint/architecture, 선택된 Node/Admin, 명시적 smoke를
  집계합니다. 긴 Rust 전체 테스트와 portable platform 검증은 merge admission에서 제외합니다.
- `CI Gate`는 PR에 선택된 모든 job을 계속 집계하며, 별도 사용자 승인 전까지 Ruleset required
  context로 유지됩니다.
- `Post-Merge Gate`는 정확한 integrated evidence SHA의 push 검증 전체를 집계합니다.

긴 job은 merge 뒤에 새로 시작하는 것이 아니라 Fast Gate와 병렬로 시작합니다. 따라서 admission
latency를 줄이면서 post-merge evidence 수집을 늦추지 않습니다.

## 증거 수집

결정론적 계약 테스트를 실행한 뒤 실제 GitHub 표본을 수집합니다.

```text
cargo test --locked pr_validation -- --nocapture
cargo test --locked admin_debug_harness_all_validation_scenarios_keep_board_and_scene_semantics_aligned -- --nocapture
node --test scripts/pr-validation-rollout-evidence.test.mjs
node scripts/pr-validation-rollout-evidence.mjs collect \
  --repository RefinedStone/codex-exec-loop \
  --base prerelease \
  --sample-size 10 \
  --contracts docs/validation/artifacts/post-merge-validation-rollout-2026-08-10/contract-evidence.json \
  --json-out docs/validation/artifacts/post-merge-validation-rollout-2026-08-10/evidence.json \
  --markdown-out docs/validation/artifacts/post-merge-validation-rollout-2026-08-10/README.md
```

수집기는 GitHub를 읽기만 합니다. PR 10개와 24시간 조건을 동시에 만족하지 못하거나, Actions target
SHA와 merge evidence SHA가 다르거나, core quota 사용률이 50% 이상이거나, canary/계약 증거가
없거나, 결정론적 실패가 하나라도 있으면 `observe` 유지로 판정합니다. 실제 Fast Gate run이 생기기
전의 값은 projection이라고 명확히 표시합니다.

## Canary 정책

- Production `prerelease`에서는 정상 review/merge 뒤 exact evidence SHA의 Post-Merge Gate가 성공하는
  성공 canary만 허용합니다.
- 실패 canary는 application/Admin debug harness 또는 폐기 가능한 test repository에서 수행합니다.
  의도적으로 실패하는 commit을 production `prerelease`에 병합하지 않습니다.
- 체크인된 evidence가 `ready_for_remediate`일 때만 `remediate` 전환이 가능합니다.
- Ruleset context 변경은 실제 Fast Gate 측정 뒤에도 별도의 사용자 명시 승인이 필요합니다.

## Rollback

1. `git config --local akra.prValidationMode observe`로 새 remediation admission만 즉시 차단합니다.
   Observation, record, finding, 기존 Queue correlation history는 보존합니다.
2. Provider traffic도 중지해야 하면 `off`로 전환합니다. SQLite row를 삭제하지 말고 사고 종료 뒤
   `observe`로 복귀합니다.
3. 이미 생성된 remediation task는 일반 Planning Queue 작업입니다. History를 삭제하지 말고 typed
   application workflow로 취소하거나 acknowledge합니다.
4. 별도 승인으로 Ruleset을 변경한 상태라면 먼저 required context를 `CI Gate`로 복원합니다. Bypass
   actor를 추가하거나 protection을 끄지 않습니다.
5. Active record가 settle되거나 명시적으로 acknowledge될 때까지 versioned expected context를
   보존합니다.

## Ruleset 승인 자료

승인을 요청하기 전에 Fast Gate actual/projected p50·p95, 기존 CI Gate p50·p95, Post-Merge Gate
실패율과 성공 canary, false actionable/duplicate/stale/lease takeover/phase/SHA mismatch, API quota,
위 rollback 절차를 함께 제시합니다.

이번 전환의 체크인된 증거는
[`post-merge-validation-rollout-2026-08-10`](../../validation/artifacts/post-merge-validation-rollout-2026-08-10/README.md)에 있습니다.
