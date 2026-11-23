# Task 증거와 전달 게이트

관련: [완료 기준](task-acceptance.md), [작업자 증거](task-evidence.md),
[호스트 검증](task-verification.md), [설정](configuration.md).

## 적용 범위와 설정

`parallel.evidence_mode`는 병렬 Task의 source push, PR 생성, integration push 전에
완료 기준의 증거를 확인하는 정책이다. 일반 Task와 이슈에 연결된 Task 모두에 적용한다.
기존 WorkOrder permit, 사람의 PR 리뷰, 원격 대상, 실행 중단·보류 조건과 함께 검사한다.

| 값 | 동작 |
| --- | --- |
| `observe` | 내장 기본값. 판정과 이유를 기록하고 증거 부족만으로 기존 전달을 막지 않는다. |
| `enforce` | 필수 기준이 현재 호스트 검증 또는 현재 Human 면제로 충족되지 않으면 전달을 보류한다. |

대표 checkout에서 다음과 같이 설정한다.

```sh
akra config set --project parallel.evidence_mode enforce
akra config get parallel.evidence_mode
```

기존 설정 계층의 전역 값과 `-c parallel.evidence_mode=...`도 사용할 수 있다. 허용값은
정확히 소문자 `observe`, `enforce`이며, `off`, `enabled`, 대문자 또는 공백을 붙인 값은
거절한다. 설정을 생략하면 정상적인 기본값 `observe`를 사용한다. 파일을 읽을 수 없거나
형식이 틀린 경우에는 `observe`로 대신하지 않고 전달을 보류한다.

호스트는 저장소 authority가 가리키는 canonical workspace에서 유효 정책을 한 번 해결한다.
그 정책 또는 해석 오류는 repository identity별로 프로세스 안에 고정한다. 같은 저장소의
서비스를 다시 만들거나 worker·integration worktree에서 접근해도 다른 설정으로 바꾸지
않는다. 설정 변경·수정·삭제는 다음 repository kernel/process 시작에 반영한다.
같은 경로가 다른 repository identity로 교체되면 기존 서비스의 정책을 재사용하지 않고
새 전달을 거절한다.

## Enforce의 충족 조건

필수 기준이 없는 Task는 `criteria_missing`으로 보류한다. 각 필수 기준은 다음 중 하나를
만족해야 한다.

- 현재 Task·criterion·acceptance scope에 결속된 정확한 native Human 면제가 있다.
- 현재 선택한 recipe의 호스트 실행이 성공했고, 기준 scope, session, frozen base,
  source tree, 검증 환경과 sealed capture가 여전히 해당 실행에 맞는다.

작업자가 보고한 `pass`, 비어 있지 않은 증거 파일, 과거 조회에서 본 `passed` 또는 모델의
완료 문장만으로 통과하지 않는다. recipe 미설정, 미실행, 실행 중, 실패, 중단, stale 또는
현재 파일·환경을 확인할 수 없는 필수 기준은 Human 면제 없이는 충족되지 않는다.
첫 recipe가 지원하지 않는 `data`, `http`, `screen` 기준도 같은 원칙을 따른다.

`GitDiffCheckV1`의 성공은 고정 base/source 범위의 새 공백 오류·충돌 표식 검사에 한정한다.
빈 diff도 성공할 수 있다. 빌드, 기능 테스트 또는 시나리오 전체의 성공을 뜻하지 않으므로,
사용자가 기준에 선택한 recipe의 범위를 함께 검토해야 한다. 전달 게이트는 검증 프로세스나
모델을 자동 실행하지 않는다.

Human 면제는 완료 기준 판에 대한 면제다. 코드만 바뀌면 유지되고 acceptance scope가
바뀌면 무효가 된다. source/원격 무결성, Delivery·Spec·Integrate 승인 또는 PR 사람 리뷰를
대신하는 권한으로 사용하지 않는다.

## 전달할 코드와 검사 결과의 연결

전달 판정은 공개 `task_evidence` 응답을 권한 증명으로 재사용하지 않는다. 호스트가 현재
authority와 queue를 읽고, 해당 stage의 고정 source와 원본 호스트 receipt를 직접 비교한다.
각 외부 변경 직전에 다시 확인하므로 source push 때의 결과만으로 PR 생성이나 통합까지
자동 승인하지 않는다.

증거 판정은 명시적으로 저장한 `source_commit_sha`와 그 tree를 사용한다. 빈 SHA는
증거 부재로 기록하며, `observe`에서는 기존 레거시 전달의 source 대체 동작을 유지한다.
integration에는 별도로 고정한 base와
검증된 source의 관계, 실제 통합 결과 tree의 동일성을 확인한다. 원격 push는 검사한 결과
commit OID를 사용하고, 기존 frozen-base compare-and-swap 조건도 유지한다. 같은 patch라는
이유만으로 다른 전체 tree의 검증을 재사용하지 않는다.

## 기록과 재개

queue에는 stage별 최신 판정을 최대 세 개 보관한다. mode, scope, evidence revision,
source와 기준별 이유가 같은 판정은 관측 시각만 달라졌다고 새로운 runtime event를 만들지
않는다. 감사 기록은 기존 runtime event 보존 한도를 따르며 E2 증거 revision을 바꾸지 않는다.

Enforce에서 보류된 항목은 해당 이유를 유지한다. 현재 증거나 면제가 충족되면 기존 queue
compare-and-swap으로 재개하고 전달 직전에 다시 판정한다. 이미 원격 전달을 마친 항목의
정리 복구에 새로운 전달 승인 요구를 소급 적용하지 않는다.

이 정책은 Done·dispatch·Auto 권한을 바꾸지 않는다. Desktop 기준 카드와 증거 열기,
면제 버튼은 별도 화면 범위이며 기존 native Human 확인 계약을 유지한다.
