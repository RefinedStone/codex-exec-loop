# 29-D6. 위임 실행: 에이전트 입구와 Desktop 화면

상태: **Proposed**. 현재 제품 동작을 설명하지 않는다. · 기준일: 2026-11-21 · 기준 커밋: `33a12b72` (`origin/prerelease`) ·
상위: [plan 29 위임 실행](29-akra-delegated-execution.md) (§8.3, §11, §12) ·
선행: [D1](29-d1-contract-and-ledger.md). 콜백은 [D3](29-d3-chatgpt-chat-backend.md)

## 목표

에이전트가 다른 백엔드를 단방향 서브에이전트로 쓴다: 지시를 내고 결과를 받는다. 백엔드는 콜백으로 진행과 결과를 알린다.
사람은 Desktop 에서 위임 작업, 백엔드의 건강, 자기 차례를 본다.

## 지금

- kernel 명령은 CLI `akra kernel <범위> … --json` 과 생성 skill 로 에이전트에게 열려 있다 (plan 18 K4).
- 에이전트가 다른 백엔드에 일을 맡기는 명령은 없다.
- Desktop 에는 ChatGPT 대화의 읽기 화면만 있다 (`ChatDock.tsx`).

## 서브 이슈

| ID | 결과 하나 | 바뀌는 곳 | 선행 |
| --- | --- | --- | --- |
| D6-1 | 에이전트 명령: `submit`, `wait`, `status`, 결과 읽기. skill 문서. 깊이와 예산의 한도 | kernel spec, 생성 skill | D1-4 |
| D6-2 | 콜백: `checkpoint_delegation`, `report_delegation_result`, 작업별 토큰, `callback` 근거 | kernel spec, `src/application/service/delegation/` | D1-4. 실제 확인은 D3-5 |
| D6-3 | Desktop 의 위임 화면: 목록, 상세 (시도, 근거, 결과), 취소와 다시 내기 | `desktop/src/renderer/` | D1-4 |
| D6-4 | Desktop 의 백엔드와 주의: 건강, 회로, 내 차례 (로그인, 챌린지, 결과 모름), 구동기 보기 열기 | `desktop/src/renderer/`, `desktop/src/main/` | D4-2, D3-1 |

## 계약

- 에이전트와 사람은 같은 명령을 쓴다 (KP-04). 에이전트의 접수는 `caller` 를 기록한다.
- `wait` 는 짧게 기다린다 (최대 10초). 새 상태나 끝이 오면 바로 돌아온다. 긴 호출 하나로 막지 않는다
  (akra-tunnel 의 `task_wait` 와 같다).
- 방향은 하나이다. 접수한 뒤에 지시를 바꾸지 못한다. 취소하고 다시 낸다.
- 작업은 깊이를 가진다. 콜백 토큰으로 온 접수는 깊이가 하나 오른다. 한도를 넘으면 `depth_exceeded` 이다.
- 콜백 토큰은 작업 하나의 `checkpoint` 와 `report` 만 허락한다. 작업이 끝나면 토큰은 쓰지 못한다.
- 체크포인트는 `{ stage, summary, next_action }` 이다. 진행 근거로 센다. 결과가 아니다.
- `report` 의 결과는 자료이다. 모양을 확인한 뒤에 `succeeded` 이다. 대화의 마지막 답과 다르면 콜백이 이긴다.
- 화면은 계산하지 않는다. 상태, 묶음, 이유는 조회의 값이다 (plan 23 원칙, plan 28 §16).
- 주의 항목의 대상 종류에 `delegation` 을 더한다 (plan 23).
- 사람의 확인(로그인을 끝냈다 등)은 Desktop main 을 지난다 (plan 29 Q-DE15).

## 완료 기준

- [ ] 에이전트가 CLI 로 작업을 내고 결과를 읽는다 (가짜 백엔드).
- [ ] 깊이 한도를 넘는 접수는 거절된다.
- [ ] 다른 작업의 토큰으로 온 `report` 는 거절된다. 끝난 작업의 `report` 도 거절된다.
- [ ] 체크포인트가 오는 동안 작업은 `stalled` 가 아니다.
- [ ] 화면이 작업마다 상태, 지금 백엔드, 진행 근거의 출처와 시각, 대체의 이력을 보인다.
- [ ] `awaiting_human` 인 작업이 "내 차례"에 이유와 함께 나온다.
- [ ] renderer 에 상태를 판정하는 코드가 없다.

## 검증

- `command`: 시뮬레이션 시험, CLI 의 JSON 시험, Desktop 의 vitest 와 가짜 Gateway e2e (숨긴 창).
- 실제 계정: 콜백은 D3-5 와 함께 사용자가 허락한 한 번.

## 제외

- 백엔드와의 다중 턴 대화. 결과의 스트리밍. 에이전트가 기능의 수준을 바꾸는 것. Telegram 알림.

## 미결정

| ID | 질문 | 기본값 |
| --- | --- | --- |
| Q-D6-1 | 화면의 자리 | 오른쪽 패널의 새 탭 "위임". 최대화할 수 있다 |
| Q-D6-2 | ChatGPT 대화의 읽기 화면 (`ChatDock`) 과 위임 상세를 잇는가 | 잇는다. 위임 상세에서 그 대화를 연다 |
| Q-D6-3 | 에이전트의 접수에 예산을 따로 두는가 | 둔다. thread 하나에 동시 2개, 하루 한도는 정책이 정한다 |
| Q-D6-4 | 콜백 토큰이 지시 봉투에 있으면 대화 기록에 남는다. 받아들이는가 | 예. 토큰은 작업 하나에만 쓰이고 끝나면 죽는다 |
