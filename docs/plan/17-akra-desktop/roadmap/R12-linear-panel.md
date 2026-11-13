# R12. Linear 패널

상태: 제안 (proposed) · 기준일: 2026-11-13 · 선행: R4 (업무 패널) · 규모: 약 1주 (+ 문서 인식기 0.5주)

## 목표

Linear 의 내 이슈를 Desktop 안에서 보고, 이슈 내용으로 새 스레드를 시작한다. Jira 패널([R4](R4-work-panels.md))과
같은 구조로 만든다.

- 나에게 배정된 열린 이슈를 팀·프로젝트·사이클별로 본다.
- 이슈 내용과 Linear 가 제안하는 브랜치 이름으로 새 스레드의 첫 메시지를 채운다. 보내기는 사용자가 한다.
- 대화에 나온 Linear 이슈 주소를 "관련 문서"에 보여 준다.

## 배경

- 지금 Desktop 은 GitHub(PR)과 Jira 를 연동한다. Linear 연동은 없다. 경쟁 제품 Orca 는 Linear 를 연동한다
  ([alternatives](../reference/alternatives.md)).
- GitHub Projects 안은 [R11](R11-github-projects.md) 이다. Linear 는 그 대안이다. 두 안을 비교한 뒤 하나를 먼저 한다
  (아래 "R11 과 비교").

## Linear 의 특징과 한계

아래는 기억 기준이다 (2026-11-13). 가격과 수치는 바뀌었을 수 있다. 구현 전에 Linear 공식 문서로 확인한다.

장점:

- 화면과 단축키가 빠르다. 이슈 처리 흐름(Triage, Backlog, Todo, In Progress, Done)이 정해져 있다.
- 사이클(스프린트), 프로젝트, 로드맵, 우선순위(0~4), 추정치가 기본 기능이다.
- GitHub 연동: 브랜치 이름이나 PR 제목·본문에 이슈 ID(예: `AKR-12`)가 있으면 PR 을 이슈에 연결하고, PR 상태에 따라
  이슈 상태를 바꾼다. GitHub Issues 와 양방향 동기화도 있다.
- 이슈마다 Linear 가 브랜치 이름(`branchName`)을 준다. 이 이름으로 브랜치를 만들면 PR 이 자동으로 연결된다.
- 공식 GraphQL API 와 공식 MCP 서버가 있다.

한계:

- GitHub 밖의 별도 도구이다. 계정, 워크스페이스, 연동 설정이 하나 더 필요하다.
- 무료 플랜에 활성 이슈 수와 팀 수의 상한이 있다.
- 이슈는 저장소에 속하지 않는다. 팀에 속한다. 그래서 "현재 저장소의 이슈" 필터는 기본 기능으로 되지 않는다
  (아래 "팀과 로컬 폴더").
- 공개 저장소의 외부 기여자는 Linear 이슈를 볼 수 없다. 외부에 보여야 하는 이슈는 GitHub Issues 와 동기화해야 한다.
- API 요청 수와 쿼리 복잡도에 상한이 있다. 패널은 열 때와 "새로 고침" 때만 읽으므로 문제가 되지 않는다.

## R11 과 비교

| 기준 | R11 GitHub Projects | R12 Linear |
| --- | --- | --- |
| 별도 도구 | 아니요 (GitHub 안) | 예 |
| 비용 | 공개 저장소 무료 | 무료 플랜 상한 있음 |
| 저장소별 필터 | 기본 (`repository.nameWithOwner`) | 팀과 폴더 연결이 필요 |
| 워크플로·사이클 | 약함 (선택 필드) | 강함 (기본 기능) |
| PR 연결 | `closes #123` | 이슈 ID, 브랜치 이름 |
| 외부 기여자 | 이슈를 본다 | 보지 못한다 |
| 자격 증명 | `gh` 로그인 (앱은 토큰 없음) | API 키를 `safeStorage` 에 저장 |
| 구현 크기 | `githubService.ts` 확장 | 새 서비스 (Jira 패널과 같은 크기) |

제안: 혼자 개발하는 공개 저장소에는 R11 이 더 맞다. 사이클과 처리 흐름이 필요해지면 R12 를 한다.

## 범위

| 항목 | 내용 |
| --- | --- |
| 연결 | 개인 API 키 입력, `viewer` 조회로 확인한 뒤에만 저장, 연결 해제 |
| 내 이슈 목록 | 나에게 배정되고 완료·취소가 아닌 이슈, 최근 수정 순 30개 |
| 필터 | 팀, 프로젝트, 현재 사이클 |
| 스레드로 넘기기 | 식별자, 제목, 설명, 최근 댓글 10개, 제안 브랜치 이름으로 첫 메시지를 채운다 |
| 문서 인식 | 대화의 Linear 이슈 주소를 "관련 문서"에 보여 준다 (Rust 인식기 하나) |

## 범위 밖

- 쓰기 기능 (상태 변경, 이슈 생성, 댓글 쓰기). 처음에는 읽기만 한다.
- OAuth 로그인. 개인 API 키로 시작한다.
- Linear 이슈의 "바뀜" 표시 (버전 제공자). 필요하면 다음 단계에서 `updatedAt` 으로 만든다.
- Codex 의 Linear 커넥터나 MCP 서버를 앱 패널에서 부르는 방식 (아래 "방식 선택").

## 설계

### 방식 선택

Jira 패널과 같이 앱이 Linear API 를 직접 부른다. Codex 의 커넥터를 app-server 로 부르는 방식은 고르지 않는다.
이유는 R4 의 Jira 결정과 같다: 데이터가 개인 ChatGPT 계정을 거치고, 커넥터 상태에 의존한다. 에이전트는 필요하면
Codex 의 Linear 커넥터나 MCP 서버를 그대로 쓴다.

### 데이터 흐름

```
renderer (업무 탭 > Linear)
   │  메시지 버스 요청 (토큰 없음)
   ▼
main: linearService.ts
   │  net.fetch POST https://api.linear.app/graphql
   │  Authorization: <API 키>   (safeStorage 로 복호화, main 안에서만)
   ▼
Linear GraphQL
   │  내 이슈 + 팀 + 프로젝트 + 사이클 + 상태
   ▼
main 이 파싱 → renderer 에 목록 반환
```

### 조회

```graphql
query($first: Int!) {
  viewer {
    assignedIssues(
      first: $first
      orderBy: updatedAt
      filter: { state: { type: { nin: ["completed", "canceled"] } } }
    ) {
      nodes {
        identifier title url priority updatedAt branchName
        state { name type }
        team { key name }
        project { name }
        cycle { number name }
      }
    }
  }
}
```

- 연결 확인은 `viewer { id name }` 이다.
- 넘기기 때만 그 이슈의 `description` (Markdown)과 `comments(last: 10)` 를 읽는다.
- 필터(팀, 프로젝트, 사이클)는 받은 목록에서 main 이 적용한다. 다시 호출하지 않는다.
- 필드 이름과 필터 문법은 구현 전에 Linear 스키마로 확인한다.

### 자격 증명 (D-14)

- 개인 API 키만 받는다. Linear 설정의 API 화면에서 사용자가 만든다.
- `viewer` 조회가 성공한 뒤에만 저장한다. `safeStorage` 로 암호화해서 userData 의 `linear-token.bin` 에만 둔다.
  암호화를 쓸 수 없으면 저장하지 않는다.
- renderer, 로그, 저장소, Akra 설정 파일에 키를 넣지 않는다.
- 회사 Linear 워크스페이스는 회사 데이터 정책을 먼저 확인한다. 회사 데이터를 개인 Codex 계정으로 보낼 때 특히 확인한다.

### 팀과 로컬 폴더

- Linear 이슈에는 저장소가 없다. 그래서 "스레드로 넘기기"는 다음 순서로 폴더를 정한다:
  1. 열린 스레드가 있으면 그 폴더.
  2. 없으면 폴더 선택 창.
- 다음 단계 후보: 설정에서 팀 키와 로컬 폴더를 연결한다 (예: `AKR` → `codex-exec-loop`). 처음 버전에는 넣지 않는다.

### 스레드로 넘기기 메시지

- 식별자와 제목, 설명, 최근 댓글 10개, 이슈 주소를 넣는다.
- `branchName` 을 넣고 "이 이름으로 브랜치를 만든다"고 적는다. 그러면 Linear 의 GitHub 연동이 PR 을 이슈에 연결한다.
- 댓글은 2,000자, 메시지는 20,000자에서 자른다 (R4 와 같다).

### 문서 인식기 (Rust)

- `src/domain/work_context.rs` 에 `LinearRecognizer` 하나를 더한다. 장부, Gateway, Desktop 은 바꾸지 않는다.
- 주소 형식: `https://linear.app/<워크스페이스>/issue/<팀 키>-<번호>/<slug>`. 키는 `<워크스페이스>/<식별자>` 이다.
  slug 는 키에 넣지 않는다 (제목이 바뀌면 slug 가 바뀐다).
- 단위 시험: slug 유무, 대소문자, 이슈가 아닌 주소(프로젝트, 뷰)는 건너뜀.

## 화면

- 자리: 오른쪽 패널 `업무` 탭 > 구분 버튼 "Linear" (GitHub, Jira 옆).
- 연결 전: API 키 입력 화면과 키를 만드는 위치 안내.
- 연결 후: 위에 팀·프로젝트·사이클 필터와 "새로 고침", "연결 해제". 목록은 상태별 묶음.
- 각 줄: 식별자, 제목, 우선순위, 프로젝트, 상태.
- 제목을 누르면 `linear.app` 주소만 기본 브라우저로 연다.
- 이슈마다 "스레드로 넘기기" 버튼.

## 검증

- 단위 시험 (vitest): 응답 파싱, 필터, 넘기기 메시지와 자르기, 연결 요청 검사, 키를 renderer 로 돌려주지 않음.
- 단위 시험 (Rust): `LinearRecognizer`.
- e2e (`npm run e2e`, 가짜 Gateway, 숨긴 창, 가짜 Linear GraphQL 서버, 임시 userData): 잘못된 키는 저장하지 않음,
  연결 후 목록, 필터, 넘기기로 입력창만 채워지고 보내지 않음, 키 파일이 평문이 아님.
- 실제 확인 (읽기만, 턴 없음): 실제 Linear 워크스페이스에서 내 이슈를 읽는다.
- 게이트: `cd desktop && npm run check && npx vitest run && npm run e2e`. Rust 를 바꾸는 PR 은
  `node scripts/agent-plan.mjs` 가 출력한 검사.

## 단계

1. Linear 워크스페이스와 팀을 만들고, 개인 API 키를 만든다. 필요하면 GitHub 연동을 켠다 (사용자 작업).
2. main: `linearService.ts` (연결, 저장, 조회, 파싱, 필터). 단위 시험.
3. renderer: Linear 구분 버튼, 연결 화면, 목록, 필터. e2e.
4. 스레드로 넘기기. e2e.
5. `LinearRecognizer` (Rust). 단위 시험.
6. 실제 워크스페이스로 확인하고 이 문서와 README 표의 상태를 고친다. 끝난 동작은 `docs/reference/` 로 옮긴다.

1~4 는 PR 하나, 5 는 PR 하나로 한다 (Desktop 과 Rust 를 나눈다).

## 미결

- R11 과 R12 중 무엇을 먼저 할지 정한다. 둘 다 할지도 정한다.
- 개인 워크스페이스만 쓸지, 회사 워크스페이스도 연결할지 정한다 (D-14).
- 팀 키와 로컬 폴더 연결을 설정으로 둘지 정한다.
- 쓰기 기능 (상태 변경)을 다음 단계로 할지 정한다.
