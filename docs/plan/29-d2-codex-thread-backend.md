# 29-D2. 위임 실행: Codex thread 백엔드

상태: **Proposed**. 현재 제품 동작을 설명하지 않는다. · 기준일: 2026-11-21 · 기준 커밋: `33a12b72` (`origin/prerelease`) ·
상위: [plan 29 위임 실행](29-akra-delegated-execution.md) (§9) · 선행: [D1](29-d1-contract-and-ledger.md)

## 목표

첫 실제 백엔드를 붙인다. 위임 작업이 고립된 읽기 전용 Codex thread 에서 돈다. 첫 소비자는 이슈 초안 검토이다.
이 백엔드는 ChatGPT 채팅이 막혔을 때의 대체 자리이기도 하다.

## 지금

- `ParallelAgentWorkerPort::run_isolated_new_thread_stream` 이 새 thread 를 열고 끝 영수증
  (`ConversationTurnTerminalReceipt`) 을 준다. 이 길은 슬롯과 worktree 를 전제로 쓴다.
- `IssueReviewModelPort` 의 구현은 Fake 하나이다 (`issue_review_model_fake.rs`).
- 턴의 오류 분류가 있다 (`ConversationTurnErrorInfo`).

## 서브 이슈

| ID | 결과 하나 | 바뀌는 곳 | 선행 |
| --- | --- | --- | --- |
| D2-1 | `codex_thread` adapter: 고립 thread, 읽기 전용 sandbox, 스트림을 관찰 값으로 접기, 끝 분류를 끝 원인으로 옮기기 | `src/adapter/outbound/app_server/` | D1-3 |
| D2-2 | 첫 소비자: `IssueReviewModelPort` 의 adapter 가 위임 작업을 낸다. Fake 는 시험에 남는다 | `src/adapter/outbound/`, `src/composition/production.rs` | D2-1 |
| D2-3 | 모델과 프로필의 지정: 이름이 다른 `codex_thread` 백엔드 여럿 | 백엔드 설정 | D2-1, D4-1 |

## 계약

- thread 는 새로 연다. 사용자의 thread 에 붙지 않는다. kernel 의 "대화 턴 하나" 자리를 차지하지 않는다.
- sandbox 는 읽기 전용이다. 승인 요청은 거절한다. 그래서 이 백엔드는 `read_only` 작업만 받는다.
- 능력: `sessions = [new]`, `local_tools = true` (읽기), `structured_result = true`, `self_recovery = false`, `stop = true`.
- 끝 분류를 옮기는 표는 plan 29 §9 이다. 가리지 못한 끝은 `unknown` 이다. 성공으로 보지 않는다.
- 스트림이 사는 동안의 사건은 `stream` 근거이다. 프로세스가 죽으면 시도는 `interrupted` 이다. 이 백엔드는 재개하지 않는다.
  읽기 전용이므로 새 시도가 안전하다.
- 지시는 `developer_instructions` 와 prompt 로 나눈다. 자료(초안, diff)는 자료 구역으로 감싼다.

## 완료 기준

- [ ] 가짜 app-server 의 `Completed`, `Failed`, `Interrupted`, `Unknown` 이 정한 단계와 끝 원인이 된다.
- [ ] `UsageLimitExceeded` 는 `quota` 이다. 회로가 닫히고 다음 백엔드를 고른다 (D4 와 함께 확인한다).
- [ ] app-server 가 없으면 건강은 `unavailable` 이고 보내기가 0회이다.
- [ ] 이슈 초안 검토가 위임 작업으로 돈다. 요청 밖의 글(Vault 값, 복원본)이 지시에 없다.
- [ ] 취소는 진행 중인 턴을 멈춘다.

## 검증

- `command`: 가짜 app-server 의 adapter 시험. 시뮬레이션 시험.
- 실제 계정: 사용자가 허락한 턴 한 번 (D-13). 이슈 초안 하나를 검토한다.

## 제외

- 쓰기가 있는 작업. worktree 와 슬롯. 사용 한도를 미리 읽는 것 (D4-4).

## 미결정

| ID | 질문 | 기본값 |
| --- | --- | --- |
| Q-D2-1 | `ParallelAgentWorkerPort` 의 adapter 를 다시 쓰는가, 새 adapter 를 두는가 | 같은 app-server 연결 코드를 쓰는 새 adapter. 병렬 계약을 바꾸지 않는다 |
| Q-D2-2 | 위임 thread 가 Desktop 의 thread 목록에 보이는가 | 보이지 않는다. 위임 화면에서 연다 (D6) |
| Q-D2-3 | 기본 모델과 effort | thread 의 기본값. D4 의 정책이 바꾼다 |
