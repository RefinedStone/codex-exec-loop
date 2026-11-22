# Gated 자동 시작과 예산 검증

대상: [#2556](https://github.com/RefinedStone/codex-exec-loop/issues/2556), plan 28의 IA5-1.

이 문서는 구현 계약, 사전 회귀, 최종 부모 결합 검증의 소스와 결과를 함께 기록한다.

## 사용자 결정과 범위

2026-11-22 사용자가 Q-IA1을 승인했다. 사용자가 자율 수준을 명시적으로 켠 것이 자동 시작의 동의이며 사용자 파일의 예산이 한도이다. IA5에서는 전달·통합에 사람 리뷰를 유지한다. 개발·검증에서 실제 모델 턴을 사용하는 허가는 별개의 계약이다.

이번 범위는 Gated 자동 접수, 동시·일일·이슈당 실행 한도, kernel 유지와 client 재접속 후 복구이다. 마감·기본 답변, diff 재판정, 중재자, 자동 전달, Gateway 재시작 후 무인 재기동은 각각의 후속 이슈에 남긴다.

## 시작 권한

- 유효 수준이 Gated이고 해당 등급의 start 관문이 Auto인 경우 정책에 의한 시작 권한을 사용한다.
- 사람 관문, 보호 규칙, actor 신뢰, 공개 저장소 상한은 그대로 적용한다. 파일의 모드만 높였다고 native 확인을 대신하지 않는다.
- 최초 Gated runtime 선택에도 native 확인을 요구한다. 정책 파일은 허용 가능한 상한과 규칙이며, 명시적인 runtime 동의를 위조하지 않는다.
- checkpoint의 시작 출처는 Human과 Policy를 명시적으로 구분한다. 누락된 필드는 Human으로 해석하며, Human에 결정 ID가 없거나 Policy에 사람 결정 ID를 끼워 넣은 기록은 거절한다.
- source·policy·runtime selection·authorization epoch·gate 판은 기존 권한 검증에 계속 포함한다. 가짜 사람 결정이나 확인 proof를 만들지 않는다.

병렬 실행의 명시적인 켜기·끄기는 실행기 제어이다. Gated 선택이나 이슈 Start가 저장소의 무관한 일반 task까지 실행하도록 전체 pool을 자동 활성화하지 않는다. 실행기가 꺼져 있으면 접수된 작업은 기존 ExecutionDisabled 상태로 기다린다.

## 예산의 단위와 영속성

예산은 단일 workspace의 issue automation store에서 집계한다. 다른 workspace의 계정 전체 사용량을 합산하는 기능은 이번 범위가 아니다.

| 한도 | 집계 대상 |
| --- | --- |
| max_running | 접수됐고 종결되지 않은 이슈 작업 예약. 등록 응답을 아직 못 받은 의도, Queued, InReview도 포함 |
| max_started_per_day | UTC 접수일별 최초 작업 접수 영수증. work order당 한 번 |
| max_attempts | 기존 planning admission의 이슈 작업 계보 전체 구현 시도 제한 |

Human과 Policy 접수는 같은 한도를 사용한다. 예산 최대값은 기존 정책 합성 규칙을 따르며 저장소 파일이 사용자 상한을 높이지 못한다. 누락을 무제한으로 새로 해석하지 않고 기존 내장 기본값 2/5/3과 사용자·저장소 제한을 따른다.

최초 checkpoint와 접수 영수증은 같은 BEGIN IMMEDIATE 트랜잭션에서 저장한다. 일반 checkpoint 갱신 API로 신규 작업을 등록해 예산을 우회할 수 없다. 경쟁하는 접수는 같은 SQLite 경계에서 예약을 확인한다.

등록 전 실패, 등록 ACK 유실, adapter 재생성에서 같은 작업은 다시 차감하지 않는다. 날짜가 바뀌어도 같은 작업을 새 접수로 세지 않는다. 시계가 뒤로 이동해 과거의 일일 버킷을 다시 열지 않도록 마지막 접수일보다 뒤로 가지 않는 접수일을 사용한다.

한도를 낮추더라도 이미 이루어진 접수를 되돌려 계산하지 않는다. 기존 예약의 복구는 새 접수가 아니며 새 work order의 접수에는 현재 한도를 적용한다. 종결된 작업은 동시 예약 수에서 제외한다.

## 예산 대기와 승인 판의 분리

예산 부족은 별도 영속 대기 projection으로 기록한다. 작업은 접수되지 않은 Ready 상태에 남고 budget_exhausted 이유를 표시한다.

다른 이슈의 접수로 사용량이 바뀌었다고 기존 triage의 input fingerprint를 바꾸거나 승인 판을 올리지 않는다. source·policy·runtime 등 권한 기준이 바뀐 경우 오래된 대기 표시는 현재 기준과 맞는지 확인한다. 목록·상세 query가 새 접수나 예산 예약을 수행하지 않는다.

## 기존 데이터와 runtime 유지

issue automation store의 이전 schema에는 최초 접수 시각의 충분한 증거가 없다. 따라서 migration은 과거 일일 접수 영수증을 지어내지 않는다. 기존 미종결 checkpoint는 동시 예약 수에 포함하고, 일일 접수 집계는 새 영수증으로 기록한 신규 접수에 적용한다.

Gated/Auto의 유지 조건은 검증된 자율 상태를 runtime의 기존 remaining-work 판단에 연결한다. Observe/Propose는 활성 작업 등 다른 유지 사유가 없을 때 기존 idle unload를 따른다. 이미 진행 중인 다른 작업을 무시하고 강제로 내리지 않는다. 이번 변경은 Gateway 재시작 후 등록부에서 kernel을 무인 생성하는 기능을 추가하지 않는다.

## 검증 결과

2026-11-22에 fake provider·worker·clock과 실제 SQLite를 사용해 검증했다. #2555의 별도 실제 모델 시험과 실행 횟수를 합산하지 않는다.

| 항목 | 회귀에서 확인한 결과 | 결과 |
| --- | --- | --- |
| 경쟁하는 최초 접수 | Human·Policy가 같은 마지막 동시·일일 슬롯을 경합해도 한 작업만 접수 | 통과 |
| 등록 실패·ACK 유실·재조립 | 같은 work order와 영수증을 재사용하며 한도 하향 뒤에도 재차감하지 않음 | 통과 |
| UTC 날짜 경계·시계 역행 | 새 접수만 새 일일 버킷 사용, 역행으로 과거 버킷을 다시 열지 않음 | 통과 |
| 종결·동시 예약 | 등록 intent·Queued·Running·InReview는 예약, Done·Failed·Cancelled는 해제 | 통과 |
| 신규 checkpoint 우회 | 일반 save API의 신규 Human·Policy 작업 생성 거절 | 통과 |
| 원자성·권한 경계 | 영수증 저장 실패 시 checkpoint도 롤백, 현재 policy·gate·epoch·source·actor·runtime 판 검증 | 통과 |
| 기존 DB migration | schema 9의 Human 기록 보존, 허위 과거 일일 영수증 없음 | 통과 |
| 최초 Gated 동의 | native 확인 없는 파일 변경으로 자동 접수하지 않음 | 통과 |
| 사람 필수 규칙·사용자 상한 | Gated가 신뢰·보호·정책 상한을 높이지 않음 | 통과 |
| 예산 대기 | Ready·budget_exhausted 표시, 다른 작업의 승인 fingerprint 보존, 오래된 대기 표시 무효화 | 통과 |
| 통합 관문 | process opt-in이 Gated의 사람 리뷰를 대신하지 못함 | 통과 |
| kernel 유지 | 유효 Gated의 remaining-work 유지, pending·unavailable에서는 캐시 해제 | 통과 |

사전 관련 Rust 회귀는 **285 passed / 0 failed / 0 ignored**였다. 이는 부모 `8d743a30dc1d0323456c18348ab70f012c942928` 위의 Gated 작업 트리를 검사한 결과이며, 신규 DB 접수 회귀 12개, application Gated 회귀 9개, kernel 유지 회귀 2개를 포함한다. kernel schema 검증과 생성된 wire 형식 검사도 통과했다.

### 최종 부모 결합 검증

최종 제품 코드는 [`270dcf248a9dcf350ff260596ec3fa2a93642f8c`](https://github.com/RefinedStone/codex-exec-loop/commit/270dcf248a9dcf350ff260596ec3fa2a93642f8c)이며, 부모 `313ac8e0c967e87dc13d24d0aa59563a8619f260` 위에 놓인다. 부모의 공유 gate 대기·취소 처리와 Gated 시작 모듈을 모두 유지했다.

- 최종 소스에서 부모 gate 회귀 4개와 Gated 회귀 9개를 함께 실행해 **13 passed / 0 failed / 0 ignored**를 확인했다.
- planner가 지정한 Windows **Full gate를 다시 통과**했다. native contracts·build/check/clippy·Desktop 검사를 포함한다.
- 최종 Desktop TypeScript 검사와 Vitest는 **966 passed / 7 existing skipped**(62개 파일 통과·1개 건너뜀)였다.
- 최종 `cargo fmt --all`, `git diff --check`, staged diff 검사도 통과했다. 검증 후 commit 전 formatting이 제품 파일을 바꾸지 않았음을 확인했다.

```powershell
cargo test --locked --all-features --lib -- issue_coordinator::tests::gate:: issue_coordinator::tests::gated::
powershell -NoProfile -File scripts/check_native_pr.ps1 -Mode Full
```

공용 임시 폴더의 동시 메타데이터 변경을 피하도록 시험 전용 TEMP/TMP, RUST_TEST_THREADS=1, VITEST_MAX_WORKERS=2를 사용했다. 보안 검사와 시험 시간 제한은 낮추지 않았다.

### Desktop 전체 E2E

전체 E2E는 **25개 spec 중 24개 실행 통과**, 기존 Windows terminal spec 1개 skip으로 종료했다. 이후 restack에서 변경 대상 Desktop 8개 파일이 이 E2E를 통과한 내용과 LF 정규화 기준으로 동일함을 확인했다. 최종 부모 결합은 Rust gate와 문서 변경이며, 동일한 Desktop 코드에 대한 전체 E2E를 반복하지 않았다.

hunk 동작의 assertion이 모두 통과한 뒤 임시 폴더 삭제가 Windows EBUSY로 실패한 재현에는, 이미 대기 중인 Electron·Gateway 종료 절차를 유지하고 기존 harness와 같은 최대 5회·250ms 간격의 제한된 삭제 재시도를 적용했다. assertion이나 종료 제한 시간은 바꾸지 않았고 재시도 소진 시 실패한다.

원본 로그는 `C:/dev/akra-queue/analysis/`의 다음 파일에 보관했다.

- 사전 285개 회귀: `issue-2556-scoped-rust-codegen.log`
- 최종 결합 13개 회귀: `issue-2556-restacked-gate-rust.log`
- 최종 Windows Full: `issue-2556-restacked-gate-full.log`
- 동일 Desktop 코드의 전체 E2E: `issue-2556-final-isolated-r3-e2e.log`

최종 PR head의 공식 CI 결과는 PR 본문에 연결한다. 자동 회귀의 재시작은 service·adapter 재조립을 뜻한다. 실제 OS process kill, 실제 모델 실행, Gateway 재시작 뒤의 무인 재기동을 검증한 것으로 표현하지 않는다.
