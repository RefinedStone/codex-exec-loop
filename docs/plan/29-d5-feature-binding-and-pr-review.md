# 29-D5. 위임 실행: 기능 바인딩과 PR 자동 리뷰

상태: **Proposed**. 현재 제품 동작을 설명하지 않는다. · 기준일: 2026-11-21 · 기준 커밋: `33a12b72` (`origin/prerelease`) ·
상위: [plan 29 위임 실행](29-akra-delegated-execution.md) (§10) ·
선행: [D2](29-d2-codex-thread-backend.md) 또는 [D3](29-d3-chatgpt-chat-backend.md), [D4](29-d4-routing-and-fallback.md)

## 목표

저장소마다 기능을 켜고 끈다. 첫 기능은 LLM PR 자동 리뷰이다. 사용자가 그 저장소에서 Akra 를 쓰는 동안, PR 이 생기거나
바뀌면 위임 작업이 리뷰를 만든다. 백엔드는 경로가 고른다.

## 지금

- 저장소별 기능 설정은 기능마다 따로이다: `akra.prValidationMode`, 이슈 자동화의 바인딩 (plan 28).
- Akra 는 PR 의 리뷰와 CI 를 읽는다 (`github_pr_validation_port`, `github_review_poller_port`). 리뷰를 쓰는 주체는
  밖의 봇과 사람이다. Akra 가 내는 LLM 리뷰는 없다.
- PR 댓글 쓰기는 `github_automation_port` 에 있다.

## 서브 이슈

| ID | 결과 하나 | 바뀌는 곳 | 선행 |
| --- | --- | --- | --- |
| D5-1 | 기능 등록부와 저장소 바인딩: 기능의 닫힌 목록, `set_feature_binding`, 조회 `feature_bindings` | `src/domain/delegation/`, kernel spec | D1-4, D4-1 |
| D5-2 | PR 리뷰 `observe`: 트리거, 지시 만들기, 결과 모양, 멱등, Desktop 에 보이기 | `src/application/service/`, `desktop/src/renderer/` | D5-1, D2-1 또는 D3-3 |
| D5-3 | PR 리뷰 `on`: PR 댓글 쓰기, 마스킹, 일지, 결과 모름의 처리 | `src/application/service/`, GitHub adapter | D5-2 |
| D5-4 | 주기 트리거: 마감 시각으로 도는 기능 | 런타임 호스트 | D5-1. plan 29 Q-DE8 |

## 계약

- 기능은 코드에 등록한다: `{ id, 트리거, 지시 만들기, 결과 모양, 부작용 등급, 결과 처리 }`. 동적 plugin 은 없다.
- 바인딩은 저장소와 기능의 쌍이다. 수준은 `off`, `observe`, `on` 이다. 기본은 `off` 이다.
- 수준을 올리는 것은 사람의 명령이다. 에이전트는 올리지 못한다 (plan 28 §14 와 같다).
- 기능은 그 저장소의 kernel 이 떠 있는 동안만 돈다. kernel 을 스스로 띄우지 않는다.
- PR 리뷰의 작업:
  - 멱등 키는 `(pr_review, 저장소, PR 번호, head SHA)` 이다.
  - 새 push 가 오면 앞의 head SHA 의 작업을 취소하고 새 작업을 낸다.
  - 지시는 호스트가 만든다: 리뷰 기준, PR 의 제목과 본문 (자료 구역), diff (크기 한도).
  - diff 가 한도를 넘으면 작업을 내지 않고 이유를 보인다. 자르지 않는다.
  - 결과 모양은 `findings[{ path, line, severity, body }]` 와 `summary` 이다. 모양이 틀리면 `result_invalid` 이다.
  - 부작용 등급은 `read_only` 이다. 백엔드는 PR 에 쓰지 않는다.
- `observe` 는 결과를 Desktop 에만 보인다. `on` 은 PR 댓글 하나를 만들고 고친다. 댓글을 쌓지 않는다.
- 댓글 쓰기는 기존 GitHub 쓰기 경로, 마스킹, 일지를 지난다. 결과를 모르는 쓰기는 다시 쓰지 않는다 (plan 28 §12).
- Akra 가 쓴 댓글은 새 리뷰의 트리거가 아니다.

## 완료 기준

- [ ] 기능이 `off` 인 저장소는 작업을 내지 않는다.
- [ ] 같은 head SHA 는 작업 하나이다. 재시작 뒤에도 같다.
- [ ] 새 push 뒤에 앞의 작업이 `cancelled` 이고 새 작업이 하나 생긴다.
- [ ] 경로의 첫 백엔드가 막히면 둘째 백엔드가 리뷰를 낸다. 결과에 낸 백엔드가 보인다.
- [ ] `observe` 에서 GitHub 쓰기가 0회이다.
- [ ] `on` 에서 응답을 잃어도 댓글이 하나이다.
- [ ] PR 본문의 지시문이 작업의 지시를 바꾸지 못한다 (자료 구역 시험).

## 검증

- `command`: 시뮬레이션 시험 (가짜 GitHub, 가짜 백엔드, 가짜 시계). Desktop 의 vitest 와 가짜 Gateway e2e.
- 실제 계정: 사용자가 허락한 PR 하나 (D-13).

## 제외

- 리뷰 지적의 자동 수정 (PR 검증의 `remediate` 가 따로 있다). 리뷰의 승인 표시 (APPROVE). 다른 저장소의 PR.

## 미결정

| ID | 질문 | 기본값 |
| --- | --- | --- |
| Q-D5-1 | 어떤 PR 을 리뷰하는가 | Akra 가 만든 PR 과 사람이 명령으로 지정한 PR. 저장소의 모든 열린 PR 은 뒤이다 |
| Q-D5-2 | 바인딩을 어디에 저장하는가 | 정책 파일의 `[feature.*]`. 명령은 사용자 파일을 고친다 |
| Q-D5-3 | 이슈 자동화의 바인딩과 합치는가 | 합치지 않는다. 이슈 자동화는 자기 수준을 가진다. 그 모델 호출만 위임을 쓴다 |
| Q-D5-4 | 리뷰 기준을 어디에 두는가 | 기본 skill 문서. 저장소가 같은 이름의 skill 로 바꾼다 (plan 28 §7.4 와 같다) |
| Q-D5-5 | PR 댓글의 글은 누가 쓰는가 | 호스트가 `findings` 에서 만든다. 백엔드의 글을 그대로 올리지 않는다 |
