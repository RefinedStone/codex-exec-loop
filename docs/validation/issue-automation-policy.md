# 이슈 자동화 정책 검증 (#2540)

## 책임과 입력

자동화 정책은 `$AKRA_HOME/issue-automation.toml`과 저장소의 `.akra/issue-automation.toml`에서 읽는다. 기존 `issue-policy.toml`의 공개·마스킹 정책과 파일 및 해석 서비스를 분리한다.

파일 adapter는 기존 안전한 설정 읽기를 재사용한다. application service는 `IssueAutomationPolicySourcePort`에서 글을 받고 파일 형식, 값, 병합 결과를 검증한다. 파일이 없으면 `None`, 읽기 실패면 오류다. 읽기 실패나 잘못된 문서를 설정 부재로 바꾸지 않는다.

`IssueAutomationPolicyService::resolve`와 순수 `resolve_issue_automation_policy`는 유효한 typed 정책 및 SHA-256 판을 반환한다. 실패는 파일 범위 또는 최종 병합 범위의 `policy_blocked`이며, 입력 원문과 IO 오류 원문을 공개 오류에 넣지 않는다.

## 값별 병합

| 값 | 병합 | 확인할 경계 |
| --- | --- | --- |
| 일반 설정 | 저장소 > 사용자 > 내장 | 값 하나마다 상속하며, 명시한 규칙 배열의 순서는 보존한다 |
| 자율 상한 | 사용자/내장 상한과 저장소 제한의 최솟값 | 요청한 mode도 유효 상한으로 제한한다 |
| 예산 | 사용자/내장 상한과 저장소 제한의 최솟값 | 0은 실행 가능한 예산이 없는 상한이다 |
| 외부 이슈 쓰기 | 사용자 허락과 저장소 제한의 AND | 저장소의 true로 사용자 허락을 만들 수 없다 |
| 신뢰 행위자 | 사용자 목록과 저장소 제한의 교집합 | 필드 없음은 상속, 빈 목록은 모두 제외다 |
| 보호 경로 | 내장·사용자·저장소 합집합 | 빈 목록으로 기존 경로를 제거하지 않는다 |
| 사람 필수 조건과 추가 규칙 | 내장·사용자·저장소 합집합 | 이름이나 ID로 기존 규칙을 덮어쓰지 않는다 |

신뢰 행위자는 provider, server_id, actor_id의 안정 좌표다. GitHub의 actor_id는 GraphQL node ID이며, login을 신뢰 식별자로 사용하거나 자동 변환하지 않는다. ID는 opaque 값으로 보존한다.

Jira Cloud는 선택적인 `binding.jira`에 site/cloud/project/account/JQL을 명시한다. Jira가 선택되면
GitHub 수집 mode는 Off이며, Jira의 유효 실행 상한은 Propose다. 사용자 파일이 허락한 숫자
transition/목표 상태와 동일한 출처에서만 저장소가 시작 상태 집합을 좁힐 수 있다. Jira 라벨 쓰기와
`close.when = "provider_link"`는 지원하지 않는다. 유효한 사용자·저장소 TOML과 description
템플릿은 [Jira Cloud 구성](../reference/issue-jira-cloud.md)에 정리한다.

추가 사람 확인 규칙은 labels_any 또는 paths_any 중 하나 이상과 적용 gates를 요구한다. 두 선택 조건을 함께 적으면 OR다. 실제 매칭·판정은 후속 조각에서 이 typed 정책을 사용한다.

## 기본값과 교차 검증

기본 mode는 Observe, 자율 상한은 Propose다. 이번 범위에서는 모든 저장소에 동일한 기본 상한을 적용하며 공개 여부를 추가 조회하지 않는다. 기본 예산은 동시 실행 2, 하루 시작 5, 최대 시도 3이다. 외부 이슈 쓰기는 모두 false이고 close.when은 never다.

실행 수준별·위험 등급별 관문은 기본 human이다. spec은 human, plan은 human 또는 timeout_arbiter, merge는 human 또는 auto다. propose의 start 및 propose/gated의 delivery·merge는 human을 유지한다. 타임아웃 기본은 start 24h, 사전 알림 2h이며, 사전 알림은 start보다 작아야 한다.

각 파일의 명시적 잘못된 조합과 병합 후의 조합을 모두 검증한다. 예를 들어 사용자 파일이 transition=true 및 close=integration을 허용해도, 저장소가 transition=false로 제한하면 최종 정책은 차단된다.

관찰기의 저장 모드·수집 선택, 태스크 생성, 실행 허가, 모델 호출과 정책 편집 화면의 연결은 후속 조각이다. 이 서비스는 설정을 읽어 유효한 값으로 만드는 경계다.

## 정책 판

정책 판은 기본값·제한·정규화를 적용한 typed 값의 결정적 JSON을 SHA-256으로 계산한다. TOML 원문이나 Debug 표시는 사용하지 않는다.

집합 성격의 값은 정렬하고 중복을 제거한다. 위험 규칙은 첫 일치가 의미를 가지므로 순서를 보존한다. 24h와 1440m처럼 같은 시간, 기본값 생략과 명시, 주석·공백·테이블 순서 차이는 같은 유효 정책의 판을 바꾸지 않는다. 상한에 잘린 요청값만 달라지고 유효 값이 같아도 판을 유지한다.

## 검증 범위

- 기본값, 값별 상속, 사용자 상한 제한, 저장소 권한 확대 방지.
- 신뢰 목록의 부재/빈 배열, 보호 및 사람 규칙 제거 방지.
- 파일 없음, 읽기 실패, 알 수 없는 키·판·enum, 잘못된 값·조합과 최종 병합 충돌.
- 효과가 같은 입력의 동일 hash, 효과값 또는 순서가 있는 위험 규칙 변경의 다른 hash.
- 실제 임시 파일을 사용하는 adapter의 안전 읽기와 기존 공개 정책 파일의 독립성.
- 실제 JSON Schema 검증기를 사용한 스키마 자체와 두 TOML 예시의 검증. 예시는 typed 역직렬화 이전의 원시 TOML 값을 JSON으로 변환해 검사한다.
- 기존 아키텍처 검사에서 domain·service·port·adapter의 의존 방향 유지.

JSON Schema 검증기는 테스트 전용이며 HTTP·파일 참조 기능을 사용하지 않는다. 실행 시험에는 모델이나 공급자 호출이 필요하지 않다. 최종 실행 결과는 PR 본문과 리뷰 댓글에 남긴다.
