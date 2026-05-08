# Inbound Surface Unification Architecture

## 목적

이 문서는 `DOC-INBOUND-00`의 산출물이다. 목표는 TUI, CLI, admin API, Telegram bot을
서로 다른 bounded context처럼 키우지 않고, 같은 application command/use case를 호출하는
inbound adapter로 정렬하는 것이다.

상위 기준은 다음 문서다.

- [repository-wide-rebuild-architecture.md](./repository-wide-rebuild-architecture.md)
- [planning-control-plane-architecture.md](./planning-control-plane-architecture.md)
- [parallel-control-plane-architecture.md](./parallel-control-plane-architecture.md)
- [tui-application-boundary-architecture.md](./tui-application-boundary-architecture.md)

이 문서는 새 거대 facade를 만들라는 지시가 아니다. Akra의 bounded context는
`planning`, `parallel_mode`, `conversation`, `session_browser`, `startup`,
`github_review` 같은 제품 기능 단위다. `tui`, `cli`, `admin_api`, `telegram_bot`은
surface일 뿐이며, surface별 차이는 request parsing, auth/session/context mapping,
response rendering에만 둔다.

## 문제 정의

현재 repo는 이미 application service를 공유하는 방향으로 상당히 정리되어 있다.

- CLI `status`/`queue`와 Telegram `/status`/`queue`는 `PlanningControlCommand`와
  `PlanningControlService`를 공유한다.
- admin HTML/JSON route는 같은 `PlanningAdminFacadeService`를 공유한다.
- TUI planning controller는 planning service result를 footer/status/panel copy로 낮춘다.
- parallel TUI/admin/CLI는 모두 `ParallelModeService`와 control-plane projection을 본다.

남은 위험은 표면이 많아질수록 작은 policy가 다시 adapter에 생기는 것이다.

- CLI가 queue head를 다시 계산한다.
- Telegram parser가 planning reset 의미를 자체 enum으로 키운다.
- admin JSON과 HTML route가 같은 mutation을 다른 validation path로 보낸다.
- TUI controller가 application projection을 읽은 뒤 dispatch 가능 여부를 재판단한다.
- response copy를 만들기 위해 application read model 대신 repository 내부 구조를 직접 읽는다.

`INBOUND-00` 이후의 구현은 이 위험을 막기 위해 surface별 entrypoint를 같은 command
언어와 projection 언어로 묶어야 한다.

## 핵심 원칙

### 1. Surface는 Bounded Context가 아니다

다음 디렉터리는 adapter boundary다.

```text
src/adapter/inbound/tui
src/adapter/inbound/cli.rs
src/adapter/inbound/admin_api
src/adapter/inbound/telegram_bot
```

여기에는 transport, input grammar, operator context, local UI state, rendering만 둔다.
planning/parallel/conversation 정책은 context별 application service와 domain decision에 둔다.

### 2. Application Command는 Context별로 둔다

모든 surface를 하나의 `AkraCommand` enum으로 합치지 않는다. 그렇게 하면 거대한
cross-context dispatcher가 생긴다. 대신 bounded context별 command surface를 공유한다.

| Context | 공유 command/use case 예시 | Surface가 하는 일 |
| --- | --- | --- |
| planning control | `PlanningControlCommand`, `PlanningControlService` | CLI/TG/TUI command spelling을 command enum으로 mapping |
| planning admin/authoring | `PlanningAdminFacadeService`, draft/task mutation request DTO | HTML form, JSON body, TUI overlay buffer를 typed request로 mapping |
| planning runtime | `PlanningServices::runtime`, task intake/manual intake | prompt 또는 tool input을 runtime request로 mapping |
| parallel mode | `ParallelModeService`, control-plane runtime command/event | shell/admin/CLI wake, tick, snapshot request를 application event로 mapping |
| conversation | `ConversationRuntimeEvent`, `ConversationLifecycleEvent` | keyboard/app-server stream fact를 reducer event로 mapping |
| session/startup/github review | context별 service/projection | route, key, poller result를 service request 또는 projection refresh로 mapping |

### 3. Auth와 Session은 Adapter Context다

권한과 session/context mapping은 surface마다 다르다.

| Surface | Context source | Adapter 책임 | Application에 넘기는 형태 |
| --- | --- | --- | --- |
| TUI | local operator, current workspace, active app-server thread | key focus, overlay state, current thread/workspace 선택 | typed command/event와 workspace/thread id |
| CLI | process argv, cwd 또는 explicit workspace path | argument arity, path canonicalization, stdout/stderr/exit code | typed command와 workspace path |
| Admin API | loopback HTTP, cookie/CSRF, route path/body/query | CSRF 검증, form/JSON DTO parsing, redirect/JSON status | typed request DTO와 workspace context |
| Telegram | Bot API update, allowlisted chat id, poll cursor | token/config, chat allowlist, update offset, message parsing | typed planning command와 operator chat context |

auth/session 자체는 domain invariant가 아니다. 다만 application command에는 누가 어떤
workspace/thread/context를 대상으로 요청했는지 식별 가능한 typed context를 전달해야 한다.

### 4. Response는 Projection 후 Surface Rendering이다

Application은 UI-neutral projection/result를 만든다. Surface는 같은 projection을 각자의
format으로 낮춘다.

```text
application result/projection
  -> TUI: footer, panel, overlay, transcript copy
  -> CLI: stdout lines, JSON line, exit code
  -> Admin HTML: page, fragment, redirect notice
  -> Admin JSON: serde DTO, HTTP status
  -> Telegram: short chat reply
```

예외적으로 `PlanningControlService`처럼 compact operator command가 CLI와 Telegram에서
동일한 text reply를 공유할 수 있다. 하지만 이 text reply도 application read model에서
나와야 하며, surface가 queue/proposal/domain policy를 다시 계산해서 문장을 만들면 안 된다.
rich surface가 필요하면 text reply를 확장하지 말고 projection DTO와 surface renderer를
분리한다.

## Surface별 책임

### TUI

TUI는 가장 큰 inbound surface지만 business policy owner가 아니다.

소유:

- keyboard/shell/overlay event parsing
- UI-only state: focus, cursor, selection, editor buffer, overlay step
- reducer event로 낮출 수 있는 conversation/runtime fact mapping
- application projection cache 표시
- background message를 reducer 또는 application event로 routing

비소유:

- planning queue head 산출
- parallel dispatch 가능 여부 판단
- task mutation legality
- hidden worker retry 판단
- durable authority write

### CLI

CLI는 scriptable local operator surface다.

소유:

- argv grammar와 usage text
- cwd 또는 explicit workspace path canonicalization
- stdout/JSON line/exit code rendering
- short-lived composition root wiring

비소유:

- queue/proposal projection 재계산
- admin HTML/JSON DTO 재사용 강제
- Telegram allowlist나 TUI focus state
- durable store 직접 mutation

CLI command가 long-running runtime을 시작할 때도 CLI는 driver일 뿐이다. 예를 들어
`parallel-tick`은 같은 parallel distributor queue를 수동/cron 환경에서 tick하는 진입점이고,
dispatch policy는 `ParallelModeService`와 domain decision에 남는다.

### Admin API

Admin API는 local HTTP surface다. HTML page와 JSON API는 transport contract만 다르고
같은 facade를 사용해야 한다.

소유:

- loopback HTTP server와 route table
- cookie/CSRF boundary
- form body, JSON body, query/path parsing
- HTML page, HTMX fragment, JSON DTO rendering
- admin graphic/dashboard view 조립에 필요한 surface copy

비소유:

- direction/task/draft mutation rule
- validation severity 산출
- workspace file policy
- parallel readiness/dispatch policy

HTML route와 JSON route가 같은 operation을 노출할 때는 같은 request mapping helper나
같은 application request DTO로 내려가야 한다. response shape은 달라도 mutation path는
갈라지면 안 된다.

### Telegram Bot

Telegram bot은 remote chat transport다.

소유:

- Bot API polling, update offset, retry/backoff
- token/config loading
- chat allowlist authorization
- Telegram command spelling과 usage error
- short chat reply rendering

비소유:

- planning command 실행 policy
- reset target 의미
- workspace authority mutation
- queue/proposal status 계산

`/whoami`처럼 transport 전용 command는 Telegram adapter에 남길 수 있다. planning을 건드리는
command는 `PlanningControlCommand` 같은 shared application command로 낮춘다.

## Request Mapping 규칙

Inbound mapping은 다음 순서로 진행한다.

```text
raw input
  -> surface grammar validation
  -> auth/session/context mapping
  -> typed application command/query/event
  -> application service/runtime
```

surface grammar validation은 안전을 위해 좁아야 한다. 예를 들어 reset command는
`queue`, `directions`, `all` 같은 surface spelling을 adapter에서 `PlanningResetTarget`으로
바꾸고, 여분 인자는 application까지 내려보내지 않는다.

typed command를 만들 때 지켜야 할 규칙:

- free-form string을 application mutation command로 그대로 넘기지 않는다.
- surface spelling과 domain/application enum을 같은 타입으로 합치지 않는다.
- workspace path, thread id, chat id, CSRF/session proof는 command context로 분리한다.
- destructive action은 preview/confirm/idempotency requirement를 command contract에 드러낸다.
- background/effect completion은 surface callback이 아니라 application event로 되돌린다.

## Response Rendering 규칙

Response rendering은 surface별로 다르지만 원천은 같아야 한다.

| Response kind | Application 원천 | Surface renderer |
| --- | --- | --- |
| operator status | Application Projection 또는 control snapshot | TUI footer, CLI text, Telegram text |
| queue/task overview | planning projection | TUI queue overlay, admin cards/table, CLI queue text |
| mutation result | application result DTO와 refreshed projection | admin redirect/JSON, TUI status, CLI exit code |
| runtime event feed | application/runtime event log projection | admin event feed, TUI notice, CLI diagnostic |
| validation issue | domain/application validation report | admin field error, TUI editor status, CLI/TG compact error |

Renderer는 copy, truncation, ordering display, icon/label, HTTP status, exit code를 정한다.
Renderer가 domain rule을 재판단하면 안 된다.

## 현재 Surface Inventory

| Surface | 현재 공유 boundary | 유지할 점 | 후속 정리 후보 |
| --- | --- | --- | --- |
| TUI planning shell/overlay | `PlanningServices`, planning controllers, status projection, `PlanningResetTarget` | UI-only overlay/editor state와 application service 호출 분리, `:reset` execution/hint path는 shared parser로 mapping | 남은 planning command request DTO를 admin/CLI와 같은 이름으로 정렬 |
| TUI conversation/automation | `ConversationRuntimeEvent`, post-turn automation router | reducer/effect vocabulary와 background message routing 분리 | remaining runtime bridge correlation을 application runtime store로 이동 |
| TUI parallel panel | `ParallelModeService`, domain state machine, panel controller | projection 표시와 application wake request 분리 | shell command entry와 admin/CLI tick command vocabulary 정렬 |
| CLI status/queue/reset | `PlanningControlCommand`, `PlanningControlService` | compact command surface 공유 | reset은 workspace reset path와 control reset path의 response shape 정렬 검토 |
| CLI planning-tool | `PlanningTaskToolRequest`, `PlanningTaskToolResponse` | stdin JSON contract와 JSON line response | tool command도 command context와 error taxonomy를 공유 |
| CLI parallel-tick | `ParallelModeService::run_orchestrator_tick(..., ManualDispatch)` | manual/cron driver로 제한하고 application tick result를 렌더링 | 남은 TUI/admin parallel command vocabulary와 정렬 |
| Admin HTML/JSON | `PlanningAdminFacadeService`, `PlanningAdmin*Request` DTO | HTML/JSON route가 같은 facade를 공유 | duplicate form/JSON mapping helper를 request mapper로 모으기 |
| Admin Akra dashboard | `PlanningAdminFacadeService`, `ParallelModeService` projection | read-only dashboard projection | dashboard view가 domain type을 직접 읽는 곳은 application projection으로 낮추기 |
| Telegram bot | `PlanningControlCommand`, `PlanningControlService` | chat parser와 allowlist를 adapter에 유지 | shared compact response renderer 또는 typed reply DTO 검토 |

## 금지 패턴

### Surface별 Policy Fork

```text
CLI queue command calculates next task
Telegram queue command calculates next task differently
TUI queue overlay uses another rule
```

queue ordering은 planning domain/application projection의 결과여야 한다.

### Transport DTO를 Application Model로 승격

```text
Admin form field names become application command fields
Telegram slash command string becomes domain enum
```

transport spelling은 adapter에 남긴다. application command는 context-neutral typed request여야 한다.

### Shared Renderer로 Policy를 숨김

```text
shared text formatter loads repository and decides repair eligibility
```

shared renderer는 projection/result를 받아 copy만 만든다. load, decision, mutation은
application/domain으로 간다.

### Surface Runtime Loop 중복

```text
TUI background thread mutates state
Telegram poll loop retries planning mutation internally
Admin handler compensates durable state directly
```

long-running loop나 background worker는 effect completion을 application event로 되돌린다.

## INBOUND-00 구현 기준

`INBOUND-00`은 production code를 한 번에 대규모 변경하지 않는다. 다음 순서로 작게 나눈다.

1. Surface command inventory와 regression을 먼저 고정한다. 완료 문서는
   [../plan/inbound-surface-command-inventory.md](../plan/inbound-surface-command-inventory.md)이다.
2. planning control command의 workspace/context/request mapping을 CLI와 Telegram에서 같은
   contract로 정렬한다. `PlanningControlRequest`/`PlanningControlResponse`가 shared
   request/result vocabulary다.
3. admin HTML/JSON의 같은 mutation이 같은 request DTO와 facade method를 통과하는지 고정한다.
   reset, draft, direction/task CRUD route pair regression이 이 기준을 보호한다.
4. TUI planning shell command와 admin/CLI reset/status/queue vocabulary의 차이를 문서화한다.
   `:reset` target은 shared `PlanningResetTarget`으로 고정했고 execution path와 buffered hint
   path는 같은 parser를 쓴다. 남은 `:planning`, `:task`, editor/overlay command는 공통
   application command로 내릴 수 있는 부분부터 이동한다.
5. parallel TUI/admin/CLI entrypoint가 control-plane runtime command/event를 공유하도록
   tick/wake/snapshot vocabulary를 정리한다.

각 implementation slice는 하나의 surface pair 또는 하나의 context command만 다룬다.
TUI, admin API, Telegram, CLI를 한 PR에서 모두 수정하지 않는다.

## 검증 기준

- TUI, CLI, admin API, Telegram을 bounded context로 부르는 문서를 새로 추가하지 않는다.
- 새 inbound command는 context별 application service 또는 reducer event로 내려간다.
- adapter test는 parsing/auth/context/rendering을 검증하고, domain policy를 검증하지 않는다.
- application test는 command ordering, projection assembly, side effect orchestration을 검증한다.
- domain test는 I/O 없는 invariant와 pure decision을 검증한다.
- 문서 변경은 `git diff --check`와 surface/context 용어 검색으로 검증한다.
