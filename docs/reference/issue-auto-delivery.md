# 이슈 Auto 전달

관련: [#2560](https://github.com/RefinedStone/codex-exec-loop/issues/2560),
[전달 증거 게이트](task-delivery-evidence.md), [호스트 검증](task-verification.md).

## 선택과 현재 정책

`set_issue_autonomy`의 `auto`는 현재 저장소·선택 revision·정책에 대한 Native 확인을 요구한다.
파일의 `auto` 설정만으로 실행 권한이 생기지 않는다. 사용자 상한과 저장소 상한, 사람 필수
규칙, 신뢰한 작성자, 보류와 예산 제한을 그대로 적용한다. 선택 명령의 재전송은 최초 영수증을
돌려주며 작업이나 전달 허가를 새로 발급하지 않는다.

공급자가 같은 응답에서 확인한 안정 저장소 ID, 정규 `owner/name`, 공개 범위를 사용한다.
현재 출처가 Private 또는 Internal이고 정규 좌표가 있을 때만 Auto가 유효하다. Public,
Unknown, 이전 데이터처럼 좌표가 없는 출처의 Auto는 Propose로 제한한다. Off·Observe·Propose·
Gated 선택은 이 제한으로 올리지 않는다. 표시 URL이나 다른 private fork를 출처 신원으로
대체하지 않는다.

공개 범위나 저장소 이름 변경은 새 triage 근거와 gate revision을 만든다. 새 메타데이터가 없는
기존 관찰은 종전 fingerprint를 유지한다. 이슈 제목·본문의 기존 field hash는 바꾸지 않는다.

Auto의 Start·Delivery·Merge도 각 단계의 현재 정책을 따른다. Spec은 기존 사람 경계이며
Plan의 기존 사람·timeout arbiter 정책도 유지한다. Auto 선택이나 작업자의 완료 보고는
Spec 승인 또는 새로운 Done 증명이 아니다. 전역 병렬 실행은 따로 켜야 한다.

## 전달 권한

조정기는 Start 근거와 실제 frozen diff 판정을 구분한다. Delivery·Merge가 모두 현재 Auto이고
실제 diff가 사람 예외를 요구하지 않을 때만 명시적 Auto 전달 권한을 발급한다. 보호 경로,
큰 변경, binary 등으로 요구된 사람 예외를 승인해도 그 승인이 Auto 권한으로 바뀌지는 않는다.
승인 전에는 소스 push와 PR 생성 허가가 없다.

발급한 권한에는 다음 값이 함께 저장된다.

- 현재 authorization epoch, 정책·field hash와 작업 ID
- 실제 검토한 frozen base/source commit과 Delivery revision
- 현재 Merge gate revision과 출처 `owner/name`

Dispatch에는 전달 권한을 넣지 않는다. 이전 `authority` 없는 permit은 기존 Human 계약으로
해석하며, 이슈 출처의 Integrate에서 `requires_human_review=false`만으로 Auto를 만들지 못한다.
동일한 source라도 권한 일부가 바뀌면 같은 허가로 취급하지 않는다.

## 검증과 실제 효과

Auto의 Push·PR·Integrate는 모두 `enforce`와 현재 필수 기준의 충족을 요구한다. `observe`는
Auto에 전달 권한을 주지 않으며, `auto_requires_enforce` 대기를 영속화한다. 필수 Command
기준에 `git_diff_check_v1`이 하나도 없으면 `auto_requires_machine_criterion`으로 대기한다.
화면 기준과 면제만으로 이 최소 조건을 대신하지 않는다.

각 필수 기준은 현재 scope·source tree·실행 환경·sealed capture와 맞는 호스트 검증 또는
기존 현재 Human 면제로 충족해야 한다. 작업자의 Pass만으로 통과하지 않는다. 현재 Fail이나
Blocked, stale·누락된 근거는 기존 E3 규칙을 따른다. `git_diff_check_v1`은 새 공백 오류·충돌
표식 검사이며 기능·빌드 테스트 전체를 보장하지 않는다.

현재 이슈 등록은 본문을 typed 완료 기준으로 자동 변환하지 않는다. 새 Task의 기준이 없으면
Auto는 대기하며, 기존 사람 기준 편집 경로로 명시적인 기준을 준비해야 한다. 이 과정에서
시스템이 사람 확인을 대신 만들지 않는다.

distributor는 기존 Push·PR·Integrate 경로를 사용한다. frozen 전달 대상은 출처와 같은
저장소여야 하며 현재 대상도 Private/Internal인지 확인한다. 마지막 증거 transaction에서
Task·scope·작업 control·permit 전체·queue claim·source·증거 revision을 다시 비교한다.
재확인 조회는 외부 쓰기의 허가가 아니다.

## PR 리뷰와 프로세스 opt-in

| 조건 | 동작 |
| --- | --- |
| Auto 권한 + Enforce 충족 + 프로세스 opt-in | 현재 PR의 사람 승인 생략을 허용한다. 필수 checks·clean mergeability·정확한 base/head/source는 계속 요구한다. |
| Auto 권한 + Enforce 충족 + opt-in 없음 | Push·PR 단계 이후 정확한 source SHA에 대한 현재 사람 PR 승인을 기다린다. |
| 정책 또는 실제 diff가 Human 필수 | 기존 사람 전달 승인과 PR 리뷰 계약을 유지한다. |
| PR 기능이 꺼졌거나 사용할 수 없음 | Auto는 대기한다. PR 없는 직접 통합으로 우회하지 않는다. |
| Observe 또는 근거 부족 | Auto 전달 효과를 실행하지 않고 사유를 저장한다. |

프로세스 opt-in은 `AKRA_PARALLEL_AUTONOMOUS_DELIVERY=1`이며 저장소 설정이나 Auto 선택이
대신 켜지 않는다. Auto가 아닌 기존 작업의 PR 선택 정책은 유지한다.

## 영속 호환과 다음 범위

이슈 저장소 schema v13은 v10–v12의 선택·명령 영수증·pending 변경과 기존 작업 fence를
보존하며 Auto 선택을 허용한다. 현재 선택 revision을 확인하므로 파일 정책만 바꾸거나 오래된
확인 명령을 재사용해 자동 실행을 승인할 수 없다.

바인딩 전체의 일시 정지는 별도 후속 PR이다. 여기의 기존 작업별 Hold·Cancel·Takeover를
바인딩 전체 Pause로 설명하지 않는다. client 없이 Gateway를 다시 켜는 복구는 #2561 범위다.

회귀는 실제 SQLite 재오픈, 현재 출처 변경, permit 발급·재조회와 최종 transaction,
기존 distributor의 실제 임시 Git, fake GitHub 효과 횟수, Native 확인 및 Desktop 선택을 다룬다.
실제 모델 턴은 실행하지 않는다. 실제 실행한 검증과 CI 결과는 구현 PR에 기록한다.
