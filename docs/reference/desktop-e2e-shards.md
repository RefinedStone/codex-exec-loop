# Desktop E2E 분할 실행

관련: #2690, #2313. 기본 화면 시험 목록은 여전히
`desktop/test/e2e/*.mjs`에서 자동 발견한다. 수동 제외 목록은
`desktop/scripts/e2e-specs.mjs`의 기존 두 항목만 유지한다.
새 파일은 별도 목록 편집 없이 다음 실행에 포함된다.

## 실행과 분배

```bash
# 기존 동작: 빌드하고 기본 화면 시험 전체를 순서대로 실행
npm --prefix desktop run e2e

# 실행 없이 분배 목록 확인
node desktop/scripts/run-e2e.mjs --list --shard 1/2
node desktop/scripts/run-e2e.mjs --list --shard 2/2

# CI에서는 각 명령을 서로 다른 runner에서 실행
npm --prefix desktop run e2e -- --shard 1/2
npm --prefix desktop run e2e -- --shard 2/2

# 이미 빌드했다면 중복 build 없이 실행
node --experimental-strip-types --disable-warning=ExperimentalWarning \
  --disable-warning=MODULE_TYPELESS_PACKAGE_JSON desktop/scripts/run-e2e.mjs --shard 1/2
```

`--shard`는 1부터 시작하는 index/total이다. `AKRA_E2E_SHARD`도 같은 값을
받으며 CLI가 우선한다. 잘못된 값, 빈 목록, 시험 수보다 많은 shard는 실패한다.
기본은 `1/1`이며 shard 안에서는 한 시험씩 실행한다. 같은 호스트에서 여러 Electron
시험을 병렬 실행하도록 바꾸지 않는다. 두 CI runner는 서로 다른 display, DBus와
keyring 세션을 가진다.

시간 표본이 없으면 모든 시험에 같은 가중치를 주고 이름을 기준으로 결정적으로
분배한다. 따라서 초기 두 shard의 시험 수는 최대 한 개 차이가 나며, 실행 시간이
같다는 보장은 없다. 이름별 duration 자료를 `--timings path.json` 또는
`AKRA_E2E_TIMINGS`로 주면 긴 시험부터 누적 시간이 가장 짧은 shard에 넣는다.
양쪽 shard는 같은 파일을 사용해야 한다. 새 시험은 표본의 중앙값 가중치를 받는다.

```json
{
  "approval-flow.mjs": 12000,
  "issue-create.mjs": 45000
}
```

위 숫자는 입력 형식을 보여 주는 예시이며 실제 측정값이 아니다.
이전 report의 `durationsMs` 객체를 합쳐 입력으로 사용할 수 있다. 단일
`report.json`도 직접 읽을 수 있지만 모든 shard의 표본을 합치는 편이 낫다.
`run-e2e.test.mjs`는 실제 기본 목록의 합집합·중복 없음, 수동 제외 유지,
시간 가중치 분배와 잘못된 설정의 거절을 검증한다.

## 격리와 실패 기록

실행마다 고유 결과 디렉터리를 만들고 시험마다 다음 경계를 제공한다.

- `TMPDIR`, `TMP`, `TEMP`: 시험별 임시 디렉터리. 기존 fake Gateway의
  PID/순번 socket·named pipe와 임의 포트 선택을 그대로 사용한다.
- `AKRA_DESKTOP_USER_DATA`: 자체 userData를 만들지 않는 기존 시험도 임시
  Electron 프로필을 사용한다. 직접 만드는 시험은 위 임시 디렉터리 안에서 생성한다.
- `AKRA_HOME`: 시험별 애플리케이션 상태와 DB 경로.
- `XDG_CONFIG_HOME`, `XDG_CACHE_HOME`: 시험별 설정과 캐시.
- `AKRA_E2E_ARTIFACT_DIR`: 시험별 결과물 경로. 기존 Jira/Linear의 고정
  evidence 경로도 이 값을 사용한다. 수동 단독 실행의 원래 경로는 유지한다.

runner는 stdout/stderr를 콘솔과 시험별 `output.log`에 함께 기록한다.
`desktop/test-results/e2e/shard-<index>-of-<total>-<unique>/report.json`에는 전체
목록, 선택 목록, 제외 사유, 시험별 시간·상태·종료 코드, 전체 경과 시간이 남는다.
`--output-dir` 또는 `AKRA_E2E_OUTPUT_DIR`로 결과물의 상위 경로를 정할 수 있다.
결과물은 임시 실행 상태와 별도로 보존하며 시작 시점과 매 시험 전후에 report를
갱신한다. 실행하지 못한 항목은 `not_run`이므로 성공으로 집계되지 않는다.

기존 시험의 정상 종료·finally 정리는 유지한다. SIGINT/SIGTERM을 받으면 다음
시험을 시작하지 않고 현재 시험의 소유 프로세스 트리를 종료한다. POSIX에서는
시험별 process group에 먼저 SIGTERM을 보내고 제한 시간 뒤 SIGKILL을 사용한다.
Windows에서는 해당 자식 PID의 트리에만 taskkill을 적용한다. 다른 사용자의 앱이나
전역 Electron 프로세스 목록은 건드리지 않는다. 종료 후 임시 상태를 지우고 실패·취소
report와 로그를 보존한다. OS의 즉시 강제 종료/SIGKILL에서는 마지막으로 저장한
report가 남으며 전체 정상 완료가 기록되지 않는다.

## CI 연결과 완료 조건

Desktop E2E는 Node/Admin 검사와 분리한 두 개의 `desktop_e2e` matrix job으로
실행한다. 각 job은 Node 의존성과 Electron·xvfb·DBus/keyring을 준비하고
위 shard 명령을 실행한다. `fail-fast: false`를 사용해 다른 shard의 진단을 보존한다.
`Fast Gate`, `CI Gate`, `Post-Merge Gate`는 모든 필수 shard의 성공을 요구하며
실패·취소·누락을 통과로 처리해서는 안 된다. 로그는 `if: always()`인 artifact
업로드 단계로 수집한다.

작은 fixture 검증 명령:

```bash
node --test desktop/scripts/run-e2e.test.mjs
```

실제 전체 Desktop E2E는 이 작업의 로컬 단계에서 실행하지 않았다.
이 변경의 로컬 검증은 fake Node 프로세스로 목록, 분배, 격리, 실패·취소 처리를
확인한다. 전체 화면 시험의 전후 성능·불안정성은 이 검증으로 입증하지 않는다.
#2690에 기록된 17분 37초는 이전 한 번의 CI 화면 시험 단계 관측이다.
동일 runner/toolchain/feature 조건의 최종 CI에서 두 shard의 wall time,
runner 총사용 시간과 실패 여부를 비교해야 한다. 새 runner를 더 쓰므로 wall time이
줄어도 npm 설치·Electron 준비·빌드를 합한 runner 사용량은 증가할 수 있다.
