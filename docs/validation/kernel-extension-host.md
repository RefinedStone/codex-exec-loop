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
- 저장 job 최대 128개, receipt 최대 512개, 동시 활성 job 최대 8개다. 저장 공간이 차면 가장 오래된 종료 job을 먼저 정리한다.
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
