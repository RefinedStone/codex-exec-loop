# Rust 시험 목록 보존과 nextest 검토

이 문서는 [#2690](https://github.com/RefinedStone/codex-exec-loop/issues/2690) 4단계의
**도입 검토 결과**다. 최초 조사 기준은 prerelease@70966a27f27429f9134e467c98b7b09a9894e39d다.
최종 통합 기준인 prerelease@99238e8792b25312d2f1c6ad3e5a0954f1fe05c3에서 manifest와
필수 시험 경로 변경을 다시 대조했다. 아래 경로표는 이 재검토와 최종 통합의 빌드 옵션을 반영한다.
기본 실행기는 cargo test --locked로 유지한다. nextest 및 Rust 샤딩의 기본 도입은 보류한다.

## 현재 실행 경로

| 시험 종류 | 실행 경로 | 유지할 정책 |
| --- | --- | --- |
| Linux unit, binary unit, integration | native-pr-checks.yml의 rust_tests: cargo test --locked | 기본 feature, test profile, 전체 기본 target |
| Architecture | Cargo integration 중 tests/architecture_boundaries.rs 및 lint job의 check_tui_layering.sh | 소스 경계 검사 둘 다 유지 |
| Doctest | cargo test --locked의 문서 시험 단계 | nextest 도입 시 cargo test --locked --doc 별도 실행 |
| Windows native | scripts/check_windows_portable.sh | Windows target, all-features, 계약 필터 30개, native ACL/프로세스/Vault/증거 저장 |
| macOS native | workflow의 portable_native | macOS target, all-features, 계약 필터 12개 |
| Native confirmation helper | Linux rust_tests와 macOS portable_native의 scripts/check_native_confirmation_process.mjs | 각 경로에서 built akra-gateway와 정확히 1개 ignored helper 시험; Linux default, macOS all-features 및 native target |
| 일반 ignored/manual | 소스의 #[ignore] | 일반 목록·샤드에서 --ignored 또는 --include-ignored 일괄 활성화 금지; 위 필수 helper는 별도 명시 경로 유지 |
| 전체 로컬 POSIX gate | scripts/check_native_pr.sh | Cargo, clippy, Node, architecture 경로 유지 |

현재 root integration target은 architecture_boundaries, binary_entrypoints,
issue_automation_policy_invariants, issue_automation_policy_schema, kernel_cli,
native_validation_scripts, repository_hygiene, tui_terminal_disconnect다. 이는
소스 파일 목록이며 OS/feature별 실제 시험 개수는 아니다.

Windows 30개와 macOS 12개는 선택 필터 수이며 개별 시험 수가 아니다. 99238e87에는 양쪽의
필수 필터로 task_evidence_artifact::tests::,
sqlite_planning_authority_adapter::tests::task_evidence::,
parallel_worker_commit::tree_snapshot::tests::,
parallel_mode::tests::pool::lease::task_evidence::, worker_evidence::가 추가돼 있다.
Windows에는 filesystem::issue_publish_intent_journal::와
gateway::tests::issue_shared_copy_flow::도 포함된다. 최종 통합은 이 필터를 모두 보존한다.

Native confirmation 스크립트와 Linux/macOS workflow 경로는 70966a27 조사 당시에는 없었지만
99238e87 prerelease에는 이미 존재한다. 최종 통합은 이 검사를 유지하고 Gateway 빌드와
helper 시험에 같은 feature, target, profile, target-directory 옵션을 전달한다.
해당 시험의 전체 이름은
adapter::inbound::gateway::tests::human_confirmation::native_process::native_helper_process_confirms_replays_and_revokes_on_eof다.
스크립트는 --ignored --exact로 이 시험만 실행하고 통과 개수가 정확히 1인지 확인한다.
일반 ignored 시험의 자동 활성화와 구분하며, native 계약 runner나 nextest 목록 비교로 대체하지 않는다.

## 보류 근거와 재개 조건

2026-11-24~26 CI 표본에 따른 제안 판단(측정 도구로만 사용)은
[CI 빌드 비용 비교](../validation/ci-build-cost.md#판단-제안)에 있다. 사용자 확정 전까지 아래 보류가 유효하다.

[#2690의 기존 성공 실행](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45231960486)은
Linux lib 실행 552.65초, architecture 실행 103.04초를 보고한다. baseline commit은
c63bec6e59b8c20e7247eb2ba597376d24805369이며 이 검토의 prerelease와 같지 않다.
이를 nextest 전환의 개선율이나 현재 실행 시간으로 사용하지 않는다.

nextest는 시험마다 별도 프로세스를 사용한다. 현재 libtest 안의 mutex가 보장하는 격리는
프로세스 사이로 확장되지 않는다. src/adapter/outbound/github/review_poller.rs와
src/application/service/parallel_mode/tests.rs의 환경변수 변경, Vault 초기화의 실제
자식 프로세스/OS 키 저장소, tests/native_validation_scripts.rs의 subprocess,
tests/tui_terminal_disconnect.rs의 PTY 종료를 별도로 확인해야 한다.
src/test_utils/pr_review_loopback.rs는 임의 포트 바인딩을 사용하지만, 그것만으로
모든 파일·프로세스·키 저장소 fixture의 격리를 보장할 수 없다.

샤드별 독립 runner에서 같은 큰 lib test binary를 다시 컴파일하면 피드백 시간은 줄어도
총 runner 시간·캐시 트래픽·메모리·디스크는 늘 수 있다. 먼저 같은 소스와 빌드 조건에서
목록 동일성, 실행 구간 시간, 피크 자원, 실패/취소 정리를 확인한다. 첫 비교는 단일 runner와
보수적인 동시성으로 하고, 이후 필요할 때만 2개 샤드를 비교한다. retry로 실패를 숨기지 않는다.

다음 증거가 있어야 기본 도입을 검토한다.

1. Cargo와 nextest의 unit/binary/integration 이름 및 ignored 상태가 일치한다.
2. 모든 기본 시험이 샤드 합집합에 정확히 한 번 나타나고 일반 샤드에서 ignored 시험은 선택되지 않는다.
3. doctest, 각 OS의 native 계약, 별도 confirmation helper 경로가 유지되고
   필수 job/shard의 실패·취소·누락이 gate를 실패시킨다.
4. 같은 runner/toolchain/features/profile에서 compile과 execution을 나눈 반복 표본이 있다.
5. cold, warm, unchanged, 작은 변경 조건별 중앙값·범위와 runner 총사용 시간을 비교한다.

이 4단계 검토에서는 nextest 설치, 전체 Rust 실행, 성능 반복 측정, 필수 CI 설정 변경을 하지 않았다.
99238e87 재검토도 소스 대조로 수행했으며 추가 시험이나 CI 조회를 실행하지 않았다.
비교 자료가 없으므로 성능 향상과 안정성 통과를 선언하지 않는다.

## 이미 빌드한 시험의 개별 시간 진단

stable libtest의 기본 출력에는 개별 실행 시간이 없다. 기본 CI 실행기는 유지하며, `scripts/ci-rust-test-profile.mjs`로 필요한 일반 시험만 선택해서 측정할 수 있다.

```text
node scripts/ci-rust-test-profile.mjs <local-test-executable> <output.json> [exact-test-name ...]
```

이름을 생략하면 해당 executable의 일반 시험 전체를 하나씩 실행하므로, 큰 lib의 진단은 기존 로그가 지목한 이름부터 시작한다. ignored 시험은 이름을 명시해도 실행하지 않는다. 실패·signal 종료·0개 통과는 incomplete 영수증과 실패 종료로 남으며 retry하지 않는다.

이미 빌드된 신뢰하는 로컬 executable을 해당 소스와 같은 작업 위치에서만 사용한다. 분리 실행은 원래 병렬 libtest의 공유 상태나 경합 조건을 재현하지 않는다. JSON에 이 제한과 바이너리 SHA-256·OS·선택 목록을 기록한다. 프로세스 시작·종료를 포함한 wall time을 기본 CI의 성능 개선율로 계산하지 않는다.

[#2865의 architecture 병목 및 로컬 전후 근거](../validation/ci-linux-test-performance-2026-11-27.md)를 참고한다. Linux의 실제 job 개선 여부는 같은 조건의 CI 표본으로 별도 인수한다.

## 목록 비교 도구

scripts/ci-rust-test-inventory.mjs는 Cargo가 만든 test executable의 --list 결과와
nextest의 JSON 목록을 비교한다. 이 도구는 Cargo 빌드나 실제 시험을 자동 실행하지 않는다.
capture-cargo는 이미 빌드된 **이 저장소의 표준 libtest 바이너리만** 목록 조회한다.
기존 artifact와 같은 checkout, 환경, host에서 실행하며 외부에서 받은 Cargo JSON을 입력하지 않는다.
custom harness, benchmark, cross-execution은 지원하지 않고 오류로 끝난다.

정규화된 목록은 package ID + target kind + target name으로 바이너리를 구분한다.
누락된 integration binary, 중복 이름, ignored 변경, 테스트 추가/삭제, 기본 필터 제외,
샤드 누락/중복과 사용자가 선언한 commit/toolchain/host/target/features/profile metadata 불일치를 거절한다.
이 context 비교는 선언된 값의 동일성 검사이며 실제 빌드 옵션을 검증하거나 증명하지 않는다.
잘못 작성한 같은 context를 양쪽 자료에 붙이면 목록이 같더라도 그 오류는 감지하지 못한다.
실제 실행 명령과 feature/target/profile 및 빌드 관련 환경변수, rustc -vV 결과, Cargo artifact JSON과
nextest 원본 목록을 함께 보존하여 동일한 빌드 조건인지는 별도로 대조해야 한다.
목록이 같다는 것은 실행 통과 증거가 아니다. doctest는 비교 목록에 포함되지 않으며,
ignored로 나열된 native confirmation helper의 별도 실행 성공도 이 도구가 검증하지 않는다.

### 선택적 비교 재현

이 절차는 후속 성능 실험을 수행할 때 사용한다. 같은 checkout에서 실행해야
Cargo package ID의 경로가 일치한다. 비교할 nextest 버전을 고정하고
cargo nextest --version 결과도 측정 영수증에 남긴다. 최신 버전을 임의로 CI에 설치하지 않는다.

아래는 Linux 기본 feature/test profile 비교 예다. context.json의 값은 실제 실행값으로
만들며 features는 활성화된 package feature, defaultFeatures는 CLI 선택을 나타낸다.
all-features 또는 다른 target/profile을 비교할 때는 양쪽 명령과 context를 함께 바꾼다.

~~~bash
mkdir -p target/ci-rust-inventory
node --input-type=module <<'NODE'
import fs from 'node:fs';
import { execFileSync } from 'node:child_process';
const read = (exe, args) => execFileSync(exe, args, { encoding: 'utf8' }).trim();
const rustc = read('rustc', ['-vV']);
const host = rustc.match(/^host: (.+)$/m)[1];
fs.writeFileSync('target/ci-rust-inventory/context.json', JSON.stringify({
  commit: read('git', ['rev-parse', 'HEAD']),
  rustc, host, target: host, profile: 'test', features: [], defaultFeatures: true,
}, null, 2));
NODE
cargo test --locked --no-run --message-format=json > target/ci-rust-inventory/cargo.jsonl
node scripts/ci-rust-test-inventory.mjs capture-cargo \
  target/ci-rust-inventory/cargo.jsonl target/ci-rust-inventory/context.json \
  target/ci-rust-inventory/cargo-inventory.json
cargo nextest list --locked --message-format=json > target/ci-rust-inventory/nextest.json
node scripts/ci-rust-test-inventory.mjs capture-nextest \
  target/ci-rust-inventory/nextest.json target/ci-rust-inventory/context.json \
  target/ci-rust-inventory/nextest-inventory.json
node scripts/ci-rust-test-inventory.mjs compare \
  target/ci-rust-inventory/cargo-inventory.json target/ci-rust-inventory/nextest-inventory.json
~~~

샤드를 실험할 때는 같은 nextest 명령에 --partition hash:1/2와 --partition hash:2/2를
각각 넣은 목록도 정규화한 뒤 compare의 뒤에 두 inventory 경로를 전달한다.
도구는 nextest 해시를 복제하지 않고 실제 nextest가 선택한 항목을 확인한다.
skipped suite가 있는 목록은 전수 목록을 증명하지 못하므로 오류 처리한다.
성능 실험에서는 기존 Cargo와 nextest의 기본 실행 시간을 비교하되 일반 ignored 시험을
활성화하지 않는다. doctest와 기존 필수 native confirmation helper는 각각 별도 경로로 유지한다.
모델 턴을 요구하는 ignored probe는 이 절차의 대상이 아니다.

## 근거

- [nextest 목록과 JSON 출력](https://nexte.st/docs/listing/)
- [nextest JSON 구조와 공식 예제](https://nexte.st/docs/machine-readable/list/)
- [nextest TestListSummary 타입](https://nexte.st/rustdoc/nextest_metadata/struct.TestListSummary)
- [프로세스별 시험 실행](https://nexte.st/docs/design/why-process-per-test/)
- [샤딩](https://nexte.st/docs/ci-features/partitioning/)
- [doctest 별도 실행 안내](https://nexte.st/docs/running/)
