# CI 선택 검증의 성능 인수·전환 현황 — 2026-11-27

관련: [#2911](https://github.com/RefinedStone/codex-exec-loop/issues/2911), 상위 [#2690](https://github.com/RefinedStone/codex-exec-loop/issues/2690).
기록 기준: `prerelease@caf1da09aa561d1510b3271cc663b8e5bf3a0690`.
게시 상태 확인: 2026-11-27 08:45 UTC.
기계 판독 자료: [ci-rollout-acceptance-20261127.json](ci-rollout-acceptance-20261127.json).

## 인수 판정

**최종 성능 인수는 미완료다.** 이 문서는 이미 수집한 구간별 측정과 실패 원인, 전환 정책 및 남은 인수를 묶은 현황 보고서다. 문서 작성이나 Draft PR 게시를 #2911 완료로 계산하지 않는다.

| 항목 | 현재 값 |
| --- | --- |
| 최종 조합 소스 SHA | `null` — 아직 확정·검증하지 않음 |
| 최종 검증 계획 해시 | `null` — 최종 소스에 결합한 계획 없음 |
| `acceptance.complete` | `false` |
| 전체 workspace/OS 최종 inventory | 미검증 |
| 로컬 반복 개발 | 독립 crate의 제한된 N=1 관찰 있음 |
| PR/전체 CI 개선 | 최종 조합 기준으로 불명 |
| postmerge | **full 유지**, 머지 후 대기시간 단축 주장 없음 |
| Ruleset·bypass·필수 context 변경 | 없음 |

최종 조합 SHA가 없는 상태에서 각 PR의 서로 다른 성공 결과를 더해 “전체 통과”로 만들지 않는다. Windows 원격 세션이 중단되어 게시하지 못한 native/verification-native 코드와, 읽기 리뷰 또는 Node fixture만 확인한 준비본도 제품 통합 검증 성공으로 계산하지 않는다.

## 1. 확인한 시험 실행 시간

| 구간 | 이전 | 변경 후 | 표본·범위 | 해석 |
| --- | ---: | ---: | --- | --- |
| Windows 대표 Git/SQLite 시험 1개 | 21.52초 | 18.46초 | 같은 개발 PC, 전후 각 N=1 | 해당 표본 3.06초·약 14.2% 감소 |
| Linux architecture | 127.81초 | 76.35초 | 전후 각 N=1, **144개 동일·전부 성공** | 해당 구간 51.46초·약 40.26% 감소 |
| 같은 Linux 실행의 root lib | 843.95초 | 870.59초 | 전후 각 N=1, **6,540 성공·27 ignored 동일** | 26.64초·약 3.16% 증가 |

Windows 값은 Rust 1.95.0, all features, 기본 test profile, `x86_64-pc-windows-msvc`, `--exact --nocapture --test-threads=1`의 대표 시험이다. 기준·후보에 동일한 test-only 계측을 켰고 Git 준비 510회, 계측 command-output 190회를 보존했다. 일반 데스크톱 프로세스가 있는 환경이므로 통제된 유휴 벤치마크나 hosted Windows 전체 job의 개선율이 아니다. 정확한 시험 이름·명령·하위 구간은 [#2934의 고정 문서](https://github.com/RefinedStone/codex-exec-loop/blob/6e7b44a50bbb6b829d79634efaddae3b8ad0ae07/docs/validation/ci-native-process-wakeup-20261127.md)에 있다. 이 publication SHA는 재기반된 문서 위치이며, 해당 SHA의 전체 suite를 새로 측정했다는 뜻이 아니다.

Linux 수치는 다음 두 실행의 libtest 결과다.

- 기준: [run 45936932223 / Rust job 140302241613](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45936932223/job/140302241613), 실제 checkout `caf1da09aa561d1510b3271cc663b8e5bf3a0690`.
- 변경 후: [PR #2933 / run 45953138161 / Rust job 140353445633](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45953138161/job/140353445633), PR head `74a33850ee310f20417510e5aa19031749951653`, 실제 merge checkout `49a04f9cf590f8564eb08f8a6b3461ce47173c73`.

각각 한 번의 hosted 실행이다. architecture 구간은 짧아졌지만 같은 실행의 lib 구간은 길어졌다. 두 구간의 관찰을 전체 CI·PR gate·postmerge 개선율로 확대하지 않으며 중앙값 분포나 p90을 추정하지 않는다.

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

## 3. Inventory 보존: 부분 근거와 최종 미검증

현재 부분 근거는 다음과 같다.

- 분리 전 Windows default root libtest 목록은 **6,418개, ignored 24개**다. 목록 수집은 시험 본문 실행이 아니다.
- issue-core로 매핑한 기존 시험 53개와 새 crate의 53개가 일치했고, 이동한 ignored는 0개다. compile-fail doctest 2개를 확인했다. root 잔류 6,365개/ignored 24개는 해당 단계의 **예상값**이며 최종 조합 inventory가 아니다. [#2937 근거](https://github.com/RefinedStone/codex-exec-loop/blob/4983131b3a519504562494c86d339ce30f9cacbb/docs/validation/ci-issue-core-extraction-20261127.md)
- 공개된 이전 #2938 head의 Windows/macOS에서는 issue-core 53개, verification 24개, compile-fail 2개가 각각 성공했다. verification 24개는 Windows 0.44초, macOS 0.45초였다. [Windows 로그](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45957694809/job/140370019000), [macOS 로그](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45957694809/job/140370019005)
- 같은 #2938 Linux run은 root lib 6,470개 성공/27 ignored 뒤 architecture 143 성공/1 실패에서 중단됐다. 새 crate의 Linux 시험이나 이후 통합 인수를 완료한 결과가 아니다. [Linux 로그](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45957694809/job/140370017716)

**최종 Linux/Windows/macOS별 전체 workspace inventory는 아직 없다.** default/all features, bin/integration, doctest와 compile-fail, ignored child-process/native confirmation helper, architecture, IPC/플러그인 계약을 최종 SHA에서 대조해야 한다. lease·authority·직렬화, 재시작·취소·Unknown 결과와 root의 큰 suite 잔여도 포함한다. 시험 이름이 이동한 경우 old/new mapping과 root 잔류를 함께 확인하며, 단순 시험 개수만 같다고 보존을 판정하지 않는다.

## 4. 공개 CI 실패를 성능 문제와 구분

| 증거 | 관측 | 분류·현재 상태 |
| --- | --- | --- |
| [기준 Windows job](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45936932223/job/140302241610) | 03:55:45 시작, 04:55:51 취소 | 약 60분 job 제한. 뒤 계약 결과가 없어 전체 성공 아님 |
| [#2933 Windows job](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45953138161/job/140353445607) | `host_mailbox`, `elapsed_ms=5523`, `reply=TimedOut`; 해당 filter 22 성공/1 실패 | **개별 5.523초 대기 만료**, 60분 job 제한과 다름 |
| [#2938 Linux job](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45957694809/job/140370017716) | Port shim 4개가 “Port trait을 정의해야 한다” 검사에 실패 | 재export를 따르지 않는 architecture guard. 기반 issue-core 2개 + verification 2개 |
| [#2938 Windows](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45957694809/job/140370019000) / [macOS](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45957694809/job/140370019005) | `Native contract filter matched no tests: domain::planning::verification::` | 이동한 시험과 옛 root-only prefix 조합. 0건을 성공으로 처리하지 않고 중단 |

Port shim 중 issue-core 2개는 [#2903 기반 run 45956401035](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45956401035/job/140364982858)에서도 실패했다. 실제 공개 reexport가 소유 crate의 공개 module·Port trait으로 이어지는지 검사하는 수정은 [Draft PR #2941의 독립 commit `1f00c46a`](https://github.com/RefinedStone/codex-exec-loop/commit/1f00c46ab5f3d7595d2c71f01ac95f43ef87c545)으로 게시됐다. 정확히 등록한 네 경로를 AST로 검증하며 무조건 제외하지 않는다. 공통 기반으로 옮겨 재기반할 수정이며, 새 회귀 시험 4개와 Rust compile/fmt는 로컬에서 실행하지 못했다. 같은 SHA의 [run 45967360068 / Rust Lint job 140399519798](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45967360068/job/140399519798)은 `cargo fmt --all -- --check`에서 포맷 차이로 실패했다. 해당 관측 시점의 Scope·Node 성공이나 진행 중인 Rust/native 작업을 최종 CI 통과로 계산하지 않는다. 후속 포맷 수정의 결과는 새 SHA에서 별도로 확인해야 한다.

#2933의 재시작 시험은 이전 기준 Windows run에서는 성공했다. 이번 변경이 해당 시험/프로덕션 코드를 직접 바꾼 것은 아니지만, 부하와 타이밍 경합 중 직접 원인은 이 로그만으로 확정하지 않았다.

#2938의 실패 로그는 공개 head `a3dcfcad`와 merge checkout `1d65c5ed`의 결과다. 로컬 재기반 `2e166a40`에는 새 runner 기반이 있지만 미push 상태다. 로컬 수정의 존재를 공개 CI 성공으로 승격하지 않는다.

## 5. 캐시 재현과 실제 원격 효과의 경계

[#2866](https://github.com/RefinedStone/codex-exec-loop/issues/2866) / [PR #2935](https://github.com/RefinedStone/codex-exec-loop/pull/2935)의 [고정 보고서](https://github.com/RefinedStone/codex-exec-loop/blob/dc67c7dd775a0c3ca5596b69edef33f58fd8beef/docs/validation/ci-cache-inputs-2026-11-27.md)와 [JSON](https://github.com/RefinedStone/codex-exec-loop/blob/dc67c7dd775a0c3ca5596b69edef33f58fd8beef/docs/validation/ci-cache-inputs-2026-11-27.json)은 독립 Cargo fixture의 **8조건 성공**을 기록한다.

빈 target, 변경 없음, 같은 바이트의 새 mtime/보정 없음, 검증된 mtime 복원, target tar 복원, Rust 내용 변경, `include_str!` 변경, build script 환경 입력 변경을 구분했다. mtime만으로 Dirty를 재현했고 동일 입력 복원 시 Fresh, 실제 입력 변경 시 Dirty와 새 동작을 확인했다.

그러나 [이전 원격 run 45563935880](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45563935880)의 **실제 Dirty 원인은 아직 미확정**이다. fixture 시간을 Akra 성능 표본으로 사용하지 않는다. trusted sealed snapshot의 실제 업로드/복원, checkout 경로·SDK/환경 차이, hashing 비용을 포함한 원격 효과도 남았다. 의존 crate 재사용과 root 재컴파일, 실제 시험 재실행은 따로 보고한다.

선택 PR job의 **restore-only**와 성공한 `main/prerelease` push의 저장 경계는 유지한다. 현재 다음 target cache namespace에는 동일 조건으로 저장하는 trusted producer가 없다.

- Linux 선택 행: explicit target. 기존 full Linux producer는 implicit host target.
- Windows/macOS 선택 행의 default features. 기존 portable producer는 all features.

따라서 첫 실행뿐 아니라 **반복 실행도 cold일 수 있다.** registry cache는 공유하지만 target key의 호환성 경계를 낮춰 hit처럼 만들지 않는다. producer 추가 또는 실행 조건 정렬은 #2866/#2911의 남은 성능 인수이며, 이 보고서는 설정을 바꾸지 않는다.

## 6. 구현·게시 상태

다음은 보고 시점의 증거 위치다. 각 행은 최종 조합 인수 완료를 뜻하지 않는다.

| 이슈 | 전달 상태 | 남은 한계 |
| --- | --- | --- |
| #2864 | [PR #2934](https://github.com/RefinedStone/codex-exec-loop/pull/2934), `6e7b44a5` | 대표 N=1 및 일부 회귀; 전체 native 전후 인수 남음 |
| #2865 | [PR #2933](https://github.com/RefinedStone/codex-exec-loop/pull/2933), `74a33850` | architecture 개선 관찰; Windows 실패와 전체 lib 병목 남음 |
| #2866 | [PR #2935](https://github.com/RefinedStone/codex-exec-loop/pull/2935), `dc67c7dd` | 8조건 fixture; 실제 원격 Dirty 원인·효과 미확정 |
| #2867 | [PR #2936](https://github.com/RefinedStone/codex-exec-loop/pull/2936), `fbdfbc1a` | 분리 전 빌드 기준선 |
| #2903 | [PR #2937](https://github.com/RefinedStone/codex-exec-loop/pull/2937), `4983131b` | 독립 crate·호환 compile·부분 inventory; 최종 전체 인수 남음 |
| #2904 | [PR #2939](https://github.com/RefinedStone/codex-exec-loop/pull/2939), `fe25700c` | 순수 공통 계획기; 그 자체가 실행 경로 전환은 아님 |
| #2905 | [PR #2940](https://github.com/RefinedStone/codex-exec-loop/pull/2940), `d6194406` | package/target runner와 실제 Cargo fixture; 제품 최종 인수 남음 |
| #2906 | [Draft PR #2942](https://github.com/RefinedStone/codex-exec-loop/pull/2942), `a5998beb` | Node 66개 통과; 실제 Cargo/PowerShell 및 최종 제품 반복 실행 미인수 |
| #2907 | [Draft PR #2943](https://github.com/RefinedStone/codex-exec-loop/pull/2943), `31f28cbd` | 공통 Node 110개(새 matrix 16개 포함) 통과; 실제 Actions·3 OS·실패 행 재실행 미인수 |
| #2908 | Windows에만 local `01e8c2b2`, 미push | native 추출을 최종 검증 완료로 계산하지 않음 |
| #2909 | [PR #2938](https://github.com/RefinedStone/codex-exec-loop/pull/2938)은 `a3dcfcad`; local 재기반 `2e166a40` 미push | 공개 CI와 로컬 새 기반 구분 필요 |
| #2910 | Windows local checkpoint `19359d5c`, 미push | 마지막 rebase 계속 명령 완료 여부와 독립 crate 빌드·시험 미확인 |
| #2912 | [Draft PR #2941](https://github.com/RefinedStone/codex-exec-loop/pull/2941), `1f00c46a` | Automation 경계 준비 + 공통 Port guard 별도 수정; Rust 컴파일·전체 계약 인수 미완료 |

원격 세션 복구 시 미확정 rebase 명령을 재실행하기 전에 실제 status/HEAD/rebase 상태를 읽는다. 이번 문서 작업에서는 원격 세션을 재시도하거나 코드 범위를 변경하지 않았다.

## 7. 전환 정책과 full 복귀

[#2907](https://github.com/RefinedStone/codex-exec-loop/issues/2907) / [Draft PR #2943의 고정 전환 문서](https://github.com/RefinedStone/codex-exec-loop/blob/31f28cbd7dc5c60361f55b6c6eae31038f39904e/docs/reference/ci-workflow-plan.md)는 현재 postmerge를 full로 유지한다. source SHA/plan hash에 결합된 package/target/OS/features/profile 행, 완전한 결과 집계와 기존 게이트를 연결하지만, 실제 새 Actions 경로의 인수가 끝난 상태는 아니다.

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

| 인수 항목 | 현재 상태 | 다음에 필요한 근거 |
| --- | --- | --- |
| 전체 workspace/OS inventory 보존 | 부분 근거만 있음 | 최종 SHA에서 old/new/잔류·추가 시험 및 ignored/doctest/특수 helper 대조 |
| 선택 범위 설명과 반례 | 계획기·실행기 fixture 및 읽기 리뷰 | 실제 최종 제품 graph에서 작은 수정·공통 계약·설정·삭제·rename 확인 |
| 미등록 consumer/full fallback | 보수적 정책 구현·준비 | 실제 연결에서 전체 경로와 게이트 결과 확인 |
| 동일 대표 편집의 전후 비용 | N=1·일부 범위만 있음 | 같은 조건의 compile/시험/전체 wall 및 최종 source/plan/run |
| 시험 생략·ignored·timeout 완화 없는 절감 | 개별 변경 근거 | 최종 목록·실행 조건 합집합 대조 |
| 로컬/PR/postmerge 별도 판정 | 구분해 보고함 | 각 경로의 실제 같은 조건 결과; 개선이 없으면 그대로 기록 |
| 전환 범위·full 복귀 | postmerge full 정책 유지 | #2906/#2907 최종 코드·문서·실제 실행의 일치 |
| 최종 SHA·계획에 결합한 전체 성공 | **차단됨** | 최종 조합 SHA와 planHash, 모든 필수 결과 |
| 효과 없는 경계·미확인 비용 기록 | 본 문서에 기록 | 실제 원격 캐시 원인과 남은 비용 추적 |

후속 측정은 기존 로그·artifact를 먼저 사용한다. 단순 관찰을 위해 같은 CI를 두 번 실행하지 않는다. 구체적으로 남은 불확실성을 해소할 표본만 수집하고 아래 조건을 고정한다.

1. 최종 소스 SHA·계획 해시·base/head graph·소유권 revision과 source/lock hash.
2. OS/runner/architecture/toolchain/features/profile/target/jobs/incremental 및 같은 대표 편집.
3. cold, warm unchanged, 작은 issue-core, 작은 native/adapter, 공통 계약 수정의 선택·전체 범위.
4. 가능한 범위에서 compile·link·시험, cache 복원/저장, queue·job·gate wall, runner 총점유, peak memory·target 크기를 분리.
5. 표본 수와 원값·범위를 기록. N=1에서 분산·p90 또는 일반적 개선율을 확정하지 않음.
6. 새 source/plan이면 새 필수 결과 집합을 인수. 같은 source/plan의 실패만 재실행할 때도 이전 결과의 동일성을 검사.

현재 JSON의 전체 CI·gate wall·queue·runner 총점유·peak memory·최종 target 크기는 `null`이다. 값을 0으로 채우거나 서로 다른 OS/feature/검증 범위의 시간으로 대체하지 않는다.

## 9. 이 보고서의 검증 범위

이 변경은 위 Markdown과 JSON 두 문서만 추가한다. 원문 문서·PR·job 로그의 SHA, 수치, 실행 범위를 대조하고 JSON 파싱·필수 null/false 상태·두 문서의 핵심 수치 및 공백 오류를 확인한다. Rust/Node 제품 코드, CI workflow, 캐시, Ruleset은 변경하지 않는다. 문서 PR의 CI 성공도 최종 제품 조합의 성능 인수 성공으로 계산하지 않는다.
