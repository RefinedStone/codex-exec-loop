# OpenCode 증거 원장

[English](../../../competitive/opencode/evidence.md)

이 원장은 변경 불가능한 소스 검사, 재현한 릴리스/런타임 출력, 문서화된 동작, 추론, 검증하지
않은 실험을 구분한다. 제품 결론은 [analysis.md](analysis.md)에, Akra 작업 결정은
[gap-matrix.md](gap-matrix.md)에 있다.

## 스냅샷

| 필드 | 값 |
| --- | --- |
| 제품 | OpenCode |
| 공식 저장소 | <https://github.com/anomalyco/opencode> |
| 릴리스 | [v1.17.18](https://github.com/anomalyco/opencode/releases/tag/v1.17.18) |
| 릴리스 커밋 | `b1fc8113948b518835c2a39ece49553cffe9b30c` |
| 릴리스 커밋 날짜 | 2026-07-09 18:51:29 +00:00 |
| 릴리스 게시 | 2026-07-09 18:51:45 +00:00 |
| 감사 날짜 | 2026-07-12 (Asia/Seoul) |
| Akra 기준선 | `prerelease`의 `354f4782a73ffabab6abf342cb0f37285a96c177` |
| Akra 버전 | 1.3.5 |
| 환경 | WSL2의 Ubuntu 24.04.2 LTS, Linux 6.18.33.2, x86_64, Bun 1.3.14 |

OpenCode 소스 checkout은 공개되어 있었고 로컬에서 사용할 수 있었으며 깨끗한 상태로 릴리스
태그에 detach되어 있었고 소스 수정 없이 남겨 두었다. Akra 기준선은 `origin/prerelease`에서
만든 깨끗한 worktree였다.

GitHub 릴리스 API는 릴리스 `351708464`를 변경 불가능하고 draft 및 prerelease가 아닌 것으로
식별했다. Linux x64 archive checksum은 릴리스 asset digest와 정확히 일치했다.

```text
e149d32ee5667c0cd5fb84d0bf8393b312e93782eeb4d74d29bbb0392de7133c
```

## 증거 등급

- `verified`: 변경 불가능한 커밋에서 검사했거나, 변경 불가능한 릴리스 metadata/artifact에서
  관찰했거나, 로컬에서 재현함;
- `documented`: 공식 프로젝트 문서가 명시했지만 재현하지 않음;
- `proposed`: experimental, beta, transitional, incomplete로 표시되었거나 출시된 기본 경로
  밖에 존재함;
- `inferred`: 검증된 사실에서 논리적으로 도출한 제한적 결과이지만 종단 간 재현하지 않음;
- `unverified`: 필요한 환경, 인증된 실행, 원시 sample, 외부 정책 증거가 없음.

inventory count와 bundle size는 방향 설정을 위한 증거이지 품질 또는 runtime-performance
점수가 아니다. 소스 route는 명시된 런타임 전제 조건 아래에서 도달 가능한 구현 경로만 입증한다.

## 재현 명령

### 소스 및 릴리스 식별 정보

```bash
git clone https://github.com/anomalyco/opencode.git /tmp/akra-opencode-v1.17.18-source
git -C /tmp/akra-opencode-v1.17.18-source checkout --detach v1.17.18
git -C /tmp/akra-opencode-v1.17.18-source rev-parse HEAD
git -C /tmp/akra-opencode-v1.17.18-source describe --tags --exact-match
git -C /tmp/akra-opencode-v1.17.18-source log -1 --format='%cI %s'
git -C /tmp/akra-opencode-v1.17.18-source status --short

curl -fsSL https://api.github.com/repos/anomalyco/opencode/releases/tags/v1.17.18 \
  | jq '{id,tag_name,target_commitish,draft,prerelease,immutable,published_at}'
curl -fsSL https://api.github.com/repos/anomalyco/opencode/releases/tags/v1.17.18 \
  | jq -r '.assets[] | select(.name == "opencode-linux-x64.tar.gz")
      | [.name,.size,.digest] | @tsv'
```

관찰한 소스 식별 정보:

```text
b1fc8113948b518835c2a39ece49553cffe9b30c
v1.17.18
2026-07-09T18:51:29+00:00 release: v1.17.18
```

릴리스 산출물 검증:

```bash
curl -fL \
  https://github.com/anomalyco/opencode/releases/download/v1.17.18/opencode-linux-x64.tar.gz \
  -o /tmp/opencode-v1.17.18-linux-x64.tar.gz
sha256sum /tmp/opencode-v1.17.18-linux-x64.tar.gz
mkdir -p /tmp/opencode-v1.17.18-bin
tar -xzf /tmp/opencode-v1.17.18-linux-x64.tar.gz \
  -C /tmp/opencode-v1.17.18-bin
/tmp/opencode-v1.17.18-bin/opencode --version
du -b /tmp/opencode-v1.17.18-linux-x64.tar.gz \
  /tmp/opencode-v1.17.18-bin/opencode
```

관찰 결과:

| 산출물 | 결과 |
| --- | ---: |
| Linux x64 압축 파일 | 69,427,073바이트 |
| 압축 해제한 바이너리 | 188,979,328바이트 |
| 바이너리 버전 | `1.17.18` |
| 압축 파일 SHA-256 | 릴리스 API 다이제스트와 일치 |

### 인벤토리 및 관문

```bash
cd /tmp/akra-opencode-v1.17.18-source
git ls-files | wc -l
git ls-files '*.ts' '*.tsx' | wc -l
git ls-files -z '*.ts' '*.tsx' \
  | while IFS= read -r -d '' file; do wc -l < "$file"; done \
  | awk '{sum += $1} END {print sum}'
git ls-files | rg '(test|spec)\.(ts|tsx|js|mjs)$' | wc -l

bun install --frozen-lockfile
bun typecheck
mkdir -p /tmp/opencode-bun-bin
ln -sf "$(command -v bun)" /tmp/opencode-bun-bin/bunx
PATH=/tmp/opencode-bun-bin:$PATH GITHUB_ACTIONS=false bun turbo test
PATH=/tmp/opencode-bun-bin:$PATH CI=true GITHUB_ACTIONS=false bun turbo test

cd packages/opencode
bun test --timeout 30000 --only-failures

cd ../app
CI=true bun run test
bun test --preload ./happydom.ts ./src/i18n/parity.test.ts

cd ../desktop
bun test --timeout 30000
```

이 host의 Bun 설치가 `bunx` 실행 파일을 제공하지 않았기 때문에
`/tmp/opencode-bun-bin/bunx`는 설치된 Bun binary에 대한 외부 symlink였다. 이 환경 제약을
위해 저장소를 patch하지 않았다.

관찰한 inventory:

| Inventory | 수량 |
| --- | ---: |
| 추적 파일 | 6,230 |
| 추적 TypeScript 파일 | 3,033 |
| 추적 TypeScript LOC | 585,795 |
| JS 계열 test/spec 경로명 | 687 |

관찰한 gate:

| Gate | 결과 |
| --- | --- |
| `bun install --frozen-lockfile` | 통과, 4,696 packages |
| `bun typecheck` | 통과, 30/30 Turbo tasks, 범위 내 36 packages |
| CI 조건 monorepo `bun turbo test` | 통과, 6m 7.628s에 9/9 Turbo tasks |
| `packages/opencode` | 245개 파일에서 3,110 pass, 22 skip, 1 todo, 0 fail |
| `CI=true`의 app `test:unit` | 86개 파일에서 567 pass, 4 skip, 0 fail |
| `CI=true`의 app `test:browser` | 10개 파일에서 27 pass, 0 fail |
| desktop 직접 test | 12개 파일에서 61 pass, 0 fail |
| `CI` 없는 focused i18n parity | 3 pass, 1 fail, Arabic에 key 3개 누락 |

최초 non-CI monorepo 실행은 host에 `bunx`가 없어 처음 실패했다. 외부 symlink를 사용하자 Arabic
parity failure에 도달했다. 해당 parity suite는 `describe.skipIf(!!process.env.CI)`를 사용하므로
CI 조건 실행은 누락 key 사례를 실행하지 않는다. negative-path test 내부의 예상된 error logging은
test failure로 계산하지 않았다. 두 실행 뒤에도 checkout은 깨끗하게 유지됐다.

### 서버 인증

```bash
audit_root="$(mktemp -d /tmp/opencode-auth-audit.XXXXXX)"
pid=''
cleanup() {
  if [[ -n "$pid" ]]; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
  rm -rf "$audit_root"
}
trap cleanup EXIT
mkdir -p "$audit_root/home" "$audit_root/workspace"
cd "$audit_root/workspace"

HOME="$audit_root/home" \
OPENCODE_DISABLE_PROJECT_CONFIG=1 \
OPENCODE_DISABLE_DEFAULT_PLUGINS=1 \
OPENCODE_DISABLE_MODELS_FETCH=1 \
OPENCODE_SERVER_PASSWORD='AKRA_TEST_PASSWORD_11718' \
  /tmp/opencode-v1.17.18-bin/opencode serve \
  --hostname 127.0.0.1 --port 45123 >"$audit_root/server.log" 2>&1 &
pid=$!
ready=0
for _ in $(seq 1 100); do
  if curl --max-time 1 -fsS -u 'opencode:AKRA_TEST_PASSWORD_11718' \
    http://127.0.0.1:45123/global/health >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 0.1
done
test "$ready" = 1

curl --max-time 10 -sS -o "$audit_root/health-unauth.json" -w '%{http_code}\n' \
  http://127.0.0.1:45123/global/health
curl --max-time 10 -sS -u 'opencode:AKRA_TEST_PASSWORD_11718' \
  -o "$audit_root/health-auth.json" -w '%{http_code}\n' \
  http://127.0.0.1:45123/global/health
cat "$audit_root/health-auth.json"
```

관찰 결과:

```text
401
200
{"healthy":true,"version":"1.17.18"}
```

<a id="resolved-secret-canary"></a>
### 해석 완료 시크릿 카나리

두 번째 실험에서는 이 감사를 위해서만 만든 가짜 값을 사용했다. 실제 provider, MCP, GitHub,
OpenAI credential은 사용하지 않았다.

```bash
audit_root="$(mktemp -d /tmp/opencode-canary-audit.XXXXXX)"
pid=''
cleanup() {
  if [[ -n "$pid" ]]; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
  rm -rf "$audit_root"
}
trap cleanup EXIT
mkdir -p "$audit_root/home" "$audit_root/workspace"
cd "$audit_root/workspace"

export AKRA_CANARY_PROVIDER_11718='provider-canary-value-11718'
export AKRA_CANARY_MCP_11718='mcp-canary-value-11718'
export OPENCODE_CONFIG_CONTENT='{
  "provider": {
    "openai": {
      "options": {"apiKey": "{env:AKRA_CANARY_PROVIDER_11718}"}
    }
  },
  "mcp": {
    "akra-canary": {
      "type": "local",
      "command": ["printf", "ready"],
      "enabled": false,
      "environment": {"AKRA_TOKEN": "{env:AKRA_CANARY_MCP_11718}"}
    }
  }
}'
env -u OPENCODE_SERVER_PASSWORD \
  HOME="$audit_root/home" \
  OPENCODE_DISABLE_PROJECT_CONFIG=1 \
  OPENCODE_DISABLE_DEFAULT_PLUGINS=1 \
  OPENCODE_DISABLE_MODELS_FETCH=1 \
  /tmp/opencode-v1.17.18-bin/opencode serve \
  --hostname 127.0.0.1 --port 45124 >"$audit_root/server.log" 2>&1 &
pid=$!
ready=0
for _ in $(seq 1 100); do
  if curl --max-time 1 -fsS \
    http://127.0.0.1:45124/global/health >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 0.1
done
test "$ready" = 1

curl --max-time 20 -sS -o "$audit_root/config.json" -w '%{http_code}\n' \
  http://127.0.0.1:45124/config
curl --max-time 20 -sS -o "$audit_root/provider.json" -w '%{http_code}\n' \
  http://127.0.0.1:45124/provider
curl --max-time 20 -sS -o "$audit_root/v2-provider.json" -w '%{http_code}\n' \
  http://127.0.0.1:45124/api/provider
curl --max-time 20 -sS -o "$audit_root/v2-session.json" -w '%{http_code}\n' \
  'http://127.0.0.1:45124/api/session?limit=1'
curl --max-time 20 -sS -o "$audit_root/legacy-session.json" -w '%{http_code}\n' \
  'http://127.0.0.1:45124/session?limit=1'
rg -o 'provider-canary-value-11718|mcp-canary-value-11718' \
  "$audit_root/config.json" "$audit_root/provider.json"
rg -F 'server is unsecured' "$audit_root/server.log"
```

관찰 결과:

- 시작 시 `Warning: OPENCODE_SERVER_PASSWORD is not set; server is unsecured.`를 출력했다;
- 인증하지 않은 `/config`가 HTTP `200`과 해석 완료된 두 카나리 값을 반환했다;
- 인증하지 않은 `/provider`가 HTTP `200`과 프로바이더 카나리를 반환했다;
- 인증하지 않은 마운트된 `/api/provider`가 HTTP `200`을 반환했지만 테스트한 v2 catalog 응답에는
  설정된 카나리가 없었다. 동적인 v2 시크릿 공개 주장은 하지 않는다;
- 인증하지 않은 마운트된 `/api/session?limit=1`과 레거시 `/session?limit=1`이 모두 HTTP
  `200`을 반환하여 두 route 계열이 릴리스 binary에 존재함을 확인했다;
- 서버는 `127.0.0.1`에만 bind했으며 capture 뒤 종료했다.

이는 테스트한 설정의 응답 경계를 입증한다. loopback인 기본 hostname에서의 remote exposure나
model에 의한 exploitation은 입증하지 않는다.

### 제품 빌드 규모

표면 품질 감사는 exact-SHA export에서 production application 및 Electron asset build를 실행했다.

```bash
rm -rf /tmp/opencode-v1.17.18-build
mkdir -p /tmp/opencode-v1.17.18-build
git -C /tmp/akra-opencode-v1.17.18-source archive \
  b1fc8113948b518835c2a39ece49553cffe9b30c \
  | tar -x -C /tmp/opencode-v1.17.18-build
cd /tmp/opencode-v1.17.18-build
bun install --frozen-lockfile

/usr/bin/time -f 'elapsed=%e max_rss_kib=%M' \
  bun --cwd packages/app run build 2>&1 | tee /tmp/opencode-app-build.log
find packages/app/dist -type f ! -name '*.map' -printf '%s\n' \
  | awk '{bytes += $1; files += 1} END {print bytes, files}'

/usr/bin/time -f 'elapsed=%e max_rss_kib=%M' \
  bun --cwd packages/desktop run build 2>&1 | tee /tmp/opencode-desktop-build.log
find packages/desktop/out -type f ! -name '*.map' -printf '%s\n' \
  | awk '{bytes += $1; files += 1} END {print bytes, files}'
```

관찰 결과:

| 빌드 관찰 | 결과 |
| --- | ---: |
| 웹 모듈 | 2,398 |
| 웹 빌드 소요 시간 | 13.99 s |
| 웹 진입점 JavaScript | 2,815.62 kB, gzip 후 842.34 kB |
| 웹 CSS | 450.35 kB, gzip 후 약 66 kB |
| 웹 소스 맵 제외 자산 | 42,374,239바이트, 858개 파일 |
| Electron 렌더러 진입점 | 6,731.22 kB |
| 번들 Node 서버 청크 | 31,153.76 kB |
| Electron 소스 맵 제외 출력 | 83,169,455바이트 |
| Electron 자산 빌드 소요 시간 | 37.89 s |

소요 시간은 반복 sample이나 보존한 원시 timing artifact가 없는 단일 방향 설정용 실행이다. 크기는
build 및 bundle 사실이다. 어느 것도 packaged startup, steady-state RAM, CPU, terminal input
latency, long-session behavior를 측정하지 않는다.

Desktop installer 크기는 다음으로 갱신할 수 있다.

```bash
curl -fsSL https://api.github.com/repos/anomalyco/opencode/releases/tags/v1.17.18 \
  | jq -r '.assets[]
      | select(.name|test("opencode-desktop.*(dmg|exe|AppImage|deb|rpm)$"))
      | [.name,.size] | @tsv'
```

주요 installer asset은 약 103.9 MiB~158.3 MiB였다. architecture와 packaging format은 교란
변수이므로 이 범위를 제품 간 점수로 볼 수 없다.

## 변경 불가능한 소스 원장

<a id="product-release-and-topology"></a>
### 제품, 릴리스, 토폴로지

- [README 제품 및 설치 표면](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/README.md#L1-L91)은
  open-source agent, provider 폭, TUI, desktop beta, 배포를 식별한다. 등급: 제품 positioning은
  `documented`, check-in된 릴리스 text는 `verified`.
- [TUI 명령 진입점](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/tui.ts#L198-L245)은
  internal worker 또는 external server mode를 선택한다. 등급: `verified`.
- [기본 TUI 종료](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/tui.ts#L217-L295)는
  client가 종료될 때 worker server에 종료를 요청하고 이를 끝낸다. 등급: `verified`.
- [TUI worker](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/tui/worker.ts#L23-L78)는
  internal server와 event bridge를 만든다. 등급: `verified`.
- [Renderer 설정](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/tui/src/app.tsx#L186-L219)은
  60 Hz renderer를 대상으로 하며, [SDK context batching](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/tui/src/context/sdk.tsx#L48-L80)은
  16 ms window 안의 event를 합친다. 등급: 측정된 frame performance가 아닌 `verified` 설계 설정.
- [Server event route](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/server/routes/instance/httpapi/handlers/event.ts#L12-L84)는
  무제한 queue를 사용하며 재생 가능한 event ID를 내보내지 않는다. [마운트된 v2 event handler](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/server/src/handlers/event.ts#L9-L50)는
  용량 256을 사용하지만 역시 ID를 내보내지 않고 overflow 시 stream을 실패시킨다. 등급: `verified`.
- [TUI 재연결 loop](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/tui/src/context/sdk.tsx#L82-L116)는
  backoff로 external event를 재시도하지만 전체 state bootstrap을 수행하지 않는다. 등급:
  `verified`.
- [Browser 재연결 동기화](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/app/src/context/server-sync.tsx#L372-L395)는
  root/directory bootstrap을 다시 실행하고 session list/status state를 갱신한다. 캐시된 모든
  session message/part window의 재로딩을 강제하지는 않는다. 등급: `verified`.
- external client에는 replay ID가 없고 어느 경로도 완전한 cached-transcript reconciliation을
  입증하지 않으므로 event 누락은 선택된 transcript window를 오래된 상태로 남길 수 있다. 등급:
  `inferred`, disconnect loss는 동적으로 재현하지 않음.
- [Server route 구성](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/server/routes/instance/httpapi/server.ts#L271-L303)은
  배포된 listener의 root, event, instance, UI route 옆에 v2 server layer를 마운트한다. 등급:
  `verified`, 개별 endpoint 성숙도는 별도로 표시함.
- [Headless run event loop](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/run.ts#L670-L871)는
  event에서 stdout과 idle completion을 도출하고 성공한 prompt response body로 누락 출력을
  복원하지 않는다. event loss 뒤 불완전한 output 또는 비정상적인 completion waiting은
  `inferred`이며 재현하지 않았다.

<a id="tui-commands-and-long-sessions"></a>
### TUI, 명령, 긴 세션

- [기본 keybinding registry](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/tui/src/config/keybind.ts#L41-L117)는
  leader와 session/model/navigation action을 정의한다. 등급: `verified`.
- [Which-key plugin](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/tui/src/feature-plugins/system/which-key.tsx#L184-L248)은
  사용 가능한 binding을 group하고 filter한다. 등급: `verified`.
- [Session list](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/tui/src/component/dialog-session-list.tsx#L188-L357)는
  search, pinning, rename/delete, quick slot, status를 구현한다. 등급: `verified`.
- [Session action](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/tui/src/routes/session/index.tsx#L458-L665)은
  compaction, fork, revert, timeline, share, diff, child navigation을 포함한다. 등급: `verified`.
- [Permission UI](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/tui/src/routes/session/permission.tsx#L20-L87)와
  [question UI](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/tui/src/routes/session/question.tsx#L35-L94)는
  Akra의 현재 binary modal보다 풍부한 structured decision을 보존한다. 등급: `verified`.
- [Mini-mode 수명 주기](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/run/runtime.lifecycle.ts#L305-L400)는
  split footer, replay, reset, native scrollback을 처리한다. 등급: `verified`.
- [TUI 초기 hydration](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/tui/src/context/sync.tsx#L588-L650)은
  최신 100개 message를 유지하고 [timeline](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/tui/src/routes/session/dialog-timeline.tsx#L22-L46)은
  현재 memory state를 읽는다. 등급: `verified`.
- [Web message paging](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/app/src/context/server-session.ts#L599-L715)은
  200개 message batch로 이전 history를 불러온다. 등급: `verified`.

<a id="session-continuity-forks-and-background-work"></a>
### 세션 연속성, 포크, 백그라운드 작업

- [비동기 prompt handler](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/server/routes/instance/httpapi/handlers/session.ts#L311-L328)는
  prompt 실행을 server scope로 fork한다. 등급: `verified`.
- [Attach 명령](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/attach.ts#L7-L61)과
  [attach 시작](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/attach.ts#L107-L146)은
  별도로 실행 중인 server에 연결하고 session/fork를 선택할 수 있다. 등급: topology는 `verified`,
  종단 간 live detach/reattach 결과는 재현하지 않음.
- [Legacy run state](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/session/run-state.ts#L35-L68)는
  in-memory map이며 scope dispose 시 active run을 취소한다. 등급: `verified`.
- [Legacy session fork](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/session/session.ts#L693-L734)는
  명시적 fork-source lineage를 설정하지 않고 session을 만들어 message를 복사한다. 등급:
  `verified`.
- [Background-job registry](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/core/src/background-job.ts#L113-L124)는
  process-local, non-durable ownership을 명시적으로 설명한다. 등급: `verified` 소스 진술.
- [Task completion injection](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/tool/task.ts#L202-L229)은
  parent에 synthetic prompt를 추가한다. 등급: `verified`.
- [Legacy runner join 동작](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/effect/runner.ts#L115-L138)은
  새 작업이 들어올 때 독립적으로 schedule하지 않고 현재 run에 join한다. loop의 마지막 history
  read 뒤 삽입된 completion은 pending으로 남을 수 있다. 등급: 두 소스 경로에서 `inferred`,
  race는 재현하지 않음.
- [v2 runner 상태](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/core/src/session/runner/llm.ts#L43-L90)는
  미완성인 durable ownership, retry, cancellation, plugin parity, recovery를 명시한다. 등급:
  마운트된 코드와 명시된 한계는 `verified`, 성숙도: incomplete/experimental.
- [v2 coordinator](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/core/src/session/run-coordinator.ts#L5-L103)는
  process-local per-session serialization과 cross-session concurrency를 제공한다. 등급: 마운트된
  component는 `verified`, durable multi-node behavior는 구현되지 않은 상태.

<a id="web-desktop-ide-and-sharing"></a>
### 웹, 데스크톱, IDE, 공유

- [Hosted app 진입점](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/app/src/entry.tsx#L102-L181)은
  기본적으로 local server를 사용한다. 등급: `verified`.
- [Server credential persistence](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/app/src/context/server.tsx#L181-L303)는
  server connection data에 애플리케이션의 [local persistence helper](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/app/src/utils/persist.ts#L379-L458)를
  사용한다. 등급: `verified`, 소스 밖의 OS/browser storage protection은 평가하지 않음.
- [Embedded UI serving](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/server/shared/ui.ts#L7-L107)은
  bundled asset을 serve하거나 hosted app을 proxy한다. 등급: `verified`.
- [Desktop package](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/desktop/package.json#L12-L75)는
  Electron 42를 사용하고 [README](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/README.md#L67-L83)는
  desktop을 beta로 표시한다. 등급: `verified` 및 `documented`.
- [Desktop sidecar](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/desktop/src/main/sidecar.ts#L51-L89)는
  loopback과 생성된 Basic Auth를 사용하며 [desktop 시작 경로가 credential을 생성한다](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/desktop/src/main/index.ts#L307-L354).
  등급: `verified`.
- [Desktop window 보안](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/desktop/src/main/windows.ts#L164-L227)과
  [permission allowlist](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/desktop/src/main/windows.ts#L423-L457)는
  방어적인 renderer default를 설정한다. 등급: `verified`.
- [6개 대상 publish matrix](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/.github/workflows/publish.yml#L220-L400)는
  macOS, Windows, Linux x64/arm64를 다룬다. [Builder 설정](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/desktop/electron-builder.config.ts#L41-L146)은
  hardened runtime/notarization과 platform packaging을 설정한다. 등급: `verified`.
- [VS Code 확장](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/sdks/vscode/src/extension.ts#L8-L137)은
  CLI를 시작하고 file/selection context를 덧붙인다. 등급: `verified`.
- [VS Code test 설정](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/sdks/vscode/.vscode-test.mjs#L1-L5)은
  생성된 test를 찾지만 감사한 extension tree에는 test/spec 소스 파일이 없었다. 등급: 설정 및
  exact-SHA filename inventory는 `verified`.
- [Share upload 경로](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/share/share-next.ts#L274-L335)는
  session, message, part, diff, model data를 share service에 업로드한다. 등급: `verified`.
  Hosted access control, retention, server-side redaction은 검증하지 않음.
- [GitHub Action share 입력](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/github/action.yml#L16-L18)은
  public repository가 기본적으로 공유된다고 명시하며 [handler는 `share`가 설정되지 않고 repository가 public이면 이를 활성화한다](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/github.handler.ts#L511-L515).
  등급: `verified`, hosted access/retention policy는 `unverified`.
- [Nix desktop runtime](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/nix/desktop.nix#L1-L16)은
  Electron 41을 사용하고 package manifest는 Electron 42.3.3을 사용한다. [Nix evaluation job](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/.github/workflows/nix-eval.yml#L40-L77)은
  desktop evaluation을 optional/warning으로 취급한다. 등급: version drift는 `verified`, runtime
  impact는 `unverified`.

<a id="worktrees-github-and-delivery"></a>
### 워크트리, GitHub, 전달

- [Worktree 생성](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/worktree/index.ts#L174-L293)은
  `opencode/<slug>` workspace를 만들고 startup command를 실행할 수 있다. 등급: `verified`.
- [Worktree reset](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/worktree/index.ts#L525-L610)은
  non-primary worktree와 submodule을 hard-reset하고 clean한다. 등급: `verified`.
- [GitHub event 선택](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/github.handler.ts#L376-L427)은
  issue, pull request, review comment, schedule, dispatch를 다룬다. 등급: `verified`.
- [GitHub 전달](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/github.handler.ts#L519-L630)은
  stage, commit, push하고 PR을 생성/갱신한다. 등급: `verified`.
- [생성된 workflow](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/github.handler.ts#L329-L367)는
  `anomalyco/opencode/github@latest`를 사용한다. 등급: `verified` mutable dependency.
- [Git config credential injection 및 restore](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/github.handler.ts#L1013-L1038)는
  extra header를 기록하고 이전 header가 없으면 일찍 return한다. 등급: `verified`.
- [GitHub 실행 순서](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/github.handler.ts#L475-L531)는
  actor authorization decision 전에 token/configuration setup과 attachment 작업을 수행한다.
  등급: `verified`.
- 검사한 capability에서는 agent 또는 prompt가 그 active token을 exfiltrate할 수 있다. 등급:
  `inferred`, 이 감사에서는 실제 token을 노출하거나 exfiltrate하지 않음.
- 검사한 handler에서 Akra와 견줄 reviewed-check/integration/cleanup authority를 찾지 못했다.
  등급: 고정된 구현과 repository search에서 `inferred`, 외부 GitHub policy에 관한 주장은 아님.

<a id="authentication-secrets-permissions-and-extensions"></a>
### 인증, 시크릿, 권한, 확장

- [Server auth 설정](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/server/auth.ts#L17-L47)은
  optional password-variable-driven Basic Auth를 제공한다. 등급: `verified`.
- [Authorization middleware](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/server/routes/instance/httpapi/middleware/authorization.ts#L101-L149)는
  인증이 필요하지 않으면 요청을 통과시킨다. 등급: `verified`.
- [Credential 추출](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/server/routes/instance/httpapi/middleware/authorization.ts#L73-L82)은
  Basic Auth 또는 `auth_token` query parameter를 허용한다. 등급: `verified`, URL history/proxy
  log 노출은 `inferred`.
- [Serve 명령](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/cmd/serve.ts#L13-L23)은
  경고하지만 password 없이 시작한다. 등급: `verified` 및 동적 재현.
- [Network option](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/network.ts#L7-L20)은
  기본적으로 loopback을 사용하고, [mDNS/config resolution](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/cli/network.ts#L62-L79)은
  `0.0.0.0`을 선택할 수 있다. 등급: `verified`.
- [Config handler](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/server/routes/instance/httpapi/handlers/config.ts#L9-L29)는
  effective config를 반환하고 [variable substitution](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/config/variable.ts#L33-L90)은
  environment 및 file reference를 해석한다. 등급: `verified` 및 동적 재현.
- [Provider config schema](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/core/src/v1/config/provider.ts#L76-L120)는
  `options.apiKey`를 포함하고 [MCP config](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/core/src/v1/config/mcp.ts#L6-L59)는
  local environment, remote header, OAuth client secret을 포함한다. 등급: `verified`.
- [Provider public mapping](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/provider/provider.ts#L1046-L1085)은
  legacy `key`를 유지하고 [provider handler](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/server/routes/instance/httpapi/handlers/provider.ts#L40-L58)는
  그 mapping을 반환한다. 등급: environment canary에 대해 `verified` 및 동적 재현.
- [마운트된 v2 provider endpoint](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/protocol/src/groups/provider.ts#L7-L31)는
  [catalog provider object](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/server/src/handlers/provider.ts#L8-L29)를
  반환하며, 그 [schema는 request header와 body를 유지한다](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/schema/src/provider.ts#L46-L61).
  [v1 provider lowering](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/core/src/v1/config/provider-options.ts#L29-L114)은
  API key 또는 auth token을 `Authorization`, `x-api-key`, `api-key` header에 넣을 수 있다.
  등급: 소스 경로는 `verified`, 이 v2 응답을 통한 configured-secret disclosure는 `unverified`.
- [기본 agent permission](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/agent/agent.ts#L119-L136)은
  wildcard allow를 설정하고 `.env` read는 묻는다. 등급: `verified`.
- [Shell file parsing 및 check](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/tool/shell.ts#L263-L290)와
  [permission request 경로](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/tool/shell.ts#L378-L425)는
  workspace 내 `cat .env`에 direct-read permission을 적용하지 않는다. 등급: 소스 구성은
  `verified`, prompt bypass 영향은 `inferred`.
- [Plugin input](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/plugin/index.ts#L139-L177)은
  client, project/worktree, shell capability를 노출한다. 등급: `verified` trusted-code boundary.
- [Server plugin API](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/plugin/src/index.ts#L222-L334)는
  공급자/인증, 채팅 매개변수/헤더, 도구, 권한, 셸 환경, 도구 실행, 압축 훅을 노출하고
  [TUI plugin API](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/plugin/src/tui.ts#L53-L120)는
  터미널 동작을 확장한다. 등급: 기능 범위와 신뢰 코드 노출면은 `verified`.
- [Local MCP process](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/mcp/index.ts#L340-L369)는
  `process.env`를 상속한다. 등급: `verified`.
- [Legacy provider auth storage](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/auth/index.ts#L58-L89)와
  [MCP OAuth storage](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/opencode/src/mcp/auth.ts#L59-L101)는
  mode-`0600` plaintext JSON을 사용한다. [v2 credential table](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/core/src/credential/sql.ts#L5-L13)은
  credential value를 SQLite에 JSON으로 저장한다. 등급: `verified`, 이 경로 밖에서 제공되는
  filesystem/database encryption은 `unverified`.

<a id="performance-and-quality"></a>
### 성능과 품질

- [Performance suite README](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/app/e2e/performance/README.md#L1-L55)는
  manual renderer metric, machine dependence, packaged-Electron 한계를 정의한다. 등급:
  `documented`.
- [Performance Playwright config](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/app/e2e/performance/playwright.config.ts#L1-L20)는
  production build와 하나의 worker를 사용한다. 등급: `verified`.
- [Unit 및 generated-client CI](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/.github/workflows/test.yml#L23-L80)는
  Linux와 Windows test 및 Linux 전용 generated API check를 실행한다. 등급: `verified`.
- [E2E CI](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/.github/workflows/test.yml#L82-L151)는
  retry artifact와 함께 Linux 및 Windows에서 Chromium을 실행한다. 등급: `verified`.
- [Arabic parity suite](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/app/src/i18n/parity.test.ts#L1-L85)는
  `CI`에서 skip하며 3개 key mismatch를 로컬에서 재현했다. 등급: `verified`.
- [Desktop package script](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/packages/desktop/package.json#L12-L24)는
  직접 test가 통과했음에도 `test`를 생략한다. 등급: `verified`.
- [Publish dependency](https://github.com/anomalyco/opencode/blob/b1fc8113948b518835c2a39ece49553cffe9b30c/.github/workflows/publish.yml#L408-L521)는
  test/typecheck job에 구조적으로 의존하지 않는다. 등급: `verified`, branch protection과 외부
  release policy는 `unverified`.

<a id="akra-baseline-evidence"></a>
## Akra 기준선 증거

- [직접 app-server 연결](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/src/adapter/outbound/app_server/connection.rs#L968-L1066)은
  공식 stdio server를 초기화한다. 등급: `verified`.
- [필터링된 app-server 환경](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/src/adapter/outbound/app_server/connection.rs#L443-L520)과
  [spawn environment policy](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/src/adapter/outbound/app_server/connection.rs#L725-L773)는
  기본적으로 exact API-key opt-in과 함께 child environment를 지우고 allowlist한다.
  [설정 resolver](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/src/adapter/outbound/app_server/connection.rs#L377-L405)는
  exact `AKRA_APP_SERVER_PROCESS_ENVIRONMENT=all`을 명시적인 full-environment override로
  허용한다. 등급: `verified`, default와 override가 하나의 safety claim을 공유해서는 안 됨.
- [Approval domain](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/src/domain/conversation.rs#L233-L269)은
  제한된 request fact를 보존하지만 accept/decline만 노출한다. 등급: `verified`.
- [Fail-closed server request](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/src/adapter/outbound/app_server/connection.rs#L1817-L1884)는
  검사할 수 없는 file-change approval, user input, MCP elicitation을 decline한다. 등급:
  `verified`.
- [Admin server bootstrap](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/src/adapter/inbound/admin_api/mod.rs#L75-L125)은
  loopback에 bind하고 noninteractive일 때 explicit token을 요구한다. 등급: `verified`.
- [Admin security](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/src/adapter/inbound/admin_api/security.rs#L17-L115)는
  64-character capability, random `.localhost` origin, digest comparison, 제한된 session lifetime을
  사용한다. 등급: `verified`.
- [TUI command registry](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/src/adapter/inbound/tui/app/inline_shell_commands.rs#L38-L92)는
  이미 typed 및 palette-accepted command의 source다. 등급: `verified`.
- [공식 app-server schema fork](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/schema/codex_app_server_protocol.v2.schemas.json#L1320-L1360)와
  [fork lineage field](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/schema/codex_app_server_protocol.v2.schemas.json#L16630-L16655)는
  존재하지만 Akra request/port 경로는 찾지 못했다. 등급: schema는 `verified`, repository-wide
  implementation absence는 `inferred`.
- [공식 thread projection](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/schema/codex_app_server_protocol.v2.schemas.json#L16734-L16749)은
  read/resume/fork를 위해 turn을 채울 수 있지만 [schema는 저장된 ThreadItem을 명시적으로 lossy라고 설명한다](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/schema/codex_app_server_protocol.v2.schemas.json#L19581-L19597).
  command execution을 포함한 모든 agent interaction이 영속되는 것은 아니기 때문이다. 등급:
  `verified`, recovery는 protocol-guaranteed field와 event-only history를 구분해야 함.
- [현재 병렬 전달 계약](https://github.com/RefinedStone/codex-exec-loop/blob/354f4782a73ffabab6abf342cb0f37285a96c177/docs/supersession/current-contract.md#L100-L164)은
  고정된 소스/대상 식별 정보, 기본 검토/검사 관문, 직렬화된 통합, 원격 검증, 정리를 기록한다.
  등급: 참조 구현이 뒷받침하는 `verified` 계약.
- [기존 jcode 단위](../jcode/gap-matrix.md#implementation-slices)는 성능, 실시간 실행, 조정/재시작,
  검증, 전달 결과, Admin 지표/제어, 병렬 활동, 검토 응답, 세션 출처를 소유한다. 등급:
  `documented` Akra 계획.
- [기존 Canvas 단위](../agent-canvas/gap-matrix.md#implementation-slices)는 현재 활동, 사실에 충실한
  디오라마, 자동화 유입/실행 출처, 읽기 전용 원격 노드를 소유한다. 등급: `documented` Akra 계획.

## 감사 한계

- 인증된 공급자/모델 추론을 실행하지 않았으므로 실제 LLM의 모델 품질, 도구 성공률, 토큰 지연,
  권한 동작은 검증되지 않았다.
- 공정한 대화형 Akra/OpenCode 벤치마크를 실행하지 않았다. 시작, 입력, 첫 토큰, 정상 상태 메모리,
  CPU, 렌더러 지연, 장기 세션 안정성에는 승자가 없다.
- 시크릿 카나리는 가짜 값과 루프백을 사용했다. 실제 자격 증명은 노출하지 않았다.
- 동적 시크릿 재현은 설정으로 제공한 `openai.options.apiKey`, 비활성 로컬 MCP `environment` 값,
  레거시 `/config`, 레거시 `/provider`만 다뤘다. `{file:...}`, 인증 파일 키, 원격 MCP 헤더/OAuth,
  실제 MCP 실행, v2 설정 시크릿, GitHub 토큰은 동적으로 테스트하지 않았다.
- 내장 TLS 종단 처리를 평가하지 않았다. 비루프백 HTTP 배포에는 별도로 운영되는 기밀성 경계가
  필요하며 그 정확성은 검증되지 않았다.
- `.env` 셸 우회, GitHub 토큰 유출, SSE 넘침/손실, 백그라운드 주입 경합, 포크/되돌리기 동시성,
  악성 플러그인/MCP/LSP, Windows 프로세스 트리 취소는 동적으로 악용하지 않았다.
- 호스팅 공유의 접근 제어, 보존, 삭제, 정제, 기업 정책은 평가하지 않았다.
- GitHub 브랜치 보호, 비공개 조직 정책, 마켓플레이스 검사, 외부 릴리스 승인은 저장소에서 보이지
  않는 관문을 추가할 수 있다.
- 실험적 v2 패키지에 코드가 있다는 사실은 출시된 기본 제품이 이를 사용함을 입증하지 않는다.
- 깨끗한 개별 패키지 테스트 결과가 모든 패키지 또는 패키징된 데스크톱 경로를 릴리스 작업 흐름에서
  다룬다는 뜻은 아니다.
