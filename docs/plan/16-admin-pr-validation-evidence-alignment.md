# Admin PR validation 실행 선택·evidence 정합성 계획

상태: **implemented**

아래의 "현재 상태와 공백"은 계획 작성 시점의 진단 기록이다. 구현 결과와 전달 lineage는
[11. 구현 결과](#11-구현-결과)에 정리한다.

## 1. 목적

PR [#2112](https://github.com/RefinedStone/codex-exec-loop/pull/2112)의 리뷰에서 확인된 두 가지
정확성 원칙을 Akra Admin의 실시간 validation read model과 운영 화면에도 동일하게 적용한다.

1. 서로 다른 workflow run 사이에서는 `run_attempt`가 아니라 run 생성 시각을 기준으로 최신 run을
   선택하고, attempt 비교는 같은 run 안에서만 수행한다.
2. 독립적으로 수집한 actual Fast Gate 표본이 기존 historical projected 분포에 포함되지 않았다면
   historical 분포를 mixed로 표시하지 않는다. actual과 projected는 출처와 표본 수를 분리한다.

이 문서는 이미 완료된 [비동기 병합 후 검증·복구 Queue·Akra Admin 대응 계획](15-async-pr-ci-remediation-admin-rollout.md)을
대체하지 않는다. 기존 durable validation authority, scheduler, bounded Admin board, SSE invalidation,
typed command, Debug Harness 위에 정확성·설명 가능성·운영 evidence를 추가하는 후속 계획이다.

## 2. 현재 상태와 공백

### 2.1 이미 제공되는 기반

- `PrValidationQueryPort`는 durable authority에서 bounded board와 detail record를 조회한다.
- Admin Validation Rail은 `Integrated`, `Verifying`, `Remediation`, `Verified`를 구분한다.
- detail drawer는 checks, workflow attempt, provider/backoff, timeline, remediation correlation을 표시한다.
- rollout banner는 scheduler mode를 `OFF`, `SHADOW`, `REMEDIATION`으로 표현하고 Queue admission과
  Ruleset 승인 경계를 분리한다.
- SSE는 validation revision invalidation을 전달하고 브라우저가 authoritative snapshot을 다시 읽는다.
- application-owned Debug Harness가 API, DOM, Pixi projection을 결정론적으로 재현한다.

### 2.2 workflow run 선택 공백

GitHub adapter의 `GithubValidationWorkflowRun`에는 provider run ID, `run_attempt`, `created_at`,
`updated_at`이 존재한다. 그러나 runtime의 `observation_projection()`은 현재 workflow 이름을 key로
사용하고 `run_attempt`를 우선 비교한다. 이 방식은 다음 상황에서 오래된 run을 최신 run으로 오인할
수 있다.

```text
old run: created_at 10:00, attempt 3
new run: created_at 11:00, attempt 1
```

또한 durable `PrValidationObservedWorkflow`와 Admin `PrValidationAdminWorkflow`에는 provider run
identity와 `created_at`이 남지 않으므로, Admin은 어떤 기준으로 실행이 선택됐는지 설명할 수 없다.

### 2.3 rollout evidence 표현 공백

PR #2112의 evidence collector는 historical Fast Gate와 독립 actual Fast Gate를 분리한다. 반면
Admin rollout snapshot은 scheduler stage와 Queue admission만 제공하며 다음 정보는 제공하지 않는다.

- historical Fast Gate의 `projected | mixed_actual_and_projected` 출처
- actual Fast Gate의 별도 표본 수와 p50/p95
- CI Gate와 Post-Merge Gate의 비교 지표
- evidence 생성 시각, 표본 범위, freshness
- `ready_for_remediate | hold_observe | stale | unavailable` 판정 근거

Admin 사용자는 운영 mode를 볼 수 있지만, 그 mode를 뒷받침하는 evidence의 품질과 출처는 같은
화면에서 검증할 수 없다.

## 3. 목표 상태

```text
GitHub workflow/check observations
  -> adapter normalization
  -> canonical latest-run reducer
       1. group by provider run identity
       2. select latest attempt inside the same run
       3. select newest run by created_at
       4. use updated_at/provider identity only as deterministic tie-breakers
  -> provider-neutral durable observation projection
  -> PrValidationQueryPort
  -> Admin board/detail API
  -> Validation Rail/detail drawer

rollout evidence collector or durable evidence source
  -> schema validation + freshness decision
  -> application-owned rollout evidence projection
  -> compact Admin summary + lazy bounded history
```

Admin은 최종 상태만 표시하지 않고 다음 질문에 답할 수 있어야 한다.

- 어떤 workflow run이 최신으로 선택됐는가?
- attempt 번호는 어느 run 내부의 값인가?
- 선택 기준과 관측 시각은 무엇인가?
- Fast Gate 수치는 actual인가 projected인가?
- 각 분포의 표본 수와 수집 범위는 무엇인가?
- evidence가 mode 전환 판단에 사용 가능한 상태인가?

## 4. 설계 원칙

1. **선택 규칙은 application/domain 소유다.** Admin JavaScript가 run 정렬이나 최신성 판정을 다시
   구현하지 않는다.
2. **attempt는 run identity에 종속된다.** 서로 다른 run의 attempt 번호를 직접 비교하지 않는다.
3. **check context가 성공 계약이다.** workflow run은 container와 진단 metadata이며 required check와
   이중으로 성공 조건을 만들지 않는다.
4. **actual과 projected를 합성하지 않는다.** 값이 실제로 같은 분포에 포함된 경우에만 mixed label을
   허용한다.
5. **누락은 0이 아니다.** 표본이나 timestamp가 없으면 `unavailable`로 표현한다.
6. **브라우저는 GitHub나 SQLite를 직접 읽지 않는다.** typed application projection만 소비한다.
7. **provider identity는 내부 선택에만 사용한다.** raw opaque ID, credential, response body는 Admin
   JSON에 노출하지 않는다.
8. **기존 durable JSON을 계속 읽는다.** 새 필드는 additive/default-compatible하게 도입한다.
9. **SSE는 invalidation이다.** 대형 evidence payload를 stream에 싣지 않고 revision 변경 후 snapshot을
   다시 읽는다.
10. **게임 scene은 phase만 표현한다.** 통계와 선택 근거는 Validation Rail과 detail drawer에 두어 map을
    운영 숫자로 과밀화하지 않는다.
11. **Ruleset 변경은 별도 승인이다.** 이 계획은 required context 변경이나 bypass 추가를 자동화하지
    않는다.

## 5. 작업 패키지

### 5.1 canonical workflow run reducer

- [x] provider run identity별로 observation을 그룹화한다.
- [x] 같은 run 안에서 높은 `run_attempt`를 선택한다.
- [x] 같은 run·attempt 중복은 `updated_at`으로 최신 observation을 선택한다.
- [x] workflow 이름별 대표 run은 `created_at`, `updated_at`, provider identity 순으로 결정한다.
- [x] timestamp 누락 시 사용할 결정론적이고 fail-closed인 fallback 규칙을 정의한다.
- [x] reducer를 pure decision으로 분리해 collector fixture와 Rust runtime fixture가 같은 결과를
      검증하게 한다.
- [x] required check 평가와 workflow metadata 선택을 별도 함수로 유지한다.
- [x] 최대 workflow 수와 입력 bounds를 유지한다.

필수 fixture:

| 상황 | 기대 결과 |
| --- | --- |
| 오래된 run attempt 3, 새로운 run attempt 1 | 새로운 run 선택 |
| 같은 run attempt 1, 2 | attempt 2 선택 |
| 같은 run·attempt의 오래된/새로운 update | 새로운 update 선택 |
| 생성 시각이 같은 서로 다른 run | update 시각과 provider identity로 결정론적 선택 |
| 완료 run과 진행 중인 더 새로운 run | 더 새로운 진행 중 run을 표시하고 이전 성공으로 settle 금지 |
| 잘못된 target SHA | 선택 대상에서 제외하고 integrity blocker 유지 |

### 5.2 provider-neutral durable projection

- [x] `PrValidationObservedWorkflow`에 `created_at`과 선택 provenance를 additive하게 추가한다.
- [x] legacy record에서 새 필드가 없을 때 안전한 기본 상태로 deserialize한다.
- [x] provider run ID는 reducer 입력으로만 사용하고 durable/Admin projection에서는 제거한다.
- [x] 운영자가 run 교체를 인지할 수 있도록 비식별 selection basis를 보존한다.
- [x] timestamp validation, 길이 제한, workflow count bound를 유지한다.
- [x] record replay와 process restart 후 동일한 projection이 생성되는지 검증한다.
- [x] SQLite authority의 기존 JSON fixture와 migration compatibility test를 추가한다.

권장 projection 의미:

```text
name
status
run_attempt
created_at
started_at
updated_at
selection_basis = newest_run | latest_attempt | deterministic_tie_break
```

### 5.3 Admin workflow read model과 API

- [x] `PrValidationAdminWorkflow`에 `createdAt`, `selectionBasis`, `selected`를 추가한다.
- [x] board row에는 latest selected run의 간결한 요약만 포함한다.
- [x] detail endpoint에는 bounded workflow history와 선택 근거를 포함한다.
- [x] board payload가 과도하게 커지지 않도록 history는 detail에서만 조회한다.
- [x] API가 run ID, check suite ID, token, raw provider body를 redaction하는지 검증한다.
- [x] legacy workflow는 `selectionBasis=legacy_unknown`으로 명확히 표시한다.
- [x] cursor revision 불일치와 record 갱신 경합 시 기존 authoritative refresh 동작을 유지한다.

### 5.4 rollout evidence application contract

- [x] rollout evidence를 읽는 narrow port와 application service를 정의한다.
- [x] collector schema version, repository, base branch, generated-at, evidence SHA를 검증한다.
- [x] evidence 상태를 `ready`, `hold`, `stale`, `unavailable`, `invalid`로 normalize한다.
- [x] historical Fast Gate와 actual Fast Gate를 서로 다른 metric snapshot으로 모델링한다.
- [x] 각 metric에 `label`, `sample_count`, `p50_seconds`, `p95_seconds`, `source`를 보존한다.
- [x] CI Gate와 Post-Merge Gate, failure rate, quota usage, sample window를 함께 project한다.
- [x] actual run을 historical 분포에 실제 포함하지 않았다면 historical label을 projected로 유지한다.
- [x] mixed label은 historical sample row 자체에 actual Fast Gate가 포함됐을 때만 허용한다.
- [x] evidence 누락·schema 오류·SHA mismatch·stale 상태는 mode 승인 근거에서 fail-closed 처리한다.
- [x] 브라우저가 artifact 경로나 JSON 파일을 직접 해석하지 않게 한다.

### 5.5 evidence 저장과 이력

- [x] 최신 evidence snapshot과 bounded history를 durable authority에 저장한다.
- [x] repository/base/evidence SHA/generated-at 조합으로 중복 snapshot을 억제한다.
- [x] actual 표본이 누적될 때 projected history를 덮어쓰지 않는다.
- [x] 수집 실패도 성공 snapshot과 구분되는 typed observation으로 기록한다.
- [x] retention과 pagination 기준을 정의한다.
- [x] scheduler ownership과 evidence collection ownership을 분리한다.
- [x] 여러 Admin/TUI process가 동시에 떠 있어도 중복 수집·중복 저장하지 않도록 lease/CAS를 적용한다.

### 5.6 Admin Validation Rail

- [x] rollout banner 아래에 compact evidence summary를 추가한다.
- [x] `Projected Fast Gate`와 `Actual Fast Gate`를 별도 카드로 표시한다.
- [x] 각 카드에 label, sample count, p50, p95, generated-at을 표시한다.
- [x] CI Gate와 Post-Merge Gate 비교값을 같은 단위로 표시한다.
- [x] `stale`, `unavailable`, `invalid`, `hold` 상태를 색상 외 text/icon으로 구분한다.
- [x] actual 표본이 없을 때 `0s`가 아니라 `미수집`을 표시한다.
- [x] 좁은 화면에서는 카드가 단일 열로 전환되고 수치가 잘리지 않게 한다.
- [x] evidence가 mode와 불일치하면 attention strip에 원인과 안전한 조치를 표시한다.

### 5.7 validation detail drawer

- [x] 선택된 workflow에 `최신 run`, `run 내부 attempt N`을 분리해 표시한다.
- [x] created/start/update timestamp를 같은 시간대와 포맷으로 표시한다.
- [x] selection basis를 운영자 문장으로 변환한다.
- [x] 더 오래된 고차 attempt가 선택되지 않은 이유를 확인할 수 있게 한다.
- [x] evidence section에 actual/projected 출처와 표본 범위를 표시한다.
- [x] check context, workflow container, rollout metric을 시각적으로 구분한다.
- [x] keyboard navigation, focus return, screen-reader label을 유지한다.
- [x] 외부 링크가 필요하면 application이 허용한 canonical URL만 사용한다.

### 5.8 Debug Harness와 fixture

- [x] 기존 lifecycle 시나리오를 보존하고 evidence/run-selection fixture를 추가한다.
- [x] 오래된 run attempt 3과 새로운 run attempt 1 시나리오를 재현한다.
- [x] 같은 run의 attempt 증가 시나리오를 재현한다.
- [x] actual 표본 없음, 독립 actual 1개, sample 내 actual 혼합을 각각 재현한다.
- [x] stale evidence, invalid schema, SHA mismatch, partial pagination을 재현한다.
- [x] provider rate-limit과 retry/backoff 중에도 마지막 valid evidence가 구분되어 보이게 한다.
- [x] API, DOM, accessibility projection이 같은 selected run과 metric label을 보고하는지 검증한다.
- [x] Pixi scene은 validation phase만 반영하고 metric 수치에 따라 가짜 worker를 만들지 않게 한다.

### 5.9 추세와 이상 징후

- [x] actual Fast Gate, CI Gate, Post-Merge Gate의 bounded history를 제공한다.
- [x] actual/projected 편차와 sample count 변화를 계산한다.
- [x] 같은 이름의 workflow가 새 run으로 교체된 이력을 표시한다.
- [x] Post-Merge failure, stale evidence, SHA mismatch, duplicate remediation을 별도 지표로 유지한다.
- [x] 표본 부족, 급격한 지연, evidence freshness 초과를 typed warning으로 만든다.
- [x] 차트가 없어도 표와 텍스트로 동일 정보를 이해할 수 있게 한다.
- [x] history query에 cursor, upper bound, stable ordering을 적용한다.

### 5.10 Ruleset 승인 패키지

- [x] actual/projected Fast Gate p50/p95와 표본 수를 함께 제공한다.
- [x] 현재 CI Gate 대비 차이를 동일 기준으로 계산한다.
- [x] Post-Merge Gate failure rate와 production success canary를 연결한다.
- [x] false actionable, duplicate remediation, stale, lease takeover, phase mismatch, SHA mismatch를
      표시한다.
- [x] GitHub API quota와 evidence freshness를 포함한다.
- [x] rollback 절차와 현재 required context를 읽기 전용으로 표시한다.
- [x] 승인 전에는 Ruleset 변경 command를 제공하지 않는다.
- [x] 승인 작업이 별도로 수행되더라도 broad bypass actor 추가와 protection disable을 금지한다.

### 5.11 성능과 복구

- [x] dashboard bootstrap에는 latest summary만 포함한다.
- [x] detail/history는 lazy load하고 response bound를 둔다.
- [x] SSE validation revision이 바뀔 때 필요한 surface만 reconcile한다.
- [x] reconnect, duplicate event, cursor reset, revision regression을 테스트한다.
- [x] 긴 Admin 세션에서 CPU, memory, DOM node 수, payload 크기를 측정한다.
- [x] hidden tab에서는 불필요한 metric animation과 polling을 중단한다.
- [x] evidence port 장애가 parallel control plane이나 기존 Admin dashboard를 중단시키지 않게 한다.

## 6. API 초안

기존 board/detail 계약을 확장하되 browser가 판정을 재구현하지 않도록 이미 계산된 의미를 제공한다.

```text
PrValidationRolloutEvidenceSnapshot
  status
  generated_at
  repository
  base_branch
  evidence_short_sha
  sample_window
  historical_fast_gate
  actual_fast_gate
  ci_gate
  post_merge_gate
  post_merge_failure_rate
  quota
  blockers

PrValidationGateMetricSnapshot
  label
  sample_count
  p50_seconds
  p95_seconds
  source

PrValidationAdminWorkflow
  name
  status
  run_attempt
  created_at
  started_at
  updated_at
  selection_basis
  selected
```

API 규칙:

- `GET /api/admin/akra/dashboard`: latest compact evidence summary
- `GET /api/admin/akra/pr-validation/{recordKey}`: selected workflow와 bounded detail
- proposed `GET /api/admin/akra/pr-validation/evidence`: latest evidence와 bounded history
- SSE: evidence/validation revision과 cursor-reset 신호만 전달
- mutation: 기존 CSRF, expected revision, stable command identity 계약 유지

## 7. 작업 순서와 PR 경계

### PR A — run 선택 의미와 durable projection

- canonical reducer
- provider identity 기반 grouping
- additive observed workflow fields
- legacy deserialization
- domain/application/adapter fixture

이 PR에서는 Admin 레이아웃을 변경하지 않는다.

### PR B — rollout evidence read model과 API

- evidence port/service
- actual/projected metric 분리
- freshness와 fail-closed decision
- compact board summary와 bounded detail/history
- API redaction, cursor, serialization test

이 PR에서는 Ruleset과 scheduler mode를 변경하지 않는다.

### PR C — Admin Rail, detail drawer, Debug Harness

- evidence summary cards
- selected run 설명
- stale/unavailable/error states
- responsive/accessibility behavior
- deterministic fixture와 browser inspection

### PR D — durable history와 운영 추세

- evidence snapshot persistence
- retention/pagination
- trend/anomaly projection
- multi-process ownership과 recovery

### PR E — 승인 패키지와 운영 문서

- Ruleset approval package
- rollback/read-only protection facts
- English/Korean current-product, architecture, Admin, rollout reference 동기화
- checked-in validation artifact와 브라우저 캡처

각 PR은 앞 PR의 shipped contract만 소비한다. 후속 PR을 위해 임시 browser-owned state나 concrete
adapter import를 추가하지 않는다.

## 8. 검증 매트릭스

| 경계 | 필수 검증 |
| --- | --- |
| Domain | run identity/attempt/newest-time reducer table tests |
| GitHub adapter | ID/timestamp normalization, target SHA, pagination, malformed response |
| Durable record | legacy JSON read, additive write, restart replay, bound validation |
| Application | required check와 workflow metadata 비중복, fail-closed evidence decision |
| Query/API | board/detail/history bounds, redaction, cursor, revision conflict |
| Admin DOM | actual/projected 분리, selected run 설명, empty/stale/error state |
| Accessibility | keyboard drawer, focus return, text/icon status, narrow viewport |
| SSE | invalidation, reconnect, cursor reset, duplicate/reordered revision |
| Debug Harness | API/DOM/Pixi phase 일치, run/metric fixture 일치 |
| Performance | bootstrap/detail payload, CPU, memory, DOM node, long-session reconciliation |
| Architecture | Admin concrete DB/GitHub import 금지, dependency direction gate |

기본 검증 명령은 변경 scope에 맞춰 `node scripts/agent-plan.mjs`가 선택하게 한다. 전체 기능 완료 시
다음을 포함한다.

```text
cargo fmt --all -- --check
cargo test --locked pr_validation -- --nocapture
cargo test --locked admin_debug_harness -- --nocapture
node --test scripts/pr-validation-rollout-evidence.test.mjs
node --check assets/admin/scripts/akra-dashboard.js
npm --prefix assets/admin/game run check
npm --prefix assets/admin/game run build
cargo test akra_graphic_dashboard --lib
bash scripts/check_admin_graphic_visual.sh
```

## 9. 완료 조건

- [x] 오래된 run의 높은 attempt가 새로운 run을 이기지 않는다.
- [x] 같은 run 안에서는 최신 attempt가 선택된다.
- [x] 선택 결과가 restart, replay, pagination 순서와 무관하게 결정론적이다.
- [x] required check success 계약은 workflow metadata와 중복 계산되지 않는다.
- [x] Admin board와 detail이 같은 selected run을 표시한다.
- [x] historical projected와 독립 actual Fast Gate가 서로 다른 분포로 표시된다.
- [x] mixed label은 실제 혼합 표본이 있을 때만 사용된다.
- [x] 표본·timestamp 누락이 0 또는 success로 표현되지 않는다.
- [x] evidence freshness와 mode 불일치가 fail-closed warning으로 표시된다.
- [x] raw provider ID, credential, response body가 durable/Admin projection에 노출되지 않는다.
- [x] legacy durable validation record를 데이터 초기화 없이 읽는다.
- [x] Debug Harness, API, DOM, accessibility projection의 의미가 일치한다.
- [x] desktop과 narrow viewport에서 Validation Rail과 drawer가 정상 동작한다.
- [x] Admin 장시간 세션에서 bounded payload와 안정적인 reconciliation을 유지한다.
- [x] Ruleset은 별도 사용자 승인 없이는 변경되지 않는다.
- [x] English/Korean 운영 문서와 checked-in evidence가 실제 shipped behavior를 설명한다.

## 10. 비목표

- GitHub Ruleset required context 자동 변경
- branch protection 해제 또는 broad bypass actor 추가
- Admin browser의 GitHub/SQLite 직접 접근
- workflow run을 required check 성공 조건으로 중복 사용
- actual과 projected 표본의 출처 없는 합산
- validation metric을 이유로 passive CI용 가짜 worker 생성
- 기존 planning Queue 밖의 별도 remediation queue 도입
- 기존 validation record 삭제나 DB 초기화

## 11. 구현 결과

- PR A: [#2115](https://github.com/RefinedStone/codex-exec-loop/pull/2115) — canonical run selection과
  durable provenance.
- PR B: [#2116](https://github.com/RefinedStone/codex-exec-loop/pull/2116) — rollout evidence read model,
  freshness, bounded API.
- PR C: [#2117](https://github.com/RefinedStone/codex-exec-loop/pull/2117) — Validation Rail, detail drawer,
  Debug Harness와 responsive/accessibility projection.
- PR D: [#2118](https://github.com/RefinedStone/codex-exec-loop/pull/2118) — SQLite snapshot history,
  retention, trend/warning, collection lease/CAS와 recovery.
- PR E: 이 전달 — 읽기 전용 Ruleset 승인 패키지, English/Korean reference 동기화, production Admin
  browser/performance evidence.

최종 운영 근거는
[`admin-pr-validation-approval-package-2026-08-11`](../validation/artifacts/admin-pr-validation-approval-package-2026-08-11/README.md)에
있다. 캡처 시 현재 required context는 `CI Gate`, bypass actor는 0명이었고 Ruleset write는 수행하지
않았다. Desktop/narrow evidence drawer는 전역 horizontal overflow 0px, 브라우저 warning/error 0건,
15초 관찰 중 DOM node와 scroll height 변화 0으로 검증했다.
