# R6. 자체 compact

상태: 계획 · 기준일: 2026-11-05 · 선행: P3, Q-02 · 규모: 수준에 따라 다름

## 목표

긴 대화의 문맥을 줄이는 방식(compact)을 사용자의 기준으로 바꾼다. D-05(A+)를 지킨다. 원본 대화는
Codex 가 갖고, 요약본과 정책은 Akra 가 갖는다.

## 수준 (Q-02 로 선택)

| 수준 | 방법 | 개입 정도 |
| --- | --- | --- |
| 1 | 설정 `compact_prompt` (요약 지시문), `model_auto_compact_token_limit` (시작 기준) | 요약 방식과 시점을 바꾼다 |
| 2 | Gateway 가 원하는 시점에 `thread/compact/start` 를 보낸다 | 시점을 제어한다 |
| 3 | Akra 가 `thread/items/list` 로 기록을 읽고 직접 요약한다. 요약을 `developerInstructions` 에 넣어 새 스레드를 시작하거나 `thread/fork` 를 쓴다 | 요약 내용을 완전히 제어한다 |
| 4 | 매 턴 문맥을 직접 구성한다 | app-server 로는 할 수 없다. 엔진을 다시 만들어야 하므로 하지 않는다 |

## 설계 메모

- 수준 2 와 3 은 Gateway 의 interceptor 와 `akra/*` 메서드로 구현한다.
- 수준 3 에서는 "이 스레드는 저 스레드를 요약해서 이어 간다"는 연결 정보를 Akra 가 저장한다. 앱은 이
  연결을 따라 기록을 이어서 보여 준다.
- 요약에는 모델 호출이 필요하다. 실험할 때 D-13 의 허락 규칙을 따른다.
- 근거: [codex-app-server.md](../reference/codex-app-server.md) §6

## 미결

- Q-02: 어느 수준부터 할지
- 요약을 만드는 모델과 프롬프트를 어디에 둘지 (Akra 설정, Project metadata)
