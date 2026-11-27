# Issue-core Workspace 추출 검증 (2026-11-27)

관련: #2902, #2903, #2867, #2911. 기준 코드: caf1da09aa561d1510b3271cc663b8e5bf3a0690.

## 실제 변경

- root package akra-ade와 library/binary/배포 이름은 유지하고 workspace resolver 3을 사용한다.
- issue_* 값·정책·가림·검증·템플릿 규칙 10개 모듈과 단위시험을 crates/issue-core로 함께 이동했다.
- 순수 정책/템플릿 서비스와 두 source Port도 함께 이동했다. 파일/Git/Vault 구현과 제품 조립은 root에 남긴다.
- Markdown parser는 root triage도 사용하는 실제 공용 API다. parser 타입과 필요한 메서드만 공개하며 상태 필드는 private이다. root 호환 alias를 유지한다.
- IssueValidationPassed의 생성 필드는 계속 private이고 Deserialize를 구현하지 않는다. 외부 위조/역직렬화를 거절하는 compile-fail doctest 2개를 추가했다.
- root로 향하는 normal/dev/build 의존은 없다. 소유권 registry의 source·시험 prefix·schema 외부 입력을 갱신했다.
- 전체 test/check/clippy는 --workspace를 사용한다. Windows/macOS portable 경로에도 새 member의 단위시험·doctest를 넣었다. root native helper·격리·기존 필터는 유지한다.

## 검증

- cargo check --locked --offline --workspace --all-targets --all-features: 성공, 1분 36초.
- cargo test --locked --offline -p akra-issue-core --all-features: 단위시험 53개와 compile-fail 2개 성공.
- CI scope/native runner/crate boundary/agent-plan Node 회귀 44개 성공.
- 실제 --locked Cargo metadata 경계 검사 성공.
- 분리 전 Windows default libtest 목록 6,418개에서 registry로 매핑한 53개가 새 crate 목록 53개와 정확히 일치한다. 이동 시험 중 ignored는 0개다.
- root 잔류 예상은 6,365개/ignored24다. 이 단계에서는 root libtest를 다시 링크하지 않고 all-targets check로 호환성을 확인했다. 최종 통합 inventory에서 잔류 목록도 대조한다.

## 동일한 작은 편집의 독립 빌드 관측

Windows x86_64, Rust 1.95.0, default features, test profile, CARGO_INCREMENTAL=1, jobs=8, registry warm, 독립 target. 각 조건 N=1이다.

| 조건 | 분리 전 root lib test | 분리 후 issue-core만 |
| --- | ---: | ---: |
| 변경 없음 | 0.48초 | 0.20초 |
| issue_validation 동일 주석 추가 | 33.43초 | 1.06초 |

비교 편집은 파일 끝에 빈 줄과 // Local CI baseline probe: domain only; no behavior change. 주석을 추가하는 것이다. 측정 뒤 원본 바이트를 복구했다. source 복구용 추가 build는 성능 비교에서 제외한다.

두 명령은 검증 범위가 다르다. 분리 후 값은 작은 crate의 반복 개발 비용이며 전체 CI 또는 root 통합 검증을 대체하지 않는다. root consumerCoverage는 계속 full이다. 같은 독립 빌드의 Cargo JSON artifact 50개 중 root akra-ade는 없었고 작은 수정에서는 issue-core 1개만 재컴파일했다.

분리 직후 새 target에서 issue-core 단위·doc 시험을 실행한 Cargo 시간은 7.30초였다. 그 뒤 shared Markdown 공개 API를 보정했으므로 최종 소스의 cold 수치로 사용하지 않는다. Linux/macOS/hosted CI 시간과 중앙값·p90는 아직 측정하지 않았다.

## 근거와 후속

- 기계 판독 결과와 53개 old/new 시험 이름: [ci-issue-core-extraction-20261127.json](ci-issue-core-extraction-20261127.json).
- 로컬 raw: artifacts/ci-issue-core의 Cargo JSON/verbose logs/timings와 workspace-check.log. 사용한 target은 삭제하지 않았다.
- 원본 기준선: #2867 / docs/validation/ci-local-build-baseline-2026-11-27.md.
- #2904~#2907은 동일 소유권 plan을 실행 경로에 연결한다. 알 수 없는 입력이나 불완전한 root 계약은 전체 검증을 유지한다.
- #2911에서 최종 조합 SHA의 전체 검증과 원격 CI 성능을 판단한다. 이 PR은 전체 CI 성능 인수 또는 이슈 종료를 선언하지 않는다.

## Port re-export guard regression (2026-11-27)

The first issue-core CI run exposed an architecture guard that only searched each root Port file for an inline trait declaration. The two extracted Port shims correctly re-exported their real public crate traits, but the guard rejected them. The verification extraction exposed the same problem in two additional shims. Evidence: [issue-core CI job](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45956401035/job/140364982858) and [verification CI job](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45957694809/job/140370017716).

The guard now resolves only four explicitly registered transfers through the actual public module chain and public trait in the destination crate. It rejects private, conditional, missing, aliased, forged, ambiguous, or non-trait destinations. Other outbound Port checks retain their original policy. No production behavior or test exclusion was changed.

Validation on Windows with Rust 1.95: compiled the real architecture test source against the existing matching proc-macro2, quote, and syn test dependencies (without rebuilding the root libtest), then ran all five `outbound_port_` tests: **5 passed, 0 failed, 0 ignored**. Four are new positive/negative regression tests; the existing repository-wide naming contract also passed. Compile wall time was 3.266 seconds; test process wall time was 0.247 seconds. This targeted check is not a full architecture-suite or CI-duration result. Raw output remains under `artifacts/ci-port-guard-foundation/`.
