# Akra ADE 명명 계약

- 관련: [N1 #2587](https://github.com/RefinedStone/codex-exec-loop/issues/2587),
  상위 [#2584](https://github.com/RefinedStone/codex-exec-loop/issues/2584)
- 기준: N1 이후 변경과 2026-11-27 `prerelease@fa72e69a8` 대조
- 이 문서는 자체 식별자의 현재 값과 남은 과거 명칭의 이유를 기록한다. N2·R1 코드 이전과
  실제 설치·자격 증명·산출물 검증은 구분한다. 아래 N2 근거표와 [제품 상태표](product-status.md)를
  따른다. 저장소 rename(G1)은 아직 수행하지 않았다.

## 현재 값

| 대상 | 값 | 상태 |
| --- | --- | --- |
| 제품명, Desktop 창 제목 | Akra ADE | N1에서 변경 |
| Rust package | `akra-ade` | N1에서 변경 (이전 `codex-exec-loop-native`) |
| Rust library / import 경로 | `akra_ade` | N1에서 변경 (이전 `codex_exec_loop_native`) |
| `cargo run` 기본 대상 | `akra` | N1에서 변경. 이전 기본 binary와 같은 `akra_ade::run()`을 실행한다 |
| 배포 binary, archive, npm launcher 대상 | `akra-ade`, `akra-ade-<version>-<target>.tar.gz` | R1에서 변경 (이전 `codex-exec-loop-native`) |
| Release 제목 | `Akra ADE <tag>` | R1에서 변경 |
| protocol snapshot URN | `urn:akra-ade:app-server-protocol:*` | R1에서 변경 |
| 운영 명령 | `akra`, `akra-gateway`, `akra-admin`, `akra-telegram` | 유지 |
| 로그 target 접두사 | `akra_ade::...` | N1에서 변경. `AKRA_TRACE`·`RUST_LOG`의 `codex_exec_loop_native` 값은 `akra_ade`로 바꿔 적용한다 |
| GitHub API User-Agent | `akra-ade/<version>` | N1에서 변경 |
| 루트 npm manifest 이름 | `akra-ade` | N1에서 변경. npm 배포 package는 `npm/`의 `@refinedstone/akra`이며 이름을 바꾸지 않았다 |

## 남긴 과거 명칭과 이유

| 값 | 위치 | 이유 | 담당 |
| --- | --- | --- | --- |
| `.codex-exec-loop/planning/`, `.codex-exec-loop/runtime/` | 이전 코드·호환 fixture·`.gitignore` | 새 경로는 `.akra/`. 커널 잠금 아래에서 이전하며 충돌·복구 확인용 구 경로를 유지한다 | N2 #2588 |
| `CODEX_EXEC_LOOP_*` 환경 변수 | 제거 변수 검사·호환 fixture | 새 변수는 `AKRA_*`. 구 변수는 사용하지 않으며 남아 있으면 시작을 거부하고 대체 이름을 알린다 | N2 #2588 |
| app-server `clientInfo.name`의 과거 값 `codex-exec-loop-native` | 과거 기록·fixture | 현재 네이티브 adapter는 `akra-native`를 보낸다 | N2 #2588 |
| Electron package `@refinedstone/akra-desktop` | `desktop/package.json`, ready 전 앱 정체성 | 표시명은 `Akra ADE`, userData는 `appData/Akra ADE`. ready 전 구 이름은 기존 safeStorage 키 정체성을 보존한다 | N2 #2588, R1 #2589 |
| `RefinedStone/codex-exec-loop` URL | manifest의 `repository`·`homepage`·`bugs`, 문서 링크 | 저장소 rename 전에는 이 주소만 동작한다 | G1 #2591 |
| 시험 fixture의 저장소 이름, 임시 디렉터리 접두사 | `src/**/tests*`, `tests/` | 시험 입력 값이다. 실행 동작에 영향이 없다 | 유지 |
| `docs/validation/`, `docs/reviews/`, `.omo/evidence/` | 과거 기록 | 당시 사실을 기록한 문서다 | 유지 |

## 외부 고유명

`codex app-server`, Codex protocol 타입, 사용자 Codex 홈은 외부 제품 이름이다. 자체 명칭 정비
대상이 아니다.

## N2 호환 확인 (2026-11-26)

- 관련: N2 [#2588](https://github.com/RefinedStone/codex-exec-loop/issues/2588).
  macOS에서 임시 HOME·`AKRA_HOME`·git 저장소와 가짜 자격 증명만 썼다. 실제 사용자 데이터와 OS 키
  저장소 항목은 읽거나 바꾸지 않았다.
- 위 표의 N2 행 중 다음은 이전을 마쳤다: `.codex-exec-loop/`는 `.akra/`로 옮긴다.
  `CODEX_EXEC_LOOP_*`는 제거했고, 남아 있으면 모든 진입점이 시작을 멈춘다. app-server
  `clientInfo.name`은 `akra-native`이다. Desktop userData는 `appData/Akra ADE`로 옮긴다.
- 바꾸지 않은 것: `AKRA_HOME`과 `~/.akra`, 저장소 incarnation marker, authority DB 위치, Gateway
  주소, OS 키 저장소 이름(`akra-evidence-vault`/`device-key-v1`, `akra-provider-credentials-v1`),
  운영 명령 `akra`.

| 경우 | 확인 방법 | 결과 | 확인하지 않은 것 |
| --- | --- | --- | --- |
| Vault 키와 provider 자격 증명이 이름 변경 뒤에도 같은 OS 키 저장소 항목을 쓴다 | 단위 시험 `key_store_names_stay_the_same_across_the_product_rename`, `key_store_service_stays_the_same_across_the_product_rename`. 기존 키 재사용은 기존 시험 `initialization_reuses_the_durable_key_without_persisting_key_bytes`(가짜 키 저장소) | 통과. N1·R1·N2가 이 이름을 바꾸지 않았다 | 실제 OS 키 저장소 |
| Desktop의 기존 safeStorage 암호문(이전 Jira·Linear 토큰, 브라우저 쿠키)을 계속 푼다 | Electron 45.2.3 소스 확인: 앱 이름으로 macOS `<이름> Safe Storage`, Linux libsecret application을 ready 전에 정한다. `productName` 변경이 이 이름을 바꿨다. 결함을 고쳤다: ready 전에는 이전 이름 `@refinedstone/akra-desktop`을 쓰고 ready 뒤 표시 이름을 `Akra ADE`로 돌린다. 시험 `keepCredentialIdentity` | 고침. 단위 시험 통과. 고치기 전 버전(`dddf125b`~)을 실행한 동안 새로 저장한 쿠키는 다시 로그인해야 한다 | 실제 키체인에서 복호화. 시험이 실제 키체인 항목을 만들기 때문이다. Windows(DPAPI 키가 userData 안에 있어 폴더와 함께 옮겨진다) |
| `.codex-exec-loop`와 `.akra`에 같은 항목이 함께 있다 | 단위 시험 `layout_migration_refuses_both_layouts_without_changes` | 아무것도 옮기지 않고 두 경로를 오류에 적는다. 사용자가 하나를 남긴다 | |
| 이전이 중간에 멈춘 뒤 다시 시작한다 | 단위 시험 `layout_migration_failure_keeps_the_original_and_resumes`. DB 경로는 `authority_schema_v21_path_conflict_rolls_back_the_whole_migration` | 옮기지 못한 항목은 원래 자리에 남고, 다음 시작이 나머지를 옮긴다. DB는 transaction 전체를 되돌린다 | 프로세스 강제 종료 |
| 두 프로세스가 같은 저장소에서 동시에 시작한다 | 단위 시험 `workspace_layout_moves_once_per_worktree_under_the_repository_lock` | 저장소 커널 잠금을 쥔 쪽만 옮긴다. 기다리는 쪽은 `RepositoryKernelLockHeld`로 멈추고 아무것도 옮기지 않는다 | |
| linked worktree와 별칭 경로 | 같은 시험. 본 checkout과 linked worktree에 각각 이전 폴더를 두고, worktree를 가리키는 symlink로도 연다 | 각 worktree는 자기 폴더만 옮긴다. 별칭으로 다시 열어도 두 번 옮기지 않는다. 같은 저장소 신원과 잠금을 쓴다 | Windows junction |
| 이전 뒤 이전 버전을 다시 실행한다 | 이전 버전의 schema 범위 검사(같은 코드의 시험 `authority_schema_migrates_v7_through_v17_additively_and_rejects_unsupported_versions`, 현재보다 높은 22를 거절) | 이전 버전은 schema 21 DB를 거절한다(다운그레이드 불가). 이전 버전이 `.codex-exec-loop/` 항목을 다시 만들면 새 버전은 병합하지 않고 위 충돌 오류로 멈춘다 | 실제 이전 버전 binary 실행 |
| 원격 URL 변경(저장소 rename)과 checkout 이동 뒤 같은 project ID | 시험 `repository_incarnation_is_stable_across_restart_and_move_but_not_clone`에 `git remote set-url`을 더했다 | 신원과 authority DB 경로가 같다. 신원은 git common dir의 marker로 정하고 원격 URL을 읽지 않는다 | 실제 GitHub rename |
| 설치한 binary를 저장소 밖에서 실행한다 | debug `akra-ade`를 임시 설치 폴더에 복사하고 release launcher와 같은 `akra` script로 실행했다. `env -i`, 임시 HOME, 저장소 밖 작업 폴더. `config set/get`, `queue`, `status`, `doctor`, `reset queue --confirm` | 모두 exit 0. 데이터는 임시 `~/.akra`에만 생긴다 | release profile binary, Linux·Windows |
| 이전 설치 구성(`codex-exec-loop-native`)에서 올린다 | 같은 시험에서 `codex-exec-loop-native-<v>-<target>/` 구성으로 설정과 저장소 DB를 만들고, 그 폴더를 지운 뒤 `akra-ade-<v>-<target>/` 구성으로 다시 실행했다. 저장소에는 이전 `.codex-exec-loop/planning/`을 두었다 | 설정 값, `projects/<namespace>/runtime/planning-authority.db`가 그대로이다. 잠금을 잡는 `reset`이 `.codex-exec-loop/planning/`을 `.akra/planning/`으로 내용 그대로 옮겼다. 데이터는 설치 폴더가 아니라 `AKRA_HOME`과 작업 폴더에 있다. npm은 launcher와 platform package의 버전을 고정하므로 함께 바뀐다 | 실제 이전 버전 binary. 두 구성 모두 새 binary 내용이다 |
| Desktop userData 신·구 폴더가 함께 있다 | 기존 시험 `never overwrites an existing new folder` | 새 폴더를 쓰고 이전 폴더를 그대로 둔다. 경고는 console에만 남는다 | |

남은 일:

- 저장소 rename(G1 [#2591](https://github.com/RefinedStone/codex-exec-loop/issues/2591))이 마지막
  단계이다. 아직 하지 않았다.
- 외부 기여 조건은 이 작업과 별개인 열린 결정이다.
- 읽기만 하는 명령(`queue`, `status`, `doctor`)은 잠금을 잡지 않으므로 `.codex-exec-loop/`를 옮기지
  않는다. 처음 옮기기 전에는 이전 폴더의 planning 파일을 보이지 않는다. 데이터는 바뀌지 않고, 잠금을
  잡는 다음 시작(TUI, Gateway 커널, 쓰기 명령)이 옮긴다.
- `akra mcp skill install`이 적은 binary 경로 안내는 설치 폴더를 바꾸면 이전 경로로 남는다. 다시
  설치하면 고쳐진다.
- `AKRA_HOME=""`(빈 값)은 설정·Gateway·Desktop이 없음으로 보고 `~/.akra`를 쓰지만, authority store는
  오류로 멈췄다. 아래 후속 호환성 검사에서 이 불일치를 제거했다.


### N2 후속 프로필 경계 (#2588)

- 명시적 AKRA_HOME은 Rust 설정/authority/Gateway 클라이언트와 Desktop 접속 주소에서 공통 의미를 갖는다. 빈 값, 상대 경로, 상위 경로 이동은 오류다. 기본값을 쓰려면 변수를 제거한다. 명시된 오류를 다른 프로필로 조용히 fallback하지 않는다.
- 변수가 없으면 Unix HOME 또는 Windows USERPROFILE의 .akra를 사용한다. Windows USERPROFILE이 없는 특수 실행 환경에서는 기존 HOME/LocalAppData 두 fallback의 서로 다른 저장소를 자동 선택하지 않고, 사용자가 기존 프로필의 절대 AKRA_HOME을 지정하도록 한다. 기존 OS-account 기준 보안 조정 경로 및 Windows 사용자 SID pipe 이름 자체는 변경하지 않는다. 테스트 전용 AKRA_GATEWAY_ENDPOINT의 명시적 우회 계약은 유지한다.
- Windows Electron 45.2.3의 폐기 가능한 appData에서 실제 safeStorage/DPAPI 암호문, 지속 쿠키, 설정을 생성하고 제품의 keepCredentialIdentity와 prepareUserData로 구 profile을 새 이름에 옮겼다. 세 개의 독립 Electron 프로세스(seed/upgrade/reopen)에서 복호화 및 쿠키·설정 보존을 확인했다. 명령: desktop에서 node test/e2e/userdata-dpapi.mjs. 실사용 계정 자격 증명 또는 실제 외부 서비스는 이용하지 않았다.
- macOS/Linux 키체인 시험은 별도의 폐기 가능한 OS 계정이 필요하므로 아직 수행하지 않았다. 실제 이전 binary 다운그레이드, 이미 옛 이름으로 열린 프로세스와의 동시 사용은 여전히 별도 검증이다. 이전/새 폴더를 자동으로 병합하거나 사용자 데이터를 덮어쓰지 않는다.

- 이 변경은 로컬 profile root 해석의 일치까지만 보장한다. Windows named pipe는 SID에 묶여 있으므로 유효한 다른 AKRA_HOME으로 실행한 클라이언트도 이미 실행 중인 같은 SID의 Gateway에 붙을 수 있다. 로컬 설정 경로가 올바르다는 사실은 접속한 Gateway가 그 프로필을 소유한다는 증거가 아니다. 서로 다른 프로필을 동시에 사용하지 말고, 전환할 때 기존 Gateway를 정상 종료한 뒤 원하는 절대 경로로 재시작한다. #2588 종료 전에 프로필 식별자 일치 계약 또는 다른 프로필 접속 거절과 실제 두 프로필 회귀를 별도로 검증해야 한다. 기존 SID pipe/보안 저장소 이름을 이번 변경에서 바꾸지 않는다.
