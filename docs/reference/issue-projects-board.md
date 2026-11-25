# GitHub Projects 보드 연동

관련: [#2565](https://github.com/RefinedStone/codex-exec-loop/issues/2565),
[#2435](https://github.com/RefinedStone/codex-exec-loop/issues/2435),
[이슈 자동화 피드백](current-product.md#issue-automation-feedback),
[plan 28 §13](../plan/28-akra-issue-automation.md#13-수집-provider-와-자격-증명).

## 범위

GitHub 이슈 자동화는 Projects (v2) 보드의 단일 선택 필드 하나(보통 `Status`)를 두 곳에 쓴다.

- 선택 조건: 정한 열(선택지)에 있는 이슈만 선택한다.
- 진행 반환: 정책이 켠 때만 실행 단계에 맞는 선택지로 항목의 필드를 바꾼다.

보드 화면, 보드에 이슈 넣기, 여러 필드 편집은 이 범위가 아니다. Jira 바인딩을 고른 저장소에서는
쓰지 않는다.

## 설정

보드와 필드, 선택지는 표시 이름이 아닌 안정 node ID 로 적는다. 이름은 바뀔 수 있다.

```toml
# 저장소 .akra/issue-automation.toml 또는 사용자 $AKRA_HOME/issue-automation.toml
[binding.github.board]
project_id = "PVT_..."            # Projects (v2) node ID
field_id = "PVTSSF_..."           # 단일 선택 필드 node ID
select_option_ids = ["f75ad846"]  # 이 열의 이슈만 선택한다. 비면 열 조건이 없다
```

```toml
# 사용자 $AKRA_HOME/issue-automation.toml 에서만
[feedback]
board_field = true                # 기본값 false

[binding.github.board]
project_id = "PVT_..."
field_id = "PVTSSF_..."

[binding.github.board.progress]   # 없는 단계는 쓰지 않는다
running = "47fc9ee4"
in_review = "98236657"
done = "e1e1a1b2"
```

- 표 `binding.github.board` 는 한 단위로 덮는다. 저장소 파일이 있으면 저장소 파일의 표를 쓴다.
- `feedback.board_field` 는 다른 `feedback` 값과 같이 사용자 파일만 켠다. 저장소 파일은 끌 수만 있다.
- `progress` 는 사용자 파일에서만 온다. 저장소 파일에 있으면 `binding.github.board.progress:
  user_scope_only` 로 정책을 막는다. 유효 보드가 사용자 파일과 같은 `project_id`·`field_id` 일 때만
  사용자 파일의 `progress` 를 붙인다. 저장소 파일이 다른 보드를 고르면 진행 쓰기는 없다.
- 보드 설정이 없거나 `board_field` 가 꺼진 정책은 정규 바이트와 정책 판이 바뀌지 않는다.

## 선택 조건

- 열 조건은 라벨 선택 조건과 AND 이다. 보드만으로 고르려면 `select.mode = "all_open"` 을 쓴다.
- 열 조건이 있을 때만 수집이 이슈마다 `gh api graphql` 로 그 이슈의 보드 항목을 한 번 읽는다.
  읽은 값은 관찰의 `board` 에 남는다. 보드 열이 바뀌면 판정을 다시 한다.
- 다른 열, 빈 값, 보드에 없는 이슈는 `selection_not_matched` 이다.
- 읽지 못한 보드는 불일치가 아니다. 판정은 `unknown` 이고 이유 코드는 `board_unavailable` 이다.
  이 이슈는 시작하지 않는다. 정책의 보드·필드와 다른 관찰, 보드 사실이 없는 과거 관찰도 같다.

### `read:project` scope 가 없는 로그인

`gh` 로그인에 `read:project` (쓰기는 `project`) scope 가 없으면 GitHub 은 보드 읽기를 거절한다.
Akra 는 이것을 빈 보드나 "열 불일치" 로 바꾸지 않는다. 보수적인 기본값을 쓴다.

- 선택: `board_unavailable` 이유 코드로 `unknown` 이다. 이미 추적 중인 이슈의 상태를 철회하지 않고,
  새로 시작하지도 않는다.
- 쓰기: 보드를 읽지 못하면 의도를 기록하지 않는다. 다음 조정에서 다시 본다.
- 해결: `gh auth refresh -s read:project` (쓰기는 `-s project`) 뒤 다시 동기화한다.

## 진행 쓰기

- 목표: 실행 상태 `Running` → `progress.running`, `InReview` → `progress.in_review`,
  `Done` → `progress.done`. 대기, 실패, 취소는 쓰지 않는다.
- 허가: 보드 쓰기는 본문·댓글·라벨 쓰기와 다른 허가이다. `feedback.board_field` 만 켠다.
  보내는 요청은 `IssueWriteOperation::BoardField` 승인을 가진다. 본문 쓰기 승인으로는 보드 adapter 가
  쓰지 않는다. adapter 는 쓰기 직전에 대상 항목·필드·선택지가 그 이슈의 것인지 원격에서 다시 읽는다.
- 일지: 보드 쓰기도 기존 피드백 일지(`issue_feedback_intents`)를 지난다. `Recorded → Sent` claim
  성공자만 한 번 보낸다. 보내기 직전에 정책 판, 허가, 지금 단계의 목표를 다시 확인한다.
  응답을 잃으면 결과 모름이다. 복구는 읽기만 하고 다시 보내지 않는다. 결과 모름이 남은 이슈는 새
  피드백을 보내지 않는다. 확인은 `issue_feedback_posted` (`board_field_set`) 사건을 남긴다.
- 사람의 변경: 첫 쓰기는 현재 값이 `select_option_ids` 의 열일 때만 한다 (열 조건이 없으면 현재 값과
  무관하다). 다음 쓰기는 현재 값이 마지막 자기 쓰기의 값일 때만 한다. 사람이 열을 바꿨으면 다음
  조정은 그 값을 덮지 않는다. 보드에 없는 이슈를 보드에 넣지 않는다.
- 자기 쓰기 숨김: 확인된 자기 쓰기가 남긴 값은 판정 입력에서 첫 쓰기 전의 값으로 되돌린다. 자기 쓰기로
  선택 조건이 바뀌지 않는다. 사람이 바꾼 값은 그대로 보인다. 결과 모름인 보드 쓰기가 있으면 그 이슈의
  관찰을 미룬다.

## 한계

- 확인은 다시 읽은 값이 목표와 같은가이다. 같은 계정의 사람이 같은 값을 고른 경우와 구분하지 못한다.
- 현재 값 확인과 원격 쓰기 사이의 짧은 시간에 사람이 바꾼 값은 덮을 수 있다. 다음 조정부터는 존중한다.
- 보드 읽기는 한 쪽만 읽는다 (보드 20개, 필드 50개, 값 50개). 넘으면 `board_unavailable` 이다.
- 이슈마다 보드 읽기가 한 번 더 있다. 열 조건이 없는 정책은 읽지 않는다.
