# Core Runtime Boundary Architecture

## 문서 목적

이 문서는 현재 폴더 구조를 유지하면서 `src/core`를 새로 만든다면 어떤 책임을
가져야 하는지 정의한다. 핵심은 `core`가 `domain`을 대체하는 새 중심 계층이
아니라, TUI 안에 섞여 있는 app orchestration state를 UI 없는 runtime으로 빼는
선택지라는 점이다.

Spring Boot 경험으로 비유하면 다음에 가깝다.

```text
src/main.rs, src/lib.rs
  ~= SpringBootApplication main/bootstrap

src/composition
  ~= @Configuration / bean wiring

src/adapter/inbound/*
  ~= Controller / CLI handler / TUI handler

src/application/service
  ~= @Service / use case

src/application/port/outbound
  ~= Repository interface / external client interface

src/adapter/outbound/*
  ~= RepositoryImpl / external API client

src/domain
  ~= Entity / Value Object / domain rule

src/core
  ~= AppCoordinator / ApplicationRuntime bean
```

`core`는 Spring Boot의 `main` class가 아니다. 여러 application service를 묶어
앱 상태, command/event 흐름, background effect ordering을 관리하는 headless
coordinator다.

## 현재 구조에서의 문제

현재 hexagonal architecture의 큰 방향은 이미 잡혀 있다.

```text
adapter -> application -> domain
application -> outbound port -> outbound adapter
composition -> concrete wiring
```

문제는 `src/adapter/inbound/tui/app` 아래의 `NativeTuiApp`이 두 종류의 상태를
동시에 들고 있다는 점이다.

TUI에 남아도 되는 presentation state:

- overlay open/close
- cursor/focus
- prompt input buffer
- board/list selection
- viewport/scroll/redraw cadence
- exit confirmation
- last rendered projection cache

TUI 밖으로 옮길 수 있는 app orchestration state:

- startup lifecycle
- session catalog lifecycle
- conversation lifecycle
- turn stream orchestration
- post-turn automation state
- planning runtime snapshot
- parallel-mode control-plane status
- background effect ordering

이 두 상태가 한 struct 안에 있으면 TUI가 점점 `Controller + ViewModel + Runtime
Coordinator`를 모두 맡게 된다. 기능이 늘어날수록 admin API, CLI, Telegram,
automation surface가 같은 application state를 재사용하기 어려워진다.

## 목표 구조

`src/core`를 도입할 경우의 목표 구조는 다음과 같다.

```text
src/
  core/
    mod.rs
    app/
      command.rs
      event.rs
      snapshot.rs
      state.rs
      controller.rs
    runtime/
      driver.rs
      background.rs
      effect_runner.rs
      stream_reducer.rs

  domain/
    ...

  application/
    service/
    port/

  adapter/
    inbound/
      tui/
      cli/
      admin_api/
      telegram_bot/
    outbound/
      app_server/
      db/
      filesystem/
      git/
      github/
      telegram/

  composition/
    production.rs
```

의존성 방향은 아래처럼 고정한다.

```text
adapter/inbound/tui
  -> core
  -> application
  -> domain

adapter/inbound/cli/admin_api/telegram_bot
  -> core 또는 application

adapter/outbound/*
  -> application::port::outbound

composition
  -> core/application/outbound adapter concrete wiring
```

금지 방향:

```text
core -> adapter/inbound/tui
core -> ratatui / crossterm
core -> axum route type / Telegram update type
application -> core
domain -> core
domain -> adapter/application/framework
```

## Core의 책임

`core`는 UI 없는 app runtime이다.

해야 할 일:

- `AppCommand`를 받는다.
- application service를 호출한다.
- app-level state를 갱신한다.
- background completion queue를 다시 `CoreInput`으로 drain한다.
- background effect를 시작하거나 effect runner에 위임한다.
- streaming event를 app event로 줄인다.
- `AppSnapshot` 또는 `AppEvent`를 inbound adapter에 제공한다.
- 여러 inbound surface가 공유할 수 있는 app orchestration 흐름을 한곳에 둔다.

하지 말아야 할 일:

- Ratatui widget 만들기
- terminal key binding 해석
- HTTP request/response DTO 직접 다루기
- Telegram message parsing
- DB, git, filesystem 직접 호출
- domain invariant를 UI 편의 분기로 복제하기

## 핵심 타입

초기 후보 타입은 다음과 같다.

```rust
pub enum AppCommand {
    RunStartupChecks,
    LoadSessionCatalog { limit: usize },
    OpenConversation { thread_id: String },
    SubmitTurn { prompt: String },
    StopAllSessions,
}

pub enum AppEvent {
    StartupChanged(StartupSnapshot),
    SessionCatalogChanged(SessionCatalogSnapshot),
    ConversationChanged(ConversationSnapshot),
    TurnStreamUpdated(TurnStreamSnapshot),
    OperatorAlert(OperatorAlertSnapshot),
}

pub struct AppSnapshot {
    pub startup: StartupSnapshot,
    pub sessions: SessionCatalogSnapshot,
    pub conversation: ConversationSnapshot,
    pub planning: PlanningRuntimeSnapshot,
    pub parallel_mode: ParallelModeSnapshot,
}
```

이 타입들은 UI/transport 독립이어야 한다. 필요하면 serialization 가능한 DTO는 별도
protocol layer로 나중에 분리한다. 처음부터 HTTP/SSE 프로토콜을 목표로 삼지 않는다.

## Core Input Event의 종류

`core`가 처리하는 입력은 사용자 명령만이 아니다. TUI/Admin/CLI 같은 inbound
adapter에서 온 command, 외부 stream에서 온 event, application 내부 background
effect가 완료되며 생기는 completion event를 모두 같은 app runtime 입력으로 다룬다.

```text
CoreInput
  - Command(AppCommand)
  - ExternalEvent(...)
  - EffectCompleted(...)
```

### 1. User Command

사용자 또는 외부 surface가 의도를 보낸다.

```text
TUI key / CLI args / Admin HTTP / Telegram message
  -> inbound adapter
  -> AppCommand
  -> core.dispatch(command)
```

예시:

- `RunStartupChecks`
- `LoadSessionCatalog`
- `SubmitTurn`
- `EnableParallelMode`
- `StopAllSessions`

### 2. External Stream Event

외부 시스템이 stream/SSE/notification을 보낸다. 이때 outbound adapter가 raw event를
application/core가 이해할 수 있는 event로 낮춘다. outbound adapter가 TUI를 직접
호출하거나 core state를 직접 변경하면 안 된다.

```text
external SSE / app-server notification / worker stream
  -> outbound adapter
  -> application-level stream event
  -> core input event sink
  -> core state update
  -> AppEvent/AppSnapshot
  -> inbound TUI render
```

예시:

- assistant token delta
- app-server turn notification
- worker progress message
- worker stream failure
- turn completed notification

### 3. Internal Completion Event

application/core가 시작한 background effect도 나중에 완료 event로 core에 재진입해야
한다. 이 event는 외부 SSE가 아니지만 app state를 바꾸는 비동기 입력이다.

```text
AppCommand
  -> core state = Loading/InFlight
  -> application service/effect runner starts background work
  -> background work completes
  -> EffectCompleted event
  -> core state = Ready/Failed/Updated
  -> AppEvent/AppSnapshot
  -> inbound adapter render/response
```

예시:

- `StartupService::run_checks()` 완료
- session catalog load 완료
- conversation snapshot load 완료
- post-turn evaluation 완료
- planning repair/refresh 완료
- parallel supervisor refresh 완료
- parallel orchestrator wake/tick 완료
- parallel worker completion 처리

이 규칙의 목적은 “비동기 작업이 끝난 thread가 곧바로 TUI state를 수정하는” 경로를
없애는 것이다. completion은 반드시 core input event로 환원하고, core만 app state를
변경한다.

## StartupCheckRequested 기준 목표 흐름

현재 흐름:

```text
TUI shell_entrypoint
  -> NativeTuiApp.dispatch_shell_chrome(StartupCheckRequested)
  -> shell_chrome reducer
  -> RunStartupChecks effect
  -> NativeTuiApp thread spawn
  -> StartupService::run_checks()
  -> BackgroundMessage::StartupLoaded
  -> ShellRuntime dispatch
  -> shell_chrome reducer
  -> TUI state update
```

`core` 도입 후 목표 흐름:

```text
TUI shell_entrypoint
  -> core.dispatch(AppCommand::RunStartupChecks)

core
  -> AppState.startup = Loading
  -> StartupService::run_checks() effect 실행
  -> StartupDiagnostics 수신
  -> AppState.startup = Ready/Failed
  -> 필요하면 AppCommand::LoadSessionCatalog scheduling
  -> AppEvent::StartupChanged 발행

TUI
  -> AppEvent/AppSnapshot 수신
  -> startup overlay rendering
```

TUI는 startup check를 직접 thread spawn하지 않는다. TUI는 `RunStartupChecks`라는
의도를 보내고, `StartupSnapshot`을 받아 화면에 표시한다.
`StartupReadySnapshot`은 domain `StartupDiagnostics`를 그대로 보관하는 대신 cwd,
workspace, probe 결과, attachment label, warning, schema snapshot을 UI/transport
독립 projection으로 풀어 둔다. 그래서 TUI는 이후 domain diagnostics를 직접 들고 있지
않아도 기존 startup overlay 정보를 잃지 않는다.
마이그레이션 중간 단계에서는 TUI가 아직 startup effect를 직접 실행하더라도, reducer와
rendering state는 `StartupReadySnapshot`을 읽어 domain diagnostics 보관 책임을 core 쪽
계약으로 옮긴다.

## 상태 소유권 표

| 상태 | 현재 위치 | 목표 소유자 | 이유 |
| --- | --- | --- | --- |
| startup loading/ready/failed | TUI shell chrome | core app state | TUI 외 surface도 startup readiness를 공유할 수 있다. |
| session catalog lifecycle | TUI shell chrome | core app state | preload/reload 정책은 화면보다 app lifecycle에 가깝다. |
| conversation lifecycle | TUI conversation reducers | core app state | CLI/admin/automation도 현재 conversation 상태를 읽을 수 있다. |
| turn stream reduction | TUI runtime path | core runtime | app-server stream은 UI가 아니라 app runtime event다. |
| post-turn automation | TUI post-turn path | core runtime | turn completion 이후 정책은 화면과 독립적이다. |
| planning runtime snapshot | TUI app cache | core app state 또는 application projection | 여러 surface가 같은 planning 상태를 본다. |
| parallel-mode status | TUI app cache/application | application/core projection | control-plane 상태는 UI-only가 아니다. |
| overlay open/close | TUI | TUI | presentation state다. |
| prompt input buffer | TUI | TUI | terminal editing state다. |
| selection/cursor/scroll | TUI | TUI | render interaction state다. |
| redraw scheduler | TUI frontend/runtime | TUI | terminal frame policy다. |

## Hexagonal Architecture와의 관계

`core`를 만든다고 hexagonal architecture가 깨지는 것은 아니다. 오히려 inbound TUI
adapter가 더 얇아질 수 있다.

유지해야 할 원칙:

```text
domain은 순수 규칙이다.
application은 use case와 port 계약이다.
core는 headless runtime/coordinator다.
inbound adapter는 입력과 표시만 다룬다.
outbound adapter는 외부 시스템 mapping만 다룬다.
composition은 concrete wiring만 다룬다.
```

깨지는 경우:

- `core`가 `ratatui`/`crossterm`을 import한다.
- `application`이나 `domain`이 `AppCommand`를 알아야 한다.
- `domain`이 snapshot DTO를 UI 표시 기준으로 바꾼다.
- TUI-only state를 core/domain에 올린다.
- outbound adapter response DTO가 core/domain으로 직접 새어 들어온다.

## Migration Plan

### Phase 0. 이름과 금지선 고정

목표:

- `core`가 domain 대체 계층이 아니라는 점을 문서와 module comment로 고정한다.
- `core`에서 UI framework import를 금지한다.

작업:

- `src/core/mod.rs` skeleton 추가
- `AppCommand`, `AppEvent`, `AppSnapshot` 빈 shell 또는 최소 startup 타입 추가
- module-level comment로 dependency rule 기록

완료 조건:

- production behavior 변경 없음
- `cargo test` 통과

### Phase 1. Startup lifecycle 이동

목표:

- 가장 작은 vertical slice로 `StartupCheckRequested` 흐름을 core로 옮긴다.

작업:

- `AppCommand::RunStartupChecks`
- `AppState.startup`
- `AppEvent::StartupChanged`
- core effect runner에서 `StartupService::run_checks()` 호출
- TUI는 startup 상태를 core snapshot에서 읽어 render

완료 조건:

- 기존 startup reducer 테스트가 core test로 이전되거나 동등 테스트가 추가된다.
- TUI는 startup check thread를 직접 spawn하지 않는다.
- startup overlay rendering 결과가 유지된다.

### Phase 2. Session catalog lifecycle 이동

목표:

- startup 성공 후 session preload와 manual session reload를 core로 옮긴다.

작업:

- `AppCommand::LoadSessionCatalog`
- session page limit/workspace scope policy를 core에 둔다.
- `SessionService` 호출은 core effect runner에서 수행한다.
- TUI는 session overlay open/close와 selection만 가진다.

완료 조건:

- TUI가 session catalog load effect를 직접 실행하지 않는다.
- selection/cursor는 TUI에 남는다.
- session catalog projection은 core snapshot으로 제공된다.

### Phase 3. Conversation lifecycle 이동

목표:

- conversation snapshot load, new conversation draft, active session 상태를 core로 이동한다.

작업:

- `AppCommand::OpenConversation`
- `AppCommand::StartNewConversation`
- `AppEvent::ConversationChanged`
- `ConversationService` 호출 경로 core 이동

완료 조건:

- TUI는 active conversation projection을 읽어 render한다.
- prompt input buffer와 cursor는 TUI에 남는다.

### Phase 4. Turn stream orchestration 이동

목표:

- app-server stream event reduction을 core runtime으로 이동한다.

작업:

- `AppCommand::SubmitTurn`
- stream event reducer를 core로 이전
- `ConversationStreamEvent`를 core event로 줄인 뒤 snapshot 발행
- TUI는 stream snapshot을 화면에 그린다.

완료 조건:

- TUI가 app-server stream completion policy를 직접 판단하지 않는다.
- terminal redraw cadence는 계속 TUI가 소유한다.

### Phase 5. Parallel/planning projection 연결

목표:

- 이미 application single-writer gate로 정리 중인 parallel/planning 상태를 core snapshot에 연결한다.

작업:

- core snapshot에 planning/parallel projection field 추가
- TUI panel은 projection rendering과 UI-only selection만 담당
- application control-plane command/event와 core command/event의 경계 정리

완료 조건:

- parallel domain decision은 domain/application에 남는다.
- TUI panel controller는 presentation state만 가진다.

## Open Questions

- `core`를 같은 process module로 유지할지, 장기적으로 별도 process/server로 뺄지.
- `AppEvent`를 실시간 stream으로 유지할지, `AppSnapshot` polling/subscribe 혼합으로 갈지.
- admin API와 Telegram이 core를 직접 사용할지, 기존 application service를 계속 직접 사용할지.
- app-server stream event 타입을 core public contract로 노출할지, core 전용 snapshot으로 완전히 숨길지.
- protocol DTO를 지금 만들지, 별도 web/admin frontend가 필요해질 때까지 미룰지.

## Non-Goals

- 이번 설계는 `domain` 재배치를 목표로 하지 않는다.
- `application/service`를 없애지 않는다.
- TUI rendering을 web frontend처럼 HTTP client로 바꾸지 않는다.
- 처음부터 SSE/WebSocket server를 만들지 않는다.
- core에 DB/git/filesystem adapter를 직접 넣지 않는다.

## 첫 구현 Slice 제안

가장 작은 첫 slice는 startup이다.

```text
Slice: CORE-STARTUP-01
Goal: StartupCheckRequested flow를 core command/event/snapshot으로 이전한다.
Scope:
  - src/core skeleton
  - AppCommand::RunStartupChecks
  - AppState.startup
  - AppEvent::StartupChanged
  - core effect runner에서 StartupService 호출
  - TUI startup rendering은 기존 view 유지
Out of scope:
  - session catalog 이동
  - conversation stream 이동
  - protocol/server 분리
Verification:
  - cargo test startup
  - cargo test shell_entrypoint
  - cargo test shell_runtime
```

이 slice가 성공하면 `core`가 추상 논의가 아니라 실제로 TUI orchestration을 줄이는
방향인지 검증할 수 있다.
