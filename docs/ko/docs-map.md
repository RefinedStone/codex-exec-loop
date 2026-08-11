# 문서 지도

[English](../README.md)

이 페이지는 하나의 진실 소스를 빠르게 고르기 위한 지도입니다. 현재 구현 참조는 compact한 영문
원본과 한국어 번역을 함께 관리합니다. 저장소 테스트가 marker나 inventory를 직접 읽는 검증 계약은
독립 문서로 유지합니다. 미래 계획은 상태를 명시하고 경쟁 앱 조사는 제품별 최신 원문 한 벌만
유지합니다.

## 현재 구현

| 필요한 정보 | 영문 원본 | 한국어 |
| --- | --- | --- |
| 제품 화면, 명령, planning, parallel 흐름, 제한 | [current-product.md](../reference/current-product.md) | [reference/current-product.md](reference/current-product.md) |
| 전역·프로젝트·환경변수·명령행 설정 | [configuration.md](../reference/configuration.md) | [reference/configuration.md](reference/configuration.md) |
| 계층·상태 권한·보안 경계 | [architecture.md](../reference/architecture.md) | [reference/architecture.md](reference/architecture.md) |
| 저장소 지도, 코딩 규칙, 테스트, worktree, GitHub 전달 | [development.md](../reference/development.md) | [reference/development.md](reference/development.md) |
| Admin 게임 프런트엔드 runtime·renderer | [admin-game-frontend.md](../reference/admin-game-frontend.md) | [reference/admin-game-frontend.md](reference/admin-game-frontend.md) |
| Post-merge validation 운영·evidence·rollback | [pr-validation-rollout.md](../reference/pr-validation-rollout.md) | [reference/pr-validation-rollout.md](reference/pr-validation-rollout.md) |
| 설치와 첫 실행 | [README.md](../../README.md) | [README.md](README.md) |
| 네이티브 패키징과 배포 | [영문 runbook](../plan/13-native-packaging-and-operator-runbook.md) | [reference/release.md](reference/release.md) |

## TUI와 검증 계약

- [TUI 계층/시각 계약 원문](../design/07-tui-layered-architecture-and-aesthetic-contract.md) · [한국어](reference/tui-contract.md)
- [플랫폼 검증 matrix 원문](../plan/12-platform-validation-matrix.md) · [한국어 통합 안내](reference/validation.md)
- [TUI 테스트 방법론 원문](../validation/terminal-ui-testing-methodology.md) · [한국어 통합 안내](reference/validation.md)
- [TUI coverage inventory 원문](../validation/tui-coverage-matrix.md) · [한국어 통합 안내](reference/validation.md)
- [검증 기록](../validation/README.md)

## 계획 및 외부 조사

다음 문서는 현재 구현 계약이 아닙니다.

- [Codex for OSS 지원서 초안](../plan/14-codex-for-oss-application.md)
- [경쟁 앱 최신 compact 원문](../competitive/README.md) · [한국어 요약](competitive/README.md)

각 문서에 적힌 상태와 근거 시점을 기준으로 읽어야 하며, 이 자료의 내용이 곧 구현 완료를 뜻하지
않습니다.

## 유지 규칙

- 같은 계약을 여러 파일에 복사하지 말고 canonical current reference 하나를 수정합니다.
- 영문 원본과 `docs/ko/` 번역은 양방향 링크를 유지하고 같은 PR에서 갱신합니다.
- 경쟁 제품별 canonical 원문 한 벌과 compact 한국어 index만 유지하고 과거 snapshot은 Git 이력을
  사용합니다.
- 미래 계획이나 경쟁 앱 결론을 구현 완료 문구로 승격하지 않습니다.
- validation 상태 숫자는 손으로 고치지 않고 실제 기록에 summary helper를 실행해 확인합니다.
