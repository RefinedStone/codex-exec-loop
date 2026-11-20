# 이슈 자동화 장부 (IA1-1, #2536)

이 문서는 plan 28의 첫 구현 조각을 설명한다. 추적 상태와 영속 장부를 제공하며, 원격 수집·판정·실행·kernel 명령·화면은 후속 조각의 범위다.

## 저장 위치와 의존 방향

장부 파일은 저장소의 private runtime 아래 `issue-automation.db`다. SQLite adapter는 runtime 경로를 찾는 함수를 주입받으며 planning authority 타입이나 DB를 호출하지 않는다. 새 파일은 자체 application ID와 schema version을 사용한다. 기존 planning DB의 schema version과 표는 변경하지 않는다.

도메인과 StorePort는 기존 `IssueReference`, `IssueSharedCopy`, `IssueFieldHash`, `IssueComparisonBasis`를 재사용한다. planning·parallel·core의 타입은 자동화 경계로 들어오지 않는다. 아키텍처 검사는 실제 소스뿐 아니라 묶음 import, 별칭, 상대 경로로 금지 타입을 참조하는 사례도 확인한다.

## 행의 정체성과 정보 경계

추적 행의 키는 provider, server ID, repository/project ID, issue ID, canonical project scope의 전체 조합이다. 안정 ID는 대소문자를 보존한다. Binding ID는 행의 소유 설정을 나타내지만 이슈의 중복을 가르는 키에는 추가하지 않는다.

바인딩은 원격 좌표와 로컬 프로젝트를 연결한다. ID와 좌표는 저장 후 불변이며 자율 수준만 revision CAS로 변경할 수 있다. 기본 수준은 `observe`다. `off / observe / propose / gated / auto` 값을 표현할 수 있다는 사실은 실행이나 원격 쓰기를 허용하지 않는다. 정책·사용자 승인·실행 관문은 후속 조각에서 연결한다.

장부는 공유본과 원격 원문의 필드 해시를 함께 보존한다.

- `snapshot.basis.field_hash`: provider 원본 제목·본문으로 계산한 변경 감지 값이다.
- `snapshot.shared_copy`: 마스킹된 저장 가능 제목·본문이다.
- `snapshot.basis.provider_updated_at`: provider가 보고한 변경 시각이다.
- `snapshot.policy_version`: 공유본을 만들 때 적용한 정책 판이다.

원문의 필드 해시가 공유본의 해시와 같아야 한다고 검사하지 않는다. 가린 부분만 바뀌어도 원문 해시는 달라질 수 있다. StorePort는 원문 snapshot, Vault 평문, 임의 JSON을 입력으로 받지 않는다.

## 상태와 표시

상태는 다음 11개로 닫혀 있다.

| 상태 | 허용되는 다음 단계 |
| --- | --- |
| discovered | ineligible, needs_info, ready, 결정 요청, 취소 |
| ineligible | 재판정(discovered), 취소 |
| needs_info | 재판정(discovered), 취소 |
| awaiting_decision | 저장한 이전 phase로 복귀, 시작 전 재판정, 명시적 실패 재시도, 취소 |
| ready | queued, 결정 요청, 재판정, 취소 |
| queued | running, cancelled |
| running | in_review, 결정 요청, failed, 명시적 취소 |
| in_review | done, 보완 실행(running), 결정 요청, failed, 명시적 취소 |
| failed | 결정 요청, 명시적 재시도에 따른 재판정, 취소 |
| done | 종료 |
| cancelled | 종료 |

같은 상태의 멱등 적용은 값을 바꾸지 않는다. 결정 대기는 진입 전 phase를 저장하며 일반 복귀는 그 phase로만 가능하다. 실패 후 재시도와 재판정은 별도 전이로 구분한다. 이 표는 작업을 실행하거나 사람의 승인 사실을 증명하지 않는다.

결정 대기 중 재판정은 이전 phase가 discovered 또는 ready일 때만 가능하고, 실패 재시도는 이전 phase가 failed일 때만 가능하다. 실행·리뷰 단계의 결정 대기를 임의의 시작 상태로 바꾸지 않는다.

재판정을 discovered로 되돌리고 실패 후 명시적 재시도를 허용하는 부분은 plan 28 §6에 맞춘 보수적 구현 가정이다. 이미 queued인 작업은 철회 계약 없이 재판정 상태로 되돌리지 않는다.

`held`, `stale`, `unknown`은 동시에 설정될 수 있는 독립 표시다. 상태 전이는 표시를 자동으로 지우지 않는다. 표시는 다음 조정기의 새 행동 허용 판단에 사용하며, 이미 발생한 실행 사실을 장부에 반영하지 못하게 만드는 잠금으로 쓰지 않는다.

## 멱등 저장과 동시성

새 추적 값의 revision은 0이고 최초 저장은 1을 부여한다. 도메인에서 상태·공유본·표시를 바꿔도 읽은 revision은 유지된다. 저장소가 최신 행과 후보를 검증하고 CAS에 성공할 때만 revision을 1 올린다.

- 재삽입은 기존 행을 반환한다. 수집이 반복돼도 상태·표시·바인딩·공유본이 초기화되지 않는다.
- 내용이 같은 정상 no-op은 revision을 올리지 않는다.
- 저장 응답을 잃은 경우, 현재 revision이 expected+1이고 revision을 뺀 모든 내용이 같을 때만 같은 결과로 인정한다.
- 더 높은 revision에서 내용이 다시 같아진 경우는 충돌이다. 오래된 쓰기가 중간 변경을 건너뛰지 않는다.
- 후보의 base revision, 불변 ID·binding, 허용 상태 전이를 검사한다. persistence DTO로 금지 전이를 꾸며도 저장 단계에서 거절한다.
- SQLite transaction과 revision 조건으로 서로 다른 adapter와 프로세스의 경쟁을 처리한다.

디스크 오류, 알 수 없는 schema, 손상된 행은 오류로 반환한다. 이를 빈 장부나 정상 미존재로 바꾸지 않는다.

## 파일 보호

runtime은 이미 private하게 준비된 절대 디렉터리여야 한다. adapter는 고정 파일만 열고 디렉터리·DB·rollback journal의 파일 신원과 소유권을 검사한다. Unix의 symlink/hardlink/비정규 파일 및 Windows의 경로 재지정과 ACL 경계를 검사한다. 새 DB는 DELETE journal mode를 사용하며 WAL/SHM가 있는 파일을 임의 변환하지 않는다.

쓰기 중단으로 main DB 헤더가 손상돼도 보호된 rollback journal에 유효한 복구 헤더가 남아 있으면 SQLite의 native rollback을 먼저 시도한다. journal mode 조회가 복구를 수행한 직후 main 헤더를 엄격히 재검사한다. 체크섬과 실제 복구 여부는 SQLite가 판단한다. WAL을 뜻하는 형식 바이트 2는 journal 유무와 무관하게 사전에 거절한다. 손상 결과가 우연히 2가 된 경우도 이 보수적인 거절에 포함된다.

## 검증 범위

의미 있는 회귀는 전이표와 결정 재개, hydrate 검증, 동일 안정 ID의 멱등 삽입, 교차 scope와 binding, 실제 SQLite CAS 경쟁·응답 유실 재시도·ABA, 재시작 복구, 공유본만 있는 디스크 내용, 파일 보호, 이웃 planning DB 무변경을 확인한다. 외부 provider나 모델 turn을 호출하지 않는다.
