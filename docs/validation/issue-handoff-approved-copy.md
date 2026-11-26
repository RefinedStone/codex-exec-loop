# 승인 공유본의 인계 계약과 완료 근거 (#2701)

상위 [#2414](https://github.com/RefinedStone/codex-exec-loop/issues/2414).
[#2701](https://github.com/RefinedStone/codex-exec-loop/issues/2701)의 승인판 검사는
[PR #2709](https://github.com/RefinedStone/codex-exec-loop/pull/2709)에 구현되었다.
이 문서는 2026-11-26의 `prerelease` 기준 `d960d7d62e810f8cd94c7aa78918b0ba0104b6d6`에서
현재 코드와 기존 실행 기록을 대조한 인수 기록이다. 새 시험 실행이나 CI 통과 기록이 아니다.

## 적용 중인 수동 인계 계약

프로젝트 범위에서 Akra가 게시한 이슈를 인계할 때 최근 확정한 공유본의 해시와 원격 필드
해시가 같아야 한다. 외부 편집, 미확정 게시, 일지 조회 실패는 인계를 보류한다.
불일치는 최신 공개본을 다시 검토하고 별도로 승인·게시한 뒤 재시도한다. 조회 실패는 원인을
복구한 뒤 다시 읽는다. 로컬 재연결은 원문 열람 승인일 뿐 AI 인계의 승인판을 바꾸지 않는다.

| 요청과 이력 | 인계에 사용할 판과 실패 처리 |
| --- | --- |
| 작업 폴더가 있고 확정 수정 게시 이력이 있음 | 대표 checkout으로 scope를 해석하고 실제 안정 이슈 ID의 최신 확정 publication을 사용한다. 미확정 전송, 중복 순서, 응답 불일치, 해시 불일치가 있으면 과거 승인으로 돌아가지 않는다. |
| 수정 게시 이력은 없고 생성 이력이 있음 | creation journal의 저장소·안정 이슈 ID·intent·승인 공유본 해시·확정 상태를 확인한다. 공유본만 생성했거나 Vault adoption이 끝나지 않아도 같은 검사를 적용한다. |
| 같은 저장소의 생성 결과나 안정 이슈 ID가 불명확함 | 일반 외부 이슈로 간주하지 않고 보류한다. 생성 일지 읽기 실패도 동일하다. |
| 프로젝트의 게시·생성 이력이 모두 없음 | 현재 공개본을 현재 사용자·프로젝트 AI 공개 정책으로 가려 수동 초안에 넣는다. 이전에 승인한 판이나 자동 실행 권한이 있다고 주장하지 않는다. |
| 작업 폴더가 없음 | 프로젝트 승인 이력과 독립된 공개본 읽기다. 현재 사용자 AI 공개 정책을 적용하며 프로젝트에 종속된 실행 권한을 부여하지 않는다. |

후속 수정 게시가 확정되면 그 최신 판이 처음 생성한 판보다 우선한다. publication lease는
원격 조회와 승인 일지 확인을 묶고, 생성 일지는 writer lease를 추가로 잡지 않는 검증된
읽기 snapshot을 사용한다. 생성 writer가 publication lease를 기다리는 동안 인계가 반대로
creation writer lease를 기다리는 잠금 순서 역전을 만들지 않는다.

### Gateway의 `approved: None`이 의미하는 것

[`akra/issue/handoffCopy`](../../src/adapter/inbound/gateway/issue.rs)는 현재도
`approved: None`을 전달한다. 클라이언트가 제출한 해시를 승인으로 믿지 않고,
[`IssueHandoffService`](../../src/application/service/issue_handoff.rs)가 scope에 연결된
publication/creation journal에서 승인판을 직접 찾기 때문이다.
[`production` 조립](../../src/composition/production.rs)은 대표 workspace 해석기와 두 journal을
모두 연결한다. 따라서 `None`이라는 호출부 값만으로 승인 검사 누락이라고 판단하면 안 된다.

정책·템플릿 버전은 [수정 게시 승인 검사](../../src/application/service/issue_publish.rs)와
[생성 승인 검사](../../src/application/service/issue_creation.rs)에서 게시 전에 대조한다.
수동 인계 시에는 현재 AI 공개 정책을 다시 적용한다. publication의 해시를 자동 실행 승인으로
변환하거나, 수동 인계에서 과거의 정책·템플릿 버전으로 현재 실행 권한을 새로 부여하지 않는다.
자동 실행은 [기존 이슈 자동화 계약](../reference/current-product.md#issue-automation-execution)의
현재 관찰·정책·사람 승인·실행 허가 경계를 그대로 사용한다.

## 화면 결과와 취소의 구분

- 성공하면 Projects의 **스레드로 넘기기**가 공유본의 제목·본문과 이슈 URL을 새 대화의
  입력 초안에 넣는다. 저장소에 맞는 폴더를 찾지 못하면 실제 전송 시 폴더를 고른다.
  인계 자체는 대화나 모델 턴을 시작하지 않는다.
- 조회·판 일치 검사·정책 적용이 실패하면 항목에 오류가 표시되고 초안을 새로 만들지 않는다.
  `handoffCopy`는 별도의 `held` DTO가 아닌 JSON-RPC 오류를 반환한다.
- 모든 인계는 공유본을 다시 AI 공개 정책으로 가린다. Vault 원문이나 댓글을 반환하지 않는다.
  일반적인 이름·경로를 자동으로 모두 탐지하는 기능은 없으므로 프로젝트 승인판의 외부 변경을
  거절하는 검사와 비프로젝트 수동 읽기의 한계를 구분해야 한다.
- 게시의 `abandon`은 `NeedsConfirmation`인 결과 확인을 사용자가 그만두는 동작이다.
  실제 전송 가능성이 있는 최신 `Abandoned` 기록을 무시하고 과거 `Confirmed` 승인으로
  인계하지 않는다. 이미 확정된 게시 승인을 철회하는 API라는 뜻은 아니다.
- 초안을 보내지 않는 것, Vault 열람 승인 취소, 게시 결과 확인 중단, 자동 실행 취소는 서로
  다른 동작이다. 현재 인계 조회에는 진행 중 RPC를 중단하는 전용 API/버튼이 없다.
  이미 확정된 게시 승인을 철회한 뒤 인계가 거절되는 흐름도 아래 기존 회귀의 검증 범위가 아니다.

화면 연결 근거는 [Projects 인계 조립](../../desktop/src/renderer/App.tsx),
[공유본 초안 생성](../../desktop/src/renderer/work/githubProjects.ts),
[항목의 대기·오류 표시](../../desktop/src/renderer/work/GitHubProjectPanel.tsx)다.

## 완료 조건별 근거

아래 표의 회귀명은 해당 기준 소스에 존재하는 assertion의 범위를 가리킨다. 소스 확인과
해당 OS에서의 새 실행 성공은 구분한다.

| #2701의 완료 조건 | 현재 근거 | 판정 및 한계 |
| --- | --- | --- |
| 수동/자동·외부 편집·UI 결과의 계약 | 위 정책 표, `IssueHandoffService`, Desktop 초안·오류 경로 | 구현된 정책을 문서화했다. 수동 공유본 읽기로 자동 실행 권한을 만들지 않는다. |
| 실제 Gateway의 승인판 검사 | `handoff_uses_confirmed_journal_and_holds_unconfirmed_or_edited_copies` ([production 조립 Gateway 회귀](../../src/adapter/inbound/gateway/tests/issue_shared_copy_flow.rs)) | 실제 WebSocket Gateway와 production journal 조립으로 동일 판·외부 편집·조회 실패·미확정 게시를 검사한다. 원격과 키/Vault API는 합성이다. |
| 같은 판·변경된 판·다른 이슈/판·불명확한 승인 | `approval_selection_uses_stable_identity_and_never_rolls_back_an_uncertain_write` ([서비스](../../src/application/service/issue_handoff.rs)), `creation_handoff_holds_unknown_or_inconsistent_creation_approval` ([생성/인계 결합](../../src/application/service/issue_creation/tests/handoff.rs)) | 안정 ID 선택, 최신 판 우선, 미확정·손상·중복 생성 승인과 다른 intent를 검사한다. 이력 자체가 없는 외부 이슈의 수동 읽기는 정책상 허용한다. |
| creation-only·Vault adoption 실패 우회 차단 | `creation_handoff_checks_shared_copy_only_and_pending_vault_adoption`, `creation_handoff_uses_a_later_confirmed_update_instead_of_the_original_creation` ([생성/인계 결합](../../src/application/service/issue_creation/tests/handoff.rs)) | PR #2709의 P1 리뷰를 반영한 경계다. 복원 링크 저장 성공 여부와 승인판 검사를 분리한다. |
| 생성 일지 조회 실패·읽기 잠금 | `creation_handoff_does_not_fall_back_when_the_snapshot_is_unavailable`, `read_only_creation_snapshot_does_not_wait_for_the_writer_lease` ([Unix journal](../../src/adapter/outbound/filesystem/issue_creation_journal/tests.rs)), `absent_provenance_is_read_only_and_sent_is_visible_under_reservation` ([Windows journal](../../src/adapter/outbound/filesystem/issue_creation_journal/windows/tests.rs)) | 프로젝트 조회 실패 시 일반 공개본 읽기로 우회하지 않는다. 비프로젝트 읽기의 독립 계약과 writer reservation 중 읽기를 각각 검사한다. |
| 취소 | 위 서비스의 `approval_selection_uses_stable_identity_and_never_rolls_back_an_uncertain_write` | 전송 가능성이 있는 최신 `Abandoned` 기록에서 이전 승인으로 돌아가지 않음을 검사한다. 확정 승인 철회나 진행 중 인계 RPC 중단까지 검증한 것으로 확대하지 않는다. |
| 기존 ignored 회귀 처리 | `the_gateway_hand_off_refuses_an_issue_edited_after_its_approved_publish` ([원래 Gateway 흐름](../../src/adapter/inbound/gateway/tests/issue_flow.rs)) | 현재는 `#[tokio::test]`이며 `#[ignore]`가 없다. 이 파일의 원래 Vault 통합 프로필은 Unix 한정이다. |
| 공유본만 인계·원문 비노출 | 위 Gateway 흐름의 `assert_no_plaintext_left`, [Desktop 단위 회귀](../../desktop/test/githubProjects.test.ts), [Projects 화면 회귀](../../desktop/test/e2e/github-projects.mjs) | 합성 canary가 journal·Vault 파일·녹화·Gateway 답·원격 쓰기·초안에 남지 않는 경계와 인계만으로 `turn/start`가 없음을 각각 검사한다. 실제 키 저장소·실제 모델 시험은 아니다. |
| 자동화/보안 회귀 및 CI | PR #2709의 아래 기존 실행 기록 | 이 문서 변경에서 CI를 조회·대기하거나 시험을 재실행하지 않았다. 전체 CI 통과를 새로 확인했다고 표시하지 않는다. |

### 기존 실행 기록

[PR #2709의 리뷰 대응 기록](https://github.com/RefinedStone/codex-exec-loop/pull/2709#issuecomment-7156935103)은
Windows 통합 작업트리의 all-features 라이브러리 빌드, 생성 서비스 28개, Gateway 인계 1개,
creation journal 6개, 관련 동시 편집·취소 복원·재연결 4개 통과를 보고한다.
child-process fixture 1개는 독립 실행에서 제외됐다고 명시한다. 이 기록의 취소 복원 결과를
인계 요청 취소나 확정 게시 승인 철회의 검증으로 대신하지 않는다.

## 종료 판정

승인판을 읽지 않는다는 원래 결함 설명과 ignored 회귀 설명은 현재 구현에 맞지 않는다.
위 근거로 해당 두 항목을 정정할 수 있으며, creation-only 우회도 수정 근거가 있다.

### 채택한 취소 범위 — 2026-11-26

#2701은 **승인 공유본의 판 일치**를 검증하는 작업으로 인수한다. 이 이슈의 취소 조건은
미확정 게시의 결과 확인을 `abandon`한 뒤에도 그 불확실성을 무시하고 이전 확정 승인으로
돌아가지 않는 것이다. 이미 확정된 게시 승인의 영구 철회나 진행 중인 읽기 RPC의 중단을
새 기능으로 포함하지 않는다. 수동 인계는 입력 초안만 만들며 실행을 승인하거나 모델 턴을
시작하지 않는다는 기존 제품 계약을 유지한다.

| 동작 | #2701 판정 | 근거 또는 재검토 조건 |
| --- | --- | --- |
| 미확정 게시 `abandon` 후 인계 | 필수 범위에 포함, 최신 불확실한 기록에서 보류 | `approval_selection_uses_stable_identity_and_never_rolls_back_an_uncertain_write` |
| 인계된 초안을 보내지 않음 | 기존 초안 처리 범위 | 인계만으로 `turn/start`가 발생하지 않는 Desktop 회귀 |
| 확정 게시 승인을 영구 철회 | 이번 인수 범위 밖, 구현 완료로 표시하지 않음 | 같은 공개본을 유지하면서 이후 인계만 금지하는 제품 요구가 정해지면 영속 철회 기록·재시작·동시 조회를 별도 설계한다. |
| 진행 중 인계 RPC 취소 | 이번 인수 범위 밖, 구현 완료로 표시하지 않음 | 사용자 취소 UI를 도입할 때 요청 식별·취소 후 늦은 응답 폐기를 별도 검증한다. 서버 읽기 중단과 화면 응답 폐기는 다른 보장이다. |
| Vault 열람·자동 실행 취소 | 기존 기능의 독립 계약 | 각각 #2704/#2724, #2535/#2555의 범위를 따른다. |

이 범위 결정에 따라 위 표의 기존 구현과 회귀가 #2701의 기능 조건에 대응한다. 이는 새 시험
성공의 증거가 아니다. 이 결정 PR에서는 문서 변경에 필요한 `git diff --check`를 수행하고,
PR의 CI 결과와 연결된 기존 실행 기록을 검토한 뒤 인수를 확정한다. PR 제출 단계에서는 이슈를
열어 두며, 상위 #2414와 실제 OS/모델 검증까지 함께 종료하지 않는다.
