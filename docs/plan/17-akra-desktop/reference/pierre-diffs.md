# 참고: `@pierre/diffs` 조사

상태: 조사 기록 · 조사일: 2026-11-12 · 관련 제안: [P-12](../03-decisions.md#p-12-코드-보기와-편집-엔진은-pierrediffs)

## 범위와 원칙

- 목적: Akra Desktop 의 diff 표시와 이후 파일 편집에 쓸 엔진을 고른다. 지금은 직접 만든 unified diff
  표시(`desktop/src/renderer/conversation/DiffView.tsx`)를 쓴다 ([R1](../roadmap/R1-change-review.md)).
- 출처: npm 레지스트리, 공개 저장소 `pierrecomputer/pierre`, 패키지의 `.d.ts` 파일, 로컬 시험.
- 이 문서는 Desktop 의 현재 동작을 설명하지 않는다. 아직 의존성을 추가하지 않았다.

## 1. 기본 정보

| 항목 | 내용 (2026-11-12) |
| --- | --- |
| 패키지 | `@pierre/diffs`. 만든 곳: The Pierre Computer Company. 문서: <https://diffs.com> |
| 저장소 | <https://github.com/pierrecomputer/pierre> (모노레포 `packages/diffs`). 별 약 6,500개, 매일 커밋 |
| 라이선스 | Apache-2.0. 의존하는 `@pierre/theme`, `@pierre/theming`, `@pierre/highlights` 도 Apache-2.0 |
| 버전 | 1.0.0 (2025-12-12) → 안정판 1.6.4 (2026-11-04). 1.7.0-beta 진행 중 |
| peer 의존성 | React `^18.3.1 \|\| ^19.0.0`. Desktop 은 React 19.3.1 이다 |
| 주요 의존성 | `shiki` (문법 색), `diff` 9.0.0, `hast-util-to-html`, `lru_map` |
| 크기 | 패키지 압축 해제 11 MB. `shiki` 와 `@shikijs/*` 를 합치면 약 17 MB 가 더해진다 |
| 언어 | 본체는 TypeScript 만 쓴다. 화면은 DOM 과 Shadow DOM 으로 그린다 |
| 소스 규모 | `packages/diffs/src` 242개 파일, 약 65,000줄 (그중 `src/editor` 약 18,000줄). 시험 코드 약 91,000줄 |

## 2. 공개 진입점

| 경로 | 내용 |
| --- | --- |
| `@pierre/diffs` | 바닐라 JS 컴포넌트, diff·patch 해석 함수 (`parsePatchFiles`, `getSingularPatch`, `parseDiffFromFile`) |
| `@pierre/diffs/react` | `PatchDiff`, `FileDiff`, `MultiFileDiff`, `File`, `CodeView`, `Virtualizer`, `WorkerPoolContextProvider` |
| `@pierre/diffs/edit` | 편집기: piece table 문서, 실행 취소 기록, 검색 패널, 괄호 짝, 마커, 키맵, 편집 예측 인터페이스 |
| `@pierre/diffs/worker` | 문법 색을 Web Worker 풀에서 계산한다 |
| `@pierre/diffs/ssr` | 서버에서 HTML 을 미리 만든다. Desktop 에는 필요 없다 |

이 계획에 관계있는 기능:

- 보기와 편집이 같은 컴포넌트이다. `edit` prop 을 주면 편집 모드가 된다.
- 좌우 비교와 인라인 비교, 줄 선택, 줄 주석(annotation) 프레임워크.
- hunk 단위 승인/거부 UI (`DiffAcceptRejectHunkConfig`: `accept`, `reject`, `both`).
- 병합 충돌 표시와 해결 (`UnresolvedFile`).
- 다음 편집 예측 인터페이스 (`EditPredictProvider.predict`).
- 가상 스크롤과 Worker 풀.

## 3. 문법 색 엔진

| 엔진 | 형태 | 비고 |
| --- | --- | --- |
| Shiki, JavaScript 정규식 | 순수 JS | 기본값 |
| Shiki, WebAssembly | Oniguruma (C) 를 WASM 으로 컴파일 | 선택 |
| `@pierre/highlights` | WebAssembly Text (`.wat`) 로 직접 작성. 152개 언어 | 선택. 문서 편집 단위로 다시 계산한다 |

## 4. Akra 데이터와의 연결 (로컬 시험)

`@pierre/diffs@1.6.4` 를 설치하고 Node 에서 `getSingularPatch` 를 실행했다.

| 입력 | 결과 |
| --- | --- |
| hunk 만 (`@@ -1,3 +1,3 @@ ...`) | 실패: `FileDiff: Provided patch must contain exactly 1 file diff` |
| 앞에 `--- a/<경로>`, `+++ b/<경로>` 를 붙임 | 성공. hunk 1개, `isPartial = true` |

- `fileChange` 아이템의 update 변경은 `@@` hunk 부터 시작한다 ([R1](../roadmap/R1-change-review.md) 진행 기록).
  그래서 표시 전에 파일 머리 두 줄을 붙여야 한다. add/delete 는 파일 내용 그대로 오므로 별도로 변환한다.
- 작업 트리 패널의 `gitFileDiff` 는 `git diff HEAD -- <경로>` 출력이라서 git 머리가 이미 있다.
- `isPartial` patch 는 보기만 된다. 편집하려면 `parseDiffFromFile(변경 전, 변경 후)` 에 파일 전체 텍스트를
  넣어야 한다. 변경 전은 `git show HEAD:<경로>`, 변경 후는 디스크 파일이다. 둘 다 main 의 git 서비스에서 읽는다.

## 5. 관찰

- 공식 Codex(ChatGPT) 데스크톱 앱 `26.1103` 의 번들에도 이 라이브러리가 있다. diff 표시와 파일 편집에 쓴다
  (번들 안의 `diffs-container` 요소와 편집기 문자열로 확인. [codex-desktop-app.md](codex-desktop-app.md) 와
  같은 정적 관찰 방식이다).

## 6. 위험

| 위험 | 내용 | 대응 |
| --- | --- | --- |
| API 변경 | 1.3 에서 1.6 까지 약 두 달 동안 마이너 버전이 네 번 올랐다. 동작 변경도 있었다 | 정확한 버전으로 고정한다. 올릴 때 변경 기록과 시험을 확인한다 |
| Worker 와 편집 | Worker 풀과 편집을 함께 쓸 때의 문법 색 갱신은 아직 확인하지 않았다 | 1단계는 Worker 를 끈다. 편집 단계에서 따로 확인한다 |
| 번들 크기 | shiki 를 포함하면 수십 MB 가 늘어난다 | 쓰는 언어만 불러온다. 크기는 1단계에서 실측한다 |
| 공급자 의존 | 한 회사가 관리한다 | Apache-2.0 이라서 필요하면 포크하거나 패치할 수 있다 |

## 7. 비교한 대안

| 대안 | 판단 |
| --- | --- |
| 지금의 직접 만든 표시 | 가볍다. 문법 색, 좌우 비교, 편집이 없다 |
| Monaco | VS Code 편집기. 편집 기능이 가장 많다. 무겁고 diff 는 전체 텍스트 두 개가 필요하다 |
| CodeMirror 6 + `@codemirror/merge` | 가볍고 확장하기 쉽다. hunk 승인/거부와 주석 UI 는 직접 만들어야 한다 |
| Rust 로 직접 구현 | Electron 에서는 화면을 DOM 으로 그리므로 성능 이득이 작다. 비용이 크다 |
