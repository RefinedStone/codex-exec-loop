# 마감 뒤의 중재자 결정 (#2559)

## 적용 범위

현재 정책이 `timeout_arbiter`인 관문에서 새로 만든 `on_timeout: arbiter` 요청만 처리한다.
Gated Start와 Propose/Gated Plan 중 준비가 끝난 Low/Normal 위험 요청이 대상이다.
Human 필수 규칙, 높은 위험, Spec/Merge, 작업 중 질문·복구·닫기 요청은 중재자가 답하지 못한다.
기존에 `on_timeout: wait`로 저장된 요청은 소급하여 모델에 보내지 않는다.
기본 중재 설정은 정책의 정규 JSON에서 생략해 기존 기본 정책 판과 결정 ID도 유지한다.

중재 결과를 적용해도 자율 수준은 바뀌지 않는다. Start 이후에는 기존 admission 예산,
원격 이슈 재확인, 현재 인가 세대, 실행 permit, Delivery 및 통합 관문을 계속 거친다.
일반 Agent/Human 답변 API나 일반 저장 API로 `by: arbiter`를 기록할 수 없다.
사용자는 기존 Human 답변, 보류·취소 경로를 그대로 사용할 수 있다.

## 모델과 입력

`IssueAssessmentModelPort`는 설정 읽기, 최초 dispatch, 결과 관찰, 취소, 완료 후 정리만 제공한다.
기존 coordinator의 직렬 tick이 SQLite 예약과 적용을 소유하며 별도 실행 오케스트레이터를 만들지 않는다.
모델 완료를 기다리는 동기 루프도 없다. 기존 추적 worker가 Pending을 관찰한다.

Production은 기존 `CodexThreadDriver`의 새 읽기 전용 임시 thread와 고정된
`akra-issue-assessment` 지침을 사용한다. 모델은 `delegation.toml`의
`feature.issue_assessment`에 선택된 단일 Codex backend 설정을 따른다.
읽기 전용 sandbox, network access false, approval never를 실제 thread 응답에서도 확인한다.
이 backend에는 로컬 읽기 도구가 남아 있으므로 모든 도구가 제거된 격리라고 보장하지 않는다.
저장소 workspace를 thread 작업 디렉터리로 넘기거나 callback 권한을 발급하지 않는다.
모델을 호출할 수 없으면 사전 설정·인증을 바꾸지 않고 Wait로 남긴다.

입력은 현재 공개 정책을 통과한 이슈 공유본, 현재 판정, 선택지, 유효 정책 및 선택한 Git 근거다.
Git 근거는 하나의 고정 commit에서 읽으며, 이슈의 inline code 중 slash(`/`) 또는 점(`.`)이 있는
명시적 상대 파일 경로와 README.md/Cargo.toml/package.json을 선택한다. `gated`, `timeout_arbiter` 같은
bare 상태·필드 이름은 읽기 후보에 넣지 않으며, malformed/incomplete 경로는 여전히 전체 Wait로 남긴다.
루트 문서·manifest와 src/tests/docs, desktop/src, desktop/test, desktop/scripts의 정적 허용 범위만
읽으며 보호 경로가 섞이면 전체 입력을 거절한다. 다른 desktop 하위 경로까지 허용하지 않는다.
없는 파일은 Absent로 명시하며 누락된 내용을 추측하지 않는다.
최대 16개 경로, 파일당 16 KiB, 전체 파일 내용 64 KiB, 전체 입력 JSON 256 KiB로 제한한다.
보호 경로, symlink/submodule, binary·비 UTF-8, 불완전한 선택, 읽기 오류, 비밀값, 공개 정책상 마스킹이 필요한 내용은
전체 입력을 거절하고 Wait로 남긴다. 정책·선택지·경로 이름을 포함한 모든 문자열 값에 같은 공개 검사를
적용하며, Windows 경로도 JSON escape 이전 값에서 검사한다. 모델 입력에 workspace 절대 경로를 추가하지 않는다.

입력 지문에는 decision 기준과 revision, repository commit, 근거 지문, 모델 route 판,
전체 JSON 지문을 묶는다. 완료 응답을 적용하기 전 원격 이슈를 다시 읽고 현재 정책과 근거를 재구성한다.
근거 조회가 끝난 뒤에도 자동화 정책·공개 정책·model route·현재 commit이 그대로인지 다시 확인한다.
결정 기준이 바뀌면 Superseded, Git 근거만 달라지면 Open Wait가 된다.
먼저 확정된 Human 답변은 그대로 보존하고 늦은 모델 응답은 Discarded로 기록한다.

응답은 `{ decision, confidence, reasons, blockers }` 형식만 허용한다.
confidence는 0~10000 정수이며, decision은 제시한 option ID여야 한다.
알 수 없는 필드·과도한 크기·잘못된 JSON을 거절한다. 이유는 기존 공개 정책을 다시 통과한
안전한 메모만 저장하고, 원본 응답이나 blocker 본문을 이벤트/DB에 남기지 않는다.

## 예산과 재시작

이슈 자동화 정책의 기본 설정은 다음과 같다. 이 설정만 추가해도 자율 수준이나 관문은 바뀌지 않는다.

```toml
[arbiter]
min_confidence = 9000
max_running = 1
max_started_per_day = 5
timeout_seconds = 180
```

저장소 설정은 사용자 설정보다 confidence를 낮추거나 예산·시간 제한을 늘릴 수 없다.
실제 호출은 이 설정과 delegation route 한도, 선택한 backend 동시 한도 중 더 작은 값을 적용한다.
중재 호출은 별도 읽기 전용 driver를 사용하며 작업 실행 슬롯을 차지하지 않는다.
Arbiter가 시작시킨 실제 작업은 다른 시작 권한과 동일한 작업 예산을 사용한다.

SQLite schema 12가 호출 예약, 완료 영수증/미전달 이벤트, 단조 증가하는 UTC 일자 예산을 추가한다.
호출 전에 같은 트랜잭션에서 예약과 한도를 확인한다. 동일 결정에는 예약 하나만 허용하며,
예약 뒤 중단되어 실제 전송 여부를 모르는 경우도 사용한 호출로 계산한다.
입력 준비가 끝난 예약 시점부터 `timeout_seconds`를 계산한다. 예약·dispatch·finish 직전에는
worker가 받은 UTC에 monotonic 경과 시간을 더해 시각을 갱신한다. 완료 응답을 검증하는 동안
호출 기한이 지나도 이전 시각으로 적용하지 않고 OutcomeUnknown Wait로 남긴다.
재시작은 기존 예약을 관찰할 뿐 재전송하지 않는다. backend resume와 fallback도 사용하지 않는다.
시간이 뒤로 이동해도 일일 한도를 다시 열지 않는다.

낮은 확신, blocker, 예산 소진, 모델/근거 오류, 불명확한 결과 및 시간 초과는 Open Wait로 확정한다.
같은 판에 대한 자동 재시도는 없다. Wait는 결정 revision과 선택·메모 초안을 바꾸지 않으며,
사용자가 답할 수 있는 요청을 그대로 남긴다.

## 영속 이벤트와 Desktop

Applied/Waiting/Superseded/Discarded를 하나의 `issue_decision_arbiter_finished` 이벤트로 보관한다.
`issue-arbiter-finished:` ID는 안정적이며, 커널 append 뒤 ACK한다. ACK는 영수증이나 Wait 상태를 지우지 않는다.
성공을 일반 `issue_decision_resolved`로 한 번 더 발행하지 않는다.
모델 응답을 받은 뒤 DB 저장에 실패하면 같은 응답을 다음 관찰에서 다시 사용할 수 있게 보존한다.

Desktop은 현재 detail의 decision ID·revision·basis와 일치하는 `arbiter_wait`만 표시한다.
Applied와 Waiting 알림도 이벤트 본문만 보고 합성하지 않고 연결된 커널의 현재 detail을 확인한다.
이미 끝난 결정의 늦은 Waiting, 중복 이벤트, 조회 중 재연결은 알림이나 Open 상태를 되살리지 않는다.
다른 이슈의 완료 알림과 연결된 worktree의 canonical workspace도 현재 조회 기준으로 검증한다.
완료 이력은 원래 `by: arbiter`를 표시하며 사람의 확인으로 바꾸지 않는다.

## 회귀 검증 범위

- Domain/SQLite: eligible 관문, Human 선답, stale 기준, 호출 한도, 시계 역행,
  재예약 방지, 첫 영수증 보존, ACK 뒤 Wait 조회, schema 이관, 기존 Start admission 연결.
- Application: Fake 모델의 dispatch/관찰/재시작, timeout/Off/Hold, 정책·Git 근거·route 변경,
  공개 정책을 적용한 메모, 일반 Agent의 중재자 출처 위조 거절.
- Kernel simulation: 4초 기한과 2초 전 재알림, 실제 다음 스케줄, 모델 dispatch 한 번,
  영속 완료 이벤트와 ACK, 재조립 뒤 대기 상태.
- Model/Git adapter: 고정 read-only profile, Pending/complete 관찰, 응답 보존, resume/fallback 금지,
  실제 Git의 고정 commit·정상 파일·보호 경로·symlink·크기·변경 경계.
- Desktop: wire 검증, 작성 초안 보존, 기존 Human 확인, 늦은/중복 이벤트,
  background 알림, 연결 변경 및 canonical workspace 경계.

실행 결과와 검사한 commit은 PR에 기록한다. 회귀 검증에서 실제 모델 턴을 호출하지 않는다.
