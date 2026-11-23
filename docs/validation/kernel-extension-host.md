# 커널 확장 호스트 검증

검증 환경: 2026-11-23, Windows `x86_64-pc-windows-msvc`, Rust 1.95.0, Node 24.13.0.
기준 소스: `prerelease@70966a27f27429f9134e467c98b7b09a9894e39d`.
대상: K1–K6 공통 호스트와 화면 없는 예제. 저장 공간 분석기의 볼륨 검사, 색인 reader 수명 및 AI 공유 권한은 이 호스트를 사용하는 후속 기능에서 검증한다.

## 이슈와 피드백 반영

| 이슈 | 반영한 계약 | 검증 근거 |
| --- | --- | --- |
| [K1 #2598](https://github.com/RefinedStone/codex-exec-loop/issues/2598) | 패키지/API 버전 구분, manifest와 handler 전수 검사 후 패키지 단위 등록, 실패 사유를 catalog에 보존, 정상 중복 ID 덮어쓰기 금지. operation마다 독립 JSON Schema와 로컬 참조를 사용한다. | 정상·잘못된·중복·handler 불일치 패키지를 함께 등록하는 테스트, 서로 다른 확장의 동명 `$defs` 테스트. 기존 내장 정의와 합치지 않는다. |
| [K2 #2599](https://github.com/RefinedStone/codex-exec-loop/issues/2599) | local-user와 repository attachment를 별도로 유지한다. 서버가 호스트 ID와 OS 사용자 저장 경로를 결정한다. 하나의 Gateway registry와 프로세스 잠금이 로컬 호스트를 공유한다. | 두 클라이언트의 동시 최초 attach, repository/local 동시 사용, A→B 전환과 local detach·재연결 뒤 같은 job 조회, 잘못된 host와 이전 generation 거부, native store 이중 소유 거부. |
| [K3 #2600](https://github.com/RefinedStone/codex-exec-loop/issues/2600) | desired setting, lifecycle, activation revision, host generation 분리. CAS와 저장 성공 이후 통지. 접수·dispatch 및 즉시 응답 완료 시 권한/활성 revision 재검사. 비활성 관리 조회는 상태를 보여주되 result/progress를 숨긴다. | 저장 실패 시 활성화와 통지가 발생하지 않는 테스트, revision 충돌, 미지원 API·미허가 permission, 비활성 호출, disable 중 결과 숨김, 재시작 설정 복원. 실제 볼륨 범위 검증은 storage handler 책임이다. |
| [K4 #2601](https://github.com/RefinedStone/codex-exec-loop/issues/2601) | 요청 키 재시도, handler가 정규화한 조건의 진행 중 작업 공유, 완료 뒤 새 명시적 실행을 구분한다. 접수는 durable job ID를 반환하고 작업은 command loop 밖에서 실행한다. | 동시 최초 요청 1회 실행, 다른 input으로 동일 요청 키 재사용 거부, 기존 receipt 재조회와 완료 후 rescan ID 구분. bounded event lag, host generation 및 snapshot sequence, 재연결 job 조회. |
| [K5 #2602](https://github.com/RefinedStone/codex-exec-loop/issues/2602) | 비활성화는 새 호출을 차단하고 활성 job을 `cancelling`으로 저장한다. 워커 종료와 cleanup을 별도로 관찰하며 cleanup 완료까지 확장은 `disabling`이다. 재시작은 미완료 작업을 `interrupted`로 복원하고 자동 실행하지 않는다. | 중복 취소, 취소 직전 성공 결과, 이미 완료된 결과의 취소, worker unwind panic, cleanup 지연·실패·재시도, 빠른 off/on 거부, 옛 cleanup의 새 job 비침범. 실제 자식 호스트 프로세스 강제 종료 후 SQLite 재오픈도 검사한다. |
| [K6 #2603](https://github.com/RefinedStone/codex-exec-loop/issues/2603) | `akra.example`은 composition 등록만으로 노출된다. CLI는 catalog 입력 검사와 Gateway의 동일한 호스트 호출을 사용한다. JSON-RPC 거절의 code/message/data를 보존한다. | 미등록 ID, 미지원 버전, 잘못된 입력, 비활성 호출에서 Desktop/CLI 거절 JSON 동일성. catalog가 거절했는데 서버가 성공한 경우 별도 계약 불일치 오류. 예제 출력 schema 검증과 headless catalog, 생성 명세·타입 확인. |

댓글의 추가 조건은 각 이슈에 연결된 2026-11-22 리뷰를 기준으로 대조했다.

## 취소·복구의 정확한 범위

워커는 현재 프로세스 안의 Rust thread다. 취소는 공유 원자 플래그를 전달하는 협력적 방식이며, 이미 제출한 OS I/O를 강제로 끊는 API는 제공하지 않는다. 취소 요청을 받은 작업은 워커 반환 전까지 `cancelling`으로 유지한다. 워커가 반환한 뒤 job terminal 상태와 별도의 `cleanup` 상태를 기록한다. 따라서 `cancelled`만 보고 물리적 파일 삭제까지 완료됐다고 판단하면 안 된다. 확장의 `disabled`는 워커와 cleanup이 모두 끝난 뒤에만 제공한다.

Rust unwind panic은 job 실패로 수습하며 다른 정상 호출은 계속된다. 네이티브 abort, 메모리 손상, 프로세스 강제 종료를 별도 worker 프로세스로 격리하지는 않는다. 그 경우 호스트가 다시 시작되어야 durable `interrupted` 상태를 조회할 수 있다. 강제 종료 회귀는 disposable test host에만 실행하며 실제 Gateway나 사용자 작업을 종료하지 않는다.

정리 callback은 이전 job ID와 그 job의 generation을 받는다. 공통 호스트는 재활성화를 정리 완료까지 차단하고 동일 job cleanup의 동시 실행을 막는다. 실제 색인 reader가 열린 동안 파일 제거를 조정하는 일과 정리 실패가 사용자 원본 경로로 확장되지 않는 보장은 storage adapter의 별도 회귀에서 확인해야 한다.

## 보존과 재구독

- 결과 보존은 완료 뒤 24시간이다. 시작 시, 새 작업 접수 시, 60초 간격 sweep에서 만료 결과를 정리한다.
- 저장 job 최대 128개, receipt 최대 512개, 동시 활성 job 최대 8개다. 모든 결과가 아직 보존 중이면 새 작업을 거절한다. 만료 또는 명시적 비활성화로 이미 정리된 종료 metadata만 신규 접수와 같은 저장에서 교체한다.
- 아직 만료되지 않은 요청 receipt는 job이 제거되어도 tombstone으로 남아 재시도가 새로운 작업을 만들지 못하게 한다.
- 이벤트 broadcast 버퍼는 64개이며 진행 기록은 최대 초당 10회다. 느린 구독자는 `resync:true`를 받고 catalog/jobs snapshot을 다시 읽는다.
- host generation과 sequence가 기준이다. 재접속은 과거 이벤트의 무한 replay를 요청하지 않고 새 snapshot에서 시작한다. catalog의 각 row에 해당 snapshot의 sequence가 들어간다.

## 내장 계약 보존

기준 명세와 현재 명세를 JSON 값으로 비교한 결과 `protocol_version`, `commands`, `queries`, `errors`와 확장 이외의 내장 타입 정의 205개가 동일했다. 기존 `akra.tech-blog`는 호환 adapter로 유지하며 기존 worker 및 다른 Desktop 구독으로 변경 통지가 전달되는 회귀를 함께 실행했다.

## 검증 명령

```text
cargo test --locked --lib extension --no-fail-fast
cargo test --locked --lib adapter::inbound::kernel_protocol::tests::
cargo test --locked --lib composition::kernel_host::simulation
node scripts/generate-kernel-skill.mjs --check
node desktop/scripts/generate-kernel-protocol.mjs --check
powershell -NoProfile -File scripts/check_native_pr.ps1 -Mode Full
cargo fmt --all -- --check
git diff --check
```

최종 실행 결과: 확장 필터 31개, 커널 프로토콜 32개, 커널 시뮬레이션 24개가 모두 통과했다. Full native gate는 exit 0으로 완료했고, 그 안의 Desktop 검사는 1,059개 통과·7개 환경별 skip, 타입 검사·번들 및 Windows 계약 검사·clippy가 통과했다. schema/skill 생성 결과 일치, fmt와 diff 검사도 통과했다.

이 환경에서는 실제 다른 OS 계정, 원격 호스트와의 동시 사용, Linux/macOS native 실행을 수행하지 않았다. 호스트 ID/generation 격리와 Windows private store·ACL·process ownership은 자동 회귀로 확인한다. 모델을 실행하는 `turn/start`나 `turn/steer`는 사용하지 않는다.

## PR #2663 CI 및 자동 리뷰 후속 검증

첫 원격 실행의 Rust 라이브러리 테스트는 Linux에서 5,301개 통과·14개 ignored였지만, `architecture_boundaries`는 142개 통과·2개 실패였다. 로컬 Windows Full 스크립트는 해당 통합 테스트를 실행하지 않으므로 이후에는 이를 별도 필수 검증으로 실행한다.

- Gateway는 application inbound `ExtensionHostPort`만 의존한다. 구체 `ExtensionService`를 transport가 직접 import하지 않으며, 호출 신원 `ExtensionContext`는 domain이 소유한다.
- composition spawn 검사는 기존 `is_test_only_path` 판별을 적용한다. `#[cfg(test)] mod tests`에서 실행하는 disposable 프로세스가 production worker로 잘못 집계되지 않는다. production spawn 제한은 유지한다.
- 블로그 catalog는 실제 Python studio의 입력 필드, 길이, enum, 선택 입력 기본값을 반영한 schema를 제공하고 Gateway가 동일 schema를 검사한다. 실제 Python 워커의 configure/create/list/save/open을 CLI 경로로 검증하여 생성이 한 번만 일어나고 성공이 CLI 오류로 변하지 않음을 확인한다. write/review는 fake Gateway 응답으로 확인하며 모델 턴은 실행하지 않는다.
- 128개 보존 작업, 8개 실행 작업 상태에서 병렬 중복 요청과 추가 작업 거부가 미만료 결과를 변경하지 않는지 검증한다. 24시간 미만 결과의 조기 용량 eviction은 하지 않는다. 정리된 metadata 교체와 신규 접수는 같은 저장에 포함하며 저장 실패, 동시 접수, 기존 receipt tombstone 재시도를 검증한다.

후속 로컬 검증 결과:

| 명령 / 검사 | 결과 |
|---|---|
| `cargo test --locked --lib extension --no-fail-fast` | 37개 통과 |
| `cargo test --locked --test architecture_boundaries` | 144개 통과. 원격 CI 실패 두 검사를 포함 |
| `cargo test --locked --lib adapter::inbound::kernel_protocol::tests --no-fail-fast` | 32개 통과 |
| `cargo test --locked --lib composition::kernel_host::simulation --no-fail-fast` | 24개 통과 |
| `bash scripts/refresh_akra_kernel_spec.sh`, skill 및 Desktop 타입 생성 확인 | 통과. `ExtensionManifest.operations`만 추가. 기존 protocol_version/commands/queries/errors와 나머지 216개 정의 동일 |
| `node scripts/agent-plan.mjs` | Full 요구 확인 |
| `powershell -NoProfile -File scripts/check_native_pr.ps1 -Mode Full` | exit 0. Desktop 1,059개 통과·7개 skip, 타입/빌드, Windows native contracts, 전체 target/features clippy 통과 |
| `cargo fmt --all`, `git diff --check` | 통과 |

보존 한도가 찼을 때 새 작업이 거절될 수 있으나, 기존 결과는 24시간 만료 또는 명시적 비활성화 정리 전까지 보존한다. 이미 정리된 metadata만 신규 접수와 원자적으로 교체한다. 외부 Gateway와의 최종 원격 CI 상태는 PR에서 확인한다.

## PR #2668: 블로그 v2 base와 만료 정리 경합

`prerelease@bf87a3c643fe798a3258b4a0776ae314af655f5f`가 블로그 확장을 v2로 갱신하여 기존 K 변경과 실제 merge 충돌이 생겼다. v2 VERSION/ACTIONS와 generate·resume의 busy guard를 유지하면서, machine catalog와 Gateway 입력 검증을 실제 Python v2 계약으로 갱신했다. CLI 기본 version 1은 유지하며 블로그 호출에는 명시적인 `--version 2`가 필요하다.

실제 Gateway 회귀는 fixture DB에 저장된 v1 글과 v2 stopped 글을 넣고 configure/targets/list/open/save/cancel/delete 및 잘못된 입력의 무변경을 확인한다. generate·resume 성공 응답과 요청 1회는 fake socket으로 검사한다. 추가 회귀에서 public action과 machine schema의 일치, commits의 형식·중복·한도, targets limit, 생략 가능한 기본값, 명시 CLI version 2와 기본 version 1을 확인한다. 기존 bundled worker 회귀는 collect 단계 한 번 뒤 취소하며 모델 단계로 진행하지 않는다.

[자동 리뷰의 만료 정리/disable 경합](https://github.com/RefinedStone/codex-exec-loop/pull/2668#discussion_r4661680408)은 sweep이 cleanup claim을 소유한 동안 disable 작업이 먼저 반환하면 발생한다. sweep의 cleanup 완료 뒤 worker callback을 다시 실행하지 않고 비활성화 완료 조건만 재검사하도록 수정했다. barrier 회귀는 sweep의 cleanup을 막은 상태에서 disable worker가 먼저 종료한 것을 확인한 후 cleanup을 풀고, durable disabled 전이·정리 1회·claim 해제·재활성화를 확인한다.

통합 소스 `cc02a098f98c9b619e444f4ad60ec6bbb9a774ae`의 focused 검증 결과: extension 44개, architecture_boundaries 144개, kernel protocol 32개, kernel simulation 24개가 모두 통과했다. 최신 `origin/prerelease`를 기준으로 planner가 요구한 Windows Full도 동일 소스에서 exit 0으로 완료했다. Desktop은 65개 test file 통과·1개 skipped, 1,064개 test 통과·7개 skipped였다. Node/admin 검사, Desktop 타입 검사·빌드, Windows native contracts, 전체 target/features clippy도 통과했다. 최종 fmt와 diff 검사 통과 및 clean worktree를 확인했으며 모델 턴은 실행하지 않았다. 이 결과를 기록하는 후속 문서 커밋은 Rust·Desktop 소스를 변경하지 않는다. 최종 원격 CI 상태는 PR의 최신 HEAD 검증 결과로 확인한다.

## PR #2668: 최신 prerelease의 편집·내보내기 계약 통합

2026-11-23에 확인한 실제 기준 브랜치 `prerelease@ca2fe075a2ef2c499dfdda030208ec1fd52e5155`를 병합했다. 이전 기준 `bf87a3c` 이후의 블로그 다시 쓰기(`revise`), 검수(`review`), 이전 판 복원(`restore`), 작업 폴더 내보내기(`export`)를 포함한다. upstream의 Python 실행 동작, 15개 public action, version 2, 네 개 모델 작업의 busy guard 및 `workspace.write` 권한을 유지한다.

Machine catalog에 새 action의 실제 입력 계약을 등록한다. `revise.instruction`은 생략·빈 문자열이 가능하고 최대 1,000자이며, 네 새 action 모두 article ID와 현재 revision을 받는다. 목록의 `task`, 이전 v2 글의 `edition/previous/exported/slug` 기본값과 새로운 판·내보내기 응답을 보존한다. CLI 기본 version 1은 그대로이고 블로그에는 `--version 2`를 명시한다.

실제 Gateway 회귀는 저장 fixture만 사용하여 새 판 저장·이전 판 복원·같은 글의 반복 내보내기·기존의 타인 그림 덮어쓰기 거절을 확인한다. 모델 작업 및 그림 등록의 성공은 fake socket에서만 검사한다. busy guard와 잘못된 입력 회귀는 worker/runtime 생성 이전의 거절을 검증한다. 모델 턴은 실행하지 않는다.

소스 커밋 전 focused 결과: extension 45개, architecture_boundaries 144개, kernel protocol 32개, kernel simulation 24개와 Python studio 15개가 모두 통과했다. Python 모델 응답은 FakeModel로 대체했다. 생성 skill·Desktop protocol 일치 및 fmt·diff 검사도 통과했다. 최신 기준의 Python·Desktop·관련 upstream 테스트 파일은 원본과 동일함을 확인했다. Planner가 요구한 Windows Full은 통합 소스 `0f4b26be198346232ed4b3c6fcf5ecc42809b3e3`에서 exit 0으로 완료했다. Desktop은 65개 test file 통과·1개 skip, 1,066개 test 통과·7개 skip이었고, 타입 검사·번들, Node/admin 검사·dependency audit, Windows native target·계약 검사, 전체 target/features clippy가 통과했다. Full 로그는 `target/kernel-ca2-full.log`다. 화면 E2E는 최종 Desktop 통합 브랜치에서 수행하므로 이 worktree에서는 중복 실행하지 않았다. 이 결과를 기록한 후속 문서 커밋은 제품 소스를 변경하지 않는다. 원격 CI Gate는 PR의 정확한 최신 HEAD를 별도로 확인한다.

## PR #2668: 자동 병합 기준의 읽음 상태 계약

[자동 PR 병합 검증의 Rust 실패](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45118519288/job/137506994563)는 최신 기준 브랜치에 추가된 `seen`을 기존 15개 machine action 목록이 제공하지 못해 발생했다. 기존 branch HEAD 검증과 자동 병합 tree 검증은 서로 다른 소스를 실행한 것이다. 이를 재시도만으로 해결하지 않고, 한 번 fetch해 확인한 `prerelease@d7d165bbb76ae06f9c51768640e2bee6a6d721a9`를 이번 통합 기준으로 고정했다.

원본의 Python·Desktop 변경을 유지하고 machine catalog를 16개 action으로 맞췄다. `seen`은 `id`만 필수로 받는 command이며 revision 없이 호출할 수 있다. 실제 저장 fixture를 통과하는 CLI/Gateway 회귀는 unread를 false로 저장한 뒤에도 revision·updated·edition·글 내용이 유지되는지, 반복 호출과 저장된 v1 글이 같은 응답을 유지하는지 확인한다. 목록의 unread/failed 필드도 실제 Python 결과로 확인한다. 다른 모델 명령의 성공은 계속 fake socket으로만 검사하며 실제 모델 턴은 실행하지 않는다.

소스 커밋 전 검증: 원격에서 실패했던 action 일치 검사를 포함한 extension focused 45개와 최신 Python studio 16개가 모두 통과했다. Python의 모델 응답은 FakeModel로 대체했다. fmt·diff 검사도 통과했으며 Python·Desktop·관련 upstream 테스트 파일이 d7 기준과 동일함을 확인했다. Planner는 Windows Full을 요구했다. 소스 커밋 뒤 수행하는 Windows Full과 정확한 자동 병합 tree의 최종 CI 결과는 [PR #2668 검증절](https://github.com/RefinedStone/codex-exec-loop/pull/2668)에서 관리한다. 아직 완료되지 않은 Full을 성공으로 기록하지 않으며, 결과 기록만을 위한 후속 문서 push는 하지 않는다. 화면 E2E는 최신 App 알림 동작을 보존한 최종 Desktop 통합 브랜치에서 수행한다.
