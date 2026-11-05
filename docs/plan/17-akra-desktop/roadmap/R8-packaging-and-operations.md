# R8. 배포와 운영

상태: 계획 · 기준일: 2026-11-05 · 선행: P3 · 규모: 1~2주

## 목표

개발 환경 없이 Akra Desktop 과 Gateway 를 설치하고 매일 쓴다.

## 범위

| 항목 | 내용 |
| --- | --- |
| Gateway 상주 | macOS LaunchAgent 로 로그인 때 `akra-gateway` 를 시작한다. 비정상 종료 때 다시 시작한다 (Q-05) |
| 앱 패키징 | electron-builder 로 `.app` 과 `.dmg` 를 만든다. `akra-gateway` 바이너리를 함께 넣는다 |
| 서명 | 개인 사용은 로컬 서명으로 시작한다. 배포가 필요하면 Developer ID 서명과 공증을 추가한다 |
| 업데이트 | 처음에는 수동 설치로 한다. 자동 업데이트는 필요할 때 추가한다 |
| 로그 | Gateway 와 앱의 로그 위치, 보존 기간, 크기 제한을 정한다 |
| codex 업그레이드 | [04](../04-target-architecture.md) §9 의 절차를 문서와 스크립트로 만든다 |

## 설계 메모

- Akra 의 기존 배포 문서([13 native packaging](../../13-native-packaging-and-operator-runbook.md))와
  npm 배포 경로를 먼저 확인한다. 같은 방식을 쓸 수 있으면 재사용한다.
- 앱이 포함한 `akra-gateway` 와 이미 실행 중인 Gateway 의 버전이 다를 수 있다. 앱은 `akra/gateway/info`
  로 버전을 확인하고, 다르면 재시작을 안내한다.

## 미결

- Q-05: 데몬 시작 방식 (LaunchAgent, 앱이 시작)
- Q-06: Windows, Linux 지원 여부
- Q-08: 앱 이름과 아이콘
