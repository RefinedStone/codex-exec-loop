# 제품 상태와 배포 경계

[English](../../reference/product-status.md)

조사 기준: [`prerelease@fa72e69a8`](https://github.com/RefinedStone/codex-exec-loop/commit/fa72e69a868826e6b61ec0f935260d1e5b9344a8),
2026-11-27 (Asia/Seoul). 구현 경로와 남은 검증을 작은 표로 연결한다. 계획 전체의 완료표가 아니다.
동작의 상세 기준은 [현재 제품 계약](current-product.md)에 둔다. 아래 시험 링크는 검증 범위의
근거이며, 시험 파일이 있다는 사실만으로 특정 CI 실행이나 실제 OS 시나리오가 통과한 것은 아니다.

## 사용자가 할 수 있는 일

Akra ADE는 Codex 대화, 코드 검토, 작업 완료 기준, 실행 증거와 전달 결과를 하나의 개발 작업공간에서
확인하게 한다. Desktop은 `prerelease` 소스에서 실행한다. TUI·CLI에는 게시된 native/npm 배포판도
있지만, 그 배포판에 이 브랜치의 모든 기능이 포함되는 것은 아니다.

| 구성요소 | 역할 | 현재 진입 경로 |
| --- | --- | --- |
| Desktop | 대화·업무 패널에서 상태를 보고 동작을 요청 | 준비된 소스 checkout에서 `npm --prefix desktop run dev` |
| Gateway | 로컬 클라이언트 연결, app-server 중계, 저장소 Kernel 호스트 | Desktop `predev`가 준비·빌드하며 `akra-gateway` 독립 실행도 가능 |
| Kernel | 저장소 명령·결정·상태·증거 정책의 소유자 | [Kernel 명세](../../../schema/akra-kernel-spec.json), `akra describe --json` |
| TUI | 터미널 대화·운영 화면 | `akra` |
| CLI | 스크립트에서 쓰는 명령·조회 | `akra config`, `akra kernel`; [현재 제품 계약](current-product.md) |

## 구현·UI 연결·검증 상태

아래 Desktop 기능은 모두 소스 개발판 기준이다. 설치 패키지 배포 완료를 뜻하지 않는다.

| 영역과 계획 | 코드·계약 | UI 연결 | 검증 근거와 남은 확인 | 배포 |
| --- | --- | --- | --- | --- |
| 대화·Gateway ([계획 17](../../plan/17-akra-desktop/README.md)) | [Gateway](../../../src/adapter/inbound/gateway/), [세션](../../../desktop/src/renderer/session/desktopSession.ts) | [입력창](../../../desktop/src/renderer/conversation/Composer.tsx), 승인·서버 요청 | [fake Gateway 화면 흐름](../../../desktop/test/e2e/server-requests.mjs); 실제 OS 차이·수동 항목은 [#2846](https://github.com/RefinedStone/codex-exec-loop/issues/2846)에 남음 | 소스 |
| 다음 턴 예약·모델·추론·엔진 문맥 압축 | [예약](../../reference/desktop-next-turn-queue.md), [설정](../../reference/desktop-next-turn-settings.md), [압축 상태](../../../desktop/src/renderer/session/threadCompaction.ts) | [다음 턴 설정](../../../desktop/src/renderer/conversation/NextTurnSettings.tsx), [압축 조작](../../../desktop/src/renderer/conversation/ThreadCompactionControl.tsx) | fake 화면 시험: [예약](../../../desktop/test/e2e/next-turn-queue.mjs), [설정](../../../desktop/test/e2e/next-turn-settings.mjs), [압축](../../../desktop/test/e2e/thread-compaction.mjs). 승인·sandbox 선택은 [#2832](https://github.com/RefinedStone/codex-exec-loop/issues/2832)에 남음 | 소스 |
| 작업 기준·증거·호스트 검증·PR 결과 ([계획 24](../../plan/24-akra-evidence-harness.md)) | [완료 기준](../../reference/task-acceptance.md), [검증](../../reference/task-verification.md), [전달 관문](../../reference/task-delivery-evidence.md) | [기준 편집](../../../desktop/src/renderer/kernel/TaskCriteriaEditor.tsx), [검증](../../../desktop/src/renderer/kernel/TaskVerificationView.tsx), [결과](../../../desktop/src/renderer/kernel/TaskResultView.tsx) | [작업 확정](../../../desktop/test/e2e/task-authoring.mjs), [결과](../../../desktop/test/e2e/task-results.mjs), [면제](../../../desktop/test/e2e/task-criterion-waiver.mjs). 결합 인수는 [#2809](https://github.com/RefinedStone/codex-exec-loop/issues/2809) | 소스 |
| 이슈 초안·마스킹 게시·Vault 복원 ([#2414](https://github.com/RefinedStone/codex-exec-loop/issues/2414)) | [초안 API](../../../src/adapter/inbound/gateway/issue_draft.rs), [Vault 잠금 해제](../../reference/evidence-vault-unlock.md), [Windows 검증기](../../reference/issue-validator-windows.md) | [초안](../../../desktop/src/renderer/work/IssueDraftPanel.tsx), [생성](../../../desktop/src/renderer/work/IssueCreationPanel.tsx) | [초안](../../../desktop/test/e2e/issue-draft.mjs), [복원](../../../desktop/test/e2e/vault-restore.mjs). [실제 OS 근거](../../validation/vault-real-os-manual.md), [Windows #2724](https://github.com/RefinedStone/codex-exec-loop/issues/2724), [통합 #2705](https://github.com/RefinedStone/codex-exec-loop/issues/2705)는 별도 관문 | 소스 |
| Git 검토·그래프 ([계획 27](../../plan/27-akra-git-graph.md)) | [Gateway Git API](../../../src/adapter/inbound/gateway/git.rs) | [Git 그래프](../../../desktop/src/renderer/git/GitGraphPanel.tsx) | [그래프 화면 시험](../../../desktop/test/e2e/git-graph.mjs). cherry-pick [#2847](https://github.com/RefinedStone/codex-exec-loop/issues/2847), 보호 [#2848](https://github.com/RefinedStone/codex-exec-loop/issues/2848), 묶음 rebase [#2849](https://github.com/RefinedStone/codex-exec-loop/issues/2849), 조작 [#2851](https://github.com/RefinedStone/codex-exec-loop/issues/2851)·[#2852](https://github.com/RefinedStone/codex-exec-loop/issues/2852)가 남음 | 소스 |
| 브라우저 패널·에이전트 조작 ([계획 17 R9](../../plan/17-akra-desktop/roadmap/R9-agent-browser-and-screenshots.md)) | [main의 브라우저 bridge](../../../desktop/src/main/browserAgentService.ts)와 shared bridge 경계 | 실제 Desktop 브라우저 패널. blueprint는 별도 mock UI | [브라우저](../../../desktop/test/e2e/browser.mjs)·[자동화](../../../desktop/test/e2e/browserAutomation.mjs) 시험이 있음. 모든 외부 사이트 로그인이나 [#2846](https://github.com/RefinedStone/codex-exec-loop/issues/2846)의 실제 OS 항목 통과를 뜻하지 않음 | 소스 |
| 외부 업무 문서·MCP ([계획 21/22](../../plan/22-akra-work-context.md)) | [Context API](../../../src/adapter/inbound/gateway/context.rs), [MCP API](../../../src/adapter/inbound/gateway/mcp.rs) | [문서 패널](../../../desktop/src/renderer/work/DocumentsPanel.tsx), [MCP 쓰기 승인](../../../desktop/src/renderer/conversation/McpWriteApprovals.tsx) | [문서 화면 시험](../../../desktop/test/e2e/documents-panel.mjs). 실계정 검증은 [#2860](https://github.com/RefinedStone/codex-exec-loop/issues/2860), 최신성·인계는 [#2823–#2829](https://github.com/RefinedStone/codex-exec-loop/issues/2823) | 소스 |
| 로컬 진단 | [진단 계약](../../reference/desktop-local-diagnostics.md) | [진단 화면](../../../desktop/src/renderer/views/DiagnosticsDialog.tsx) | [fake 화면 시험](../../../desktop/test/e2e/diagnostics.mjs), 구현·검증 보고 [#2780](https://github.com/RefinedStone/codex-exec-loop/pull/2780) | 소스 |

`parallel.evidence_mode`의 기본값은 `observe`이다. 필수 증거가 없을 때 전달을 보류하려면 사용자가
`enforce`를 선택해야 한다. 현재 `GitDiffCheckV1`은 공백 오류·충돌 표식 검사이며 빌드나 기능시험
통과를 증명하지 않는다. [전달 계약](../../reference/task-delivery-evidence.md)을 따른다.

지원 엔진 동작은 [실행기 capability 계약](../../reference/runtime-capabilities.md)을 따른다.
미래 백엔드 계획이나 blueprint 화면을 실제 엔진·제품 UI로 취급하지 않는다. 계획 17의 오래된 P2/R9
단계표는 당시 계획 기록이며, 현재 구현 여부는 위 코드·근거 링크로 판단한다.

## 소스 Desktop 설치와 시작

`prerelease` 소스 checkout에 [rust-toolchain.toml](../../../rust-toolchain.toml)이 선언한 Rust,
Node/npm, 공식 Codex CLI와 `codex login`을 준비한다. 그 checkout에서 실행한다.

```sh
git switch prerelease
npm --prefix desktop ci
npm --prefix desktop run dev
```

[predev 스크립트](../../../desktop/scripts/prepare-dev.mjs)가 Electron을 준비하고 `akra-gateway`를
빌드한다. Desktop은 Gateway를 시작하거나 연결하며, 앱에서 작업공간을 선택한다.
`AKRA_GATEWAY_ENDPOINT` 또는 `AKRA_GATEWAY_BIN`을 명시하면 predev는 Gateway 빌드를 생략한다.
개발용 override는 [Desktop 지침](../../../desktop/AGENTS.md)에 있다.

위 절차는 스크립트와 대조한 소스 진입 경로다. 이 문서 갱신에서 앱 실행, 모델 턴, 설치판 업그레이드
검증을 수행한 것은 아니다. `npm install -g @refinedstone/akra`는 게시된 TUI·CLI launcher를 설치하며
Desktop을 설치하지 않는다.

## 배포·이름 전환의 경계

| 채널·변경 | 구현 또는 게시 상태 | 남은 검증·실행 |
| --- | --- | --- |
| 게시된 GitHub 릴리스 | 조사일 최신은 [v1.3.5](https://github.com/RefinedStone/codex-exec-loop/releases/tag/v1.3.5), 2026-06-20 게시, 제목 `codex-exec-loop-native v1.3.5` | 현재 브랜치의 리브랜딩·Desktop 변경 이전 배포판 |
| native/npm 릴리스 후보 | [#2742](https://github.com/RefinedStone/codex-exec-loop/pull/2742)가 binary·archive를 `akra-ade`로 전환. npm `@refinedstone/akra`와 명령 `akra`는 유지 | [#2589](https://github.com/RefinedStone/codex-exec-loop/issues/2589)의 대표 산출물·지원 환경 설치/업그레이드 확인. 새 버전 게시 완료는 아님 |
| Desktop | [manifest](../../../desktop/package.json): `0.0.0-development`, `productName: Akra ADE` | 설치 패키지·서명·업데이트 배포는 [R8](../../plan/17-akra-desktop/roadmap/R8-packaging-and-operations.md) |
| 데이터·인증 | [#2732](https://github.com/RefinedStone/codex-exec-loop/pull/2732), [#2737](https://github.com/RefinedStone/codex-exec-loop/pull/2737), [#2743](https://github.com/RefinedStone/codex-exec-loop/pull/2743), [#2782](https://github.com/RefinedStone/codex-exec-loop/pull/2782)로 N2 이전 구현 | [명명·호환 근거](../../reference/naming.md)는 fake/단위/macOS 임시 데이터 확인과 실제 OS 자격 증명 복호화·구 바이너리 업그레이드 미검증을 구분. [#2588](https://github.com/RefinedStone/codex-exec-loop/issues/2588)에서 추적 |
| 소스 이용조건·고지 | [L1 결정](../../plan/32-akra-ade-source-policy.md), [LICENSE](../../../LICENSE), [기여 정책](../../../CONTRIBUTING.md), [제3자 고지](../../../THIRD_PARTY_NOTICES/README.md)를 [#2739](https://github.com/RefinedStone/codex-exec-loop/pull/2739)로 적용 | L2 archive/npm staging 계약은 아래 CI에서 통과. 최종 release profile 설치·업그레이드는 R1에 남음. 외부 기여 조건은 미결정이며 현 문서 정책은 외부 PR을 받지 않음 |
| GitHub 저장소 이름 | 현재 활성 URL은 `RefinedStone/codex-exec-loop` | [G1 #2591](https://github.com/RefinedStone/codex-exec-loop/issues/2591)이 rename·redirect·remote·소비자 검증을 소유. 준비 전 새 URL로 다운로드 주소를 바꾸지 않음 |

L2 패키징 근거: `e3c181c5e`의 [Node staging 검사](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45909896894/job/140215516270)와
[Rust archive 검사](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45909896894/job/140215516446)가
통과했다. fixture archive·package를 생성해 프로젝트 LICENSE와 필수 고지 경로를 검사한다.
해당 커밋부터 조사 기준까지 패키징·이용조건·고지·관련 시험 파일 변경은 없다. 패키징 계약의
검증이며 게시된 바이너리나 설치판 업그레이드 검증을 뜻하지 않는다.

패키징·호환·복구의 상세 기준은 [배포 안내](release.md)와 [명명 계약](../../reference/naming.md)이다.
과거 태그·릴리스 자산·제3자 고지는 당시 정체성을 유지한다.

## 갱신 책임과 후속 범위

[D1 #2590](https://github.com/RefinedStone/codex-exec-loop/issues/2590)은 진입 문서의 오류 정정,
이 상태표와 한국어 번역을 담당한다. [#2481 R03](https://github.com/RefinedStone/codex-exec-loop/issues/2481)의
명백한 모순 정정·최소 상태 추적 범위를 수행한다. A01–A12 전체, R03의 향후 기계 검증 확장,
위에 연결한 미검증 제품·배포 작업까지 완료 처리하지 않는다.

기능 PR이 표의 동작을 바꾸면 해당 코드·검증 링크와 한영 문서를 함께 갱신한다. 기계적으로 확인할
사실은 기존 프로토콜 생성·CI 범위 검사에 둔다. 이 문장을 반복하는 시험은 추가하지 않으며,
시험 파일 링크를 통과 결과로 바꾸어 적지 않는다.
