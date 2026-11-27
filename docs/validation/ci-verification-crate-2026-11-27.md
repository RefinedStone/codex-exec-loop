# Verification crate 경계와 시험 소유권

관련 이슈: [#2909](https://github.com/RefinedStone/codex-exec-loop/issues/2909). 선행 경계는 [CI crate ownership](../reference/ci-crate-boundaries.md)에 따른다.

## 실제로 분리한 책임

`akra-verification`은 acceptance 정규화, evidence/verification/delivery observation의 순수 검증, 실행 결과의 byte capture를 소유한다. SQLite, Git, 프로세스 실행, planning 문서 전체, kernel event, pool을 의존하지 않는다.

- `acceptance`, `evidence`, `verification`, `delivery_evidence`: 기존 데이터 형식과 검증 규칙을 그대로 이동했다. 필드, serde closed shape, hash 입력, revision 및 출력 크기 제한을 유지한다.
- `ports::task_verification_runner_port`: 준비한 실행을 한 번 소비하는 기존 runner 계약이다. manifest를 durable bind한 뒤 실행하는 조립은 루트에 남는다.
- `ports::task_evidence_artifact_port`: 기존 filesystem capture 계약이다. worker artifact와 host verification capture의 namespace 의미를 유지한다.
- `service::TaskVerificationCaptureService`: 실행 관측값 검증, 두 출력의 크기/hash 비교, 기존 schema version 1 envelope 생성, 반환 artifact의 hash/size 검증을 수행한다.
- `continuation::TaskVerificationContinuation`: 기존 취소 관찰과 짧은 commit 직렬화 계약만 이동했다.

캡처 서비스는 workspace와 artifact Port만 받는다. 새 종합 repository Port나 planning 전체 DTO를 만들지 않았다. root execution은 기존 capture 함수를 통해 이 서비스에 위임한다.

## 루트에 유지한 권한과 통합

`TaskVerificationService`의 reservation, pending/running 상태 전환, 취소, retry, completion, issue binding, lease 및 authority 재확인, owner/run key, repository commit, wakeup, worker pool 직렬화는 기존 구현에 남는다. `PostTurnContinuationPermit`의 생성과 trait 구현 및 bounded commit helper도 루트 소유다.

owner/run key는 기존 private construction, redacted Debug, 외부 deserialization 금지를 유지한다. 새 crate의 관측값이나 sealed artifact는 delivery 또는 Done 권한을 발급하지 않는다.

기존 root module 경로는 재export shim으로 보존했다. cross-crate 호출에 필요한 timestamp 검증 함수 하나만 새 crate의 명시적 public 순수 API로 노출했고, root 경로에서는 기존 `pub(crate)` 범위를 유지한다.

## 시험 이동과 보존

| 기존 위치 | 새 위치 | 기존 시험 수 | 처리 |
| --- | --- | ---: | --- |
| `domain::planning::acceptance::tests` | `acceptance::tests` | 3 | 순수 정규화·wire shape 시험 이동 |
| `domain::planning::acceptance::tests` | 기존 root 위치 | 4 | planning 문서·semantic validation 통합 보존 |
| `domain::planning::evidence::tests` | `evidence::tests` | 4 | 전체 이동 |
| `domain::planning::verification::tests` | `verification::tests` | 5 | 전체 이동 |
| `domain::planning::delivery_evidence::tests` | `delivery_evidence::tests` | 5 | 전체 이동 |
| `application::service::task_verification::tests::capture` | 기존 root 위치 | 3 | 실제 root service 조립 회귀 보존 |

대상 domain 시험 21개의 합집합은 새 crate 17개와 root 4개로 보존한다. 새 crate에는 host revision 순수 회귀 1개와 capture fake 회귀 6개를 추가해 총 24개의 단위시험을 둔다. 이동한 시험에는 ignored 항목이 없다. 기존 root integration capture 3개는 이동하거나 삭제하지 않는다.

새 capture 회귀는 binary stdout/stderr와 기존 JSON shape, 잘못된 관측값, 길이/hash 불일치, seal 오류 및 위조 metadata, 실패·불완전 결과 보존, byte limit를 확인한다. fake Port는 worker writable root나 worker artifact 경로 호출을 허용하지 않는다.

acceptance 파일은 일부만 이동했으므로 `scripts/ci-crate-ownership.json`에 `split-unit-tests`와 `movedTests`/`retainedTests`를 명시한다. 전체 prefix가 사라졌다고 추론하면 안 된다.

## 선택 실행 및 남은 비용

독립 규칙과 capture 회귀는 다음 명령으로 실행한다.

```sh
cargo test --locked -p akra-verification --lib
```

이 명령은 root libtest 또는 실제 Git/SQLite fixture를 요구하지 않는다. root 소비자 검증은 ownership registry의 `consumerCoverage: full`로 유지한다. package 추출만으로 기존 root 통합시험을 생략하지 않는다.

느린 evidence_gate, post_turn automatic_verification, orchestrator automatic_verification, pool lease task_verification 시험군은 모두 root 업무 통합 소유를 유지한다. 이 PR은 그 시험군의 runtime 단축이나 Linux/macOS CI 개선율을 주장하지 않는다. Git adapter와 native 계약시험은 #2910 후속 경계다.

## 검증 기록

- 기존 rule 정의 및 분리하지 않은 단위시험은 namespace, 제한된 timestamp 함수 visibility, formatting 이외의 내용이 baseline과 같음을 대조했다.
- 기존 crate-boundary Node 회귀 8개 통과.
- `cargo fmt --all`, `git diff --check` 통과.
- #2903 workspace commit `4983131b3a519504562494c86d339ce30f9cacbb` 위로 연결했고 issue-core ownership과 transfer를 보존했다.
- `cargo metadata --offline --format-version 1 --all-features`로 공통 lockfile을 갱신했다. 추가된 path package와 root 의존 항목 외 기존 의존 버전 변경은 없다. 실제 metadata의 workspace member/normal/dev/build 경계 검사가 통과했다.
- `cargo test --offline --locked -p akra-verification --all-features --target x86_64-pc-windows-msvc --target-dir target/ci-verification`: 24 passed, 0 failed, 0 ignored. 문서시험 target은 0개다.
- Windows Rust 1.95.0, `CARGO_BUILD_JOBS=2`, 빈 전용 target 및 기존 registry 캐시에서 단일 실행했다. Cargo test profile 컴파일 11.49초, libtest 0.27초, 명령 wall time 12.286초다. 호스트 부하는 통제하지 않았으며 기존 CI와의 직접 속도 비교 수치가 아니다.
- 같은 package/target의 `cargo clippy --offline --locked -p akra-verification --all-targets --all-features --target x86_64-pc-windows-msvc --target-dir target/ci-verification -- -D warnings` 통과.
- 실제 test binary의 전체/ignored 목록과 baseline 소스 inventory를 대조했다. [JSON 증거](ci-verification-crate-2026-11-27.json)에 이전·새 이름, 유지한 root 시험, 새 회귀를 기록했다.
- `node scripts/agent-plan.mjs`는 Windows Full gate를 요구한다. 병렬 PR마다 전체 root를 다시 빌드하지 않도록 최종 통합 인수에 모으는 작업 계획에 따라 root all-targets compile/Full gate는 이 PR의 단독 로컬 검증에서 실행하지 않았다.
- 모든 OS와 최종 full gate는 통합 인수에서 확인한다. 이 문서의 로컬 결과로 해당 검증이 완료되었다고 간주하지 않는다.

- 후속 stack 연결: #2908 native commit `01e8c2b285541c8a2ba1ab8c3be899ab979b06f7` 위로 재기반했다. issue-core/native/verification 소유권·transfer를 모두 보존하고 `cargo metadata --offline --locked` 및 실제 dependency 경계 검사를 다시 통과했다. 위 24개 실행의 측정 base는 JSON의 기존 baseSha이며, 재기반 자체를 새 성능 측정으로 세지 않는다.
