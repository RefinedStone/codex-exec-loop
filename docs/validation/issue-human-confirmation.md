# 이슈 사람 확인의 증거 경로

대상: #2545 · plan 28 §10.1, §14, Q-IA11.

## 제공하는 경로

Desktop main의 native 확인 창을 승인한 뒤 만든 증거를 Gateway가 검증한다.
검증된 metadata만 커널에 전달하며, caller 표시명이나 일반 명령의 자기 신고로
사람 답변을 만들지 않는다. 결정 카드와 알림은 #2550의 범위다.

현재 지원하는 행동은 결정 요청의 선택지에 답하는 것과 Off에서 Observe로
수집 수준을 올리는 것이다. 아직 지원하지 않는 Propose·Gated·Auto의 실행 기능을
추가하지 않는다. 저장된 모드가 없을 때의 기존 기본 Observe 동작도 유지한다.

## main, utility process, helper, Gateway의 역할

main은 앱 창·main frame·수명을 검증하고 native 확인 창과 전역 확인 순서를 소유한다.
JSON-RPC 연결·응답 해석·등록·준비·제출은 행동마다 새로 시작하는 Electron utility process에서
처리한다. 해당 프로세스는 별도 번들로 빌드되며 renderer와 MessagePort를 공유하지 않는다.
renderer는 결정 ID·선택지·메모·멱등 키 또는 목표 모드를 기존 전용 IPC로 요청한다.
질문과 선택지 효과는 Gateway의 현재 공유 장부 조회에서 가져온다.
main과 utility 사이에는 입력, 표시 문구, 승인/취소, 제출 직전 수명 확인, 정제된 결과만
오간다. 등록·intent·proof·Gateway frame은 이 통로에도 전달하지 않는다.

utility process는 고정 subcommand `desktop-confirmation-bridge`로 helper를 시작한다.
비밀은 argv나 새 환경변수로 넘기지 않는다. helper의 stdin/stdout pipe는 해당 utility만
보유하며, main 확인 창의 기본·취소 선택은 취소다. native 승인 응답 전에는 proof를 만들지
않고, 제출 직전 main이 원래 창의 수명을 다시 확인한 뒤에만 명령을 보낸다.
창·renderer·연결이 교체되면 대기 중 확인과 helper를 폐기한다. utility가 죽거나 제한 시간
안에 시작하지 않으면 실패로 닫으며 재시작해서 요청을 재생하지 않는다. 제출을 허용한 뒤
응답을 잃으면 취소가 아닌 결과 불명으로 반환한다. 정상 종료는 helper의 EOF 처리와
종료를 기다려 등록 파일과 proof를 정리한다. 강제 종료 시에는 private IPC 연결을 잃은
Gateway가 해당 등록을 더 이상 사용할 수 없다.

helper의 등록은 브라우저 등록과 별개 목적과 이름 공간을 사용한다.
등록 자격은 owner-only 임시 파일에 보관하고, Gateway는 소유권·권한·파일 정체성과
목적을 검사한다. Unix의 private socket과 Windows의 owner-only named pipe를 사용한다.
등록 철회와 helper 종료 시 해당 등록과 proof를 폐기한다.

| 통로 | 허용하는 동작 |
|---|---|
| utility가 보유한 helper stdin | 등록 준비, 승인 후 proof 생성, 철회 |
| 인증된 helper private IPC | 연결 바인딩, 수명 감시, proof 검증·소비 |
| 일반 Gateway 명령 | agent 답변, 모드 하향·no-op, 기존 영수증 재전송 |

private IPC에는 proof를 생성하는 approve 동작이 없다.
등록 자격만 가진 일반 Gateway client도 native 승인 후에만 진행되는 utility 경로를 대신 호출할 수 없다.

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

### 자동 회귀의 경계

| 시험 | 실제 경로 | 대역과 미검증 범위 |
| --- | --- | --- |
| Desktop `issueConfirmationService` | 승인·취소·재시도·소유 창 검사 순서 | dialog 응답·helper·RPC 의존성은 시험 대역 |
| Desktop `issue-confirmation`/`issue-interventions` E2E | Electron IPC, 별도 utility process, stdio, 창 수명 | native dialog 응답·helper·Gateway는 시험 대역 |
| 기존 Rust `human_confirmation` 및 `confirmation::tests` | 실제 helper `run_io`, private IPC, Gateway 검증, SQLite·커널 명령·영수증 | helper를 같은 OS 프로세스의 Tokio task로 실행 |
| `native_helper_process_confirms_replays_and_revokes_on_eof` | 빌드된 `akra-gateway desktop-confirmation-bridge` 자식의 stdin/stdout, private IPC와 실제 Gateway 검증 | 시험 harness가 승인 응답을 제공. Electron native dialog·Node transport 결합·실제 모델은 별도 |

실제 프로세스 회귀는 [native_process.rs](../../src/adapter/inbound/gateway/tests/human_confirmation/native_process.rs)에 있다.
기존 Gateway 세션·커널 harness를 재사용하고 helper만 실행 파일로 교체한다.
등록 준비·Gateway 등록·intent 준비·승인·confirmed 명령·동일 영수증 재전송을 연결하고,
stdin EOF 뒤 정상 종료·등록 파일과 IPC 디렉터리 정리·미소비 proof 거절을 검사한다.
타임아웃과 자식 프로세스 guard로 실패 시에도 자식을 종료·회수하며 모델·GitHub를 호출하지 않는다.
이 결과를 실제 사람의 클릭 확인이나 [#2555 실제 모델 확인](issue-automation-propose-flow.md)의 완료로 기록하지 않는다.

이 회귀는 사전 빌드가 필요해 `#[ignore]`로 표시하며 POSIX Full, Linux Rust CI와 macOS portable CI에서 별도로 명시 실행한다.
누락된 환경을 통과로 반환하지 않는다. 실행 전 같은 소스의 `akra-gateway`를 빌드하고
`AKRA_CONFIRMATION_HELPER_TEST_BIN`에 그 절대 경로를, `AKRA_HOME`에 짧은 경로의 시험 전용 임시 디렉터리를 지정한다.
시험이 프로세스 전역 환경을 바꾸지 않으며 해당 변수로 제품의 검증을 우회하지도 않는다.

자동 준비·실행은 `node scripts/check_native_confirmation_process.mjs`로 한다.
runner는 Cargo가 반환한 실제 실행 파일 경로를 사용하고, 짧은 private 임시 홈을 만들어
시험 후 정리한다. 정확히 한 개 시험이 통과했는지도 확인하므로 필터 변경으로 0개가 실행된
결과를 성공으로 취급하지 않는다.

```bash
cargo build --locked --bin akra-gateway
# 위 두 환경변수를 지정한 상태에서 실행한다.
cargo test --locked --lib \
  adapter::inbound::gateway::tests::human_confirmation::native_process::native_helper_process_confirms_replays_and_revokes_on_eof \
  -- --ignored --exact
```

### 추가 이름 필터에서 확인한 기존 WSL 제한

넓은 `confirmation` 필터는 이 변경과 무관한 TUI 방향 편집기 시험 두 개도
포함했다. 해당 묶음은 75개 통과, 2개 실패였으며, 실패는 planning 작업의
5초 완료 대기 한도 초과였다. #2542의 기존 WSL 로그에서도 두 시험이 같은
위치와 메시지로 실패했고, 동일 #2542 head의 네이티브 Linux CI에서는 통과했다.
#2544 CI에서도 두 시험은 통과했다. 이번 diff에는 해당 TUI 경로 변경이 없다.
CRLF가 직접 원인이라고 단정하지 않는다. 변경 모듈의 직접 회귀와 필수 Full,
게시 head의 네이티브 CI 결과는 이 추가 필터 결과와 구분해 보고한다.
