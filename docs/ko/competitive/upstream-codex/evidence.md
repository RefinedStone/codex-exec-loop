# 업스트림 OpenAI Codex v0.144.1 증거 원장

[English](../../../competitive/upstream-codex/evidence.md)

이 원장은 변경 불가능한 소스 검사, 출시 바이너리 재현, 공식 문서,
추론 및 검증되지 않은 동작을 구분한다. 제품에 관한 결론은
[analysis.md](analysis.md)에 있으며, Akra의 결정과 구현 계약은
[gap-matrix.md](gap-matrix.md)에 있다.

## 스냅샷

| 필드 | 값 |
| --- | --- |
| 제품 | OpenAI Codex |
| 공식 저장소 | <https://github.com/openai/codex> |
| 릴리스 | [`rust-v0.144.1`](https://github.com/openai/codex/releases/tag/rust-v0.144.1) |
| 주석이 달린 태그 객체 | `db75c19352d29ef29c17dbcf73a7244f1b1a8d10` (아래에서 peel됨) |
| Peel된 릴리스 커밋 | `44918ea10c0f99151c6710411b4322c2f5c96bea` |
| 소스 커밋 시각 | 2026-07-09 15:10:27 -07:00 |
| 릴리스 게시 시각 | 2026-07-09 23:02:40 UTC |
| 이전 릴리스 커밋 | `767822446c7a594caa19609ca435281a9ec67e0d` (`rust-v0.144.0`) |
| 감사 날짜 | 2026-07-12 (Asia/Seoul) |
| Akra 기준선 | `prerelease`의 `226e4794b84107704378ecc1ea65f7d5c27750e5` |
| Akra 버전 | 1.3.5 |
| 환경 | Ubuntu 24.04.2 WSL2, Linux 6.18.33.2, x86_64 |
| 툴체인 | Rust/Cargo 1.95.0, Node 24.14.1, npm 11.6.1 |

소스는 깨끗한 detached 체크아웃으로 시작했다. 릴리스 워크스페이스에서 Cargo를 실행하자
임시 `Cargo.lock`의 워크스페이스 패키지 버전 항목이 `0.0.0`에서 `0.144.1`로 다시 작성되었다.
따라서 변경 불가능한 링크와 소스에 관한 결론은 peel된 커밋 객체를 사용하며, 어떤 주장도
그 이후의 임시 작업 트리 바이트에 의존하지 않는다. 원래 커밋된 잠금 파일의 SHA-256은
`175793a40a3147db1fee08fd9db0acc59312c344b3513dd7ee316f5446d8119e`였다.

## 증거 등급

이 감사에서는 상위 문서의 [증거 규칙](../README.md#근거-분류)을 사용한다.

- `verified/source`: 정확히 peel된 커밋에서 읽음;
- `verified/artifact`: 체크섬이 릴리스 API와 일치하는 릴리스 자산으로 재현함;
- `verified/local`: 이 환경에서 관찰했지만 이식 가능한 릴리스 아티팩트로 보존하지 않음;
- `documented`: 일치하는 로컬 실행 없이 공식 Codex 문서에 명시됨;
- `proposed`: 실험적, 개발 중, 베타 또는 미지원으로 명시됨;
- `inferred`: 검사한 제어 흐름 또는 검증된 여러 사실에서 도출한 제한적 결론;
- `unverified`: 결론을 뒷받침할 만큼 실행하지 않았거나 충분히 식별하지 못함.

소스의 존재는 구현을 입증하지만 기능 단계, 기본 활성화, 운영 규모 또는 최종 사용자 성능을
입증하지는 않는다. 생성된 실험적 스키마는 어휘를 입증하지만 성공적인 런타임 협상을 입증하지
않는다. 자사 TUI 경로는 토폴로지를 입증하지만 지연 시간이나 메모리 측면의 우위를 입증하지 않는다.

## 릴리스 식별 정보 및 아티팩트

### 소스 체크아웃

```bash
git clone https://github.com/openai/codex.git \
  /tmp/akra-upstream-codex-v0.144.1-source
git -C /tmp/akra-upstream-codex-v0.144.1-source \
  checkout --detach rust-v0.144.1
git -C /tmp/akra-upstream-codex-v0.144.1-source rev-parse HEAD
git -C /tmp/akra-upstream-codex-v0.144.1-source \
  show -s --format='%H%n%aI%n%cI%n%s' HEAD
```

관찰 결과:

```text
44918ea10c0f99151c6710411b4322c2f5c96bea
2026-07-09T15:10:27-07:00
2026-07-09T15:10:27-07:00
## Bug Fixes
```

[워크스페이스 버전](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/Cargo.toml#L132-L139)은
`0.144.1`이다. 등급: `verified/source`.

### 패치 범위

```bash
git -C /tmp/akra-upstream-codex-v0.144.1-source \
  diff --shortstat 767822446c7a594caa19609ca435281a9ec67e0d..44918ea10c0f99151c6710411b4322c2f5c96bea
git -C /tmp/akra-upstream-codex-v0.144.1-source \
  diff --name-status 767822446c7a594caa19609ca435281a9ec67e0d..44918ea10c0f99151c6710411b4322c2f5c96bea
```

관찰 결과: 파일 8개가 변경되었고 549개가 삽입되고 161개가 삭제되었다. 해당 경로는 워크스페이스 매니페스트,
코드 모드 원격 세션 구현/테스트, 코어 코드 모드 구현/테스트 및 설치 프로그램
구현/테스트였다. 이는 v0.144.1이 범위가 좁은 패치임을 검증한다. 감사 대상인 모든
app-server 또는 TUI 기능이 이 패치에서 추가되었다는 뜻은 아니다. 등급: `verified/source`.

### 출시된 Linux 자산

릴리스 API는 다음과 같은 정확한 자산을 보고했다. 다운로드한 두 아카이브는 API 다이제스트와 일치했다.

| 자산 | 아카이브 바이트 | SHA-256 | 추출된 바이트 | 추출된 SHA-256 |
| --- | ---: | --- | ---: | --- |
| `codex-x86_64-unknown-linux-musl.tar.gz` | 109,308,813 | `84091ae20c65fcc7d4120db97d1bd57d7ff8df9c7609fb781c78c2ebbd4f5a28` | 298,520,624 | `a96f944d1a596dbfb7fdd84f482be5c50e34b04bb371126840d873e4ebf26902` |
| `codex-app-server-x86_64-unknown-linux-musl.tar.gz` | 93,163,891 | `a6705726bb5ca1c9e803231dcebb6dae1aaa7be13b83270f4762a30853c13e58` | 248,062,016 | `b68de8340b2ccb8aeb23b6f33a4b4dff4f203ff84f6d82b6feedf334aba9a4fb` |

추출된 두 바이너리 모두 `0.144.1`을 보고했다. 이들은 x86_64 정적 PIE 실행 파일로 심벌이 제거되어 있었으며,
실행 불가능 스택, GNU RELRO 및 즉시 바인딩이 적용되어 있었다. 다운로드한 표본에는 별도의
`.sigstore` 자산이 포함되지 않았으므로, 이 감사에서는 릴리스 다이제스트 식별 정보를 검증했지만 독립적으로
cosign 검증을 실행하지는 않았다. 등급: `verified/artifact`.

대표적인 재현 절차:

```bash
ARTIFACT_DIR=/tmp/akra-upstream-codex-v0.144.1-bin
mkdir -p "$ARTIFACT_DIR"
curl -fL \
  https://github.com/openai/codex/releases/download/rust-v0.144.1/codex-x86_64-unknown-linux-musl.tar.gz \
  -o "$ARTIFACT_DIR/codex.tar.gz"
curl -fL \
  https://github.com/openai/codex/releases/download/rust-v0.144.1/codex-app-server-x86_64-unknown-linux-musl.tar.gz \
  -o "$ARTIFACT_DIR/codex-app-server.tar.gz"
printf '%s  %s\n' \
  84091ae20c65fcc7d4120db97d1bd57d7ff8df9c7609fb781c78c2ebbd4f5a28 \
  "$ARTIFACT_DIR/codex.tar.gz" | sha256sum --check --strict -
printf '%s  %s\n' \
  a6705726bb5ca1c9e803231dcebb6dae1aaa7be13b83270f4762a30853c13e58 \
  "$ARTIFACT_DIR/codex-app-server.tar.gz" | sha256sum --check --strict -
tar -tzf "$ARTIFACT_DIR/codex.tar.gz"
tar -tzf "$ARTIFACT_DIR/codex-app-server.tar.gz"
tar -xzf "$ARTIFACT_DIR/codex.tar.gz" --directory "$ARTIFACT_DIR"
tar -xzf "$ARTIFACT_DIR/codex-app-server.tar.gz" --directory "$ARTIFACT_DIR"
CLI_BIN="$ARTIFACT_DIR/codex-x86_64-unknown-linux-musl"
APP_SERVER_BIN="$ARTIFACT_DIR/codex-app-server-x86_64-unknown-linux-musl"
"$CLI_BIN" --version
"$APP_SERVER_BIN" --version
```

이 두 단순 tar 아카이브에는 각각 바이너리 하나가 들어 있다. 별도의 완전한 패키지 아카이브도
게시되며 `codex-resources` 레이아웃을 포함한다. 이 차이는 Linux에서 중요하다.

### Linux 단일 바이너리 샌드박스 검사

README는 단순 플랫폼 tar를 직접 설치 경로로 제시한다.
[Linux 런처](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/linux-sandbox/src/launcher.rs#L36-L65)는
시스템 `bwrap` 또는 인접한 번들 헬퍼가 필요하다. 단순 tar에는 둘 다 없다.
완전한 [패키지 레이아웃](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/scripts/codex_package/layout.py#L34-L94)은
헬퍼를 포함하며 [이를 검증한다](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/scripts/codex_package/layout.py#L136-L163).

시스템 `bwrap`이 없던 감사 호스트에서 출시된 단순 CLI 아카이브로 다음 명령을 실행하면
종료 코드 101로 실패했다.

```bash
mkdir -p /tmp/codex-home
CODEX_HOME=/tmp/codex-home "$CLI_BIN" \
  sandbox -C /tmp -P :read-only /usr/bin/true
```

단순 app-server 아티팩트의 안정적인 `command/exec` 경로도 같은 헬퍼 누락 조건에서
`exitCode: 101`을 반환했다. 이는 시스템 Bubblewrap이 없는 호스트에 제공된 아티팩트/패키지 형태의
결함이다. 완전한 패키지 아카이브 또는 호환되는 시스템 헬퍼가 있는 호스트에는 적용되지
않는다. 등급: `verified/artifact`.

## 바이너리 표면 재현

### CLI 및 하위 명령

```bash
BIN=/tmp/akra-upstream-codex-v0.144.1-bin/codex-x86_64-unknown-linux-musl
APP_SERVER=/tmp/akra-upstream-codex-v0.144.1-bin/codex-app-server-x86_64-unknown-linux-musl

"$BIN" --version
"$APP_SERVER" --version
"$BIN" --help
"$BIN" app-server --help
"$BIN" app-server daemon --help
"$BIN" remote-control --help
"$BIN" doctor --help
"$BIN" resume --help
"$BIN" fork --help
```

관찰된 CLI 계열에는 대화형 TUI, `exec`, `review`, login/logout, MCP, plugins,
MCP-server, app-server, remote-control, doctor, sandbox, resume/archive/delete/unarchive/fork,
cloud 및 features가 포함되었다. App-server와 remote-control에는 experimental이라는 표시가 있었다. App-server는
stdio, Unix 소켓, WebSocket 및 off 모드와 capability-token 및 signed-bearer WebSocket 인증을 제공했다.
등급: `verified/artifact`.

### 기능 카탈로그

```bash
CODEX_HOME="$(mktemp -d)" "$BIN" features list
```

출력에는 기능 행 92개가 있었다. 소스 검사를 통해 stable, experimental,
under-development 및 제거된 호환성 항목을 구분했다. 이 스냅샷의 예시는 다음과 같다.

- stable/default-on에는 1세대 multi-agent, hooks, goals, apps/plugins, remote
  compaction v2, Linux unified exec 및 shell snapshot/tool 동작이 포함되었다.
- experimental/default-off에는 memories와 network proxy가 포함되었다.
- under-development/default-off에는 multi-agent V2, fanout, permission-request tools,
  exec-permission approvals, local thread compression 및 token/rollout budgets가 포함되었다.
- 이전 steer/collaboration switches와 같은 제거된 항목은 호환성을 위한
  no-op으로 계속 표시될 수 있다.

[기능 단계 계약](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/features/src/lib.rs#L34-L50)이
이러한 구분을 뒷받침한다. 행의 등급은 `verified/artifact`이고 단계 의미의 등급은
`verified/source`이다.

## 스키마 및 핸드셰이크 증거

### 안정 스키마와 실험적 스키마 비교

```bash
CODEX_HOME="$(mktemp -d)" "$BIN" app-server generate-json-schema \
  --out /tmp/codex-schema-stable
CODEX_HOME="$(mktemp -d)" "$BIN" app-server generate-json-schema \
  --out /tmp/codex-schema-experimental --experimental

jq '.oneOf | length' /tmp/codex-schema-stable/ClientRequest.json
jq '.oneOf | length' /tmp/codex-schema-stable/ServerNotification.json
jq '.oneOf | length' /tmp/codex-schema-stable/ServerRequest.json
jq '.oneOf | length' /tmp/codex-schema-stable/ClientNotification.json
jq '.oneOf | length' /tmp/codex-schema-experimental/ClientRequest.json
jq '.oneOf | length' /tmp/codex-schema-experimental/ServerNotification.json
jq '.oneOf | length' /tmp/codex-schema-experimental/ServerRequest.json
jq '.oneOf | length' /tmp/codex-schema-experimental/ClientNotification.json
```

관찰 결과:

| 스키마 | 클라이언트 요청 | 서버 알림 | 서버 요청 | 클라이언트 알림 |
| --- | ---: | ---: | ---: | ---: |
| stable | 87 | 68 | 10 | 1 |
| `--experimental` | 122 | 68 | 11 | 1 |

app-server [게이팅 계약](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server/README.md#L2118-L2177)은
클라이언트에 스키마가 있더라도 실험적 메서드/필드에 initialize capability가 필요하다고 규정한다.
등급: `verified/artifact` 및 `verified/source`.

### Akra 스키마 정규화

Akra에 체크인된 스키마 메타데이터는 `codex-cli 0.144.0`을 식별하며 experimental
definitions로 생성되었다. v0.144.1 experimental export를
`scripts/normalize_codex_app_server_schema.mjs`로 처리했다. 소스 CLI 및 생성 날짜
메타데이터를 통제한 뒤, 정규 `jq -S '.definitions'` 출력은 SHA-256
`99ade25a6cbfa75a35abc5841cb6d6f98b90d1a1e9b2ec3eca2772a3bde968ac`으로 바이트 단위까지 동일했다. 등급:
`verified/local`.

이 결과로 내릴 수 있는 결론은 하나뿐이다. v0.144.1은 Akra의 v0.144.0 스냅샷과 비교해 정규화된 experimental
definition 어휘를 변경하지 않았다. 이는 stable-vs-experimental
계약의 정확성, 런타임 capability 활성화, 페이로드 감소 또는 source-to-sink 충실도를 입증하지 않는다.

### 격리된 긍정 핸드셰이크

체크인된 [검사 하네스](../../../competitive/upstream-codex/scripts/probe-app-server.mjs)는 새로운 `CODEX_HOME`/`HOME` 아래에서 출시된 app-server를 실행했고,
자식 환경은 로캘, 경로, 인증서 및 시간대 변수로 제한했다. experimental capability를 활성화해 `initialize`를 보낸 다음 `initialized`를 보냈다.
다음 읽기 요청은 모델 인증 없이 성공적인 JSON-RPC 결과를 반환했다.

| 메서드 | 관찰 결과 |
| --- | --- |
| `account/read` | 비어 있음/활성 계정 상태 없음이 반환됨 |
| `thread/list` | 빈 카탈로그가 반환됨 |
| `thread/loaded/list` | 빈 로드 목록이 반환됨 |
| `model/list` | 명시적 제한 100과 함께 모델 행 7개가 반환됨 |
| `collaborationMode/list` | 모드 2개가 반환됨 |
| `app/list` | 성공적인 목록 응답 |
| `skills/list` | 성공적인 목록 응답 |
| `plugin/list` | 성공적인 목록 응답 |
| `remoteControl/status/read` | 비활성화 상태가 반환됨 |
| `permissionProfile/list` | 프로필 3개가 반환됨 |
| `hooks/list` | 성공적인 목록 응답 |
| `config/read` | 유효한 격리 구성이 반환됨 |
| `mcpServerStatus/list` | 성공적인 목록 응답 |
| `experimentalFeature/list` | 기능 행 92개가 반환됨 |

Initialize는 예상된 사용자 에이전트, 격리된 Codex 홈 및 플랫폼을 반환했다. 이러한 관찰은
이 바이너리와 상태에서 요청을 사용할 수 있음을 검증한다. 인증이 필요한 필드,
네트워크 기반 서비스, 실제 모델 실행 또는 크로스 플랫폼 동작을 검증하지는 않는다. 등급: `verified/local`.

`permissionProfile/list` 자체는 stable이지만 소스에서는 명명된
[`thread/start.permissions`](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-protocol/src/protocol/v2/thread.rs#L86-L94)
선택과 응답의
[`activePermissionProfile`](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-protocol/src/protocol/v2/thread.rs#L185-L196)
출처를 experimental로 표시한다. Stable `thread/start`에도 범용
[`config` 맵](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-protocol/src/protocol/v2/thread.rs#L87-L96)이 유지되며,
그 항목은
[CLI 재정의](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server/src/config_manager.rs#L220-L249)로 병합된다.
따라서 기존의 사용자 지정 default 또는 `config.default_permissions`는 stable 연결에서 사용자 지정 프로필을
선택할 수 있지만, legacy sandbox만 반환되고 형식화된 프로필 식별 정보/출처는
없다. 이 범용 우회 수단은 전용 stable 명명 프로필 계약과 동등하지 않다.
등급: `verified/source`.

### 부정 핸드셰이크 및 기능 검사

별도의 새 프로세스에서 다음 결과가 나왔다.

- initialize 전 요청: JSON-RPC `-32600`, `Not initialized`;
- capability가 없는 experimental 요청: 필요한 `experimentalApi` capability를 명시하는 JSON-RPC `-32600`;
- initialize 반복: JSON-RPC `-32600`, `Already initialized`;
- 제어 연결은 초기 `remoteControl/status/changed`를 하나 받았으며, 그 외에는 동일하지만 수신을 거부한
  연결은 하나도 받지 않았다.

`experimentalApi: false`일 때 stable `fs/readFile`은 `/etc/hostname`을 읽었지만 `process/spawn`은
해당 메서드가 experimental이므로 `-32600`을 반환했다. 읽기는 폐기 가능한 직접 요청을 사용했으며,
쓰기/제거/process-spawn 변경 작업은 실행하지 않았다. 다음 명령으로 이러한 긍정 및 부정 경로를
재현할 수 있다. 이 명령은 여러 개의 새로운 기본 app-server 프로세스를 시작한다. 활성화된 plugin
시작 작업은 폐기 가능한 홈에 대한 공개 GitHub marketplace 읽기를 반복할 수 있으므로, 이는
offline 검사가 아니다.

```bash
APP_SERVER_BIN=/tmp/akra-upstream-codex-v0.144.1-bin/codex-app-server-x86_64-unknown-linux-musl
printf '%s  %s\n' \
  b68de8340b2ccb8aeb23b6f33a4b4dff4f203ff84f6d82b6feedf334aba9a4fb \
  "$APP_SERVER_BIN" | sha256sum --check --strict -
CODEX_APP_SERVER="$APP_SERVER_BIN" \
  node docs/competitive/upstream-codex/scripts/probe-app-server.mjs
```

스크립트는 범위가 제한된 요약, 개수, 오류 코드/메시지 및 아티팩트 다이제스트만 내보내며,
구성, 자격 증명, 파일 내용 또는 원시 환경은 출력하지 않는다. 등급: `verified/local`.

## 불변 소스 원장

### 런타임 및 자사 클라이언트 토폴로지

- [App-server 클라이언트 전송 모델](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-client/README.md#L27-L43)은
  JSON-RPC 응답 엔벌로프를 유지하면서 형식화된 프로세스 내 요청/이벤트 채널을 정의한다.
  등급: `verified/source`.
- [백프레셔 및 종료](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-client/README.md#L58-L67)는
  제한된 큐, 과부하, 지연 및 제한된 종료를 정의한다. 아래의 표적 클라이언트 테스트는
  대표 사례를 실행했다. 등급: `verified/source`.
- [TUI 대상 유형](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/tui/src/lib.rs#L260-L279)은
  임베디드, 로컬 데몬 및 명시적 원격 경로를 정의한다.
  [실행 선택기](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/tui/src/lib.rs#L799-L920)는
  실행 구성을 재생할 수 있을 때만 호환되는 기본 소켓을 검사하며, 그렇지 않으면 임베디드로
  폴백한다. 등급: `verified/source`.
- [원격 연결 해제 처리](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-client/src/remote.rs#L396-L456)는
  유효하지 않은 프레임, 연결 해제, 전송 실패 및 닫힘을 이벤트로 변환한다. 이 클라이언트에서는
  범용 재연결 루프가 발견되지 않았다. 재연결 부재는 검사한 인벤토리에서 내린 결론이지,
  어떤 외부 표면도 재연결할 수 없다는 증거가 아니다. 등급: `verified/source` 및 제한된 `inferred`.
- [데몬 상태](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-daemon/README.md#L1-L15)는
  데몬을 experimental 및 Unix 전용으로 표시하며, 그
  [업데이트 수명 주기](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-daemon/README.md#L34-L70)는
  재부팅 후에도 지속되지 않고 app-server를 재시작할 수 있다. 등급: `proposed` 제품 표면과
  `verified/source` 구현.

### app-server 프로토콜

- [스레드/턴 모델](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-protocol/src/protocol/v2/thread_data.rs#L167-L275)은
  세션 및 포크 계보, 기록/출처/에이전트/Git 출처, 턴 항목/뷰, 상태,
  오류, 타임스탬프 및 지속 시간을 전달한다. 등급: `verified/source`.
- [턴 상태](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-protocol/src/protocol/v2/turn.rs#L30-L38)는
  `completed`, `interrupted`, `failed` 및 `inProgress`이다. 등급: `verified/source`.
- [오류 알림](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-protocol/src/protocol/v2/notification.rs#L38-L48)은
  형식화된 오류를 중첩하고 `willRetry: true`가 턴을 중단하지 않는다고 명시한다. 등급:
  `verified/source`.
- [스레드 항목 어휘](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-protocol/src/protocol/v2/item.rs#L222-L396)는
  이 스냅샷에서 18개의 형식화된 항목 variant를 정의한다. 등급: `verified/source`.
- [모델 기능 응답](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-protocol/src/protocol/v2/model.rs#L40-L146)은
  기본값, reasoning 옵션, 입력 modality 및 service-tier/capability 사실을 노출한다. 등급:
  `verified/source`.
- [추론 노력 어휘](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/protocol/src/openai_models.rs#L40-L50)에는
  이 스냅샷에서 `max`와 `ultra`가 포함된다. 등급: `verified/source`.
- [적용된 스레드 응답](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-protocol/src/protocol/v2/thread.rs#L167-L201)은
  유효한 model/provider/cwd/approval/sandbox/reasoning/service-tier 상태를 반환한다. 등급:
  `verified/source`.
- [승인 수명 주기](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server/README.md#L1444-L1498)는
  응답 또는 수명 주기 정리 후의 `serverRequest/resolved`를 보여 준다. 이는 서버 측의
  보류 요청이 해제되었음을 입증할 뿐, 선택한 결정이 수락되거나 적용되었음을 입증하지 않는다. 등급:
  시퀀스는 `verified/source`, 좁은 증거 해석은 `inferred`.

### 세션, 컨텍스트 및 에이전트

- [롤아웃 재구성](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core/src/session/rollout_reconstruction.rs#L113-L190)은
  교체 기록, 롤백, world state, 설정 및 compaction 상태를 재구성한다. 등급:
  `verified/source`.
- [포크 경계](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core/src/thread_manager.rs#L128-L156)는
  턴 중간 스냅샷의 한계를 설명하고 중단된 턴 경계를 삽입한다. 등급:
  `verified/source`.
- [압축 교체](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core/src/compact.rs#L322-L376)와
  [최근 입력 상한](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core/src/compact.rs#L585-L658)은
  로컬 교체와 최근 사용자 메시지의 20k-token 제한을 구현한다. 등급: `verified/source`.
- [턴 이전 압축 TODO](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core/src/session/turn.rs#L142-L165)는
  초기 추정에서 새 input/context diff를 누락한다. 이후의
  [턴 중간 로직](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core/src/session/turn.rs#L300-L370)은
  sampling이 시작된 뒤 반응할 수 있다. 대용량 입력의 첫 시도 실패는 가능하지만 재현되지
  않았다. 등급: `verified/source` 및 `inferred` 위험.
- [V1 하위 에이전트 상속](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core/src/tools/handlers/multi_agents_common.rs#L154-L231)은
  자식에 대한 런타임 정책과 환경 사실을 보존한다. 등급: `verified/source`.
- [V2 단계](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/features/src/lib.rs#L1035-L1046),
  [깊이 게이트 차이](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core/src/tools/spec_plan.rs#L339-L351) 및
  [실행 제한기](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core/src/agent/control/execution.rs#L59-L101)는
  분석에서 제한된 소스 우려를 뒷받침한다. V2는 개발 중이며 의심되는
  동시성 초과는 부하 환경에서 재현되지 않았다. 등급: `proposed`, `verified/source` 및
  명시적으로 제한된 `inferred`.

### TUI 승인 동작

- [승인 컨텍스트 및 선택지](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/tui/src/bottom_pane/approval_overlay.rs#L621-L940)는
  제공되는 경우 스레드/환경 컨텍스트와 일회성/세션/영구/거부/중단 선택지를
  보여 준다. 등급: `verified/source`.
- [프로토콜 결정 의미](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/protocol/src/protocol.rs#L4022-L4057)는
  거부 후 계속하기와 중단을 구분하지만,
  [일반 기본 목록](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/protocol/src/approvals.rs#L277-L331)은
  거부를 생략할 수 있다. 등급: `verified/source`.
- [보류 승인 저장소](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/tui/src/bottom_pane/approval_overlay.rs#L158-L214)는
  `push`를 사용하고, [진행](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/tui/src/bottom_pane/approval_overlay.rs#L474-L525)은
  `pop`을 사용한다. 초기 배치는 다른 곳에서 삽입 순서를 뒤집지만, 표시 중인 모달에 도착하는 요청은
  새 요청을 우선할 수 있다. 등급: `verified/source` 및 제한된 `inferred` 기아 위험.

### 보안 및 원격 경계

- [전송 계약](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server/README.md#L20-L53)은
  WebSocket을 experimental/unsupported로 표시하고 stdio/Unix/WS 형태를 문서화한다. 등급:
  소스가 뒷받침하는 `documented`.
- [Unix 소켓 생성](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-transport/src/transport/unix_socket.rs#L21-L43)은
  모드를 `0600`으로 설정한다. 등급: `verified/source`.
- [WebSocket 보호](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-transport/src/transport/websocket.rs#L89-L152)는
  인증되지 않은 비루프백 구성과 Origin이 있는 업그레이드를 거부한다. 출시된
  바이너리도 인증 없는 `--listen ws://0.0.0.0:0`을 exit 1로 거부했다. 등급:
  `verified/source` 및 `verified/artifact`.
- [원격 클라이언트 토큰 정책](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-client/src/remote.rs#L117-L124)은
  비루프백 평문 `ws`에 토큰을 넣지 않는다. 등급: `verified/source`.
- [안정 파일시스템 메서드](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-protocol/src/protocol/common.rs#L743-L789)와
  그 [샌드박스되지 않은 프로세서](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server/src/request_processors/fs_processor.rs#L64-L191)는
  호스트 경로에서 동작한다. `thread/shellCommand`도 마찬가지로 전체 액세스/샌드박스되지 않은 것으로 문서화되어 있다.
  등급: `verified/source`.
- [프로세스 생성](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server-protocol/src/protocol/common.rs#L1056-L1087)은
  experimental이며, 그 [프로세서](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server/src/request_processors/process_exec_processor.rs#L68-L140)는
  샌드박스되지 않았고 app-server 환경을 상속한다. 등급: `verified/source`.
- [셸 환경 기본값](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/protocol/src/config_types.rs#L187-L243)은
  `inherit: All`을 선택하고 기본 제외를 비활성화하며,
  [필터 규칙](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/protocol/src/shell_environment.rs#L46-L109)은
  따라서 key/secret/token 패턴이 기본으로 적용되지 않음을 보여 준다. 등급:
  `verified/source`.
- [신뢰도에 따른 샌드박스 기본값](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core/src/config/permissions.rs#L48-L58)은
  명시적 `Trusted` 또는 `Untrusted` 상태 뒤에는 workspace 프로필을 선택하고, 신뢰 수준이 없을 때만
  읽기 전용을 선택한다. 두 프로필 모두 root 읽기를 유지한다.
  [workspace-write](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/protocol/src/permissions.rs#L568-L603)는
  제한된 쓰기를 추가한다. 등급: `verified/source`.
- [인증 저장소 기본값](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/config/src/types.rs#L87-L100)은
  평문 파일 저장소이다. [페이로드](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/login/src/auth/storage.rs#L38-L61)는
  API/OAuth/PAT/private-key 자료를 포함할 수 있다. Unix에서 생성 경로는
  [모드 `0600`을 요청](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/login/src/auth/storage.rs#L202-L218)하지만,
  그 생성 모드는 권한이 더 느슨한 기존 파일을 수정하지 않는다. Keyring과
  암호화된 로컬 비밀 옵션이 존재한다. 등급: `verified/source`.
- [롤아웃 정책](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/rollout/src/policy.rs#L36-L57)은
  민감한 상호작용 내용을 유지하지만,
  [레코더 생성 경로](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/rollout/src/recorder.rs#L1530-L1543)는
  자체적으로 `0600`을 강제하지 않는다. 노출은 상위 디렉터리 권한과 umask에 따라 달라지며 재현되지
  않았다. 등급: `verified/source` 및 조건부 `inferred` 위험.
- [원격 제어 지속성](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/state/migrations/0024_remote_control_enrollments.sql#L1-L10)은
  원격 토큰이 아니라 서버/환경 식별자를 저장한다. 등급: `verified/source`.

### 메모리 안전 경계

- [메모리 기능 단계](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/features/src/lib.rs#L925-L934)는
  experimental/default-off이다. 등급: `proposed`.
- [1단계 필터링](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/memories/write/src/phase1.rs#L403-L473)과
  [비밀정보 정제 패턴](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/secrets/src/sanitizer.rs#L4-L21)은
  최선형 방식이며 범용 비밀정보 탐지가 아니다. 등급: `verified/source`.
- [속도 제한 가드](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/memories/write/src/guard.rs#L9-L46)는
  검사에 실패해도 처리를 허용한다. 등급: `verified/source`.
- [2단계 워커 제한](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/memories/write/src/phase2.rs#L297-L347)은
  네트워크와 확장을 비활성화하고 쓰기를 제한한다. 등급: `verified/source`.

### 백프레셔 및 수명 주기

- [코어 채널 구성](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core/src/session/mod.rs#L468-L539)은
  제출은 제한하지만 무제한 이벤트 채널을 생성한다.
  [포워더](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core/src/session/mod.rs#L1946-L1983)는
  [제한된 app-server 채널](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server/src/lib.rs#L447-L467)보다 앞에 있다.
  정체된 클라이언트에서의 메모리 누적은 소스 추론이며 스트레스 환경에서 재현되지 않았다.
  등급: `verified/source` 및 `inferred`.
- [롤아웃 큐 및 비우기](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/rollout/src/recorder.rs#L832-L930)와
  [쓰기 재시도](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/rollout/src/recorder.rs#L1545-L1654)는
  유의미한 내구성을 제공한다. 등급: `verified/source`.
- 사이드 스레드 정리는 즉시 중단하고 구독을 해제하지만,
  [스레드 수명 주기 언로드](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server/src/request_processors/thread_lifecycle.rs#L4-L105)는
  구독자가 없고 비활성 상태인 경우 스레드별 30분 지연을 사용한다. 그
  기한 이전의 일시적 누적은 제한된 추론이며, 수명 누수 주장은 기각된다. 등급: `verified/source` 및
  `inferred`.

### 릴리스 및 품질

- [릴리스 도구 체인 설정](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/.github/workflows/rust-release.yml#L165-L174)은
  Rust 1.95를 고정한다.
  [Linux/빌드 경로](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/.github/workflows/rust-release.yml#L216-L362)는
  Bubblewrap를 빌드하고 해시하며, Codex 바이너리를 빌드하고, 심볼을 보관하고, strip하고, Linux
  cosign을 적용하고, helper 번들을 스테이징한다. 별도의
  [macOS 작업](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/.github/workflows/rust-release.yml#L496-L612)은
  바이너리에 서명하고 공증한다. 이 원장의 ELF 강화 속성은 이 workflow 범위가 아니라 출시 아티팩트
  검사에서 비롯된다. 등급: `verified/source`.
- [릴리스 게이트 의존성](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/.github/workflows/rust-release.yml#L1109-L1124)은
  일반 테스트 작업을 직접적인 릴리스 의존성으로 삼지 않는다. 정확한 태그에는 성공한
  `rust-release` 실행 하나와 첨부된 검사 44개가 있었으며, 대부분 릴리스 작업이었다. 등급: `verified/source` 및
  `verified/local` GitHub API 관찰.
- [Bazel macOS/Linux 매트릭스](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/.github/workflows/bazel.yml#L17-L108)는
  macOS와 Linux GNU/musl을 실행하며, 별도의
  [Windows 샤드](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/.github/workflows/bazel.yml#L133-L220)는
  Windows 호스트에서 실행된다. 등급: `verified/source`이며, 정확한 태그 실행 증거는 아니다.
- Cargo 릴리스 명령은 `--locked`를 일관되게 사용하지 않으며, 커밋된 릴리스 lockfile은
  workspace 패키지를 `0.0.0`으로 저장해 Cargo가 로컬에서 항목 132개를 다시 쓰게 한다. 외부
  의존성 변경은 관찰되지 않았다. 등급: `verified/source` 및 `verified/local`; 재현 가능한 빌드에
  미치는 영향은 `unverified`로 남는다.
- 저장소에는 광범위한 테스트 인벤토리가 있지만 체크인된 수치 성능 벤치마크는
  prompt-image 작업으로 제한되며 CI는 회귀 임계값 없이 smoke로 실행한다. 추적되는
  app-server 시작, 처리량, 큐 포화, 원격 재연결 또는 TUI latency budget은
  발견되지 않았다. 등급: `verified/source` 인벤토리 제한.

## 표적 소스 테스트

릴리스 workspace에는 `just` 1.56.0과 `cargo-nextest` 0.9.140이 필요했다. 감사 환경에는
`pkg-config`와 OpenSSL 개발 헤더도 없었다. 정확한 Ubuntu 24.04 패키지를 다운로드하여
시스템을 변경하지 않고 `/tmp` 아래에 압축을 풀었다. Rust/Cargo 1.95.0과 정확한
소스 체크아웃으로 시작했으며, 관련 clean-shell 설정 및 최종 명령은 다음과 같았다.

```bash
TOOLS=/tmp/akra-upstream-codex-tools
DEPS=/tmp/akra-upstream-codex-build-deps
mkdir -p "$TOOLS" "$DEPS/debs" "$DEPS/root"
cargo install --locked --root "$TOOLS" --version 1.56.0 just
cargo install --locked --root "$TOOLS" --version 0.9.140 cargo-nextest

cd "$DEPS/debs"
apt-get download \
  pkgconf=1.8.1-2build1 \
  pkgconf-bin=1.8.1-2build1 \
  libpkgconf3=1.8.1-2build1 \
  libssl-dev=3.0.13-0ubuntu3.11 \
  libssl3t64=3.0.13-0ubuntu3.11
for package in ./*.deb; do
  dpkg-deb --extract "$package" "$DEPS/root"
done

export PATH="$TOOLS/bin:$DEPS/root/usr/bin:$HOME/.cargo/bin:$PATH"
export LD_LIBRARY_PATH="$DEPS/root/usr/lib/x86_64-linux-gnu${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export PKG_CONFIG_SYSROOT_DIR="$DEPS/root"
export PKG_CONFIG_LIBDIR="$DEPS/root/usr/lib/x86_64-linux-gnu/pkgconfig:$DEPS/root/usr/lib/pkgconfig:$DEPS/root/usr/share/pkgconfig"
export CFLAGS="${CFLAGS:+$CFLAGS }-I$DEPS/root/usr/include/x86_64-linux-gnu"

cd /tmp/akra-upstream-codex-v0.144.1-source/codex-rs
just test -p codex-app-server-protocol
just test -p codex-app-server-client
```

관찰된 최종 결과:

```text
codex-app-server-protocol: 256 passed, 0 skipped
codex-app-server-client:    27 passed, 0 skipped
```

클라이언트 suite에는 형식화된 요청과 원격 요청의 왕복, 인증 토큰 전송 정책,
WebSocket 및 Unix 소켓 동작, 서버 요청 해결, 연결 해제 이벤트, 백프레셔,
지연 마커, 종료 및 세션 출처 동작이 포함되었다. 프로토콜 suite에는 stable 필터링,
experimental 마커, 직렬화, 스키마 fixture, 스레드 페이징, 샌드박스 형태 및 적용된
스레드/턴 유형이 포함되었다. 등급: `verified/local`.

이는 표적 증거이지 전체 소스 suite 통과가 아니다. 첫 빌드 시도는 누락된
로컬 도구/개발 헤더 때문에 실패했으며, 이는 제품 테스트 실패가 아니라 환경 설정 실패다.

## 직접 app-server 성능 측정

### 경계

각 샘플은 다음과 같이 수행되었다.

1. 출시된 `codex-app-server-x86_64-unknown-linux-musl` 바이너리를 사용했다.
2. 격리된 새로운 `CODEX_HOME`을 생성했다.
3. plugin 시작 작업이 활성화된 기본 stdio app-server를 생성했다.
4. initialize와 initialized를 보냈다.
5. `account/read`와 `thread/list`를 요청했다.
6. 생성부터 최종 응답까지의 경과 시간을 측정했다.
7. 생성부터 최종 응답까지 재귀적으로 열거한 프로세스 트리의 RSS를 샘플링했다.
8. 프로세스를 종료했다.

바이너리 및 파일시스템 페이지 캐시는 warm 상태였다. TTY, rendering, 인증, 모델
호출, 명시적 MCP/plugin 요청, stream, Akra 프로세스 또는 병렬 워커는 없었다. 기본 plugin 시작은
비활성화하지 않았다. 이 경로를 수동 검사한 결과 app-server가
`git ls-remote https://github.com/openai/plugins.git HEAD`와 Git 전송 helper를 실행하는 것이 관찰되었다. 응답
clock은 해당 공개 네트워크 백그라운드 작업이 끝날 때까지 기다리지 않았다. 위의
스냅샷 환경에서 10회를 실행했다.

이 기본 동작은 app-server
[시작 훅](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/app-server/src/message_processor.rs#L428-L438)과
[선별 저장소 동기화를 시작하는](https://github.com/openai/codex/blob/44918ea10c0f99151c6710411b4322c2f5c96bea/codex-rs/core-plugins/src/manager.rs#L1926-L1937)
활성화된 plugin 경로와 일치한다.
등급: `verified/source` 및 `verified/local` 프로세스 트리 관찰.

### 원시 결과

| 실행 | 경과 시간 ms | 샘플링된 최대 RSS KiB | 프로세스 수 |
| ---: | ---: | ---: | ---: |
| 1 | 120.781 | 82,440 | 4 |
| 2 | 122.712 | 90,524 | 4 |
| 3 | 123.812 | 93,328 | 4 |
| 4 | 120.261 | 92,052 | 4 |
| 5 | 117.292 | 87,676 | 4 |
| 6 | 120.708 | 88,272 | 4 |
| 7 | 122.124 | 88,352 | 4 |
| 8 | 120.605 | 88,336 | 4 |
| 9 | 119.439 | 91,224 | 4 |
| 10 | 120.829 | 87,920 | 4 |
| 최솟값 | 117.292 | 82,440 | 4 |
| 중앙값 | 120.745 | 88,344 | 4 |
| 최댓값 | 123.812 | 93,328 | 4 |

체크인된 [벤치마크 하네스](../../../competitive/upstream-codex/scripts/benchmark-app-server.mjs)는 clock 경계를 정의하고
[공유 Linux 샘플러](../../../competitive/upstream-codex/scripts/app-server-harness.mjs)를 사용해 모든 스레드의 `/proc`
자식 목록을 재귀적으로 합치고 요청된 2 ms마다 프로세스 `VmRSS`를 poll한다. sampler는
root-task `children` 인벤토리를 읽을 수 없을 때 root-only RSS를 완전한
트리로 보고하지 않고 실패 처리한다. 생략된 warmup 1회와 기록된 실행 10회를 다음 명령으로 재현한다.

```bash
APP_SERVER_BIN=/tmp/akra-upstream-codex-v0.144.1-bin/codex-app-server-x86_64-unknown-linux-musl
printf '%s  %s\n' \
  b68de8340b2ccb8aeb23b6f33a4b4dff4f203ff84f6d82b6feedf334aba9a4fb \
  "$APP_SERVER_BIN" | sha256sum --check --strict -
CODEX_APP_SERVER="$APP_SERVER_BIN" \
  node docs/competitive/upstream-codex/scripts/benchmark-app-server.mjs
```

이벤트 루프 스케줄링은 poll을 지연시킬 수 있으며 RSS 샘플링은 더 짧은 peak를 놓칠 수 있다. 이 결과는
`verified/local` 방향성 자료로 남으며 릴리스 회귀 게이트로 사용해서는 안 된다. 기본 plugin
warmup과 완료되지 않은 네트워크 작업 때문에 이는 격리된 프로토콜 비용 샘플도 아니고 별도로
통제된 plugin-disabled 계층을 대체하지도 않는다. 이는 기존 Akra Native Performance Evidence Contract에 대한
기본 경로 하한 입력이며, 비교 대화형 결과가 아니다.

## Codex doctor 진단

새롭게 격리된 홈에서 `codex doctor --json`은 구조화되고 마스킹된 검사 결과를 반환했다. App-server,
config, Git, state, sandbox 검사가 실행되었다. credential, terminal, installation-match 검사는
격리된/비대화형/불일치 아티팩트 설정에서 예상대로 실패했다. 이 명령은 reachability 작업도
시도했다. 분류: `verified/local`.

이 프로브는 doctor 출력을 제한된 진단 입력으로 간주할 수 있음을 뒷받침한다. 안정적인 API,
오프라인 동작, 짧은 지연 시간, 자동 주기적 Admin polling의 안전성을 입증하지는 않는다.

## P0-A 라이브 터미널 영수증 근거

P0-A 구현은 upstream outcome, side-band observation, application-delivery acknowledgement를
분리한 닫힌 terminal receipt를 추가한다. notification reducer는 네 가지 공식 status, 중첩된
typed error, retry intent, item-view completeness, timestamp, duration을 파싱한다. connection은
non-retry grace, first-terminal compare-and-set, bounded terminal delivery를 소유한다. 현재 main,
planning, parallel, prompt-log, archive consumer는 완료에서 파생되는 작업에 대해
`completed + confirmed`만 수락한다.

결정론적 증명은 retry-then-success, 네 가지 status 전부, 누락되거나 모순되는 field,
correlation mismatch, 중복/순서가 뒤바뀐 terminal notification, changed-file non-promotion,
non-retry grace 만료, full/deadline/disconnected/recovered sink, producer/event receipt mismatch,
현재 completion consumer 전부를 다룬다. 체크인된 테스트는 릴리스 capture도 파싱하므로
아티팩트가 누락되거나 재분류되면 protocol contract suite가 실패한다.

인증된 릴리스 바이너리 capture는 다음 명령으로 생성했다.

```bash
CODEX_APP_SERVER=/tmp/akra-upstream-codex-v0.144.1-bin/codex-app-server-x86_64-unknown-linux-musl \
CODEX_AUTH_FILE="$HOME/.codex/auth.json" \
TERMINAL_CAPTURE_MODEL=gpt-5.3-codex-spark \
node docs/competitive/upstream-codex/scripts/capture-terminal-truth.mjs
```

하네스는 소유자 전용 auth-file 권한을 검증하고, auth를 일회용 `CODEX_HOME`에 복사하며,
일회용 workspace를 사용하고, 모든 종료 경로에서 둘 다 독립적으로 제거한다. transcript text,
error message, credential, thread ID, turn ID를 방출하지 않는다. 정제된
[`terminal-truth-v0.144.1-linux.json`](../../../competitive/upstream-codex/captures/terminal-truth-v0.144.1-linux.json)은
릴리스 바이너리 digest와 인증된 `completed` 및 정확한 turn ID의 `interrupted` wire shape를
기록한다. interrupt identity는 비어 있지 않은 `turn/started` notification에서 가져왔고
terminal을 수락하기 전에 `turn/start` response와 대조했다. 두 terminal object 모두 완전한
status/error/items-view/timestamp/duration field set을 포함했으며, capture된 status는 상호
배타적으로 유지되었다.

이 인증된 실행에서는 `error` notification이나 failed terminal이 방출되지 않았다. 따라서 retry와
failed error-field 처리는 릴리스 runtime 검증이 아니라 schema-and-fixture 검증 상태로 남으며,
capture는 그 분기를 live evidence라고 과장하지 않는다.

## P0-C1 닫힌 항목 식별자 근거

P0-C1 구현은 live notification과 snapshot replay가 공유하는 마스킹된 lifecycle observation을
도입한다. 정확하고 제한된 thread, turn, item ID, 18개 stable kind 중 하나 또는 제한된
`Unknown`, live started/completed 또는 snapshot-observed phase, source, live일 때 provider
millisecond timestamp, 보고된 item outcome, metadata-only summary를 보존한다. Snapshot replay는
completion boundary를 만들어 내지 않으므로 active item은 나중에 live completion을 받아도
duplicate로 잘못 분류되지 않는다. status가 없는 live completion은 lifecycle fact로 남으며,
성공한 turn, task, review, delivery의 증거가 아니다.

adapter manifest는 체크인된 모든 `ThreadItem` variant의 모든 property를 preserved,
bounded-redacted, ignored 중 정확히 하나에 할당한다. contract test는 그 manifest를 생성된
`ThreadItem.oneOf` property 및 닫힌 command, file, MCP, dynamic-tool, collaboration status enum과
비교한다. 기존 notification-vocabulary gate는 이제 `item/started`를 소유한다. 따라서 새로운 item
kind, field, closed status, notification method가 생기면 그 결정이 명시될 때까지 classification이
실패한다. Runtime unknown kind는 제한된 ID/type identity만 보존하고 다른 모든 field를 버리며
projection을 incomplete로 표시한다.

결정론적 증명에는 stable kind 18개를 모두 포함하는 schema-shaped snapshot fixture 하나, 같은 item에
대한 live start/completion parity, 누락/불일치/초과 크기 identity, 없는 timestamp, duplicate
boundary, duplicate-side-effect suppression, start 없는 completion, kind mismatch, timestamp
regression, dynamic-tool outcome contradiction, unknown kind, 그리고 prompt, reasoning,
command/output/diff/path/query/image/tool/collaboration/review payload 전반의 secret canary가 포함된다.
100,000-completion reducer test는 최신 ordered record 256개만 보존하고 정확한 truncation count를
노출한다. canonical reducer snapshot은 lifecycle mutation이 일어날 때까지 `Arc`로 공유되므로
일반적인 delta/status event와 복제된 loaded conversation snapshot은 제한된 ledger를 deep-copy하지
않는다. Hydration test는 같은 thread resume에서 counter와 sequence를 보존하고, cross-thread 또는
non-monotonic snapshot을 거부하며, live record를 append하고, 다른 thread에서는 state를 비운다.

모든 live item identity, 그 first kind, completion-seen state는 별도의 512-entry, non-evicting
SHA-256 ledger를 사용한다. 테스트는 main lifecycle window에서 record가 축출된 뒤 completion만
재생하고 전체 started/completed pair도 재생한다. 어느 경우도 effect가 반복되지 않는다. 축출 후
kind가 바뀐 identity는 새로운 lifecycle observation을 보존하고, 더 오래 유지되는 identity
ledger에서 감지되며, 가능한 final payload를 버리고 이후 terminal confirmation을 수락하는 대신
stream을 실패시킨다. 513번째 unique identity도 lifecycle observation은 방출하지만 authoritative
final-agent text나 tool effect가 손실된 뒤 turn을 거짓으로 confirmed 처리할 수 없도록 stream을
실패시킨다. malformed active `item/completed`도 final text나 tool fact를 버리고 이후 terminal
confirmation을 수락하는 대신 stream을 실패시킨다. provider timestamp가 regression한 첫 completion은
anomaly를 보존하지만 고유하게 식별된 payload는 계속 전달한다. Declined file change 뒤에 applied처럼
보이는 replay가 이어져도 suppressed 상태를 유지한다. File-change path/tool effect와 snapshot tool
message에는 typed `Completed` outcome이 필요하다. failed, declined, in-progress, unknown record는
lifecycle projection에만 표시된다. Lifecycle observation은 opt-in prompt-output log에서 명시적으로
제외된다.

이 증거는 schema-and-fixture 검증이다. 릴리스 app-server lifecycle capture, progressive delta
retention, 풍부한 TUI rendering, Admin/CLI/Telegram projection, parallel persistence, restart
recovery를 추가하지 않으며 비교 성능 주장도 하지 않는다.

## P0-C2 제한된 점진적 활동 근거

P0-C2 구현은 app-server adapter에서 application mailbox와 Core까지 이어지는 transient typed
progressive projection 하나를 추가한다. manifest는 agent-message, command-output,
terminal-interaction, file-patch, MCP, plan, 세 가지 reasoning, turn diff, turn plan, token usage,
moderation, guardian activity의 14개 method를 처리한다. Deprecated file-output delta와 thread
compaction은 schema 검증된 explicit ignore다. Model safety buffering과 verification은 이
slice에서 조용히 추론된 operator state가 아니라 schema 검증된 diagnostic-only notification으로
남는다.

처리되는 모든 item activity는 수락되기 전에 정확한 P0-C1 thread/turn/item identity와 보존된 item
kind가 필요하다. missing start, kind drift, item completion 이후 activity는 먼저 제한된
observation을 전달한 다음 stream을 실패시켜, 이후 terminal이 잠재적으로 중요한 payload를 숨기지
못하게 한다. stale scope는 event를 전달하지 않고 sequence도 소비하지 않는다. progressive처럼
보이는 future item, turn 또는 turn-correlated thread method는 UTF-8-safe 4 KiB method label과
payload byte count만 보존하고 history를 unknown으로 표시한 뒤 fail closed한다. active-turn
reducer는 counting writer를 통해 그 count를 얻는다. response/stream handoff 사이에 보존된 early
notification은 별도의 bounded pending-queue byte budget 아래에서 기존 JSON value를 먼저 다시
encode할 수 있다. malformed 또는 oversized identity나 payload data는 false terminal 또는 completed
item fact가 되지 않는다.

domain은 coalesced record를 최대 64개, dynamic detail을 8 MiB 보존한다. 개별 cap은 agent draft와
diff/patch detail에 2 MiB, 64-step plan에 128 KiB, command와 MCP detail에 64 KiB, path에 16 KiB,
guardian copy와 identifier에 4 KiB다. Agent delta는 append하고, command output은 bounded tail을
유지하며, latest-state payload는 replace하고, count-only reasoning/moderation data는 raw text를
보존하지 않는다. Command line count는 chunk boundary에 걸쳐 newline/open-line state를 보존한다.
Plan source와 omitted byte는 별도로 계산된다. discarded rename destination을 포함해 각각의 단일
wire observation은 retained byte와 unretained byte의 합이 source byte와 같아야 한다. merge된
latest-state record는 superseded byte를 truncation이라고 부르지 않고 coalescing counter 아래에
둔다. Payload-truncation event와 완전히 dropped observation은 별도 counter이며 false
distinct-observation count로 표시되지 않고 각각 표시된다. Batch/record construction은 domain
외부에 공개되지 않는다. validation은 projection 전에 non-monotonic, mixed-correlation,
malformed-range, oversized, counter-overflowing, kind/payload-invalid batch를 거부한다. Token
snapshot은 cumulative total에 모든 latest-context field가 포함되기를 요구한다. context pressure는
session cumulative total이 아니라 latest active-context total을 사용한다.

application mailbox에는 독립적인 8-control admission count와 progressive segment의 ordered queue가
있다. Core ingress는 16 control admission과 최대 17 progressive segment로 같은 policy를 반복한다.
인접하고 correlation이 같은 progressive publication만 merge된다. 따라서
`progress-1 -> item boundary -> progress-2`는 retain-then-fail late-activity path를 포함해 정확히
그 순서로 소비된다. control boundary 8개가 모두 pending이어도 최대 9개의 progressive segment는
하나의 global 8 MiB dynamic-detail budget을 공유한다. 오래된 detail은 correlated history-only
batch로 제자리에서 비워지며, source sequence와 loss counter를 control boundary의 올바른 쪽에
보존한다. Progressive pressure는 control admission을 소비하지 않으며 approval, item boundary,
failure, retry, terminal event는 FIFO admission과 ordering을 유지한다. Core는 더 새로운
generation을 admit하기 전에 superseded-generation progressive backlog를 제거하고,
current-generation segment admission failure를 silent loss가 아니라 terminal stream failure로
바꾼다. Core는 batch를 projection으로 직접 소비하며 production TUI는 Core outcome을 한 번에 하나씩
적용하므로 오래된 COW snapshot과 full batch update가 drain vector에 누적되지 않는다.

결정론적 local proof가 다루는 내용은 다음과 같다.

- domain projection을 통과하는 observation 100,000개와 인접 mailbox publication 100,000개,
  그리고 고정된 high-rate agent, command, patch, diff, plan, token, multi-agent reducer;
- application과 Core ingress에서 2 MiB agent segment 사이에 interleave된 control, 정확히 다시
  계산한 각 tracked pending-byte total이 8 MiB 이하이고 approval/terminal admission이 보존됨;
- source order의 처리 대상 method 14개 전부, deprecated ignore 2개, missing/mismatched/completed
  item boundary, stale scope, malformed value, bounded unknown method drift;
- malformed 65번째 discarded patch change 검증을 포함한 정확한 schema `$ref` 및 resolved nested
  type/array/nullability/closed-enum fingerprint;
- cross-publication overflow rejection, truncation, history-only Core application,
  stale-generation pruning, `Arc` reuse/copy-on-write, new-turn reset, post-compaction 및 schema-maximum
  context-pressure arithmetic, 정확한 UTF-8 omitted byte와 rename loss, JSON parsing 전 non-UTF-8
  line rejection;
- transient typed memory에서만 사용할 수 있고 batch/event Debug와 prompt-log output capture에는
  없는 raw command-output secret canary.

Core snapshot은 bounded projection과 incomplete-history counter를 보존한다. 기존 TUI는 typed agent
draft만 transient live buffer로 소비하며, legacy uncorrelated delta event는 제거되었다. 이
slice에서는 progressive type 어느 것도 persistence serialization을 구현하거나 prompt-log record에
들어가거나 Admin, CLI, Telegram, parallel state로 projection되지 않는다.

위 memory assertion은 allocator capacity, RSS, 비교 성능 측정이 아니라 보존된 `String`/identifier
length와 queue accounting의 정확한 합계다. P0-C2 commit 시점에는 schema-and-fixture와 결정론적
local evidence였고, 릴리스 app-server progressive capture, 풍부한 TUI rail, Diff/Output inspector,
context-pressure UI, restart persistence, 비교 latency/RSS 결과는 없었다. 아래의 이후
P0-D1/P0-D2/P0-D3 evidence는 명시된 TUI rail 및 inspector gap만 대체한다. 다른 주장은 P0-D,
recovery, 이후 native performance artifact에 그대로 남는다.

## P0-D1부터 P0-D3까지 TUI 투영 근거

P0-D1은 `b53559ca32e1fddd22f13d2de4aad028feebf9fe`에, P0-D2는
`594859a621e7213356822b008a496f4d5cfca36f`에, P0-D3는
`89264f6e8edf1dec31393e2fd2804054cc26662d`에 고정되어 있다. 이 이후 Akra commit들은 아래에
문서화된 `226e4794...` audit baseline과 별개다.

- P0-D1의 [타입 지정 요약 리듀서](https://github.com/RefinedStone/codex-exec-loop/blob/b53559ca32e1fddd22f13d2de4aad028feebf9fe/src/adapter/inbound/tui/app/conversation_model/progressive_activity.rs)는
  제한되고 payload가 없는 current activity와 approval-first arbitration을 projection한다.
  [결정론적 리듀서 테스트](https://github.com/RefinedStone/codex-exec-loop/blob/b53559ca32e1fddd22f13d2de4aad028feebf9fe/src/adapter/inbound/tui/app/conversation_model/progressive_activity_tests.rs)와
  narrow, wide, vt100 snapshot은 responsive priority 동작을 다룬다. 분류:
  `verified/source-and-local-test`.
- P0-D2의 [타입 지정 유지 상세 상태](https://github.com/RefinedStone/codex-exec-loop/blob/594859a621e7213356822b008a496f4d5cfca36f/src/adapter/inbound/tui/app/conversation_model/progressive_activity_detail.rs)는
  Core snapshot에 대한 `Weak` reference를 보유하고, strong frame-local document guard를 노출하며,
  lifecycle boundary에서 reset된다.
  [페이로드 없는 뷰포트 상태](https://github.com/RefinedStone/codex-exec-loop/blob/594859a621e7213356822b008a496f4d5cfca36f/src/adapter/inbound/tui/app/progressive_activity_overlay_ui.rs),
  [터미널 안전 제한 렌더러](https://github.com/RefinedStone/codex-exec-loop/blob/594859a621e7213356822b008a496f4d5cfca36f/src/adapter/inbound/tui/app/shell_presentation/overlays/activity.rs),
  [인라인 라우팅](https://github.com/RefinedStone/codex-exec-loop/blob/594859a621e7213356822b008a496f4d5cfca36f/src/adapter/inbound/tui/app/shell_rendering/inline_inspection.rs)은
  정확한 source, retained, truncation, incomplete-history metadata를 보존하면서 scan과 materialized
  output을 보이는 viewport로 제한한다. 분류: `verified/source`.
- [상세 수명 주기 테스트](https://github.com/RefinedStone/codex-exec-loop/blob/594859a621e7213356822b008a496f4d5cfca36f/src/adapter/inbound/tui/app/conversation_model/progressive_activity_detail_tests.rs),
  [페이지 상태 테스트](https://github.com/RefinedStone/codex-exec-loop/blob/594859a621e7213356822b008a496f4d5cfca36f/src/adapter/inbound/tui/app/progressive_activity_overlay_ui.rs),
  [컨트롤러 키맵 테스트](https://github.com/RefinedStone/codex-exec-loop/blob/594859a621e7213356822b008a496f4d5cfca36f/src/adapter/inbound/tui/app/shell_controller.rs),
  [스냅샷 테스트](https://github.com/RefinedStone/codex-exec-loop/blob/594859a621e7213356822b008a496f4d5cfca36f/src/adapter/inbound/tui/app/shell_rendering_tests.rs),
  [TestBackend/vt100 트랜잭션 테스트](https://github.com/RefinedStone/codex-exec-loop/blob/594859a621e7213356822b008a496f4d5cfca36f/src/adapter/inbound/tui/app/inline_terminal_adapter/tests.rs)는
  tab/page navigation, same-sequence new-turn 및 resize reset, 실제 approval preemption,
  narrow/wide rendering, control escaping, CJK cell width, raw-detail host-scrollback negative를
  다룬다. 분류: `verified/local-test`.
- P0-D3의 [페이로드 없는 터미널 리듀서](https://github.com/RefinedStone/codex-exec-loop/blob/89264f6e8edf1dec31393e2fd2804054cc26662d/src/adapter/inbound/tui/app/conversation_model/activity_rail.rs)는
  confirmed interrupted/failed/unknown receipt, unconfirmed recovery, generic runtime failure를
  typed marker에 raw error detail을 보존하지 않고 매핑한다. 기존 bounded Status transcript copy가
  terminal-detail owner로 남는다.
  [우선순위 조합기](https://github.com/RefinedStone/codex-exec-loop/blob/89264f6e8edf1dec31393e2fd2804054cc26662d/src/adapter/inbound/tui/app/shell_presentation/status_panels/activity_rail.rs)는
  `runtime_envelope.applied.model`만 읽고, turn start가 locally correlated submission 뒤에 오며
  그 turn이 live인 동안에만 planning handoff를 보존하고, incomplete-history truth를
  model/task/coarse context보다 앞에 보존하고, dynamic value를 128-character scan과 32/16 terminal
  cell로 제한하고, control, zero-width, rail-separator character를 sanitize하며, 우선순위가 낮은
  fact는 통째로 버린다. 분류:
  `verified/source-and-local-test`.
- P0-D3의 [런타임 리듀서 테스트](https://github.com/RefinedStone/codex-exec-loop/blob/89264f6e8edf1dec31393e2fd2804054cc26662d/src/adapter/inbound/tui/app/conversation_runtime.rs),
  [좁은/넓은 스냅샷](https://github.com/RefinedStone/codex-exec-loop/blob/89264f6e8edf1dec31393e2fd2804054cc26662d/src/adapter/inbound/tui/app/shell_rendering_tests.rs),
  [TestBackend/vt100 트랜잭션](https://github.com/RefinedStone/codex-exec-loop/blob/89264f6e8edf1dec31393e2fd2804054cc26662d/src/adapter/inbound/tui/app/inline_terminal_adapter/tests.rs)은
  receipt-state mapping, terminal lifecycle reset, requested-versus-applied model truth,
  correlated task lifetime과 recovered-start clearing, stale submitting-lane suppression,
  legacy live-fallback suppression, responsive whole-fact collapse, progressive
  raw-payload/host-scrollback negative 및 payload-free terminal-marker Debug를 다룬다.
  분류: `verified/local-test`.

이는 manual real-terminal capture, 릴리스 Akra runtime activity capture, allocator/RSS 결과,
비교 latency 결과가 아니라 결정론적 TestBackend/vt100 및 source evidence다. P0-D를
완료하지 않으며 Admin, CLI, Telegram, parallel persistence, durable restart recovery,
reconciliation recovery projection을 추가하지 않는다.

## P0-D4 보충 tmux 근거

기초 physical-resize fix는 `8b14079b9dc7bcc7cadc55176b731558819bb9e5`에,
resize-transaction guard는 `5907f6a7f1f03f23218a201637ebf06a8e020369`에, 강화된
결정론적 capture candidate는 `0a5f06ea345bd5c24998527679158eaf8643d058`에 고정되어 있다.
체크인된 [E3 캡처](../../../competitive/upstream-codex/captures/typed-activity-rail-e2e-v1-linux-tmux.json)는 명시적인
`env -i` allowlist와 `HostScrollback` 및 `StandardScrollRegion` override 아래에서 실제 tmux 3.4
detached PTY로 실행되는 source build를 기록한다. startup, 160x24 active frame, 48x18 shrink,
application-rendered 80x18 model canary, 두 번째 48x18 frame, 160x24 restore, committed completion,
clean process exit를 다룬다. Tmux는 48 -> 80 -> 48 전환에 걸쳐 빈 host-history reflow row를
만들므로 raw pane digest와 row count가 의도적으로 다르다. blank-row와 elapsed-time을 normalize한
뒤에는 두 narrow semantic current/history digest가 동일하다. transient activity, working, input,
model, task, running-prompt count는 모두 host history에서 0으로 유지된다. 관찰된 raw PTY는
committed canary에 도달하지만 raw ANSI secret은 없다. 분류: `verified/local`.

이 아티팩트는 의도적으로 `supplemental-unmatched`이며 `approvalGrade: false`다. owner-isolated
synthetic app-server와 source build를 사용하고, 16-row inline viewport보다 낮은 physical height는
제외하며, 릴리스 Akra runtime activity evidence가 아니다. raw PTY byte는 일시적인 local
observation이다. 보존되지 않으며 repository content로 digest를 다시 계산할 수 없다.
resize fix는 공통 terminal primitive를 변경하므로, 이 보충 E3 environment artifact만으로는
approval row를 충족하지 않는다.

## P0-D4 일급 수동 근거

검토된
[E1-E4 물리적 크기 조정 아티팩트](../../../validation/artifacts/pr-1926-physical-resize/README.md)는
정제된 manual evidence를 정확한 Akra candidate commit/tree 및 platform별 source-build binary
digest에 결합한다. 실제 Codex app-server session, machine-authoritative geometry, 필수 scenario,
safe frame, integrity index를 기록한다. Authentication state와 raw rollout은 제외된다.
분류: `verified/local`.

이 네 가지 first-class capture는 공유 terminal-primitive manual reviewer gate를 충족한다. 각각은
`supplemental-unmatched`로 남는다. 어느 것도 `terminal-baseline` total을 변경하거나 릴리스 Akra
runtime을 입증하지 않는다. P0-D는 릴리스 Akra runtime activity capture에 대해서만 partial로
남는다. Admin, CLI, Telegram, persistence, recovery, performance 주장은 뒤따르지 않는다.

## Akra 기준선 근거

아래의 모든 링크는 Akra commit `226e4794b84107704378ecc1ea65f7d5c27750e5`를 사용한다.

### 프로토콜 투영

- [알림 분류](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/protocol/contract_tests.rs#L21-L99)은
  handled method 8개, deferred 13개, diagnostic-only 29개, ignored 18개를 나열한다. 분류:
  `verified/source`.
- [활성 리듀서](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/protocol/turn_notifications.rs#L201-L253)는
  typed translation 없이 deferred method를 버리고, 모든 `error`를 terminate하며, 일치하는 모든
  `turn/completed`를 generic completion으로 변환하면서 terminal event send result를 버린다.
  이어서 [연결 대기 루프](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/connection.rs#L1204-L1304)는
  typed terminal receipt가 아니라 `Result<()>`를 반환한다. 분류: `verified/source`.
- [완료 항목 투영](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/protocol/turn_notifications.rs#L485-L533)은
  live에서 agent message, file change, command execution만 처리한다.
  [스냅샷 투영](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/protocol/turn_notifications.rs#L257-L300)은
  user message를 추가한다. 분류: `verified/source`.
- [Core 턴 리듀서](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/core/app/turn_stream.rs#L122-L139)에는
  generic completed 및 failed terminal branch가 있지만 upstream interrupted/retrying status는 없다.
  분류: `verified/source`.
- [병렬 아카이브 경로](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/mod.rs#L1333-L1357)는
  stream result가 `Ok`일 때 archive한다.
  [프롬프트 로그 상태](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/mod.rs#L982-L1023)도
  마찬가지로 `Ok`에서 completed를 파생한다. 분류: `verified/source`.

### 기능과 세션

- [초기화 기능](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/connection.rs#L1048-L1066)는
  `experimentalApi: false`를 보낸다. 분류: `verified/source`.
- [스레드 응답 DTO](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/protocol.rs#L482-L536)는
  start/resume에서 `thread`만 읽고 thread provenance의 subset을 projection한다. 분류:
  `verified/source`.
- [스레드 목록 매개변수](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/protocol.rs#L270-L283)은
  response `nextCursor`를 보존하면서도 cursor를 생략한다. 분류: `verified/source`.
- [정적 모델 UI](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/inbound/tui/app/model_selection_overlay_ui.rs#L31-L74)와
  [닫힌 노력 타입](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/domain/conversation.rs#L96-L116)은
  `model/list`를 소비하거나 임의의 공식 effort value를 보존하지 않는다. 분류:
  `verified/source`.

### 승인과 비밀 경계

- [승인 파서](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/approval.rs#L90-L132)는
  command approval에 command가 있기를 요구하므로 command/cwd를 생략할 수 있는 공식 network-only
  request가 손실된다. 분류: `verified/source`.
- 위 classification 링크에서 `serverRequest/resolved`는 diagnostic-only다. typed approval
  state로 사용할 수 없다. 분류: `verified/source`.
- [자식 프로세스 환경 정책](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/connection.rs#L356-L398)는
  기본적으로 scrubbed이며 정확한 `all`을 elevated override로 수락한다.
  [허용 목록/필터](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/connection.rs#L443-L517)는
  app-server를 시작하기 전에 ambient credential-bearing value를 거른다. 분류:
  `verified/source`.
- [생성 셸 실행 정책](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/connection.rs#L668-L768)는
  inheritance의 기본값을 `core`로 하고, 기본 secret exclude를 보존하며, 명시적인 Codex
  override를 통해 login-shell processing을 비활성화한다. 분류: `verified/source`.
- [실행 정책 기본값](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/execution_policy.rs#L37-L57)는
  공식 project trust decision을 참조하지 않고 일반 work turn에 user/on-request approval과 함께
  workspace-write를 제공한다. Thread setup과
  [숨은 계획 턴](https://github.com/RefinedStone/codex-exec-loop/blob/226e4794b84107704378ecc1ea65f7d5c27750e5/src/adapter/outbound/app_server/mod.rs#L628-L649)은
  read-only를 사용한다. Akra의 environment inheritance가 더 좁더라도 normal-work 기본값은
  first-party no-decision read-only 기본값보다 더 permissive하다. 분류:
  `verified/source`.
- OpenCode [알려진 카나리 제한 송신 매트릭스](../opencode/gap-matrix.md#알려진-카나리-기반-제한-외부-전송-매트릭스)가
  이미 그 boundary의 end-to-end proof를 소유한다. Upstream의 `inherit: All` 기본값은 우선순위를
  높이지만 중복 secret subsystem을 정당화하지는 않는다.

### 전달과 패키징

- [현재 계약](../../../reference/current-product.md)는 수락된 planning, parallel
  worktree/lease, review, integration, cleanup 동작을 정의한다. 일반적인 implementation caveat가
  있는 `verified/source` 분류다.
- [네이티브 패키징 런북](../../../plan/13-native-packaging-and-operator-runbook.md)과
  `scripts/package_native_release.sh`은 이미 locked Cargo build, bundle-internal checksum,
  archive checksum, safe extraction validation, npm provenance를 요구한다. 분류:
  `verified/source`.
- 동시에 active 상태인 `fix/native-validation-evidence-contract` lane은 baseline truth로
  조사하지 않았고 수정하지도 않았다. 최종 merge 시 이 audit의 release-evidence gap을 대체할
  수 있다.

## 감사 한계

감사에서는 다음을 수행하지 않았다.

- model call을 authenticate하거나 model quality를 비교하지 않았다.
- 공정한 대화형 upstream-TUI 대 Akra-TUI benchmark를 실행하지 않았다.
- 전체 Codex source suite 또는 모든 Bazel/Cargo CI matrix를 실행하지 않았다.
- cosign bundle, SBOM/provenance, macOS notarization, Windows signing, arm64 artifact를 독립적으로
  검증하지 않았다.
- remote-control relay, remote exec Noise transport, realtime, cloud task, 인증된
  app/plugin/MCP, 유료 service path를 실행하지 않았다.
- 추론된 core-event backlog, V2 limiter race, side-thread peak accumulation,
  rollout permission exposure, large-input compaction failure를 재현하지 않았다.
- 전체 package archive, system `bwrap`, macOS 또는 Windows가 있는 host에서 sandbox 동작을
  테스트하지 않았다.
- 체크인된 audit harness 또는 그 local output을 trusted release receipt로 승격하지 않았다.
- feature stage, protocol stability, default enablement, end-user maturity가 같은 속성이라고
  주장하지 않았다.
- 동시 Cargo 작업으로 변경된 공유 temporary source lockfile을 수정하거나 정리하지 않았다.

성능 결론은 의도적으로 제한되어 있다. 보안 finding은 정확한 boundary와 default를 설명하며,
upstream 또는 Akra가 안전하거나 안전하지 않다는 일반적인 주장이 아니다. source inventory는
크고 빠르게 변하므로 absence finding은 고정된 commit과 검색한 path로 제한된다.
