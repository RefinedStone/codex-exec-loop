# CI 선택 검증의 성능 인수·전환 현황 — 2026-11-27

관련: [#2911](https://github.com/RefinedStone/codex-exec-loop/issues/2911), 상위 [#2690](https://github.com/RefinedStone/codex-exec-loop/issues/2690).
기록 기준: `prerelease@caf1da09aa561d1510b3271cc663b8e5bf3a0690`.
최종 근거 갱신: 2026-11-27T10:25:57.405Z. PR/CI 전체 스냅샷은 **2026-11-27 09:50:20 UTC / 18:50:20 KST**이며 이후 실패·수정 관측은 별도 기록한다.
기계 판독 자료: [ci-rollout-acceptance-20261127.json](ci-rollout-acceptance-20261127.json).

## 인수 판정

**최종 성능 인수는 미완료다.** 이 문서는 이미 수집한 구간별 측정과 실패 원인, 전환 정책 및 남은 인수를 묶은 현황 보고서다. 문서 작성이나 Draft PR 게시를 #2911 완료로 계산하지 않는다.

| 항목 | 현재 값 |
| --- | --- |
| 게시한 최종 조합 소스 SHA | `9098f0e7aa994cd0a2f2f3ca926b47381c9bdf29` |
| 실제 Windows 상세 검증 SHA | `10df53cedda2139998f7ee6e5842926537adb472` — 후속 macOS 시험 수정 전 |
| 최종 검증 계획 해시 | `789fc27130494941b97528bafad6a5f48ff4629ab547f2534b7f372d70f1e564` — 새 9098에서 mode=full, 계획 생성만 확인 |
| `acceptance.complete` | `false` |
| 확인한 inventory | `10df53ce`의 Windows/default/lib 6,434개·ignored 24개 보존 확인; 다른 OS/범위 미검증 |
| 로컬 반복 개발 | 독립 crate의 제한된 N=1 관찰 있음 |
| PR/전체 CI 개선 | 최종 조합 기준으로 불명 |
| postmerge | **full 유지**, 머지 후 대기시간 단축 주장 없음 |
| Ruleset·bypass·필수 context 변경 | 없음 |

원격 연결 복구 후 native #2908(PR #2945), verification-native #2910(PR #2946)을 포함한 **16개 PR을 모두 게시**했다. 최종 공통 소스는 `codex/ci-final-validation-20261127`의 위 SHA다. 문서만 추가하는 이 보고서의 미래 커밋 SHA는 자기참조하지 않는다. #2944의 이전 `89a1acb1` 성공 CI는 09:50 관측 당시 문서에만 해당한다.

로컬 인수에는 Windows ACL 실패 3개가 남고 현재 PR HEAD의 원격 Rust Tests assertion 실패 2건을 확인했다. 별도로 발견한 macOS libtest 컴파일 회귀는 수정·게시했지만 새 HEAD의 macOS 검증을 기다린다. 부분 성공·다른 SHA의 성공을 합쳐 전체 통과로 표시하지 않는다.

> **실행 근거와 후속 수정 구분:** 아래 빌드·Clippy·doctest·architecture·unit·inventory의 검증 SHA는 `10df53cedda2139998f7ee6e5842926537adb472`다. 이후 [macOS fixture 수정 38efaebd](https://github.com/RefinedStone/codex-exec-loop/commit/38efaebd651c4b6c3879975650057fcbe209ccbc)를 관련 4개 PR에 반영한 최종 소스가 `9098f0e7aa994cd0a2f2f3ca926b47381c9bdf29`다. 새 소스에서는 fmt·crate 경계 검사·full 계획 생성과 Node 입력 동일성 확인을 수행했다. 기존 Windows 대형 gate를 재실행하지 않았고 실제 macOS compile/test도 아직 확인하지 않았다. 이전 실행을 새 SHA의 실행으로 승격하지 않는다.

## 1. 확인한 시험 실행 시간

| 구간 | 이전 | 변경 후 | 표본·범위 | 해석 |
| --- | ---: | ---: | --- | --- |
| Windows 대표 Git/SQLite 시험 1개 | 21.52초 | 18.46초 | 같은 개발 PC, 전후 각 N=1 | 해당 표본 3.06초·약 14.2% 감소 |
| Linux architecture | 127.81초 | 76.35초 | 전후 각 N=1, **144개 동일·전부 성공** | 해당 구간 51.46초·약 40.26% 감소 |
| 같은 Linux 실행의 root lib | 843.95초 | 870.59초 | 전후 각 N=1, **6,540 성공·27 ignored 동일** | 26.64초·약 3.16% 증가 |
| Git fixture 대표 시험 중앙값 | 1.27초 | 1.20초 | 같은 개발 PC, 전후 각 N=3 | 해당 표본 중앙값 약 5.5% 감소 |

Windows 값은 Rust 1.95.0, all features, 기본 test profile, `x86_64-pc-windows-msvc`, `--exact --nocapture --test-threads=1`의 대표 시험이다. 기준·후보에 동일한 test-only 계측을 켰고 Git 준비 510회, 계측 command-output 190회를 보존했다. 일반 데스크톱 프로세스가 있는 환경이므로 통제된 유휴 벤치마크나 hosted Windows 전체 job의 개선율이 아니다. 정확한 시험 이름·명령·하위 구간은 [#2934의 고정 문서](https://github.com/RefinedStone/codex-exec-loop/blob/6e7b44a50bbb6b829d79634efaddae3b8ad0ae07/docs/validation/ci-native-process-wakeup-20261127.md)에 있다. 이 publication SHA는 재기반된 문서 위치이며, 해당 SHA의 전체 suite를 새로 측정했다는 뜻이 아니다.

Linux 수치는 다음 두 실행의 libtest 결과다.

- 기준: [run 45936932223 / Rust job 140302241613](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45936932223/job/140302241613), 실제 checkout `caf1da09aa561d1510b3271cc663b8e5bf3a0690`.
- 변경 후: [PR #2933 / run 45953138161 / Rust job 140353445633](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45953138161/job/140353445633), PR head `74a33850ee310f20417510e5aa19031749951653`, 실제 merge checkout `49a04f9cf590f8564eb08f8a6b3461ce47173c73`.

각각 한 번의 hosted 실행이다. architecture 구간은 짧아졌지만 같은 실행의 lib 구간은 길어졌다. 두 구간의 관찰을 전체 CI·PR gate·postmerge 개선율로 확대하지 않으며 중앙값 분포나 p90을 추정하지 않는다.

Git fixture 원값은 기준 1.33/1.27/1.25초, 후보 1.18/1.22/1.20초다. 기준 `ce02eb62f`와 후보 `79bcfcafd`의 작은 순차 표본이며 전체 CI·통계적 유의성을 주장하지 않는다. 정확한 명령과 시험은 [Git native 추출 보고서](https://github.com/RefinedStone/codex-exec-loop/blob/a58d3160d4d1ce58a2e3f57ecd77bfec672bdbd8/docs/validation/ci-verification-native-extraction-2026-11-27.md)에 있다.

## 2. 로컬 빌드 기준선과 독립 crate 관찰

### 분리 전 root 기준선

[#2936](https://github.com/RefinedStone/codex-exec-loop/pull/2936)의 [고정 보고서](https://github.com/RefinedStone/codex-exec-loop/blob/fbdfbc1aa89421837e9c3a03b922ca2d6cfe9e9d/docs/validation/ci-local-build-baseline-2026-11-27.md)와 [JSON](https://github.com/RefinedStone/codex-exec-loop/blob/fbdfbc1aa89421837e9c3a03b922ca2d6cfe9e9d/docs/validation/ci-local-build-baseline-2026-11-27.json)을 기준으로 한다. 측정 소스는 `caf1da09`이며 문서를 게시한 SHA는 `fbdfbc1a`다.

| 조건 | Cargo 명령 wall | 표본 수 | 시험 본문 실행 |
| --- | ---: | ---: | ---: |
| 빈 target, 기존 다운로드 캐시 | 162.6318초 | 1 | 0개 |
| 변경 없음 | 0.4765초 | 1 | 0개 |
| 작은 domain 주석 수정 | 33.4256초 | 1 | 0개 |
| 작은 Git adapter 주석 수정 | 33.4296초 | 1 | 0개 |

환경은 Windows x64, Rust 1.95.0, default features, test profile, `CARGO_INCREMENTAL=1`, jobs=8, 독립 target이다. 공통 명령은 다음과 같다.

```text
cargo test --locked --offline --lib --no-run --timings -v --message-format=json-render-diagnostics
```

이 값은 **root libtest 빌드 비용**이다. 시험 실행·bin·integration·all-features·다른 OS 비용을 포함하지 않는다. stable timing에서 frontend/codegen/link 분리가 없어 root compile unit을 순수 frontend 시간으로 해석하지 않는다. 원래 target·캐시를 삭제하지 않았고, 정확한 주석 patch·입력/목록 해시는 원문 JSON에 있다.

### 분리 후 issue-core 단독 빌드

[#2937](https://github.com/RefinedStone/codex-exec-loop/pull/2937)의 [고정 보고서](https://github.com/RefinedStone/codex-exec-loop/blob/4983131b3a519504562494c86d339ce30f9cacbb/docs/validation/ci-issue-core-extraction-20261127.md)와 [JSON](https://github.com/RefinedStone/codex-exec-loop/blob/4983131b3a519504562494c86d339ce30f9cacbb/docs/validation/ci-issue-core-extraction-20261127.json)은 다음을 기록한다.

| 조건 | 기존 root libtest 빌드 | issue-core 단독 빌드 |
| --- | ---: | ---: |
| 변경 없음 | 0.4765초 | 0.1964초 |
| 같은 작은 domain 주석 수정 | 33.4256초 | 1.05514초 |

두 명령의 **검증 범위가 다르다.** member 명령은 위 명령에 `-p akra-issue-core`를 지정하며 root consumer를 빌드·시험한 시간이 아니다. 따라서 이 표에서 전체 CI 또는 동등한 전체 검증의 개선율을 계산하지 않는다. root `consumerCoverage`는 계속 full이다.

측정 JSON에는 `c2031a72` 위의 “uncommitted issue-core extraction” 상태가 기록돼 있고, `4983131b`는 증거를 게시한 commit이다. 분리 직후의 7.30초 cold 값도 최종 공개 API 보정 전 표본이므로 최종 소스의 cold 성능으로 사용하지 않는다.

## 3. Windows 검증과 inventory 보존 — 10df53ce

아래 실제 검증 소스 `10df53cedda2139998f7ee6e5842926537adb472`에서 locked/offline Cargo metadata의 **5개 workspace member**, fmt, default/native-host `cargo test --workspace --lib --locked --offline --no-run --message-format=json --timings`, `cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings`가 통과했다. Clippy result의 공통 context에 남은 `features=default/profile=test` 라벨은 일반 빌드 문맥이며, 실제 검증 범위는 명령의 **all-targets/all-features**다.

| 검증 | 결과와 정확한 범위 |
| --- | --- |
| Node 정책 | **197/197**, fail/skip 0. 실행 SHA는 `310bb75ed5dec5c860e6d2cf44109e695e902aec`; 최종 SHA에서는 31개 entry/helper 및 직접 repo 입력의 blob 동일성으로 연결했으며 재실행으로 기록하지 않는다 |
| Architecture | 최종 SHA에서 **150/150**, ignored/filtered 0 |
| Member doctest | 4개 member 대상 **3개 compile-fail 모두 통과**: issue-core 2, native 1, verification/verification-native 각각 0. root 전체 doctest는 포함하지 않음 |
| 최종 lib inventory | 기존 **6,418개 전부 일대일 보존 + 근거 확인 신규 16개 = 6,434개**, ignored **24→24** |
| root 대상 실제 재실행 | 최종 SHA에서 **38/38** 통과, 27개 process 그룹 |
| member 결과 재사용 | 4개 executable SHA·compiled features·ordinary/ignored 목록이 310과 모두 같아 **142개: 139 pass/3 fail**을 재사용. 최종 SHA에서 member 시험 본문 재실행은 0개 |
| 합산 distinct 단위 시험 | **180개: 177 pass/3 fail**, 전체 상태 **FAIL** 유지 |

| Package | lib 목록 | ignored |
| --- | ---: | ---: |
| akra-ade | 6291 | 23 |
| akra-issue-core | 53 | 0 |
| akra-native | 43 | 1 |
| akra-verification | 24 | 0 |
| akra-verification-native | 23 | 0 |
| 합계 | 6,434 | 24 |

이동·이름 변경 **134개**(root 내부 timeline 6개 포함), 명시적 root retained **20개**를 확인했다. 누락·ignored 변경·목적지 충돌·미검토 신규 시험은 모두 0개다. `testOwnership.moved/retained`, `movedTests/retainedTests`, 개별 `testMappings`를 일반 prefix보다 우선했다. 목록 수집은 `--list`만 사용하며 시험 본문을 실행하지 않는다.

근거: [inventory 요약](ci-final-workspace-inventory-20261127.md), [정규화 비교와 신규 16개의 소스·PR 근거](ci-final-workspace-inventory-20261127.json), [원본 artifact 경로·크기·SHA256](ci-final-workspace-inventory-artifacts-20261127.json). 큰 raw mapping/capture는 문서 PR에 복제하지 않고 manifest가 가리키는 Windows artifact에 보존했다. 이전 310 증거도 원본 및 `attempt2/`에 유지했다.

310의 최초 실행은 **168 pass/12 fail**이었다. 그중 9건은 서비스 PATH의 Git/usr/bin 누락으로 기존 `sh` fixture를 찾지 못한 실패였다. 원본을 보존한 채 자식 PATH 끝에 기존 설치 경로만 추가하고 native subprocess 15개와 root exact 2개를 재실행해 **17/17**을 확인했다. 전역 환경은 변경하지 않았다. 10df에서는 이 member 결과의 동일성을 검증하고 root 38개를 새로 실행했다.

남은 3개는 `akra-native`의 `trusted_executable::fingerprint::tests::{a_modified_pinned_executable_cannot_keep_its_prepared_identity,pinned_native_format_and_digest_describe_the_same_bytes}` 및 `trusted_executable::tests::native_host_git_and_node_formats_are_accepted`다. 기존 호스트 ACL 조건 실패로 분류하되 실패를 면제하거나 성공으로 바꾸지 않았다.

이 검증 당시 계획 `55e18b175f8f64289f3509b3fc07269471c487ef07c16e714a7881b8a572a80a`는 Windows, features=all/default, profile=test의 **full** 계획이다. base ownership 미완비, manifest/공통 입력 변경, root consumer 때문에 보수적으로 full을 선택했다. **계획 생성은 `--run --full` 전체 gate 실행 성공이 아니다.** 전체 3 OS, all-features inventory, bin/integration/root doctest, IPC/플러그인, 큰 runtime 계약 인수는 남아 있다.

최신 직접 근거는 `C:/dev/worktrees/akra-ci-final-20261127/artifacts/ci-final-validation/`의 `build-result.json`, `clippy-result.json`, `member-docs-result.json/.log`, `final-plan.json`, `architecture-10df53ce.json`, `unit-validation-10df53ce.json`, `node-policy-final.json`, `node-input-equivalence-10df53ce.json`이며 기계 판독 보고서에 경로·해시·재사용 범위를 기록했다.

## 4. 공개 CI 실패를 성능 문제와 구분

**09:50:20 UTC의 과거 스냅샷**에서는 16개 PR이 모두 mergeable이고 run 3개 success, 4개 in_progress, 3개 queued, 6개 pending이었다. 당시 완료된 실패 job은 0개였으나 이후 관측에 승계하지 않는다. 성공 3개는 #2931, #2936, 이전 보고서 #2944의 당시 HEAD다.

이후 아래 **현재 PR HEAD의 실제 assertion 실패 2건**을 확인했다. 둘 다 60분 job timeout이 아니며 최종 공통 10df의 재현 결과도 아니다. checkout tree는 해당 PR head tree와 같았다.

| PR / 실제 checkout | 실패 | 결과 / 근거 |
| --- | --- | --- |
| #2939 / `fb395bc41135f64ecebcfaad29ce079f0ca41cc1` | `codex_thread::tests::fixture_classifies_terminal_receipts_and_accepts_only_final_text`: approval에서 실제 `End{Unavailable,NotSent}`, 기대 `Complete("{\"suggestions\":[]}")` | 6,486 pass/1 fail/27 ignored, 832.15초, exit101. [run/job](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45973397203/job/140419295149) |
| #2940 / `5c51ee8840eeddf04062c0ea2d858412c897c525` | `review_poller::timeout_policy_tests::run_git_command_uses_shared_timeout_policy`: `error_chain.contains("timed out after")` | 6,486 pass/1 fail/27 ignored, 839.66초, exit101. [run/job](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45973398822/job/140419316470) |

명령은 두 run 모두 `node scripts/ci-performance.mjs run rust-tests -- cargo test --workspace --locked --timings`다. 관련 시험/직접 호출 코드가 기준 소스와 같다는 읽기 비교는 있지만, #2939는 조기 준비 오류를 Unavailable/NotSent로 압축하고 #2940은 실제 error chain을 assertion 출력에 남기지 않아 원인을 구분할 수 없다. **기존 flaky 또는 신규 회귀 어느 쪽도 확정하지 않는다.** 두 실패를 인수 차단으로 유지한다.

추가로 **#2938 macOS의 명확한 추출 호환 회귀**를 확인했다. head `0043cae72cd024a3bd4b0ee8822ea094fc9baedb`, 실제 checkout `87b92d6486672fb9109e0bf3c82e83445ba05a7f`의 [run/job](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45976377597/job/140431899770)에서 10:13:27 UTC, `src/trusted_executable.rs:843:71`의 `super::MACOS_ADMIN_GROUP_ID`가 E0425로 실패했다. production native check는 통과했지만 libtest compile/native-contract 단계는 실패했다. native 추출 뒤 root macOS launcher 시험에 남은 private 상수 참조가 원인이다. 시험 내부 fixture GID80 상수로 수정해 새 delivery SHA 9098에 게시했으며 production 정책/API, 시험 이름·범위는 유지했다. 실제 macOS 인수는 대기 중이다.

아래 표와 JSON의 `ciFindings`는 **08:47 UTC까지의 과거 관측**이며 원래 SHA/run을 보존한다.

| 증거 | 관측 | 분류·현재 상태 |
| --- | --- | --- |
| [기준 Windows job](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45936932223/job/140302241610) | 03:55:45 시작, 04:55:51 취소 | 약 60분 job 제한. 뒤 계약 결과가 없어 전체 성공 아님 |
| [#2933 Windows job](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45953138161/job/140353445607) | `host_mailbox`, `elapsed_ms=5523`, `reply=TimedOut`; 해당 filter 22 성공/1 실패 | **개별 5.523초 대기 만료**, 60분 job 제한과 다름 |
| [#2938 Linux job](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45957694809/job/140370017716) | Port shim 4개가 “Port trait을 정의해야 한다” 검사에 실패 | 재export를 따르지 않는 architecture guard. 기반 issue-core 2개 + verification 2개 |
| [#2938 Windows](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45957694809/job/140370019000) / [macOS](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45957694809/job/140370019005) | `Native contract filter matched no tests: domain::planning::verification::` | 이동한 시험과 옛 root-only prefix 조합. 0건을 성공으로 처리하지 않고 중단 |

과거 Port shim/포맷 실패의 후속 수정은 공통 기반에 반영했고, 최종 10df에서 fmt·Clippy·architecture 150개가 통과했다. 이는 이전 실패 run의 결과를 바꾸거나 새로운 원격 전체 gate 성공을 뜻하지 않는다.

#2933의 재시작 시험은 이전 기준 Windows run에서는 성공했다. 이번 변경이 해당 시험/프로덕션 코드를 직접 바꾼 것은 아니지만, 부하와 타이밍 경합 중 직접 원인은 이 로그만으로 확정하지 않았다.

#2938의 위 실패 로그는 공개 head `a3dcfcad`/merge checkout `1d65c5ed` 당시 기록이다. 당시 로컬 재기반본 미게시 제한은 해소됐고 현재 PR #2938/#2946이 게시돼 있다. package/target runner와 member 시험 근거가 추가됐지만 3 OS의 전체 native 계약 인수로 확대하지 않는다.

macOS 후속 수정은 `38efaebd651c4b6c3879975650057fcbe209ccbc`이며 2026-11-27 10:24:06 UTC / 19:24:06 KST에 #2945/#2938/#2946/#2941과 최종 `9098f0e7`에 게시했다. 변경은 macOS 전용 시험 함수의 로컬 fixture GID 80 선언과 참조뿐이다. production private 상수·권한 정책·공개 API·시험명·assertion은 유지했다. 새로운 source의 fmt/경계/계획 생성은 성공했지만 **실제 macOS 재컴파일 결과는 아직 확인하지 않았다.** 위 실패 run은 수정 전 HEAD의 기록으로 남긴다.

## 5. 캐시 재현과 실제 원격 효과의 경계

[#2866](https://github.com/RefinedStone/codex-exec-loop/issues/2866) / [PR #2935](https://github.com/RefinedStone/codex-exec-loop/pull/2935)의 [고정 보고서](https://github.com/RefinedStone/codex-exec-loop/blob/dc67c7dd775a0c3ca5596b69edef33f58fd8beef/docs/validation/ci-cache-inputs-2026-11-27.md)와 [JSON](https://github.com/RefinedStone/codex-exec-loop/blob/dc67c7dd775a0c3ca5596b69edef33f58fd8beef/docs/validation/ci-cache-inputs-2026-11-27.json)은 독립 Cargo fixture의 **8조건 성공**을 기록한다.

빈 target, 변경 없음, 같은 바이트의 새 mtime/보정 없음, 검증된 mtime 복원, target tar 복원, Rust 내용 변경, `include_str!` 변경, build script 환경 입력 변경을 구분했다. mtime만으로 Dirty를 재현했고 동일 입력 복원 시 Fresh, 실제 입력 변경 시 Dirty와 새 동작을 확인했다.

그러나 [이전 원격 run 45563935880](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45563935880)의 **실제 Dirty 원인은 아직 미확정**이다. fixture 시간을 Akra 성능 표본으로 사용하지 않는다. trusted sealed snapshot의 실제 업로드/복원, checkout 경로·SDK/환경 차이, hashing 비용을 포함한 원격 효과도 남았다. 의존 crate 재사용과 root 재컴파일, 실제 시험 재실행은 따로 보고한다.

선택 PR job의 **restore-only**와 성공한 `main/prerelease` push의 저장 경계는 유지한다. 현재 다음 target cache namespace에는 동일 조건으로 저장하는 trusted producer가 없다.

- Linux 선택 행: explicit target. 기존 full Linux producer는 implicit host target.
- Windows/macOS 선택 행의 default features. 기존 portable producer는 all features.

따라서 첫 실행뿐 아니라 **반복 실행도 cold일 수 있다.** registry cache는 공유하지만 target key의 호환성 경계를 낮춰 hit처럼 만들지 않는다. producer 추가 또는 실행 조건 정렬은 #2866/#2911의 남은 성능 인수이며, 이 보고서는 설정을 바꾸지 않는다.

## 6. 구현·게시 상태와 의존 순서

16개 이슈별 PR이 모두 게시돼 있다. 다음 표의 구성 commit은 macOS 후속 수정까지 게시한 source/restack 기록을 기준으로 하며, CI 결과는 위의 시점별 관측과 별개다. 앞의 공통 기반 뒤에 native/verification 계열과 cache/local/Actions 계열이 갈라진다. #2936과 #2944는 prerelease를 직접 base로 사용한다.

| 이슈 | PR | Base | 최종 통합 구성 commit |
| --- | --- | --- | --- |
| #2902 | [#2931](https://github.com/RefinedStone/codex-exec-loop/pull/2931) | prerelease | `c2031a72eb31d1048e0ac98070de5313d869854a` |
| #2903 | [#2937](https://github.com/RefinedStone/codex-exec-loop/pull/2937) | #2931 | `f9e39762eead950b145a78236ff6f2ae7fdaca7b` |
| #2904 | [#2939](https://github.com/RefinedStone/codex-exec-loop/pull/2939) | #2937 | `044dc3c798e60829f4af473f4b3fc96dfdb29988` |
| #2930 | [#2932](https://github.com/RefinedStone/codex-exec-loop/pull/2932) | #2939 | `501fafb59575563c022db8f05d98ccf7b8d87de8` |
| #2905 | [#2940](https://github.com/RefinedStone/codex-exec-loop/pull/2940) | #2932 | `ed03f99a6b122cfd2e56b5e48c1b922a0b953d3d` |
| #2864 | [#2934](https://github.com/RefinedStone/codex-exec-loop/pull/2934) | #2940 | `87b5886656d5e1f98c4bd6e82529d27efbe2086c` |
| #2908 | [#2945](https://github.com/RefinedStone/codex-exec-loop/pull/2945) (Draft) | #2934 | `38efaebd651c4b6c3879975650057fcbe209ccbc` |
| #2909 | [#2938](https://github.com/RefinedStone/codex-exec-loop/pull/2938) | #2945 | `a389504bfcc2dc37096a02cb5db1e27840844745` |
| #2910 | [#2946](https://github.com/RefinedStone/codex-exec-loop/pull/2946) (Draft) | #2938 | `ceb9f629b30480581ce412f0f089550ca1b085c6` |
| #2912 | [#2941](https://github.com/RefinedStone/codex-exec-loop/pull/2941) (Draft) | #2946 | `3d23199ac0d304e563f20ece52d853b5f31ea305` |
| #2866 | [#2935](https://github.com/RefinedStone/codex-exec-loop/pull/2935) | #2940 | `35f48e915a15dc9c4d7091a91ee66d6402e3c05c` |
| #2906 | [#2942](https://github.com/RefinedStone/codex-exec-loop/pull/2942) (Draft) | #2935 | `7b0eb17ccd07b63f6fe6b122fd5233c2ade2fe52` |
| #2907 | [#2943](https://github.com/RefinedStone/codex-exec-loop/pull/2943) (Draft) | #2942 | `a3243d5430dcd57ef1dbe5c6e95442e1256e1c70` |
| #2865 | [#2933](https://github.com/RefinedStone/codex-exec-loop/pull/2933) | #2943 | `0347fe71ad69d82b82abd3d54f901fdd0fdf1950` |
| #2867 | [#2936](https://github.com/RefinedStone/codex-exec-loop/pull/2936) | prerelease | `fbdfbc1aa89421837e9c3a03b922ca2d6cfe9e9d` |
| #2911 | [#2944](https://github.com/RefinedStone/codex-exec-loop/pull/2944) (Draft) | prerelease | 보고서 갱신 전 관측 `89a1acb1`; 새 문서 commit은 자기참조하지 않음 |

과거 원격 세션 중단과 #2908/#2910 미게시 상태는 해소됐다. PR 게시 완료를 성능 인수 완료나 머지 허용으로 계산하지 않는다.

## 7. 전환 정책과 full 복귀

[#2907](https://github.com/RefinedStone/codex-exec-loop/issues/2907) / [Draft PR #2943의 고정 전환 문서](https://github.com/RefinedStone/codex-exec-loop/blob/31f28cbd7dc5c60361f55b6c6eae31038f39904e/docs/reference/ci-workflow-plan.md)는 현재 postmerge를 full로 유지한다. 직전 10df 및 후속 9098에서 실제 생성 계획은 full이며 전체 gate 실행 성공은 별도 인수다. source SHA/plan hash에 결합된 package/target/OS/features/profile 행, 완전한 결과 집계와 기존 게이트를 연결하지만, 실제 새 Actions 경로의 인수가 끝난 상태는 아니다.

- 미분류 입력·미등록 consumer·공통 계약·manifest/lockfile/toolchain·불명확한 graph는 full로 남는다.
- root consumer와 검증되지 않은 native/root 영역을 생략하지 않는다. 현재 root가 issue-core 등의 consumer인 변경도 full이다.
- 필수 계약 행 누락, 지원하지 않는 compile-only/target/feature/격리 조건, 빈 선택, matrix 한도 초과도 full로 돌아간다.
- `Fast Gate`, `CI Gate`, `Post-Merge Gate` 이름과 기존 admission 범위를 보존한다. Ruleset·bypass·필수 context 변경은 [#2868](https://github.com/RefinedStone/codex-exec-loop/issues/2868)의 별도 범위이며 여기서는 하지 않는다.
- postmerge 선택 전환, nextest 기본 전환, self-hosted runner 도입 또는 release 보호 변경을 자동 후속으로 시행하지 않는다.

명시적 복귀/검증 경로는 다음과 같다. 이는 실행 절차를 기록한 것이며 이 보고서를 위해 전체 gate를 중복 실행한 것은 아니다.

```text
Actions: Native PR Checks → workflow_dispatch → scope=full
Local:   node scripts/agent-plan.mjs --run --full
Windows: powershell -File scripts/check_native_pr.ps1 -Mode Full
POSIX:   bash scripts/check_native_pr.sh --full
```

현재 push/postmerge와 최종 통합·릴리스는 전체 검증을 유지한다. 로컬/PR에서 효과가 확인되더라도 postmerge가 full이면 “머지 후 전체 대기시간도 단축됐다”고 표현하지 않는다.

동일 workflow run에서 **같은 SHA·같은 계획**으로 코드 수정 없이 재실행할 때만 이전 성공 행의 source SHA, plan/matrix hash와 실행 좌표를 검증하여 실패한 작업만 다시 실행할 수 있다. 코드 또는 계획이 바뀌면 이전 성공 결과를 새 전체 검증의 성공으로 재사용하지 않는다. artifact 누락·중복·실패·취소·다른 조건은 성공으로 처리하지 않는다.

## 8. 남은 인수와 필요한 자료

| 인수 항목 | 현재 상태 / 필요한 근거 |
| --- | --- |
| 현재 원격 실패 | #2939/#2940 assertion 실패 2건의 원인 미확정. #2938 macOS E0425는 시험 fixture 수정 후 새 HEAD 재검증 대기. 이전 결과를 새 SHA 성공으로 승계하지 않음 |
| Windows native | ACL 실패 3개가 남아 있다. 신뢰 정책·시험 조건을 완화해 성공으로 만들지 않음 |
| 전체 inventory | Windows/default/lib의 6,418개 보존과 신규16은 확인. Linux/macOS, all-features, bin/integration, root doctest와 특수 helper·IPC/플러그인은 남음 |
| 실제 선택 실행 | 최종 source/planHash 및 full fallback은 확인. 실제 selected matrix·동일 SHA/계획의 실패 행 재실행은 미인수 |
| 큰 runtime 계약 | lease/authority/직렬화, 재시작·취소·Unknown, root 큰 suite 잔여의 최종 3 OS 인수 필요 |
| 원격 캐시 효과 | 실제 Dirty 원인, trusted sealed cache 복원 효과, trusted producer가 없는 namespace 반복 cold 여부와 hashing/전송 비용 미확정 |
| 동일 범위 성능 | 원래 N=1 표본과 단독 crate 비교는 보존. 같은 조건의 compile/link/test/wall/queue/runner 점유와 p50/p90은 미측정 |
| 운영 전환 | postmerge·최종 통합·릴리스 full 유지. Ruleset/bypass/필수 context 변경 없음 |

최종 로컬 정확성 명령의 wall은 lib no-run **59.018초**, Clippy **77.900초**, member doctest 명령 **13.539초**였다. 이미 채워진 전용 target, jobs=4, 달라진 workspace/시험 범위와 동시 작업 조건에서 얻은 단일 검증 시간이다. 이전 root 기준선(jobs=8)이나 전체 CI의 개선율로 계산하지 않는다. architecture 69.02초 역시 최종 Windows correctness 실행이며 이전 Linux 144개 표본과 동등 비교가 아니다.

**`acceptance.complete=false`를 유지한다.** 이 문서는 구현 PR 게시와 확인한 근거를 정리하며, 미검증 범위·실패·과거 관측을 성공으로 합치지 않는다. 최신 CI 시각별 추가 상태와 이 보고서 출판 후 #2944 gate는 부모 이슈/PR 댓글에서 별도로 마감한다.
