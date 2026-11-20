# 이슈 사람 확인의 증거 경로

대상: #2545 · plan 28 §10.1, §14, Q-IA11.

## 제공하는 경로

Desktop main의 native 확인 창을 승인한 뒤 만든 증거를 Gateway가 검증한다.
검증된 metadata만 커널에 전달하며, caller 표시명이나 일반 명령의 자기 신고로
사람 답변을 만들지 않는다. 결정 카드와 알림은 #2550의 범위다.

현재 지원하는 행동은 결정 요청의 선택지에 답하는 것과 Off에서 Observe로
수집 수준을 올리는 것이다. 아직 지원하지 않는 Propose·Gated·Auto의 실행 기능을
추가하지 않는다. 저장된 모드가 없을 때의 기존 기본 Observe 동작도 유지한다.

## main, helper, Gateway의 역할

main은 일반 renderer 중계와 별도의 Gateway 연결을 소유한다.
이 연결의 응답을 renderer 메시지 버스로 전달하지 않는다.
renderer는 결정 ID·선택지·메모·멱등 키 또는 목표 모드를 전용 IPC로 요청한다.
질문과 선택지 효과는 Gateway의 현재 공유 장부 조회에서 가져온다.

main은 고정 subcommand `desktop-confirmation-bridge`로 helper를 시작한다.
비밀은 argv나 새 환경변수로 넘기지 않는다. helper의 stdin/stdout pipe를 main만
보유하며, 확인 창의 기본·취소 선택은 취소다. 승인 전에는 proof를 만들거나
답변 명령을 제출하지 않는다. 창·renderer·연결이 교체되면 대기 중 확인을 폐기한다.

helper의 등록은 브라우저 등록과 별개 목적과 이름 공간을 사용한다.
등록 자격은 owner-only 임시 파일에 보관하고, Gateway는 소유권·권한·파일 정체성과
목적을 검사한다. Unix의 private socket과 Windows의 owner-only named pipe를 사용한다.
등록 철회와 helper 종료 시 해당 등록과 proof를 폐기한다.

| 통로 | 허용하는 동작 |
|---|---|
| main이 보유한 helper stdin | 등록 준비, 승인 후 proof 생성, 철회 |
| 인증된 helper private IPC | 연결 바인딩, 수명 감시, proof 검증·소비 |
| 일반 Gateway 명령 | agent 답변, 모드 하향·no-op, 기존 영수증 재전송 |

private IPC에는 proof를 생성하는 approve 동작이 없다.
등록 자격만 가진 일반 Gateway client도 main의 승인 경로를 대신 호출할 수 없다.

## 공개 Gateway 계약

- `akra/kernel/confirmation/register`: 등록 ID와 파일 위치를 현재 attach에 묶는다.
- `akra/kernel/confirmation/prepare`: 기존 CommandRequest를 받아 현재 대상과 공유 확인 문구를 읽는다.
- `akra/kernel/command_confirmed`: 기존 CommandRequest와 별도 confirmation envelope를 받아 증거를 검증한다.
- 기존 `akra/kernel/command`와 CommandRequest의 논리 명령 형태는 유지한다.

준비 응답의 intent는 등록·Gateway 연결·attach·kernel client·대표 workspace,
논리 명령의 멱등 키와 입력 지문, 실제 대상에 묶인다. 결정 대상에는 ID·선택지·basis·
결정 revision이, 모드 대상에는 현재 모드·epoch·revision·목표 모드가 포함된다.
Gateway는 제출된 intent와 실제 연결 및 논리 명령을 대조한 뒤 helper에서 소비한다.

증거는 한 논리 행동에만 쓸 수 있다. 다른 결정·선택지·workspace·연결·키·입력으로
재사용하면 거절한다. 짧은 수명과 유한한 보관 한도를 적용한다.
동일 intent의 재소비는 허용하고 실제 중복 실행은 기존 CommandMemory가 막는다.
새 멱등 장부를 추가하지 않는다.

## 실제 저장 시점의 검사

Gateway는 proof를 검증할 때 새로운 basis로 원래 intent를 덮어쓰지 않는다.
결정 서비스가 현재 관찰·정책을 다시 평가하고 #2544의 stale 규칙을 적용한다.

- 이미 종료된 요청은 첫 결과를 유지한다.
- 현재 기준이 바뀌었으면 답을 적용하지 않고 superseded로 닫는다.
- 유효한 현재 관문이 Human이면 증거 없는 agent 답을 거절하고 요청을 열린 상태로 둔다.
- 검증된 대상·선택지·basis·revision이 일치하는 답만 by: human으로 기록한다.
- Human이 아닌 관문의 무증거 답은 by: agent를 유지한다.

SQLite도 현재 Human 관문의 agent 완료 후보를 거절한다.
사람이 선택했다는 사실만으로 실행 허가나 승인 효과를 만들지 않는다.
거절 선택지를 고른 경우도 human 답변이며 실제 선택의 의미는 후속 실행 단계가 해석한다.

Off→Observe는 저장 transaction 안에서 원래 모드·epoch·revision을 모두 검사한다.
확인 창 이후 수집 상태가 바뀌면 과거 증거로 새 상태를 상향하지 않는다.
하향과 실질적 no-op은 증거를 요구하지 않는다.

## 재전송과 비밀

검증 완료 타입에는 raw proof나 등록 token이 없고, 직렬화·역직렬화를 제공하지 않는다.
장부의 confirmation에는 비밀이 아닌 확인 참조만 보관한다.
이 참조는 과거 사실이며 다음 행동의 권한으로 사용할 수 없다.

proof, token, 등록 요청과 민감 응답을 Gateway 양방향 녹화에 남기지 않는다.
오류 문구에는 입력 원문을 복사하지 않는다. 질문·메모·caller·confirmation 참조는
일반 결정 fact 이벤트에 추가하지 않으며, renderer에는 명령 결과·취소·결과 불명만 반환한다.
제출 전 취소와 제출 후 응답 유실을 구분한다. 결과 불명이면 현재 작업공간의 장부를
다시 조회하고 고정 오류를 표시한다. 명령을 자동 재전송하지 않으며, 작업공간이나
renderer가 교체된 뒤 도착한 이전 결과는 새 화면에 적용하지 않는다.

응답을 잃은 클라이언트는 같은 논리 명령·같은 키를 일반 command로 다시 보낼 수 있다.
커널은 권한 검사 전에 기존 영수증을 확인한다. 이미 받은 명령은 재실행하지 않고
원래 영수증을 반환하며, 새 실행이라면 Human 관문·모드 상향 검사가 무증거 요청을 막는다.
잘못된 proof를 제출한 confirmed 명령은 고정 오류로 거절하고 agent로 낮추지 않는다.

## 보장 범위와 검증

이 경로는 OS가 실제 사람의 클릭을 인증하는 장치가 아니다.
같은 사용자 권한의 프로세스는 Electron main과 helper의 제어 통로를 흉내 낼 수 있다.
보장하는 경계는 일반 Gateway client나 renderer가 caller 이름·자기 신고·다른 행동의
증거만으로 사람 답변을 만들 수 없다는 것이다.

회귀는 fake client 두 개, 실제 private IPC와 SQLite, 실제 커널 명령·영수증·기록,
Gateway 녹화, 숨긴 Electron 창을 사용한다. 실제 모델 턴은 실행하지 않는다.
검증 결과와 게시된 head는 PR의 검증 기록에 남긴다.

### 추가 이름 필터에서 확인한 기존 WSL 제한

넓은 `confirmation` 필터는 이 변경과 무관한 TUI 방향 편집기 시험 두 개도
포함했다. 해당 묶음은 75개 통과, 2개 실패였으며, 실패는 planning 작업의
5초 완료 대기 한도 초과였다. #2542의 기존 WSL 로그에서도 두 시험이 같은
위치와 메시지로 실패했고, 동일 #2542 head의 네이티브 Linux CI에서는 통과했다.
#2544 CI에서도 두 시험은 통과했다. 이번 diff에는 해당 TUI 경로 변경이 없다.
CRLF가 직접 원인이라고 단정하지 않는다. 변경 모듈의 직접 회귀와 필수 Full,
게시 head의 네이티브 CI 결과는 이 추가 필터 결과와 구분해 보고한다.
