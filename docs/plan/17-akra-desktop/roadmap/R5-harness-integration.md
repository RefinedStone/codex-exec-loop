# R5. Akra 하네스 연동

상태: 계획 · 기준일: 2026-11-05 · 선행: P3, Q-01 · 규모: 2~4주 (Akra 수정 범위에 따라 다름)

## 목표

Akra 의 외부 루프 기능(계획 큐, worktree 슬롯, 검증, PR 전달)을 Akra Desktop 에서 쓴다. 예: 대화에서 정한
작업을 큐에 넣고, 진행 상태를 보고, 결과 PR 을 확인한다.

## 범위 후보 (Q-01 로 확정)

1. Project 와 Akra 저장소 연결: Project metadata 에 `akra.*` 키를 쓴다 (D-10).
   예: `akra.planning = "enabled"`, `akra.planningRoot = "<경로>"`.
2. 계획 보기: 큐, 작업, 상태를 패널에 표시한다.
3. 대화를 작업으로: 현재 스레드의 결론을 작업으로 큐에 넣는다.
4. 실행과 결과: 큐 실행, worktree 슬롯 상태, PR 결과를 표시한다.

## 설계 메모

- 기능은 `akra/*` 메서드로 Gateway 에 추가한다 (D-11). 예: `akra/plan/summary`, `akra/task/enqueue`.
  이름은 구현 때 정한다.
- **Akra 수정이 가장 큰 부분이다.** 현재 하네스는 프로세스 하나가 저장소 하나를 다룬다. 전역 데몬
  (D-10)에서 쓰려면 저장소별 계획 서비스를 여러 개 실행하는 구조로 바꿔야 한다.
- 하네스가 쓰는 app-server(P-08)를 Gateway 의 upstream 으로 합칠지 정한다. 합치면 Akra 내부 코드를
  Gateway 의 클라이언트로 바꾼다.
- 시작 전에 불안정 테스트(K-02)를 정리한다. 하네스 영역의 테스트가 기준선 역할을 해야 한다.
- 참고: dori-superagent 의 구조 (리드 세션, 작업별 lane, duty loop, 증거 기반 머지 게이트)
  ([alternatives.md](../reference/alternatives.md) §4).

## 미결

- Q-01: 하네스 기능 중 무엇을 언제 연결할지
- 기존 관리 웹(akra-admin)의 계획 화면을 앱 안에 그대로 띄울지, 새로 만들지
