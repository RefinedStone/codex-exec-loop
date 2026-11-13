# 18-K4-9b. 병렬 제어를 커널 명령으로 옮긴다

## 문서 상태

- 상태: **제안**. 구현하지 않았다. 현재 제품 동작의 기준은 `docs/reference/` 이다.
- 상위 문서: [K4 명령 스펙](18-k4-command-spec.md) §2.2, §3.7 의 순서 2, §4 의 K4-9b. 결함 K-D3, K-D5 는
  [18 Akra 커널 아키텍처](18-akra-kernel-architecture.md) §5.6 에 있다.
- 이 문서를 따로 둔 이유: K4 설계는 K4-9b 를 한 줄로 적었다. 2026-11-13 의 코드 조사에서 그 한 줄이 전환 네 개와
  제품 결정 세 개를 담고 있다는 것을 알았다. 그래서 구현 전에 조각과 결정을 여기에 적는다.
- 사용자가 정할 것은 §5 에 있다. 기본값을 적었고, 답이 없으면 기본값으로 간다. 답과 관계없는 조각(§4 의 9b-1,
  9b-2)은 먼저 구현한다.

## 1. 목표

| 번호 | 목표 | 확인하는 방법 |
| --- | --- | --- |
| P1 | 병렬 상태의 주인은 저장소마다 하나이다. 그 저장소의 커널이다 | Gateway 의 커널이 도는 저장소에서 admin 의 정지가 그 커널의 제어부에 닿는다 (K-D3 회귀 시험) |
| P2 | 입구는 스케줄러를 시작하지 않는다 | admin, Telegram, `parallel-tick` 의 조립이 PR 검증 스케줄러 스레드를 만들지 않는다 (K-D5) |
| P3 | 병렬 제어가 커널 명령이다. 기록에 남고 스펙에 있다 | `set_parallel_mode`, `dispatch_parallel`, `refresh_parallel`, `save_parallel_agent_profiles` 가 스펙에 있고 `command_received` 에 호출자가 남는다 |

## 2. 현재 상태 (2026-11-13 코드 조사)

### 2.1 입구

| 입구 | 하는 일 | 부르는 곳 | 사용자에게 보이는 것 |
| --- | --- | --- | --- |
| admin `/api/admin/akra/control` | `enable`, `dispatch`, `refresh`, `disable` | `ParallelModeAdminPort::execute_control` (`application/service/parallel_mode/admin.rs`) | 켜짐, 진행 중인 효과, 현재 epoch, 배분을 미룬 이유, 마지막 명령, 한국어 글 |
| admin `/admin/controls/agent-profiles` | 병렬 에이전트 프로필 저장 | `ParallelAgentProfilePort::save_config` | 알림과 함께 다시 보냄 |
| Telegram `/parallel`, `/parallel_status` | 읽기만 | `ParallelModeControlPort::load_status` | 한국어 글 |
| CLI `akra parallel-tick` | 배분 한 번. 병렬 모드를 켜지 않는다 | `run_manual_orchestrator_tick` (epoch 를 열고 tick 을 실행하고 끝까지 기다린다) | 알림 글 또는 `queue-idle`. 막히면 종료 코드 1 |
| TUI | 켜기, 끄기, 새로 고침, 배분 | 자기 런타임의 `NativeClientEvent::ParallelCommand` | 화면 |

- admin 의 `dispatch` 는 병렬 모드가 꺼져 있으면 배분하지 않고 켠다 (`EnabledInsteadOfDispatch`).
- TUI 의 켜기와 끄기는 다른 일도 함께 한다: 턴 뒤 이어 가기 값(`SetParallelPostTurnRearm`)을 바꾸고, 끌 때 배분을 미룬
  이유를 지운다. admin 은 하지 않는다.
- 프로필을 저장하는 입구는 admin 뿐이다. 파일은 `<작업 공간>/.akra/parallel-agent-profiles.json` 이다. 커널은 쓸 때마다
  그 파일을 다시 읽는다.

### 2.2 병렬 제어부

- 병렬 모드의 켜짐과 epoch 는 프로세스 메모리에만 있다 (`ParallelModeControlPlaneRuntimeStore`,
  `ParallelModeAutomationGuard`). 저장하지 않는다. 그래서 admin 의 정지는 admin 프로세스의 제어부만 바꾼다 (K-D3).
- 제어부를 조립하면 PR 검증 스케줄러 스레드가 시작한다 (`parallel_mode_control_plane_from_service` →
  `build_pr_validation_scheduler_runtime`). 기본 방식은 `observe` 이다: GitHub 의 검증 결과를 읽어 기록하고, planning 큐에
  작업을 넣지 않는다. admin 과 Telegram 은 프로세스가 사는 동안, `parallel-tick` 은 한 번 실행하는 동안 이 스레드를
  돌린다 (K-D5).

### 2.3 커널

- 커널의 런타임(`NativeClientRuntime`)은 제어부 하나를 가진다. 병렬 명령은 core 명령(`AppCommand`)이 아니다. 런타임의
  `dispatch_client_event` 가 받는다. 커널 호스트의 명령 길(`KernelHostInput::Command`, `dispatch_client_command`)은
  `AppCommand` 만 받는다.
- core 는 병렬 모드 값을 비춰 보기만 한다 (`AppCommand::SetParallelModeEnabled`, 호스트만 보낸다).
- **Gateway 의 커널에서 병렬 모드를 켜는 길이 없다.** 시험 밖에서 `Enable` 을 보내는 쪽은 TUI 와 admin 뿐이다. Gateway
  의 커널은 감독 스냅샷을 읽어 `parallel` 이벤트로 보내기만 한다.
- 커널은 병렬 제어부에 남은 일이 있으면 내려가지 않는다 (`NativeClientRuntime::has_remaining_work`). 그래서 병렬 모드가
  켜져 epoch 가 열린 Gateway 커널은 저장소 잠금을 계속 쥔다. 그 동안 그 저장소에서 TUI 를 시작할 수 없다 (K6 전).

### 2.4 이 사실에서 나오는 것

| 사실 | 결과 |
| --- | --- |
| 병렬 명령이 `AppCommand` 가 아니다 | 커널 호스트에 병렬 명령의 길이 필요하다. 영수증과 기록은 같은 길(`command_received`)을 쓴다 |
| Gateway 커널에 켜는 길이 없다 | 명령을 더하는 것만으로 새 기능이다 (Desktop 과 `akra kernel` 이 Gateway 커널의 병렬 모드를 켠다) |
| 켜짐을 저장하지 않는다 | Gateway 를 다시 시작하면 병렬 모드가 꺼진다. `auto_follow_configured` 와 같은 기록이 필요한지 정한다 (§5 의 Q-9b-3) |
| 병렬 모드가 켜진 Gateway 커널은 잠금을 쥔다 | admin 이 Gateway 커널로 병렬 모드를 켜면 그 저장소에서 TUI 를 시작할 수 없다. 지금은 시작할 수 있다 (§5 의 Q-9b-1) |
| admin 은 Gateway 도 TUI 도 없이 병렬 작업을 돌릴 수 있다 | `runtime` 명령은 Gateway 가 없으면 거절된다. 그 규칙을 그대로 쓰면 admin 만으로 병렬을 돌리는 방법이 없어진다 (§5 의 Q-9b-1) |
| 입구가 스케줄러를 돌린다 | 입구가 멈추면 PR 검증 관찰은 커널이 떠 있는 동안에만 돈다 (§5 의 Q-9b-2) |
| 입구가 보이는 값이 명령의 답에 없다 | 입구는 명령을 보낸 뒤 `parallel` 조회로 값을 읽는다. `parallel` 상태에 없는 값은 더한다 (§3.3) |

## 3. 설계

### 3.1 커널 명령

| 명령 | 입력 | `scope` | `effect` | `state` | `idempotency` | 끝 | 바로 거절 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `set_parallel_mode` | `enabled` | `parallel` | write | runtime | natural | 받아들이면 끝이다 | `kernel_unreachable` |
| `dispatch_parallel` | 없음 | `parallel` | write | runtime | keyed | 받아들이면 끝이다. 배분의 결과는 `parallel` 이벤트이다 | `kernel_unreachable`, `parallel_mode_off` |
| `refresh_parallel` | 없음 | `parallel` | write | runtime | natural | 받아들이면 끝이다 | `kernel_unreachable` |
| `save_parallel_agent_profiles` | 프로필 목록 | `parallel` | write | store | natural | 저장의 완료 (`command_completed`) | 없음 |

- `refresh_parallel` 이 `read` 가 아닌 이유: 병렬 모드가 켜져 있으면 worktree pool 을 맞춘다 (`reconcile_pool`).
- `dispatch_parallel` 은 병렬 모드가 꺼져 있으면 `parallel_mode_off` 로 거절한다 (`next`: `set_parallel_mode`, 다시
  보냄). 커널이 대신 켜지 않는다. admin 의 "꺼져 있으면 켠다"는 admin 의 규칙으로 남긴다 (§3.4).
- `set_parallel_mode` 가 하는 일은 TUI 의 켜기·끄기와 같게 한다: 턴 뒤 이어 가기 값과 미룬 이유의 정리를 포함한다. 그
  묶음을 런타임의 함수 하나로 모으고 TUI 도 그 함수를 쓴다. 그래야 "켠다"의 뜻이 입구마다 다르지 않다.
- 작업 공간은 명령의 입력이 아니다. 커널의 대표 checkout 을 쓴다. 클라이언트가 붙은 폴더를 쓰면 제어부가 "다른 작업
  공간의 명령"으로 보고 버린다.

### 3.2 커널 호스트의 길

- 프로토콜 adapter 의 `KernelCommand::into_core` 가 `AppCommand` 만 돌려준다. 이것을 커널이 받는 입력으로 넓힌다:
  core 명령이거나 병렬 명령이다.
- 런타임의 `dispatch_client_command` 는 지금처럼 명령을 받아 적고(`command_received`, 멱등 키), 병렬 명령이면 제어부로
  보낸다. core 를 지나지 않으므로 core 가 "효과 없는 명령"으로 보고 실패로 적는 일이 없다.
- 세 `runtime` 명령의 끝은 "받아들임"이라서 완료 기록이 없다. 결과는 `parallel` 이벤트로 온다.
- `save_parallel_agent_profiles` 는 `store` 명령이다. Gateway 의 커널이 없으면 명령 전용 조립이 실행한다
  (`reset_planning` 과 같은 길). 명령 전용 조립이 실행할 수 있는 명령을 표로 만든다. 지금은 `reset_planning` 하나를
  코드에 적었다.

### 3.3 `parallel` 상태에 더하는 값

입구가 지금 보이는 값 중 `parallel` 조회의 답에 없는 것을 더한다. 더하는 필드는 선택 필드이다.

| 값 | 쓰는 입구 |
| --- | --- |
| 진행 중인 제어 효과가 있는가, 현재 epoch, 배분을 미룬 이유 | admin |
| 준비 상태 글, 큐의 깊이 | Telegram |
| 마지막 배분의 알림 글 | `parallel-tick` |

### 3.4 입구의 전환

| 입구 | 바꾸는 것 |
| --- | --- |
| admin 의 제어 | `ParallelModeAdminPort::execute_control` 이 `KernelCommandPort` 로 명령을 보낸다. 그 뒤 `parallel` 조회로 답을 만든다. "꺼져 있으면 켠다"는 admin 서비스가 조회한 값으로 정한다 |
| admin 의 프로필 저장 | K4-9a 와 같은 방법이다. 조립이 저장을 커널 명령으로 보낸다 |
| admin 과 Telegram 의 읽기 | 제어부를 조립하지 않는다. 읽기 전용 조립을 쓴다 (스케줄러를 시작하지 않는다). 커널이 돌면 `parallel` 조회를 쓰고, 돌지 않으면 저장된 감독 스냅샷을 읽는다 |
| CLI `parallel-tick` | `dispatch_parallel` 을 보내고 `parallel` 이벤트로 끝을 기다린다. 종료 코드는 지금과 같게 한다 |

### 3.5 하지 않는 것

- PR 검증 명령 5개는 K4-9c 이다. 같은 호스트의 길을 쓴다.
- TUI 가 Gateway 의 커널에 붙는 일은 K6 이다. 그 전까지 TUI 가 커널을 쥔 저장소는 §5 의 Q-9b-1 의 답을 따른다.
- 병렬 제어부를 core 안으로 옮기는 일은 plan 19 의 Q5 이다. 여기서는 길만 만든다.

## 4. 조각 (PR 단위)

| 조각 | 내용 | 동작 | 확인 |
| --- | --- | --- | --- |
| 9b-1 | 커널의 병렬 명령 세 개 (`set_parallel_mode`, `dispatch_parallel`, `refresh_parallel`): 스펙, 호스트의 길, 이유 코드 `parallel_mode_off`, `scope` 값 `parallel`. 켜기·끄기의 묶음을 런타임의 함수 하나로 모은다 | 확장 (Desktop 과 `akra kernel parallel …` 이 Gateway 커널의 병렬 모드를 켜고 끈다. 입구는 그대로이다) | 프로토콜 시험, 런타임 시험 (기록, 작업 공간, 꺼진 때의 거절), 시뮬레이션, 스펙·Desktop 타입·skill 일치 |
| 9b-2 | `save_parallel_agent_profiles` (store 명령). 명령 전용 조립의 명령 표. admin 의 프로필 저장이 커널 명령으로 간다 | 유지 (같은 결과. 기록에 남는다) | 입구 시험 (K4-9a 와 같은 모양), 명령 전용 조립 시험 |
| 9b-3 | `parallel` 상태에 §3.3 의 값을 더한다 | 확장 | 프로토콜 시험, Desktop 타입 |
| 9b-4 | 읽기만 하는 입구(Telegram, admin 의 화면)가 스케줄러를 시작하지 않는다 | 전환 (§5 의 Q-9b-2) | 조립 시험 (스레드가 없다), Telegram·admin 시험 |
| 9b-5 | admin 의 제어와 `parallel-tick` 이 커널 명령으로 간다 | 전환 (§5 의 Q-9b-1) | K-D3 회귀 시험, admin 시험, CLI 시험 |

- 9b-1, 9b-2, 9b-3 은 §5 의 답과 관계없다. 먼저 한다.
- 9b-4, 9b-5 는 §5 의 답에 따라 모양이 달라진다.

## 5. 사용자가 정할 것

| 번호 | 질문 | 기본값 | 다른 선택 |
| --- | --- | --- | --- |
| Q-9b-1 | admin 의 병렬 제어가 Gateway 의 커널로 갈 때, Gateway 가 없거나 TUI 가 커널을 쥔 저장소에서는 어떻게 하는가 | (a) 지금처럼 admin 프로세스의 제어부로 직접 실행한다. K6 까지 남긴다. Gateway 의 커널이 도는 저장소에서만 커널로 간다. K-D3 은 그 저장소에서만 닫힌다 | (b) Gateway 가 없으면 `kernel_unreachable` 로 거절한다. admin 만으로 병렬을 돌리는 방법이 없어진다. (c) K6 (TUI 가 Gateway 커널에 붙는다)을 먼저 한다 |
| Q-9b-2 | admin 과 Telegram 이 PR 검증 스케줄러를 시작하지 않으면, PR 검증 관찰은 커널(TUI 또는 Gateway 의 커널)이 떠 있는 동안에만 돈다. 받아들이는가 | (a) 받아들인다. 관찰은 커널의 일이다. Gateway 의 커널은 클라이언트가 붙어 있거나 일이 남은 동안 돈다 | (b) admin 만 스케줄러를 남긴다 (K-D5 가 admin 에 남는다) |
| Q-9b-3 | Gateway 커널의 병렬 모드 켜짐을 기록에 남겨 다음 커널이 되돌리는가 | (a) 남기지 않는다. 병렬 모드는 켠 커널이 사는 동안의 값이다. 켜져 있으면 커널이 내려가지 않으므로, 꺼지는 경우는 Gateway 를 다시 시작한 때뿐이다 | (b) `auto_follow_configured` 처럼 기록하고 되돌린다. 그러면 Gateway 를 다시 시작한 뒤 클라이언트가 붙을 때 병렬 작업이 스스로 다시 시작한다 |

Q-9b-1 의 배경: 병렬 모드가 켜진 Gateway 커널은 저장소 잠금을 쥔다 (§2.3). (b)나 (c)를 고르면 admin 으로 병렬 모드를 켠
저장소에서 TUI 를 시작할 수 없다 (K6 전). 지금은 admin 과 TUI 를 함께 쓸 수 있다. 그 대신 제어부가 둘이다 (K-D3).

## 6. 위험

| 위험 | 대응 |
| --- | --- |
| `dispatch_parallel` 을 같은 서명으로 다시 보내면 제어부가 조용히 건너뛴다 | 명령마다 새 서명을 쓴다 (연산 id). 건너뛴 경우를 `parallel` 상태의 알림으로 보인다 |
| `parallel-tick` 이 끝을 기다리다가 멈춘다 (진행 중인 효과가 있으면 tick 이 실행되지 않는다) | 기다림에 시간 한도를 두고, 실행되지 않은 이유를 출력한다 |
| 프로필의 저장 위치가 바뀐다 (admin 의 작업 공간과 커널의 대표 checkout 이 다를 때) | 저장 위치를 커널의 대표 checkout 으로 정하고, 다른 경우를 시험한다 |
| 실제 조립을 지나는 시험이 실제 Gateway 에 붙는다 | 시험 빌드의 Gateway 주소는 아무도 듣지 않는 이름이다 (K4-9a). 새 시험도 이 규칙 안에 있다 |
| 병렬 제어부는 실제 스레드를 쓴다. 시뮬레이션이 결정적이지 않다 | 시뮬레이션은 명령의 기록과 거절만 본다. 제어부의 동작은 제어부 시험이 본다 |
