# A01-1 Task 완료 기준 검증

- 기록일: 2026-11-23 KST
- 대상: [#2651](https://github.com/RefinedStone/codex-exec-loop/issues/2651), 상위 [#2481](https://github.com/RefinedStone/codex-exec-loop/issues/2481)
- 기준: `prerelease@26f867815cf383a4ef927fa00ea4fbaeef7cac7e`
- 최종 코드·생성물 검증 기준: `506f524d959cda29ff9fcc385b99d925f34faa2b`
- 계약 설명: [Task 완료 기준](../reference/task-acceptance.md)

## 검증한 결과

사용자가 기존 Kernel·Stored·admin 경로에서 task의 완료 기준을 만들고 수정하며,
전체 task 조회에서 같은 planning snapshot의 기준과 기준 revision을 읽을 수 있다.
기준 revision은 호스트가 부여한다. 입력 생략/null은 기존 기준을 보존하고,
빈 목록은 명시적 삭제다. worker·system의 명시적 기준 변경은 typed 경계에서도 거절한다.

기존 task 문서 v1과 SQLite JSON은 빈 기준과 revision 0으로 읽힌다.
directions 버전은 1로 유지한다. 새 task 쓰기는 지원하는 task authority v2로 진행하며,
기존 WorkOrder task를 순수하게 채택하는 경로는 원본 문서 버전을 유지한다.

## 개발 중 확인하고 수정한 경계

### 실행 중 task의 batch 우회

실행 중 task를 먼저 Ready로 바꾸고 다음 명령에서 기준을 바꾸는 batch가
후보 상태만 검사하면 통과할 수 있었다. 회귀에서 실제 revision 증가를 확인한 뒤,
각 CAS 재시도에서 원본 snapshot을 먼저 검사하도록 보완했다.

상태 변경과 기준 변경의 순서를 바꾸거나 다른 task를 섞어도 원래 실행 중이던 task의
기준을 바꿀 수 없다. 거절 시 task authority, planning revision, audit를 유지한다.

### 기존 WorkOrder와 task 문서 버전

A01의 새 문서 쓰기를 v2로 바꾼 상태에서 기존 DB의 v1과 단순 equality를 비교하면
신규 WorkOrder 및 새 보충 task 등록이 실패했다. 기존 task 채택에도 버전을
일괄 변경하면 문서 전체 동일성 검사가 실패했다.

수정 전 실제 SQLite 회귀 6개에서 2개 통과·4개 실패를 확인했다.
수정 후에는 다음 7개 회귀가 모두 통과했다.

- 신규 WorkOrder 서비스의 v1→v2 등록.
- 새 보충 task 서비스의 v1→v2 등록.
- 지시가 없는 historical 채택의 task 문서 전체 보존.
- 신규 등록·새 보충·기존 채택의 지원/미래/역행 버전 조합과 거절 시 DB 행 불변.
- 버전 승격을 이용한 기존 task의 완료 기준 변경 거절.
- historical 서비스의 미래 버전 거절.
- 대기 중 사용자 지시의 기존 description 반영, 완료 기준·기준 revision·문서 버전 보존,
  재전송 시 추가 변경 없음.

신규 task 경로는 지원 버전 1·2 내부의 동일 버전 또는 1→2만 허용한다.
기존 task 전체와 순서, 단일 task 추가 및 감사 대상 검사는 유지한다.
historical 경로는 지원 버전을 확인한 뒤 기존의 문서 전체 동일성 검사를 유지한다.

### 초안 승격과 통합

source revision이 없는 staged/editor 초안 승격에서도 validation 전에 읽은 기존
task authority를 보존하고 같은 planning revision으로 CAS한다. 기존 task와
완료 기준을 빈 목록으로 대체하지 않는다.

staged 회귀의 최초 입력은 초기화 문서와 동일한 no-op이었으므로 revision 증가
기대가 잘못됐다. 제품의 no-op 동작은 유지하고, fixture에서 staged 문서를 실제로
편집하도록 바꿨다. 승격 전 revision 유지, 승격 후 본문 반영·revision 증가 및
task·완료 기준 전체 보존을 함께 확인했다.

#2652의 legacy publication reference 복구와 통합했다. 두 회귀 모듈을 모두 유지하고,
새 legacy fixture의 create 입력에는 `acceptance: None`을 명시했다.
실제 병합 SHA 위로 재배치한 전후 Git tree가 같음을 확인했다.

## 로컬 대상 검증

Windows MSVC, 저장소 pin Rust 1.95.0에서 실행했다.
`RUST_TEST_THREADS=1`, `CARGO_BUILD_JOBS=4`를 사용했고,
`TEMP`/`TMP`는 저장소 밖의 전용 임시 디렉터리로 지정했다.
기존 target cache를 재사용했다.

통합 검증은 다음 형태의 명령으로 실행했다.

```text
cargo test --locked --all-features --target x86_64-pc-windows-msvc --lib <filter> -- --nocapture
```

| 검증 | 결과 |
| --- | --- |
| 실제 repo-scoped SQLite admin acceptance | 2 통과 |
| admin Stored 명령의 nullable 기준·host revision 입력 거절 | 1 통과 |
| legacy DB → 사용자 기준 수정 → 재개방 → worker 일반 수정 → 전체 task 조회 | 1 통과 |
| task tool 조회·worker 입력·미래 문서 거절 | 10 통과 |
| admin form 변환 | 1 통과 |
| planning runtime validation | 16 통과 |
| bootstrap | 3 통과 |
| init의 Git workspace 승격 | 대상 3개 통과; 사용한 `git_workspace_` 필터 전체는 5개 통과 |
| repo-scoped directions reset의 미래 버전 거절 | 1 통과 |
| Kernel simulation | 24 통과 |
| command-only Kernel의 CRUD·실패·재전송·TUI 소유권 경계 | 20 통과 |
| SQLite WorkOrder 전체 | 80 통과; #2652 회귀 8개와 A01 버전 회귀 7개 포함 |
| task mutation 회귀 | 43 통과 |

개발 단계에서 domain acceptance 7개, Kernel protocol 100개,
Stored command 회귀 및 Kernel 스펙 생성 일치 검사도 별도로 통과했다.
최종 전체 Rust 동작은 PR의 POSIX CI에서 추가 확인한다.

## 생성물과 필수 gate

기존 생성 경로로 다음을 동기화했다.

- `schema/akra-kernel-spec.json`
- `desktop/src/shared/kernel-protocol/generated/index.ts`
- `assets/app-server/hub-skills/akra-kernel/SKILL.md`

첫 Full 실행에서 생성 skill 문서의 두 명령 설명 불일치를 발견해 공식 생성기로 갱신했다.
이후 clippy가 확인한 내부 `CommandOnlyWork`의 큰 payload는 Box로 저장하고,
회귀의 불필요한 clone도 제거했다. 이 린트 정리는 공개 wire 형식에 영향을 주지 않는다.

planner가 선택한 명령:

```text
git diff --check
powershell -File scripts/check_native_pr.ps1 -Mode Full
```

최종 **Full mode가 종료 코드 0으로 통과**했다.

| 항목 | 최종 결과 |
| --- | --- |
| CI 정책 검사 | 30 통과 |
| 생성 Kernel skill·Node/관리자/런처 검사 | 통과 |
| admin·Desktop 의존성 감사 | 취약점 0개 |
| Desktop 타입 검사·빌드 | 통과 |
| Desktop 단위 테스트 | 1,000 통과, 7 건너뜀 |
| Windows all-features lib/bins check | 통과 |
| Windows 계약 테스트 | 47 통과 |
| all-target/all-features clippy `-D warnings` | 통과 |
| rustfmt·`git diff --check` | 통과 |

admin bundle은 빌드 전후 정규화한 Git object hash가 동일했으며 기능 변경에 포함하지 않았다.


## 플랫폼·기능 범위

기존 POSIX direct-filesystem admin fixture는 Windows의 보호 경계에서 거절된다.
새 admin 기준 회귀는 생산 repo-scoped SQLite 조립을 사용하며 Windows에서 통과했다.
기존 POSIX fixture를 통과시키려고 Windows 보호를 완화하지 않았다.
해당 경로를 포함한 전체 Rust suite와 최종 CI Gate 결과는 PR에서 확인한다.

Desktop 변경은 생성된 계약 타입이다. 기준 편집·증거 탐색 화면은 이 변경의 완료 결과가 아니다.
로컬 전체 gate의 타입 검사·단위 테스트·빌드를 실행했으며 별도 로컬 UI E2E는 추가하지 않았다.

시험 입력은 합성 task, fake와 임시 SQLite/Git 저장소다.
실제 모델 turn, 사용자 native 확인, Akra의 외부 provider 게시 흐름을 시험으로 실행하지 않았다.
기준 저장은 검증 실행이나 pass 판정을 의미하지 않는다. 호스트 증거 수집,
observe/enforce 전달 게이트와 수동 면제는 상위 A01의 후속 범위다.
