# Observe 이슈 수집 검증 (#2538)

## 사용자 동작

커널의 이슈 수집 모드는 off와 observe다. 최초 설정은 observe이며 장부에 저장된 뒤에만 수집한다. 저장된 off는 재시작해도 유지한다. 장부를 읽지 못한 상태를 기본 Observe로 대체하지 않는다.

다음 명령과 조회는 생성된 akra describe --json 스키마에도 포함된다.

- 저장 명령: akra kernel issue set_issue_automation_mode --mode observe --json
- 저장 명령: akra kernel issue set_issue_automation_mode --mode off --json
- 실행 명령: akra kernel issue sync_issues --json
- 상태 조회: akra kernel query issue_automation --json
- 목록 조회: akra kernel query issues --limit 50 --json

sync_issues는 살아 있는 Gateway/커널이 필요하며 Off에서는 issue_automation_off로 거절한다. 페이지 조회의 기본 크기는 50, 최대 크기는 100이다. 다음 페이지에는 응답의 next_after를 전달한다. 이 커서는 공급자의 발견 페이지 커서와 별개이며 workspace와 완전한 이슈 ID에 결속된다.

Observe는 공유 가능한 내용과 출처 메타데이터만 수집한다. 분류, 태스크 작성, 모델 호출, 실행, GitHub 쓰기는 수행하지 않는다.

## 읽기와 저장 경계

대표 checkout의 로컬 remote.origin.url을 읽고 GitHub 저장소를 확인한다. push remote 선택은 이 바인딩에 관여하지 않는다. 안정 저장소 ID를 확인한 뒤에는 provider/server/repository/issue/workspace 전체 좌표를 사용한다. 새 발견 순회의 첫 페이지에서 origin과 기존 저장소 ID를 다시 확인하며 후속 페이지는 이미 확인한 안정 ID를 사용한다.

한 번의 수집은 발견 한 페이지(최대 25개 검사)와 재동기화 최대 10건이다. 공급자 호출 상한은 11회다. 발견에 실패해도 이미 추적 중인 ID의 재동기화는 별도로 시도한다. 재동기화는 고정된 순회 상한과 마지막 처리 ID를 저장하므로 새 항목이 계속 들어와도 기존 순회가 끝난다.

발견 페이지의 binding·관찰·새 추적 항목·다음 커서는 하나의 SQLite 트랜잭션으로 반영한다. 재동기화 묶음 역시 하나의 트랜잭션이다. 공급자 호출 중에는 DB 트랜잭션을 유지하지 않는다. 모든 commit은 현재 Observe 모드와 설정 epoch·전체 revision을 검사한다. 페이지 중간 오류나 오래된 결과가 부분 저장을 만들지 않는다.

이미 저장된 항목을 다시 읽어도 issue_tracked 대상이 다시 생기지 않는다. 최초 Withheld 항목도 관찰 장부에 식별자를 보존한다. 이후 Shared로 바뀌어 워크플로 행이 생겨도 최초 발견을 중복 보고하지 않는다. 기존 v1 장부의 워크플로 상태·플래그·revision은 유지한다.

## 주기와 중지

커널이 살아 있는 동안 Observe 수집 작업자는 최대 하나다. 초기 수집을 요청하고, 성공 후에는 완료 시각부터 5분 뒤에 다시 요청한다. 실패 후 대기는 10분, 20분, 40분, 최대 60분으로 늘어난다. 실행 중의 반복 Sync 요청은 같은 작업으로 합친다.

원격 읽기는 별도 작업자에서 수행한다. 호스트는 mailbox와 다음 deadline으로 깨어나므로 느린 공급자 조회가 명령 처리를 막지 않는다. Observe 대기와 수집은 has_remaining_work에 포함되지 않아 수집 자체가 커널 수명을 연장하지 않는다.

Off는 먼저 저장하고 진행 중 작업을 취소한다. 이미 시작된 원격 호출의 종료를 기다리지는 않는다. 저장 직전 취소와 epoch/revision을 검사하므로 늦은 응답은 반영하지 않는다. 커널 종료도 취소와 짧은 commit을 직렬화해 종료 후 뒤늦은 저장을 막는다. 작업자 panic이나 연결이 끊긴 mailbox는 수집 중 상태를 해제하고 실패 대기로 전환한다.

## 공개 상태

- 성공 관찰의 Shared에만 현재 공유본이 있다.
- Withheld는 현재 내용을 공개할 수 없다는 상태를 보존하며 과거 공유본으로 대체하지 않는다.
- Unknown은 조회 불능이다. 닫힘이나 변화 없음으로 해석하지 않으며 과거 관찰을 합치지 않는다.
- 기존 v1 행에 새 관찰이 없으면 not_yet_observed다.
- 장부 접근 실패는 명시적인 unavailable 응답이며 빈 정상 목록으로 반환하지 않는다.
- 공급자와 정책 파일의 원문 오류는 상태에 넣지 않는다. 닫힌 실패 코드만 노출한다.

새 등록 사실은 DB commit 직후 취소 gate 안에서 별도 mailbox로 전달한다. 이후 Off, 작업자 오류, 정상 커널 종료가 발생해도 이미 저장된 사실은 정확한 binding과 commit revision으로 감사 기록에 남긴다. 종료 시에는 취소를 확정한 뒤 mailbox를 비우고 기존 기록을 flush하며 원격 응답을 기다리지 않는다.

issues 이벤트에는 현재 수집 상태와 revision이 실린다. 목록은 페이지 질의로 다시 읽을 수 있으며, 커널 기록에는 실제 변경한 모드와 새로 추적한 ID만 남긴다. 복구의 기준은 SQLite 장부이며 이벤트 기록을 재생해 권한을 되살리지 않는다.

## 자동 검증

실제 GitHub 및 모델 요청은 사용하지 않는다. 가짜 공급자·origin·정책과 실제 비공개 SQLite fixture를 연결해 다음 경계를 확인한다.

- 이슈 3개 수집·반복 수집·재시작 후 중복 없음.
- 커서 저장과 재시작 시 후속 페이지 재개.
- 한 페이지/재동기화 10건 상한과 유한한 순회.
- 원격 닫힘과 Unknown의 구분 및 자동 워크플로 전이 없음.
- 최초 Off 저장, 반복 Off, 재시작 Off에서 공급자·origin·정책 호출 0회.
- 발견 도중 Off와 재동기화 묶음 도중 Off의 늦은 결과 폐기.
- 취소 후 초기/후속 commit 없음.
- 정책 변경 시 이전 정책 커서를 재사용하지 않음.
- 발견 실패 시 checkpoint 유지와 알려진 ID의 독립 재동기화.
- 잘못된 재동기화 ID를 요청한 ID의 Unknown으로 처리.
- 두 collector의 같은 fence 경쟁, 중간 페이지 실패 rollback/replay.
- v1 스키마·기존 레코드 보존과 v2 추가 테이블 migration.
- 프로토콜의 제한 모드, 잘못된 페이지 커서, Withheld/Unknown의 공유본 부재.
- 가짜 시계/작업자로 정상 주기, backoff 상한, 중복 요청 합침, Off/종료, worker 실패를 재현.
- 기존 커널 시뮬레이션에서 Observe가 작업 잔량과 host 종료 조건을 바꾸지 않음.

생성 스키마·커널 skill·Desktop 타입은 각각 생성기로 갱신한다. Desktop 생성기의 스키마 해석도 회귀로 검증한다. 최종 실행 명령과 결과는 PR 본문과 리뷰 댓글에 기록한다.
