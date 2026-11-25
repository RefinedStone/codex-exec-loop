# Linear 이슈 자동화

관련: [#2564](https://github.com/RefinedStone/codex-exec-loop/issues/2564),
[Jira Cloud 이슈 자동화](issue-jira-cloud.md), [정책 병합](../validation/issue-automation-policy.md),
[plan 28 §13](../plan/28-akra-issue-automation.md#13-수집-provider-와-자격-증명).

## 지원 범위와 연결

이 경로는 Linear 팀의 이슈를 읽고, 사람이 승인한 작업을 기존 병렬 실행·PR 전달·피드백 경로에
연결한다. Jira Cloud 경로와 같은 계약을 쓴다.

- Gateway가 소유한 현재 프로필의 Linear 연결을 사용한다 (§13.1, #2562). 토큰은 TOML에 넣지 않는다.
- Linear GraphQL을 직접 호출한다. 고정 문서만 보내며 호출자는 URL, 쿼리, 헤더를 고르지 않는다.
- 작업 시작은 Propose와 Native Human 확인을 사용한다. 파일이나 런타임 선택이 Gated/Auto여도
  Linear의 유효 상한은 Propose다. Linear API로 연결된 PR의 부재를 확인할 수 없기 때문이다
  (`linear_pull_request_lookup_unsupported`).
- 라벨 쓰기와 `close.when = "provider_link"`는 지원하지 않는다.

| 값 | 의미 |
| --- | --- |
| `team_id` | Linear 팀의 안정 ID(UUID). 팀 key(`ENG`)나 이름이 아니다. |
| `expected_viewer_id` | 연결한 사용자의 안정 `viewer.id`. 이메일이나 표시 이름으로 대체하지 않는다. |
| `server_id` | 항상 `linear.app`이다. 신뢰 작성자의 `server_id`도 이 값을 쓴다. |
| 이슈 신원 | 이슈의 안정 `id`(UUID). 표시 key(`ENG-7`)는 팀을 옮기면 바뀌므로 신원으로 쓰지 않는다. |

Linear 이슈에는 저장소가 없다. 저장소 파일의 `[binding.linear]`가 팀을 그 저장소에 결속한다.
바인딩이 없는 팀의 이슈는 읽지도 추적하지도 않는다. 한 프로필에서 같은 안정 이슈를 다른 저장소가
이미 소유하면 뒤의 저장소는 새 자동 작업을 만들지 않는다(private SQLite 소유 장부, Jira와 공유).
한 저장소의 자동화 설정은 한 출처만 선택한다. `binding.jira`와 `binding.linear`를 함께 쓰면 정책 오류다.

## 기본 구성 예제

아래 ID는 형식 예시다. 실제 연결에서 확인한 값으로 바꾼다.

사용자 파일 `$AKRA_HOME/issue-automation.toml`:

```toml
[binding.linear]
team_id = "6f1c1a52-0000-4000-8000-00000000a001"
expected_viewer_id = "6f1c1a52-0000-4000-8000-00000000b001"
mode = "propose"

[authority]
autonomy_cap = "propose"

[select]
mode = "all_open"
trusted_actors = [
  { provider = "linear", server_id = "linear.app", actor_id = "6f1c1a52-0000-4000-8000-00000000b002" }
]

[feedback]
comment = true
labels = false
transition = false
```

대상 저장소의 `.akra/issue-automation.toml`:

```toml
[binding.linear]
mode = "propose"

[close]
when = "never"
```

저장소 파일에 `[binding.linear]`가 없으면 사용자 파일의 팀 좌표를 그 저장소에 적용하지 않는다.
저장소 파일이 팀 좌표를 적으면 `team_id`와 `expected_viewer_id`를 모두 적어야 한다.

## 수집

- **발견.** 바인딩한 팀의 열린 이슈(`state.type`이 `completed`, `canceled`, `duplicate`가 아닌 것)를
  `updatedAt` 창으로 읽는다. 필터는 `team.id.eq`, `updatedAt.gte/lt`, `state.type.nin`이다.
- **페이지와 증분.** Linear의 `first/after`와 `pageInfo { hasNextPage endCursor }`는 한 창 안의 위치다.
  증분 기준(watermark)은 창의 마지막 페이지를 저장한 뒤에만 나아간다. 중간 페이지 실패는 커서를
  나아가게 하지 않는다. 다음 창은 10분을 겹쳐 읽고 하루마다 전체를 다시 읽는다. 같은 `updatedAt`의
  이슈가 페이지 경계에 걸려도 안정 ID로 다시 처리·중복 제거한다. 거절된 continuation은 같은 창을
  처음부터 다시 읽으며 페이지를 건너뛰지 않는다.
- **재동기화.** 추적 중인 이슈를 안정 ID로 한 건씩 읽는다. 선택 조건과 무관하다. 다른 팀으로 옮겨진
  이슈, 다른 ID, 읽기 실패는 `unknown`이다. 닫힘으로 보지 않는다.
- 매 작업에서 `viewer.id`와 팀을 다시 확인하고, 읽기 앞뒤로 연결 revision을 확인한다.
- 라벨은 선택 조건 판정에만 쓴다. 라벨을 누가 붙였는지는 읽지 않는다(`Unavailable`).
- 1 MiB를 넘는 본문은 공유하지 않고 `needs_info`로 남긴다.

## 브랜치, PR, 댓글, 상태

Linear가 준 `branchName`은 작업 지시의 `branch_hint`다. 슬롯 브랜치의 읽기 쉬운 부분으로만 쓴다.
`operator/eng-7-fix-login`은 `akra-agent/slot-1/operator-eng-7-fix-login-<lease>`가 된다. 슬롯 접두사,
lease 식별자, 충돌 번호는 기존 allocator가 정한다. `branchName`을 기존 슬롯·lease·worktree의 소유 근거로
쓰지 않는다. `branchName`이 바뀌면 승인 판이 바뀐다.

PR 제목은 `[ENG-7] supersession: …`, 본문은 `Issue: https://linear.app/<workspace>/issue/ENG-7`을
포함한다. Linear 작업 참조의 `close_on_merge`는 false이며 GitHub의 `Closes` 키워드를 붙이지 않는다.

상태 댓글은 기존 영속 send journal을 사용한다. Linear에는 댓글 property가 없다. 소유 확인은 작성자
(`user.id`), 이슈, 의도 표식을 포함한 Markdown 본문 hash의 정확한 일치다. Linear가 본문을 다르게
저장하면 소유로 받아들이지 않고 사람 확인으로 남긴다. 응답 유실 뒤에는 읽기로 결과를 확인하며 같은
mutation을 자동 재전송하지 않는다.

## 선택적인 완료 상태 변경

기본은 꺼져 있다. 완료 상태 변경을 허용하려면 사용자 파일에 팀 좌표와 상태 ID를 함께 허락한다.
상태 이름("Done")을 보고 ID를 추측하지 않는다.

```toml
[binding.linear]
team_id = "6f1c1a52-0000-4000-8000-00000000a001"
expected_viewer_id = "6f1c1a52-0000-4000-8000-00000000b001"
mode = "propose"

[binding.linear.close_state]
from_state_ids = ["6f1c1a52-0000-4000-8000-00000000d001"]
to_state_id = "6f1c1a52-0000-4000-8000-00000000d002"

[feedback]
comment = true
transition = true

[close]
when = "integration"
```

저장소 파일은 같은 팀에서 시작 상태 집합을 좁힐 수만 있다. 다른 팀·계정·목표 상태로 옮길 수 없다.
전송 직전에 현재 상태가 허락한 시작 상태인지, 목표 상태가 같은 팀의 `completed` 유형인지 확인한다.
`issueUpdate` 응답의 상태 ID와 이슈 기록(`history`)의 새 상태 변경 하나로 결과를 확인한다.
Linear에는 조건부 상태 변경 CAS가 없으므로 마지막 읽기 이후의 동시 변경을 완전히 막는다고 보장하지
않는다.

## 쓰기 경계와 토큰 교체

댓글·상태 변경은 실제 HTTP 전송 직전에 런타임 자율 선택을 포함한 실행 정책과 공유 정책의 현재 버전을
의도에 저장된 버전과 다시 비교한다(Jira와 같은 send-boundary 검사). 이미 준비한 쓰기는 원래의 정확한
연결 참조에 묶이며 새 토큰으로 늦게 보내지 않는다.

## 공식 API 근거와 MCP 비교

- [GraphQL API](https://linear.app/developers/graphql): 엔드포인트 `https://api.linear.app/graphql`,
  개인 API 키는 `Authorization: <API_KEY>`.
- [Pagination](https://linear.app/developers/pagination): `first/after`, `pageInfo`, `orderBy: updatedAt`.
- [Filtering](https://linear.app/developers/filtering): `IssueFilter`의 `team`, `updatedAt`, `state.type`.
- [Rate limiting](https://linear.app/developers/rate-limiting): API 키 요청·복잡도 한도. 429는 `unknown`이다.
- [공개 스키마](https://github.com/linear/linear/blob/master/packages/sdk/src/schema.graphql):
  `Issue.branchName: String!`, `Issue.description: String`, `WorkflowState.type`, `IssueHistory.fromStateId/toStateId/actorId`,
  `commentCreate`, `commentUpdate`, `issueUpdate(input: { stateId })`, `workflowState(id:)`.

공식 MCP 서버 대신 GraphQL을 직접 쓴다. MCP는 서버마다 도구 모양이 다르고, 쓰기 일지·한 번 전송
admission·응답 유실 뒤 읽기 확인을 Akra가 직접 다루기 어렵다(§13 선택 (c)). Jira 경로와 같은 Gateway
자격 증명·send-boundary 계약을 유지한다.

위 문서는 2026-11-25 구현 검토에 사용했다. 실제 Linear 계정이나 모델을 호출한 검증은 하지 않았다.
시험은 가짜 GraphQL 응답을 사용한다.
