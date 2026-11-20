# GitHub 이슈 출처 (IA1-2, #2537)

## 범위와 계약

IssueSourcePort는 발견, 안정 ID 재동기화, 능력 조회를 제공한다. adapter는 읽기만 하며 캐시, 장부 저장, 실행, 댓글 읽기, 원격 쓰기를 수행하지 않는다. 수집 주기와 커서 저장의 주인은 후속 #2538이다.

| 메서드 | 결과와 실패 의미 |
| --- | --- |
| discover | 선택 조건에 맞는 열린 이슈의 페이지와 다음 커서. 실패는 빈 성공 페이지가 아니다. |
| resync | 선택 조건과 무관한 안정 ID 한 건의 관찰 또는 Unknown. 닫힘과 라벨 제거도 관찰한다. |
| capabilities | 발견·재동기화만 제공한다. 증분 watermark, 댓글·라벨·담당자·상태·PR 연결·보드 쓰기는 제공하지 않는다. |

첫 발견은 GitHubLocator(owner/name)로 저장소 ID를 확정한다. 예상 저장소 ID가 주어졌으면 반드시 일치해야 한다. 후속 페이지는 Repository의 안정 ID로만 읽는다. 이름이 같아도 재생성된 다른 저장소에 커서를 이어 붙이지 않는다.

커서는 전체 provider/server/repository/project 좌표, 정규화된 선택 조건, 페이지 크기, 정책 판과 실제 네 가지 공개 정책 처리에 결속한다. 복원 DTO는 판과 입력을 검증하며 임의 문자열 하나를 커서 계약으로 사용하지 않는다.

## Windows 로그인 설정 경로

실제 읽기 검증에서 기존 credential subprocess 환경이 Windows의 APPDATA를 전달하지 않아, 로그인된 gh가 API 요청 전에 로그인 필요로 종료하는 문제를 확인했다. Windows에서는 include_user_config가 켜진 경우에만 APPDATA를 전달한다. 다른 설정 경로와 같은 절대경로·저장소 밖 경로·디렉터리 권한 검증을 적용하며, 검증 실패 시 경로를 전달하지 않는다. 토큰을 다시 읽거나 인증 설정을 바꾸는 우회는 추가하지 않는다.

## 페이지와 선택

GitHub repository.issues 또는 안정 Repository node의 issues를 OPEN, UPDATED_AT ASC로 읽는다. 페이지 크기는 1..100이며 **원격에서 검사하는 이슈 수**다. metadata 연결도 각각 최대 100개다. 기존 GhApiRunner를 재사용해 신뢰한 gh 실행 파일, 중립 작업 폴더, 자격 증명 환경, 호출당 15초 제한을 유지한다. JSON 해석 전 응답 크기 상한은 16 MiB다.

AllOpen 또는 비어 있지 않은 LabelsAny를 받는다. 라벨 이름은 정확히 비교하며 대소문자·공백은 바꾸지 않는다. LabelsAny는 한정된 원격 페이지에서 OR로 평가한다. 여러 라벨을 GitHub의 AND 필터로 잘못 매핑하거나 첫 라벨만 조회하지 않는다.

반환 관찰이 0개여도 원격에서 검사한 이슈가 있으면 다음 커서가 있을 수 있다. 선택 라벨을 찾지 못했는데 라벨 목록이 잘렸거나 읽을 수 없으면 MetadataIncomplete이며 커서를 진행시키지 않는다. 이 경우 충분한 metadata를 제공할 수 있을 때까지 해당 페이지는 보류된다.

다음 페이지가 있다고 했는데 커서가 없거나, 현재 커서를 반복하거나, 이슈 없이 계속 진행하려 하면 실패한다. 페이지 크기 초과, 중복 이슈, 변경 시각 정렬 위반, 다른 저장소 ID도 실패다. 호출자는 **페이지 전체를 장부에 반영한 뒤에만** 다음 커서를 저장해야 한다.

## 관찰과 정보 경계

remote_state의 Open/Closed는 원격 이슈 상태다. 자동화 장부의 작업 수명 상태를 직접 전이시키지 않는다.

- Shared는 기존 IssueAutomationSnapshot이다. provider 원본 제목·본문의 정확한 필드 해시와 공개 정책을 지난 공유본을 구분한다.
- Withheld는 원문을 읽었지만 UserConfirm 때문에 공유본을 내보낼 수 없는 상태다. 비교 해시와 확인한 metadata는 남기되 빈 공유본이나 이전 공유본으로 바꾸지 않는다.
- Unknown은 403, 404, 시간 초과, 전송 실패, 파싱 오류, identity 불일치 등으로 현재 관찰을 읽지 못한 상태다. 원문 오류나 이전 성공 결과를 포함하지 않는다.
- GraphQL data와 errors가 함께 오면 부분 성공을 채택하지 않는다.

비밀 탐지는 기존 이슈 인계의 토큰 접두사, Bearer, URL 계정 정보, key/value, PEM 규칙을 그대로 재사용한다. 사람·경로 등 모든 민감 정보를 자동으로 찾아내는 범용 탐지기는 아니다.

## Metadata와 신뢰 근거

라벨·담당자·라벨 행위자 목록은 Complete, Truncated, Unavailable을 구분한다. 담당자는 사용자 전용 assignees 대신 assignedActors로 조회해 봇·조직·mannequin도 포함한다. ID/login을 해석하지 못한 담당자가 있으면 빈 Complete 목록으로 바꾸지 않는다. 불완전한 빈 목록을 대상이 없다는 확정 근거로 사용하지 않는다. 작성자와 이벤트 행위자의 명시적 null은 계정 정보가 없다는 뜻이며 누락된 필드와 구분한다.

라벨 행위자는 현재 라벨에 대응하는 실제 최신 LabeledEvent에서만 얻는다. 작성자나 담당자로 대신하지 않는다. 최신 이벤트가 UnlabeledEvent이거나 읽은 timeline 뒤에 더 많은 이벤트가 있거나 null event가 있으면 최신 행위자를 확정하지 않는다. 지난 이벤트가 잘린 경우도 불완전 표시를 보존한다. 이 사실들은 신뢰 정책의 입력이며 그 자체가 승인이나 실행 허가는 아니다.

연결 PR의 유무는 closedByPullRequestsReferences(includeClosedPrs: true)의 totalCount다. 0은 Absent, 양수는 Present, null이나 누락은 Unknown이다. 닫힘 참조·수동 연결을 포함하며 단순 본문 언급의 모든 PR을 수집하는 기능은 아니다.

## 공용 가림 분리

domain의 plan_issue_masking은 공유본과 복원 위치 계획만 만든다. 계획에 원문 문자열이나 원문 참조를 보관하지 않는다. 기존 service의 mask_issue_text는 계획을 사용해 VaultPlaintext를 만드는 wrapper이며 API를 유지한다. 출처와 인계는 공유본만 사용하므로 Vault 평문을 만들지 않는다.

실제 겹침만 하나의 가림 그룹으로 묶는다. 맞닿은 두 Remove는 별도 placeholder를 유지한다. 그룹 내부 제거, 한글 바이트 위치, 기존 표식 충돌, 확인 정책, Remove 우선 처리와 기존 복원 결과를 회귀로 고정했다.

## 검증

Fake GhApiRunner로 두 페이지 연결, 필터 후 빈 페이지, 닫힘·라벨 제거, 403/404/시간 초과와 부분 GraphQL 오류, 비밀값 가림, 원본 해시, Withheld, metadata 누락·잘림, 실제 라벨 행위자, 잘못된 페이지·커서와 원격 호출 전 거절을 확인한다. 기존 masker·handoff 회귀도 함께 실행한다.

실제 읽기 검증은 별도의 ignored test live_read_only_source_probe다. 명시적인 owner/name/workspace 환경을 받아 이 저장소에서 발견 2회와 재동기화 1회, 총 3회만 호출한다. 출력은 요청 수와 안정 ID이며 제목·본문은 출력하지 않는다. 모델 turn이나 원격 변경은 호출하지 않는다.

### 실제 읽기 결과 (2026-11-20)

RefinedStone/codex-exec-loop에서 명시적 GH_TOKEN 인증 경로로 발견 2회와 안정 ID 재동기화 1회, 총 3회를 완료했다. 서로 다른 두 이슈를 읽었고 첫 이슈의 재동기화가 Observed로 돌아왔다. 해당 실행에서 제목·본문·토큰은 출력하지 않았다. 별도 인증 경로 비교 진단에서 발견 쿼리 2회가 성공했으므로, 이 작업에서 성공한 실제 출처 조회는 합계 5회다.

이 호스트에서는 APPDATA 기반의 저장 로그인으로는 조회가 완료되지 않았다. 실제 validate_configuration_path 진단에서 ACL 거절을 확인했다. 토큰 경로의 성공을 저장 로그인 경로의 복구로 간주하지 않는다. 제품 코드에 토큰 추출 fallback을 추가하거나 사용자 디렉터리 권한을 변경하지 않았다.

공식 계약:

- [GitHub CLI configuration environment](https://cli.github.com/manual/gh_help_environment)
- [GitHub GraphQL pagination](https://docs.github.com/en/graphql/guides/using-pagination-in-the-graphql-api)
- [GitHub Repository issues](https://docs.github.com/en/graphql/reference/repos#repository)
- [GitHub Issue·LabeledEvent·timeline](https://docs.github.com/en/graphql/reference/issues)
