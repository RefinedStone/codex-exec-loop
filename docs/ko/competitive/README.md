# 경쟁 앱 조사

[영문 원문](../../competitive/README.md)

상태: 2026-08-08 최신 스냅샷

Akra 기준: `0c94f7e8b4549c2358f8b6d4b0c1ebc7e206dd48` (`prerelease`)

빠르게 변하는 경쟁 제품 문서를 영문·한글로 통째로 복제하면 두 버전이 곧 어긋난다. 따라서 제품별
근거와 버전은 영문 원문 한 벌만 유지하고, 이 문서는 한국어 탐색과 핵심 결론만 제공한다. 이전 상세
번역과 일회성 리포트는 Git 이력에서 확인할 수 있다.

## Akra의 위치

> Akra는 공식 `codex app-server` 세션을 지속 가능하고 검사 가능하며 리뷰를 거쳐 전달되는 작업으로
> 바꾸는 Codex-first 운영·전달 계층이다.

경쟁 제품의 provider 수, tool 수, agent 수를 그대로 따라가는 것이 목표가 아니다. Akra의 핵심은 공식
프로토콜 충실도, planning/worktree 권위, review·merge·cleanup 증거, 그리고 TUI/Admin/CLI/Telegram이
공유하는 하나의 application truth다.

## 현재 조사

| 제품 | 현재 기준 | 핵심 위협 |
| --- | --- | --- |
| [OpenAI Codex](../../competitive/upstream-codex/README.md) | v0.147.0 | upstream runtime·protocol의 빠른 변화 |
| [Senpi + OmO Native](../../competitive/omo-native/README.md) | Senpi v2026.8.7, 설치된 OmO dev adapter | 캐시 친화성, 토큰 관측, 장기 child session |
| [jcode](../../competitive/jcode/README.md) | v0.68.0 | native multi-session harness, desktop, 성능 계측 |
| [OpenCode](../../competitive/opencode/README.md) | v1.18.15 | TUI/server/desktop 연속성과 넓은 생태계 |
| [Orca](../../competitive/orca/README.md) | v1.4.176 | worktree 중심 desktop fleet와 복구 UX |
| [Agent Canvas](../../competitive/agent-canvas/README.md) | v1.6.1 | browser/desktop inspector와 automation |
| [Grok Build](../../competitive/grok-build/README.md) | public HEAD `afbc0fb7...` | Rust full-stack harness, TUI, fast worktree |

조사 등급과 갱신 기준은 [방법론](../../competitive/methodology.md), 캐시 수식과 로컬 측정은
[캐시·토큰 효율](../../competitive/cache-and-token-efficiency.md)에 있다.

## 캐시·토큰 결론

Windows에 설치된 이름은 **Senpi**이며 “Senpai”가 아니다. “OmO Native”는 Senpi 안에서 실행되는
`@code-yeongyu/omo-senpi` adapter다.

2026-08-08의 개인정보 제거 rolling snapshot은 6개 session 파일, 1,979개 assistant request에서
provider가 보고한 cache-read 비율 **75.07%**를 보였다. 이는 긴 prefix가 실제로 재사용됐다는 강한
근거지만 다음을 뜻하지 않는다.

- context 사용량이 75% 줄었다.
- 전체 token이나 비용이 75% 줄었다.
- Akra보다 75% 효율적이다.
- `cacheWrite = 0`이므로 cache write가 없었다.

Senpi는 provider 요청을 직접 소유하므로 `prompt_cache_key`, affinity header, WebSocket
continuation을 제어한다. Akra는 공식 app-server client이므로 이를 복제하면 안 된다. Akra가 해야 할
일은 `cachedInputTokens / inputTokens`를 정확히 보여 주고, thread/app-server 연속성을 지키며,
중복 문맥·polling turn·큰 tool/child 결과를 줄이고 upstream compaction을 따르는 것이다.

## 현재 결정

### 도입

- cache read, context pressure, compaction, model change, reconnect/restart를 분리해 관측한다.
- tool·child·planning handoff를 bounded하게 유지한다.
- reconnect/resume/terminal recovery와 long-session 성능을 측정한다.
- worktree fleet와 review/delivery 상태를 한 운영 화면에서 읽게 한다.

### 거부

- provider/auth/cache key/private header/TTL 재구현
- cache hit를 token 절감이나 비용 절감으로 바로 환산
- 이득이 검증되지 않은 cache-warming turn과 model-visible polling
- 넓은 harness 생태계를 review-delivery 권위의 대체물로 취급

### 차별화

- 공식 Codex 의미를 가장 빠르고 정확하게 투영한다.
- planning lease와 exact-source worktree를 review·merge·cleanup까지 연결한다.
- 고위험 bypass가 사용되면 생략된 gate와 정책 근거를 숨기지 않는다.

## 유지 규칙

- 제품별 현재 스냅샷은 영문 원문에서 제자리 갱신하고 과거본은 Git 이력을 사용한다.
- 한국어 인덱스는 결론이나 탐색 경로가 달라질 때 함께 갱신한다.
- 경쟁 조사에서 나온 아이디어는 자동으로 현재 구현 계약이 되지 않는다. 채택된 미래 작업은
  명시적으로 proposed인 plan 또는 issue로 옮긴다.
