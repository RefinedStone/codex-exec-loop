# Native 원시 기능 crate 분리 검증

관련: [#2908](https://github.com/RefinedStone/codex-exec-loop/issues/2908),
[#2902](https://github.com/RefinedStone/codex-exec-loop/issues/2902),
[#2903](https://github.com/RefinedStone/codex-exec-loop/issues/2903),
[#2910](https://github.com/RefinedStone/codex-exec-loop/issues/2910).

## 경계와 실제 이동

`akra-native`는 제품 root를 normal/dev/build dependency로 사용하지 않는 workspace member다.
소유자·파일 identity·실행 파일 검사·프로세스 containment의 구현과 원래 단위시험을 함께 옮겼다.
root의 기존 경로는 현재 소비자에게 필요한 API만 `pub(crate)`로 재노출한다.

| native 모듈 | 책임 | root 소비자가 계속 소유하는 책임 |
| --- | --- | --- |
| `process_liveness` | PID 생존과 OS birth identity | lease/admission 판단 |
| `private_fs`, `windows_ace` | Windows handle identity, owner/DACL, ACE mutation 분류 | 각 store의 업무 데이터 |
| `private_directory`, `private_path` | 사용자 전용 디렉터리, ancestor chain, `PinnedPrivateDirectory` | SQLite connection/schema/transaction, journal recovery, POSIX DB lock registry |
| `stable_file` | `StableFile`, `TemporaryFile`, bounded read, no-replace publication | evidence DTO/hash/receipt 및 artifact 업무 규칙 |
| `private_temp` | 신뢰 parent에서 private temp 디렉터리의 원자적 생성과 재검증 | GitHub network/credential context |
| `trusted_executable` | native 형식, PATH 신뢰, file fingerprint와 실행 전후 identity | Codex launcher/shim 정책, 환경 설정, process-wide pinned command cache, credential 환경 전달 |
| `subprocess` | 명시적 timeout의 spawn/capture/containment와 controlled 취소 | 제품 설정 조회, timeout 환경변수 해석, 테스트 thread-local override, default-timeout wrapper |

`PinnedNativeExecutable`, `DirectoryChain`, `PinnedPrivateDirectory`, `StableFile`,
`TemporaryFile`의 내부 fields는 private다. 기존 검증을 수행하는 생성 메서드로만 만든다.
외부 코드가 fingerprint fields로 신뢰 증거를 만들 수 없다는 compile-fail doctest도 추가했다.

## 보존한 동작과 분리 시 주의점

- Windows suspended spawn → Job 배정 → resume, POSIX process group 및 Linux identity sweep의
  순서와 실패 처리를 보존한다. #2864의 Windows process-handle wakeup도 그대로 포함한다.
- 취소·deadline·출력 제한은 caller가 명시적으로 전달한다. root 설정 조회를 native의
  process-global callback이나 새로운 전역 상태로 바꾸지 않았다.
- root `pinned_codex_command`의 `cfg(test)` 경로 재조회와 production `OnceLock` 의미를
  유지한다. dependency crate의 `cfg(test)` 차이 때문에 캐시 정책이 달라지지 않는다.
- 기존 `CARGO_MANIFEST_DIR` 차단 범위는 workspace source root였다. native package의
  manifest 디렉터리만 차단하는 실수를 막기 위해 이 비공개 workspace crate에서
  `crates/native/../..`를 정규화해 기존 source root를 계속 차단하고 회귀시험으로 확인한다.
- directory traversal의 owner, no-follow/reparse, identity 및 Windows rename sharing을
  그대로 옮겼다. SQLite의 database anchor/descriptor close 순서와 rollback journal 처리는
  root가 계속 소유한다.
- 새 공개 `create_private_temp_directory(prefix)`는 filesystem 접근 전에 prefix를
  ASCII 영숫자·`-`·`_`의 비어 있지 않은 label로 제한한다. 기존 호출자의 고정 prefix는
  그대로 허용하며 경로·ADS를 고를 수 없다.
- Git config와 Git 명령의 의미는 후속 Git adapter 경계(#2910)가 소유한다.

## 시험 소유권

`scripts/ci-crate-ownership.json`의 native transfer 14개에 원래/새 파일과 prefix를 기록했다.
분할 파일에는 `testOwnership.moved`와 `retained`의 정확한 시험 이름을 넣었다.
전체 OS 소스 기준으로 기존 시험 56개가 native로 이동하고 19개가 root에 남는다.
이 숫자는 조건부 컴파일 전의 합집합이며 한 OS의 실행 수가 아니다.

- root에 남는 시험: 제품 timeout/default wrapper 4개, Codex/credential 정책 14개,
  Git context 통합 1개.
- `trusted_executable.rs`의 ACE 시험 1개는 `windows_ace::tests::`로 옮긴다.
  한 파일의 복수 이동을 표현하는 추가 `testMappings`를 registry validator가 검증한다.
- 빈 mapping 배열, 빈/잘못된 prefix, 잘못된 타입 및 중복 mapping은 실패한다.
- 기존 native confirmation의 별도 ignored helper 경로는 이 추출에 포함되지 않는다.
- Windows에서 이동한 ignored child fixture 1개는 같은 이름과 ignored 상태를 유지한다.

Windows 실행 목록의 실제 대조 결과는
[`ci-native-crate-inventory-windows-20261127.json`](ci-native-crate-inventory-windows-20261127.json)에 있다.
기존 #2864 root executable(6,419개 목록)의 대상 38개를 새 native executable에 역매핑했고,
누락·예상 밖 이동은 0개였다. 추가 native 단위시험은 5개다.
역매핑 전후 이름 목록의 SHA-256은 모두
`b6de49ad962c2d42eae37535dafd0ec23fd2a1899919c4dc2dcb05a5d7826cd5`다.

root 재빌드 이후의 workspace 전체 union과 Linux/macOS inventory 비교는 최종 통합/각 OS CI에서
확인한다. 이전 executable은 native 이동 대상의 전후 이름 대조에 사용했으며, root 전체의
새 workspace inventory까지 로컬에서 통과했다고 주장하지 않는다.

## 로컬 검증 결과

환경: Windows x86_64 MSVC, Rust 1.95.0, 전용 target directory.
다른 worktree의 target을 강제로 공유하거나 cache를 삭제하지 않았다.

| 검증 | 결과 |
| --- | --- |
| `cargo test -p akra-native --locked --all-features --no-run` | native lib/test executable 독립 빌드 통과 |
| `cargo clippy -p akra-native --locked --all-targets --all-features -- -D warnings` | 통과 |
| `cargo test -p akra-native --locked --all-features -- --test-threads=4` | 43개 중 39 통과, 기존 환경 실패 3, 기존 ignored 1; 시험 구간 0.45초 |
| `cargo test -p akra-native --locked --all-features --doc` | private construction compile-fail 1개 통과 |
| `node --test scripts/crate-boundaries.test.mjs` | 19개 통과 |
| `node scripts/check_crate_boundaries.mjs` | 실제 member/의존 방향 통과 |
| `cargo fmt --all`, `git diff --check` | 커밋 전 실행 |

단위시험 0.45초는 이 개발 PC의 새 native subset 한 번 실행한 값이다. Windows portable 전체,
호스팅 CI 중앙값 또는 native extraction의 시간 단축 비율을 나타내지 않는다.
기존 evidence_gate/post-turn/orchestrator/lease 통합시험은 root 소유이며 추출만으로 사라지지 않는다.

### 기존 호스트 환경 실패 3개

다음 시험은 새 crate와 추출 전 #2864 executable에서 같은 원인으로 실패했다.

| 시험 | 양쪽에서 확인한 실패 원인 |
| --- | --- |
| `trusted_executable::fingerprint::tests::pinned_native_format_and_digest_describe_the_same_bytes` | fixture의 Windows LocalAppData 상위 디렉터리 ACL이 다른 principal의 mutation을 허용 |
| `trusted_executable::fingerprint::tests::a_modified_pinned_executable_cannot_keep_its_prepared_identity` | 같은 LocalAppData 상위 ACL |
| `trusted_executable::tests::native_host_git_and_node_formats_are_accepted` | 이 PC의 Node 설치 상위 `C:/nvm` ACL이 다른 principal의 mutation을 허용 |

검사가 안전하지 않은 ancestor를 거부한 결과이며, 이 PR에서 호스트 ACL을 수정하거나
검사를 건너뛰거나 새 ignored를 추가하지 않았다. 따라서 로컬 native 전체 실행은 실패로 기록한다.
신뢰 조건을 만족하는 각 OS CI에서 최종 성공을 확인해야 한다.

## 재현과 최종 통합 범위

```powershell
cargo test -p akra-native --locked --all-features --no-run
cargo clippy -p akra-native --locked --all-targets --all-features -- -D warnings
cargo test -p akra-native --locked --all-features -- --test-threads=4
cargo test -p akra-native --locked --all-features --doc
node --test scripts/crate-boundaries.test.mjs
node scripts/check_crate_boundaries.mjs
```

native source/manifest/공유 fixture 변경은 소비자 전체와 Linux/Windows/macOS 검증으로 전파한다.
registry의 consumer coverage는 `full`을 유지한다. package/target을 구별하는 실행기는 #2905가
소유하며, 오래된 root prefix를 실제 소유 executable의 inventory로 역매핑한다.
로컬 broad root gate의 중복 실행은 최종 통합 단계로 모았다. 새 root facade와 SQLite/Git 소비자의
실제 통합, workspace 전체/각 OS gate 및 소스 변경 성능 인수(#2911)는 그 단계의 검증 결과로 판단한다.
