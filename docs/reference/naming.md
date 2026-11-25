# Akra ADE 명명 계약

- 관련: [N1 #2587](https://github.com/RefinedStone/codex-exec-loop/issues/2587),
  상위 [#2584](https://github.com/RefinedStone/codex-exec-loop/issues/2584)
- 기준: prerelease@48b6d49a3 이후 N1 변경
- 이 문서는 자체 식별자의 현재 값과 남은 과거 명칭의 이유를 기록한다. 데이터 경로 이전(N2),
  배포 산출물 이름 전환(R1), 저장소 rename(G1)을 완료했다는 뜻이 아니다.

## 현재 값

| 대상 | 값 | 상태 |
| --- | --- | --- |
| 제품명, Desktop 창 제목 | Akra ADE | N1에서 변경 |
| Rust package | `akra-ade` | N1에서 변경 (이전 `codex-exec-loop-native`) |
| Rust library / import 경로 | `akra_ade` | N1에서 변경 (이전 `codex_exec_loop_native`) |
| `cargo run` 기본 대상 | `akra` | N1에서 변경. 이전 기본 binary와 같은 `akra_ade::run()`을 실행한다 |
| 운영 명령 | `akra`, `akra-gateway`, `akra-admin`, `akra-telegram` | 유지 |
| 로그 target 접두사 | `akra_ade::...` | N1에서 변경. `AKRA_TRACE`·`RUST_LOG`의 `codex_exec_loop_native` 값은 `akra_ade`로 바꿔 적용한다 |
| GitHub API User-Agent | `akra-ade/<version>` | N1에서 변경 |
| 루트 npm manifest 이름 | `akra-ade` | N1에서 변경. npm 배포 package는 `npm/`의 `@refinedstone/akra`이며 이름을 바꾸지 않았다 |

## 남긴 과거 명칭과 이유

| 값 | 위치 | 이유 | 담당 |
| --- | --- | --- | --- |
| binary `codex-exec-loop-native` | `Cargo.toml` `[[bin]]`, `scripts/package_native_release.sh`, `npm/lib/platform.js`, release workflow 제목 | 배포 archive와 npm launcher가 이 이름을 실행한다. 같은 `src/main.rs`를 명시 target으로 고정했다 | R1 #2589 |
| `.codex-exec-loop/planning/`, `.codex-exec-loop/runtime/` | planning 저장소, `.gitignore` | 기존 사용자 작업 데이터 경로다 | N2 #2588 |
| `CODEX_EXEC_LOOP_*` 환경 변수 | TUI, app-server adapter | 기존 사용자 설정이다 | N2 #2588 |
| app-server `clientInfo.name` `codex-exec-loop-native` | `src/composition/production.rs` | Codex app-server에 보내는 클라이언트 정체성이다. 바꿀 때의 영향을 아직 확인하지 않았다 | N2 #2588 |
| Electron package `@refinedstone/akra-desktop` | `desktop/package.json` | Electron이 이 이름으로 userData 위치를 정한다 | N2 #2588, R1 #2589 |
| `urn:codex-exec-loop-native:app-server-protocol:*` | `schema/`, `scripts/refresh_codex_app_server_schema.sh`, 계약 시험 | 생성된 protocol snapshot의 기계 판독 ID다. 생성기로 다시 만들 때 함께 바꾼다 | R1 #2589 |
| `RefinedStone/codex-exec-loop` URL | manifest의 `repository`·`homepage`·`bugs`, 문서 링크 | 저장소 rename 전에는 이 주소만 동작한다 | G1 #2591 |
| 시험 fixture의 저장소 이름, 임시 디렉터리 접두사 | `src/**/tests*`, `tests/` | 시험 입력 값이다. 실행 동작에 영향이 없다 | 유지 |
| `docs/validation/`, `docs/reviews/`, `.omo/evidence/` | 과거 기록 | 당시 사실을 기록한 문서다 | 유지 |

## 외부 고유명

`codex app-server`, Codex protocol 타입, 사용자 Codex 홈은 외부 제품 이름이다. 자체 명칭 정비
대상이 아니다.
