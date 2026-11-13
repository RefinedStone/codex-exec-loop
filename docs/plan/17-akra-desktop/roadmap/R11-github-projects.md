# R11. GitHub Projects 패널

상태: 제안 (proposed) · 기준일: 2026-11-13 · 선행: R4 (GitHub 패널) · 규모: 약 1주

## 목표

Akra 작업을 GitHub Issues 와 GitHub Projects 로 관리한다. Desktop 은 Project 항목을 저장소별로 보여 주고,
항목 내용으로 새 스레드를 시작한다.

- 한 Project 에 여러 저장소의 이슈를 모으고, 앱에서 저장소별로 필터한다.
- 이슈 내용으로 알맞은 로컬 폴더에서 새 스레드를 연다. 보내기는 사용자가 한다.

## 배경

- 지금 GitHub 패널은 내 PR 과 리뷰 요청만 보여 준다 ([R4](R4-work-panels.md)). 이슈와 Project 는 보여 주지 않는다.
- Akra 는 공개 저장소이고 개발자 한 명이 개발한다. 앞으로 관련 라이브러리를 새 저장소로 나눌 수 있다.
- GitHub Project 는 저장소가 아니라 계정(또는 조직)에 속한다. 그래서 한 Project 가 여러 저장소의 이슈와 PR 을
  담는다. 새 저장소도 같은 Project 에 연결하면 같은 보드에서 관리한다.

## 현재 연동 상태 (2026-11-13)

| 도구 | 상태 | 할 수 있는 것 |
| --- | --- | --- |
| GitHub PR | 있음 | 내 PR, 리뷰 요청, CI 상태. 리뷰 코멘트로 새 스레드 입력창 채우기 (`desktop/src/main/githubService.ts`) |
| GitHub 이슈 | 인식만 | 패널 목록은 없다. 대화의 이슈 URL 을 "관련 문서"에 보여 주고, 바뀌면 "바뀜"으로 표시한다 |
| GitHub Projects | 없음 | - |
| Jira | 있음 | 내 티켓 목록, 티켓으로 새 스레드 시작 (`desktop/src/main/jiraService.ts`) |
| Linear | 없음 | [alternatives](../reference/alternatives.md) 의 비교 항목에만 있다 |

## GitHub Projects 를 고른 이유와 한계

장점:

- 이슈와 PR 이 같은 곳에 있다. PR 본문의 `closes #123` 으로 머지 때 이슈가 닫힌다.
- 한 Project 에 여러 저장소를 담는다. Board, Table, Roadmap 보기가 있다.
- 하위 이슈, 이슈 유형, 의존 관계(blocked by)를 쓴다.
- 공개 저장소에서는 비용이 없다. 별도 도구와 동기화가 필요 없다.
- Akra 의 work context 가 이미 GitHub 이슈 URL 을 인식하고 변경을 표시한다.

한계 (기억 기준. 수치는 바뀌었을 수 있다):

- 워크플로 규칙이 약하다. 상태 전이 규칙, 필수 필드, 승인 단계가 없다. Status 는 일반 선택 필드이다.
- 기본 자동화가 적다 (자동 추가, 자동 보관, 닫힘·머지 때 상태 변경). 그 외는 GitHub Actions 와 GraphQL 로 만든다.
- 보고서가 단순하다. 번다운, 속도, SLA, 시간 기록이 약하거나 없다. 스토리 포인트는 숫자 필드로 만든다.
- 사용자 정의 필드 수와 항목 수에 상한이 있다.
- 필드 변경은 알림을 거의 보내지 않는다.
- API 는 주로 GraphQL (`ProjectV2`) 이다. `gh` 토큰에 `project` 권한이 필요하다.
- 모든 참여자에게 GitHub 계정이 필요하다. 비개발 인원에게는 맞지 않는다.

Linear 는 화면과 사이클·트리아지 기능이 좋다. 그러나 별도 도구이고 GitHub 과 동기화가 필요하다. 지금 범위에서
제외한다 (아래 "범위 밖").

## 범위

| 항목 | 내용 |
| --- | --- |
| Project 항목 보기 | 설정한 Project 하나의 이슈·PR 항목. 제목, 번호, 저장소, 상태(열림/닫힘), Project 의 Status 값 |
| 저장소 필터 | "현재 저장소 / 전체 / 저장소별". 기본값은 현재 스레드의 저장소 |
| 스레드로 넘기기 | 이슈의 제목, 본문, 댓글로 새 스레드의 첫 메시지를 채운다. 자동으로 보내지 않는다 |
| 설정 | Project 소유자와 번호 (예: `RefinedStone/1`) |

## 범위 밖

- Linear 패널. 따로 계획한다: [R12](R12-linear-panel.md).
- 쓰기 기능 (Status 변경, 이슈 생성, 담당자 지정). 처음에는 읽기만 한다.
- Project 여러 개 동시 보기.
- 데스크톱 알림.

## 설계

### 데이터 흐름

```
renderer (업무 탭 > GitHub > Projects)
   │  메시지 버스 요청
   ▼
main: githubService.ts
   │  gh api graphql  (셸 없음, 홈 폴더, 대화형 질문 없음 — R4 와 같은 runGh)
   ▼
GitHub GraphQL (ProjectV2)
   │  항목 + repository.nameWithOwner + Status
   ▼
main 이 저장소 필터 적용 → renderer 에 목록 반환
```

### 조회 (방법 1: 모두 읽고 앱에서 필터)

```graphql
query($owner: String!, $number: Int!, $cursor: String) {
  user(login: $owner) {
    projectV2(number: $number) {
      title
      items(first: 100, after: $cursor) {
        pageInfo { hasNextPage endCursor }
        nodes {
          fieldValueByName(name: "Status") {
            ... on ProjectV2ItemFieldSingleSelectValue { name }
          }
          content {
            ... on Issue       { number title url state updatedAt repository { nameWithOwner } }
            ... on PullRequest { number title url state updatedAt repository { nameWithOwner } }
          }
        }
      }
    }
  }
}
```

- 한 번 읽으면 필터를 바꿀 때 다시 호출하지 않는다. 열 때와 "새로 고침" 때만 읽는다 (R4 와 같다).
- 100개보다 많으면 `endCursor` 로 다음 페이지를 읽는다. 상한을 둔다 (예: 500개).
- `content` 가 없는 항목(draft issue)은 "초안"으로 보여 주고 넘기기 버튼을 두지 않는다.
- 조직 소유 Project 는 `user` 대신 `organization` 으로 읽는다. 설정에 소유자 종류를 둔다.

검토한 다른 방법: 검색 API (`project:RefinedStone/1 repo:<owner/name>`). 필요한 항목만 받는다. 그러나 Status 를
읽으려면 항목마다 `projectItems` 를 따로 조회해야 하고, 검색 색인이 늦을 수 있다. 그래서 고르지 않았다.

### 저장소 필터

- 현재 저장소: 열린 스레드의 폴더에서 `git remote get-url origin` 을 읽고 `owner/name` 으로 바꾼다.
  `gitPublish.ts` 의 원격 읽기와 같은 방식이다.
- 필터 값은 항목의 `repository.nameWithOwner` 와 비교한다. 대소문자는 무시한다.
- 원격이 없거나 GitHub 이 아니면 "전체"로 시작한다.

### 저장소와 로컬 폴더 연결

- "스레드로 넘기기"는 그 이슈의 저장소에 해당하는 로컬 폴더에서 새 스레드를 연다.
- 현재 저장소의 이슈: 현재 스레드의 폴더를 쓴다.
- 다른 저장소의 이슈: 사이드바의 작업 폴더(D-16) 중 원격이 같은 폴더를 찾는다. 없으면 폴더 선택 창을 연다.
- 고른 연결은 저장하지 않는다 (처음 버전). 필요하면 나중에 설정으로 둔다.

### 권한과 자격 증명

- `gh` 로그인만 쓴다. 앱은 토큰을 읽거나 저장하지 않는다 (D-14, R4 와 같다).
- `project` 권한이 없으면 GraphQL 이 실패한다. 패널은 `gh auth refresh -s project` 안내를 보여 준다.
- 회사 계정의 Project 는 다루지 않는다 (D-14).

## 화면

- 자리: 오른쪽 패널 `업무` 탭 > GitHub > 새 구분 버튼 "Projects" (기존 "PR" 옆).
- 위: Project 이름, 저장소 필터 선택, "새로 고침".
- 목록: Status 별 묶음. 각 줄은 저장소 이름(짧게), `#번호`, 제목, 상태.
- 제목을 누르면 github.com 주소만 기본 브라우저로 연다 (R4 와 같다).
- 열린 이슈에 "스레드로 넘기기" 버튼.
- Project 를 설정하지 않았으면 설정 화면을 보여 준다.

## 검증

- 단위 시험 (vitest): GraphQL 응답 파싱, draft 항목, 페이지 넘기기, 저장소 필터, 원격 URL → `owner/name`
  변환 (https, ssh 형식), 넘기기 메시지 자르기.
- e2e (`npm run e2e`, 가짜 Gateway, 숨긴 창): 패널 열기, 필터 바꾸기, 넘기기로 입력창이 채워지고 보내지 않음.
- 실제 확인 (읽기만, 턴 없음): 실제 `gh` 로 `RefinedStone` Project 를 읽고, 저장소 두 개의 항목이 필터대로
  나뉘는지 본다.
- 게이트: `cd desktop && npm run check && npx vitest run && npm run e2e`.

## 단계

1. `RefinedStone` 계정에 Project 를 만들고 `codex-exec-loop` 저장소를 연결한다 (사용자 작업).
2. `gh auth refresh -s project` 로 권한을 더한다 (사용자 작업).
3. main: Project 조회, 파싱, 필터 (`githubService.ts` 확장). 단위 시험.
4. renderer: Projects 구분 버튼, 목록, 필터. e2e.
5. 스레드로 넘기기와 저장소-폴더 연결. e2e.
6. 실제 gh 로 확인하고 이 문서와 README 표의 상태를 고친다. 끝난 동작은 `docs/reference/` 로 옮긴다.

1~6 은 PR 하나로 한다.

## 미결

- Project 번호를 어디에 저장할지 정한다 (Desktop 설정 파일 또는 Akra 설정).
- Status 외에 보여 줄 Project 필드가 있는지 정한다 (예: Iteration, Priority).
- 쓰기 기능 (Status 변경)을 다음 단계로 할지 정한다.
- R11 과 [R12](R12-linear-panel.md) 중 무엇을 먼저 할지 정한다.
