# 29-D1. 위임 실행: 계약과 장부

상태: **Proposed**. 현재 제품 동작을 설명하지 않는다. · 기준일: 2026-11-21 · 기준 커밋: `33a12b72` (`origin/prerelease`) ·
상위: [plan 29 위임 실행](29-akra-delegated-execution.md) (§5, §6, §12)

## 목표

위임 작업의 상태, 장부, 백엔드 계약을 만든다. 가짜 백엔드로 접수부터 끝까지 돈다. 실제 백엔드는 없다. 사용자가 보는
동작은 바뀌지 않는다.

## 지금

- 모델을 쓰는 소비자마다 port 가 따로이다 (`IssueReviewModelPort`). 전달, 진행, 중단의 공통 모양이 없다.
- 고립 thread 실행은 호출 하나가 끝까지 막는다 (`run_isolated_new_thread_stream`). 재시작을 넘지 못한다.
- kernel spec 의 범위는 `approval`, `auto_follow`, `issue`, `planning`, `session`, `thread` 이다. `delegation` 은 없다.

## 서브 이슈

| ID | 결과 하나 | 바뀌는 곳 | 선행 |
| --- | --- | --- | --- |
| D1-1 | 상태와 순수 함수: 작업 상태, 시도(전달, 단계, 끝 원인), `next_action`, `select_backend` | `src/domain/delegation/` | - |
| D1-2 | 장부: 작업, 시도, 관찰, 회로 상태의 저장. 멱등 키로 1회 접수 | `DelegationStorePort`, `src/adapter/outbound/db/` | D1-1 |
| D1-3 | `AgentBackendPort`, 가짜 백엔드, 조정기, 런타임 호스트의 마감 | `src/application/service/delegation/`, `src/composition/` | D1-2 |
| D1-4 | kernel 범위 `delegation`: `submit_delegation`, `cancel_delegation`, 조회 `delegations`, `delegation`, `delegation_backends`, 기록 `delegation_*` | `src/domain/kernel_spec.rs`, `schema/akra-kernel-spec.json`, 생성 skill | D1-3 |

## 계약

- 상태와 값의 목록은 plan 29 §6 이다. 이 조각이 이름을 확정한다.
- `next_action` 과 `select_backend` 는 I/O 가 없다. 입력은 저장된 값과 시각이다.
- 장부는 저장소 runtime 폴더의 새 SQLite 파일이다. planning DB 를 건드리지 않는다.
- 접수는 멱등 키로 조회하고, 없을 때만 만든다. 조회와 생성은 한 트랜잭션이다. kernel 명령의 멱등 키에 기대지 않는다
  (plan 28 §5.4 와 같은 이유이다).
- 밖으로 나가는 행동(`dispatch`)은 부르기 전에 장부에 적는다. 응답 없이 죽으면 그 시도는 `uncertain` 이고
  작업은 `outcome_unknown` 이다.
- 모듈은 planning, parallel, `src/core` 의 타입을 import 하지 않는다. architecture test 를 더한다.
- 사건 기록에는 지시와 결과의 해시와 크기만 쓴다.

## 완료 기준

- [ ] 가짜 백엔드로 `queued` → `dispatching` → `running` → `succeeded` 가 된다.
- [ ] 가짜 백엔드의 단계마다 (`failed`, `stopped`, `disconnected`, `blocked`, `unreachable`, `unknown`) 정한 행동이 나온다.
- [ ] 같은 멱등 키의 접수는 작업 하나이다 (재전송, 재시작, 기록의 정리 포함).
- [ ] 전달이 `uncertain` 인 시도 뒤에 새 보내기가 0회이다.
- [ ] 임의 시점에 죽였다 살려도 끝 상태가 같다.
- [ ] 마감을 넘긴 작업은 `expired` 이다. 취소한 작업은 백엔드의 `stop` 을 한 번 부른다.
- [ ] `delegation` 조회가 상태, 시도, 근거의 출처와 시각을 보인다.

## 검증

- `command`: domain 의 표 기반 단위 시험. kernel 시뮬레이션 시험 (가짜 백엔드, 가짜 시계, seed).
- `data`: 장부의 줄. 사건 기록에 원문이 없는 것.
- 모델 턴이 없다.

## 제외

- 실제 백엔드 (D2, D3). 정책 파일과 회로의 실제 값 (D4). 기능 (D5). 콜백과 화면 (D6).

## 미결정

| ID | 질문 | 기본값 |
| --- | --- | --- |
| Q-D1-1 | 장부를 저장소마다 두는가, `$AKRA_HOME` 에 하나 두는가 | 저장소마다. 회로 상태와 lease 만 머신 전체이다 (plan 29 Q-DE2) |
| Q-D1-2 | 관찰 값을 모두 저장하는가 | 시도마다 최근 16개와 마지막 진행 시각만 둔다 |
| Q-D1-3 | `AgentBackendPort` 는 동기인가, async 인가 | 동기. 기존 outbound port 와 같다. 조정기가 블로킹 작업으로 부른다 |
