# 이슈 실행 계약과 초안 준비검사 검증 (#2543)

## 사용자 흐름

기존 이슈 수정과 새 이슈 초안에서 게시 전 준비검사를 확인한다. 서버가 부족한 섹션, 준비 상태, 예상 위험도와 관문을 반환하며 Desktop은 설명과 순서를 그대로 표시한다. 예상 라벨은 검사 조건일 뿐 게시 요청에 포함되지 않는다.

초안 입력이 달라지면 이전 검사와 게시 승인을 무효화한다. 예상 라벨만 바꾸면 게시할 내용은 같으므로 기존 게시 승인은 유지하고 검사만 다시 한다. 늦은 성공이나 오류 응답이 최신 입력을 덮지 않는다.

## 판정과 같은 함수

실제 수집 이슈의 판정과 초안 preview는 domain의 `inspect_issue_readiness`를 함께 사용한다. 이 함수에는 공유본, 필수 항목, 효과 정책, 라벨과 보류 상태만 들어간다. 초안에 가짜 이슈 ID나 작성자 근거를 만들지 않는다.

준비검사에는 최신 초안을 AI 전달 정책(`AiHandoff`)으로 가린 공유본을 사용한다. 명시한 민감 구간과 자동 비밀 검출을 모두 적용한다. 게시용 공개 정책(`Publish`)과 구분하며, 원문이나 Vault 평문을 결과에 넣지 않는다.

AI 전달에 사용자 확인이 필요하거나 정책·요구사항·구간·예상 라벨을 확인할 수 없으면 검사만 `cannot_assess`다. 자동화 정책 오류 때문에 기존 게시 승인·게시·게시 결과 확인·복원 흐름을 막지 않는다. 승인 후 고정된 생성 의도는 과거 게시본으로 최신 초안 검사를 대신하지 않고 `frozen_draft`로 표시한다.

예상 관문에는 `enabled`, revision, 실행 허가가 없다. 실제 대상 여부와 현재 근거가 충족될 때 적용될 관문을 설명하는 결과다.

## 라벨 정책

| 설정 | 기본 이름 | 의미 |
| --- | --- | --- |
| labels.auto | akra:auto | 자동 수준 요청 |
| labels.gated | akra:gated | 관문 확인 수준 요청 |
| labels.manual | akra:manual | 수동 처리 |
| labels.hold | akra:hold | 보류 |

이름은 사용자·프로젝트 정책에서 바꿀 수 있으며 비어 있는 값, 잘못된 공백·제어 문자, 역할 간 중복을 거부한다. 어느 정책 파일에도 `select.labels_any`를 명시하지 않았을 때만 최종 auto/gated/manual 이름으로 기본 선택 목록을 도출한다. 명시한 빈 목록과 이전 이름의 목록은 그대로 보존한다.

출처는 라벨 네 항목과 도출된 선택 목록의 실제 입력을 기록한다. 도출 값은 `derived`로 표시하며 hold의 출처를 선택 목록에 섞지 않는다. 라벨 요청은 정책의 자율 수준 상한을 넘지 못한다.

## 에이전트 생성과 사람 선택

GitHub의 `__typename`으로 User/Bot을 읽고, 없는 종류는 Unknown으로 둔다. 로그인이나 ID 모양으로 사람 계정을 추정하지 않는다.

Akra 생성 장부의 안정 ID는 공급자·서버·저장소·이슈·대표 workspace가 모두 일치할 때만 연결한다. 공급자 응답을 받은 뒤 장부 snapshot을 읽으므로 동시에 전송된 생성 시도를 먼저 읽은 옛 snapshot으로 놓치지 않는다. 제목, 본문 해시, 표시 번호 유사성으로 생성 대상을 추정하지 않는다.

안정 ID를 확인하지 못한 Sent/NeedsConfirmation, 장부 읽기 실패, 종류가 불명인 작성자와 Bot 작성자는 사람이 선택한 근거가 필요하다. 현재 선택 라벨과 대응하는 최신 적용 이벤트, 신뢰하는 stable actor ID, User 계정 종류가 함께 확인되기 전에는 대상이 아니다. `all_open`도 이 조건을 우회하지 않는다. 계정 종류와 생성 출처는 판정의 canonical evidence에 포함된다.

Akra의 생성 요청은 `labelIds`를 보내지 않으며, 배포 양식도 선택 라벨을 기본 부착하지 않는다. 따라서 Akra가 이슈 생성과 동시에 스스로 선택 라벨을 붙이는 경로는 없다.

**한계:** GitHub의 User 계정 종류는 실제 사람의 클릭을 증명하지 않는다. 외부 자동화가 신뢰하는 사람 계정의 자격증명으로 별도 라벨을 적용하는 경우는 이 공급자 메타데이터만으로 구분할 수 없다. 이 계약은 해당 계정의 현재 라벨 적용 근거와 Akra의 생성 장부를 신뢰 경계로 사용한다.

## GitHub 양식 설치

배포 자산은 binary에 포함한 `assets/issue-automation/akra-work.yml`이며, 설치 위치는 `.github/ISSUE_TEMPLATE/akra-work.yml`로 고정한다. 양식에는 완료 기준(필수), 범위, 하지 않는 것 항목이 있다. placeholder는 실제 기본 내용이나 자동 선택 라벨이 아니다.

사용자가 다음 명령을 직접 시작한다. 예시의 작업 폴더와 기준 브랜치는 대상 저장소에 맞춘다.

```bash
akra issue-form install --cwd "/absolute/path/to/repository" --base main --json
```

설치기는 기준 commit으로 Git tree와 새 commit을 구성하고 별도 branch와 PR을 만든다. 현재 checkout의 HEAD, index, dirty 파일과 untracked 파일은 변경하지 않는다. 최종 commit의 변경 경로가 고정 양식 한 개뿐인지 확인한다. 기준 브랜치에 같은 양식이 있으면 이미 설치된 결과를 반환하고, 다른 내용이나 링크이면 보류한다.

원격 대상과 commit을 고정하고, branch가 없다는 조건으로만 새 원격 branch를 만든다. 다른 원격 SHA를 덮어쓰지 않는다. PR 쓰기 전에 Sent를 영속 기록하며, 응답이 불확실하면 같은 명령을 다시 실행해도 결과 조회만 한다. 결과를 확정하지 못하면 `needs_confirmation`을 반환한다.

설치의 object/ref 기록은 Git의 fsync 설정과 CAS를 사용한다. 이 명령은 Git 2.43 이상을 요구한다. 다른 Git 기능의 최소 버전은 변경하지 않는다.

| 결과 | 종료 코드 |
| --- | --- |
| already_installed / pull_request_created / pull_request_found | 0 |
| needs_confirmation | 2 |
| conflict | 3 |

## 검증과 화면

정책 이름 변경·빈 선택·상한, 실제 판정과 초안 검사 일치, 생성 근거와 현재 라벨, 입력·공개 정책 변경, 불확실 생성, 파일·원문 비노출을 회귀로 확인한다. 설치는 로컬 bare Git과 fake GitHub로 실제 index·HEAD·작업 파일 불변, 조건부 branch 생성, CAS 및 모호한 응답의 중복 쓰기 방지를 확인한다.

Desktop 시험은 숨김 Electron, fake Gateway와 변경된 fixture 데이터만 사용한다. 실제 GitHub 쓰기나 모델 턴·조향 요청은 보내지 않는다.

- [기존 이슈 초안의 준비검사](issue-2543-draft-readiness.png)
- [새 이슈 초안의 준비검사](issue-2543-create-readiness.png)

### 실행 결과 (2026-11-20)

| 검사 | 결과 |
| --- | --- |
| Windows `check_native_pr.ps1 -Mode Full` | 통과 |
| Desktop 타입 검사·빌드 | 통과 |
| Desktop 단위 시험 | 818개 통과, 기존 제외 7개 |
| 전체 Desktop E2E | 통과, Windows 터미널 시험은 기존 조건으로 제외 |
| 아키텍처 경계 | 144개 통과 |
| 자동화 정책 스키마 / 불변식 | 14개 / 9개 통과 |
| Windows·Linux clippy | 모든 target·feature, `-D warnings` 통과 |
| Windows·Linux 양식 설치 관련 시험 | 각각 19개 통과 |
| 입력 포트 분리 뒤 서비스 / CLI / 실제 단독 파일 설치 재검증 | 4개 / 2개 / 1개 통과 |
| Linux 실제 WebSocket Gateway 흐름 | 7개 통과, 기존 제외 1개; 새 준비검사 회귀 2개 포함 |
| Linux 이슈 생성 / 초안 / 생성 출처 | 각 필터 40개 / 35개 / 5개 통과 |

각 필터에는 기존 회귀가 포함되며 서로 중복될 수 있다. 합계는 고유 시험 개수로 해석하지 않는다.

Windows canonical 경로의 Git 인자 표현 문제, CLI의 서비스 직접 의존, 관문 기준 설명의 불일치를 실행·독립 검토에서 발견하여 수정했다. Git argv 표현만 바꾸고 저장소 동일성 검사는 유지했으며, 아키텍처 검사는 기존 규칙을 유지하고 새 경로를 검사 대상에 더했다.

전체 E2E 뒤 두 이슈 화면을 다시 실행해 준비검사 영역을 캡처하고 실제 화면을 확인했다. 원격 CI와 게시 head를 기준으로 한 독립 리뷰 결과는 PR에 기록한다.
