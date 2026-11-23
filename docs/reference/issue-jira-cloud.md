# Jira Cloud 이슈 자동화

관련: [#2563](https://github.com/RefinedStone/codex-exec-loop/issues/2563),
[정책 병합](../validation/issue-automation-policy.md), [사람 확인](../validation/issue-human-confirmation.md),
[바인딩별 Pause / Resume](issue-binding-pause.md).

## 지원 범위와 연결

이 경로는 Jira Cloud의 기존 이슈를 읽고, 사람이 승인한 작업을 기존 병렬 실행·PR 전달·피드백
경로에 연결한다. Jira Server/Data Center는 이 자동화 경로의 대상이 아니다. 작업 시작은
Propose와 Native Human 확인을 사용한다. 파일이나 이전 런타임 선택이 Gated/Auto여도 Jira의
유효 상한은 Propose이며 자동 Start, timeout default/arbiter Start, Auto 전달을 허용하지 않는다.

Gateway가 소유한 현재 프로필의 Jira 연결을 사용한다. 자격 증명은 기존 공급자 연결의 private
경계에서 관리하며 아래 TOML에 토큰을 넣지 않는다. 다음 값은 서로 다른 신원이다.

| 값 | 의미 |
| --- | --- |
| `site_url` | 연결된 Cloud site의 정규 HTTPS 주소. 끝의 `/`를 생략한다. |
| `cloud_id` | 해당 site를 조회해 확인한 안정 Cloud ID. 호스트 이름으로 만들지 않는다. |
| `project_id` | 숫자 형태의 Jira project ID. 화면의 project key와 다르다. |
| `expected_account_id` | 연결한 사용자의 안정 `accountId`. 이메일이나 표시 이름으로 대체하지 않는다. |
| 저장소의 `.akra/issue-automation.toml` | 그 Cloud/project/account를 현재 canonical 저장소에 결속한다. 별도의 추측 가능한 저장소 이름 필드는 없다. |

수집·재조회는 위 site/account와 실제 Cloud/project/issue ID를 대조한다. Jira key는 표시·브랜치·
PR 참조이고, 안정 이슈 신원은 Cloud ID + project ID + issue ID다. 한 프로필에서 같은 안정
이슈를 다른 저장소가 이미 소유하면 뒤의 저장소는 새 자동 작업을 만들지 않는다. 한 저장소의
자동화 설정은 한 공급자 바인딩을 선택하며, `binding.jira`가 있으면 GitHub 이슈 수집 모드로
자동 전환하지 않는다. GitHub를 통한 코드 PR 전달은 별도 기존 기능이다.

## 기본 구성 예제

아래 ID와 주소는 형식을 보여 주는 예시다. 실제 연결에서 확인한 값으로 바꾼다. 신뢰 작성자의
`accountId`는 연결 계정과 같을 필요가 없다. `all_open`은 JQL에 맞는 미종료 이슈를 후보로
선택하며, 신뢰 작성자·담당자·본문·사람 승인 검사를 생략하지 않는다.

사용자 파일 `$AKRA_HOME/issue-automation.toml`은 허용할 계정과 외부 쓰기의 상한을 정한다.
`AKRA_HOME`을 설정하지 않았으면 기본 홈의 `.akra/issue-automation.toml`을 사용한다.

```toml
[authority]
autonomy_cap = "propose"

[select]
trusted_actors = [
  { provider = "jira", server_id = "cloud-123", actor_id = "account-author-123" }
]

[feedback]
comment = true
labels = false
transition = false
```

대상 저장소의 `.akra/issue-automation.toml`은 전체 출처 좌표와 수집 필터를 지정한다.

```toml
[binding.jira]
site_url = "https://example.atlassian.net"
cloud_id = "cloud-123"
project_id = "10001"
expected_account_id = "account-operator-123"
jql = "statusCategory != Done"
mode = "propose"

[select]
mode = "all_open"

[close]
when = "never"
```

JQL에는 필터만 적는다. `ORDER BY`는 허용하지 않으며 adapter가 고정 project와 `updated` 기반
정렬·페이지 처리를 붙인다. 일부 출처 좌표를 지정한 파일은 site/cloud/project/account/JQL을
모두 제공해야 한다. 이미 사용자 파일에 전체 좌표가 있으면 저장소 파일은 mode만 제한할 수 있다.
사용자 파일의 신뢰 목록에 없는 작성자를 저장소 파일의 목록만으로 추가할 수 없고, 저장소의
`comment = true`만으로 사용자 허락을 만들 수 없다.

정책을 준비한 뒤 기존 이슈 수집 경로에서 후보를 확인하고 Native 창에서 Start를 승인한다.
설정 파일은 현재 이슈에 대한 사람 확인을 대신하지 않는다. 댓글을 허용하지 않으려면 사용자
파일의 `comment`를 false로 두거나 저장소 파일에서 false로 제한한다.

## Description 템플릿과 준비 상태

Jira description에는 제목이 있는 완료 기준과 실제 목록 항목을 넣는다. 아래 구조는 GitHub
Issue Form YAML이 아니라 description의 내용 예시다. Jira 편집기에서 제목 2와 글머리 목록으로
작성하면 해당 ADF 구조를 공유용 Markdown으로 변환한다.

```markdown
## 완료 기준
- 같은 요청을 다시 보내도 결과가 한 번만 저장된다.
- 실패한 요청은 기존 데이터를 바꾸지 않고 원인을 표시한다.

## 범위
주문 API의 입력 검증과 관련 테스트를 수정한다.

## 하지 않는 것
데이터베이스 스키마와 결제 공급자는 변경하지 않는다.
```

저장소의 기존 `issue-policy.toml`에 추가 필수 섹션이 있으면 함께 작성한다. 완료 기준 문장만
있고 목록이 없거나 내용이 비어 있으면 시작 준비를 완료한 것으로 보지 않는다. 이 본문은
작업 지시의 입력이며, 사람 소유 typed 완료 기준이나 호스트 검증 결과를 자동 생성하지 않는다.

지원하지 않는 ADF 구조, 확인할 수 없는 필수 custom field, 비어 있는 필수 필드는 typed
`content_problems`와 `needs_info`로 남긴다. 일부 텍스트만 읽었다고 준비 완료로 판정하지 않는다.
공개·마스킹 정책과 공유본 확인도 기존 이슈 정책을 따른다.

일반 Cloud REST 조회로 연결된 PR의 완전한 부재를 확인할 수 없어 PR 유무는 `Unknown`으로
유지한다. 다른 조건이 모두 확인된 경우에만 “PR이나 중복 작업이 없는지 직접 확인”하는
Native Human Start를 제공한다. 추가 메타데이터가 불명확하거나 필수 내용이 부족하면 이
예외를 적용하지 않는다. 제출 직전 다시 조회하며 key·상태·내용·정책이 바뀌면 과거 승인을
새 판의 작업 시작에 재사용하지 않는다.

## 브랜치, PR, 댓글

승인한 immutable Jira 링크에서 전체 대문자 key를 가져와 브랜치에 보존한다. 예를 들어
`AKRA_CORE-42`는 `akra-agent/slot-1/AKRA_CORE-42-…`에 남는다. 작업명 부분이 길면 hash로
줄이며 마지막 브랜치 조각은 lease 식별자·충돌 번호까지 96자 이내다. 로컬·원격 이름 충돌도
기존 allocator가 처리한다. 잘못된 key를 소문자/대문자 변환으로 고치거나 새로운 ID로 추측하지 않는다.

PR 제목은 `[AKRA_CORE-42] supersession: …`, 본문은 `Issue: https://…/browse/AKRA_CORE-42`를
포함한다. Jira 작업 참조의 `close_on_merge`는 false이며 GitHub의 `Closes` 키워드를 붙이지
않는다. Jira 상태 변경은 아래의 별도 정책과 사람 승인 경계를 따른다.

상태 댓글은 기존 영속 send journal을 사용한다. 갱신 대상은 현재 계정이 작성·수정했고 동일
이슈의 소유 표식과 본문 hash가 맞는 댓글이다. 타인의 댓글이나 수동으로 변경된 본문을
덮어쓰지 않는다. 응답 유실 뒤에는 읽기로 결과를 확인하며 같은 POST/PUT을 자동 재전송하지 않는다.
Jira 라벨 쓰기는 지원하지 않는다.

## 선택적인 완료 전이

기본 전이는 꺼져 있다. 완료 전이를 허용하려면 사용자 파일에서 **전체 출처 좌표와 숫자 ID**를
함께 허락한다. transition의 표시 이름이나 “Done”이라는 이름을 보고 ID를 추측하지 않는다.
다음은 앞의 사용자 파일을 대체하는 완전한 예시다.

```toml
[binding.jira]
site_url = "https://example.atlassian.net"
cloud_id = "cloud-123"
project_id = "10001"
expected_account_id = "account-operator-123"
jql = "statusCategory != Done"
mode = "propose"

[binding.jira.close_transition]
transition_id = "31"
from_status_ids = ["10000", "10001"]
to_status_id = "10002"

[authority]
autonomy_cap = "propose"

[select]
mode = "all_open"
trusted_actors = [
  { provider = "jira", server_id = "cloud-123", actor_id = "account-author-123" }
]

[feedback]
comment = true
labels = false
transition = true

[close]
when = "integration"
```

저장소 파일은 같은 출처와 전이의 시작 상태 집합을 더 좁힐 수 있다. 다음 전체 저장소 파일은
사용자 파일의 출처 좌표를 상속하고 `10001`에서의 전이만 허용한다.

```toml
[binding.jira]
mode = "propose"

[binding.jira.close_transition]
transition_id = "31"
from_status_ids = ["10001"]
to_status_id = "10002"

[close]
when = "integration"
```

다른 Cloud/project/account로 이 허락을 옮기거나 transition/도착 상태를 바꿀 수 없다. 저장소가
끄려면 `feedback.transition = false`와 `close.when = "never"`를 함께 적용한다.

통합 사실 이후에도 기존 Close의 Native Human 승인이 필요하다. 전송 직전 현재 시작 상태,
사용 가능한 정확한 transition, 목표 상태의 `statusCategory = done`, 추가 필수 입력 부재를
확인한다. 정확한 204 응답과 원격 상태·changelog를 확인하며, ACK가 유실된 전이를 다시 보내지
않는다. 현재 상태가 Done이라는 사실만으로 Akra가 전이를 수행했다고 확정하지 않는다. Jira가
조건부 상태 변경 CAS를 제공하는 것으로 취급하지 않으므로 최종 읽기 이후 서버의 동시 변경을
완전히 막는다는 보장은 하지 않는다.

## 토큰 교체와 검증 범위

같은 site/cloud/account의 토큰 교체 뒤 새 읽기 작업은 새 연결 revision을 사용할 수 있다.
이미 준비한 쓰기는 원래의 정확한 연결 참조에 묶이며 새 토큰을 가져와 늦게 전송하지 않는다.
disconnect·교체와 실제 전송의 admission을 직렬화하고 이미 승인된 전송의 응답 또는 Unknown
정산을 기다린다. 바인딩 Pause도 기존 실제 효과 guard를 사용한다.

같은 계정의 토큰 교체 자체는 이슈의 Human Start 승인을 만료시키지 않는다. ConnectionRef는
콘텐츠 승인 지문과 분리하며, key·상태·지원 범위·본문 및 출처/정책 변경은 승인 판에 반영한다.
다른 계정·site·Cloud, 끊긴 연결, 이전 revision의 진행 중 쓰기는 현재 연결 검증을 통과해야 한다.

회귀 범위에는 typed HTTP route/status, 자격 증명 교체·disconnect, 댓글 소유권·응답 유실,
정확한 전이, SQLite send CAS, Human Start, immutable 작업 참조와 실제 임시 Git의 브랜치·PR
전달이 포함된다. 공급자 HTTP와 모델은 대역을 사용하며, 실제 실행한 검사와 OS별 결과는 구현
PR에 기록한다. 이 문서는 실제 Jira 계정이나 모델을 호출한 검증을 주장하지 않는다.

## 공식 API 근거

- [Issue search](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-search/):
  enhanced JQL search의 `nextPageToken` 페이지와 검색 인덱스의 지연을 고려한다. 증분 조회는
  겹치는 시간 구간과 주기적인 전체 재검색을 사용하며, 알려진 이슈는 검색 필터와 별도로 읽는다.
- [Issues](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/):
  안정 ID 조회·edit metadata·현재 가능한 transition과 필수 필드를 확인한다.
- [Issue comments](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-comments/):
  ADF 본문과 댓글 ID·작성자·수정자·property를 이용하며, 생성과 갱신의 응답 코드를 구분한다.
- [Remote issue links](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-remote-links/):
  일반 remote link 목록을 개발 패널 전체 PR 목록으로 해석하지 않는다.

위 문서는 2026-11-23 구현 검토에 사용했다. 이 구현의 허용 범위와 거절 조건은
공급자 API 전체 기능보다 좁으며, 정책과 typed adapter 계약으로 고정한다.
