# Windows 사용자 이슈 검증기 실행 계약 (#2723)

## 지원 범위

Windows x64에서 SHA-256으로 승인한 **AMD64 PE32+ Windows subsystem(2) 실행 파일**을 실행한다. 창을 띄우지 않고 stdin/stdout으로 동작하는 native 검증기가 대상이다. Windows subsystem 헤더는 창 생성 허용을 뜻하지 않는다. Win32k system call을 차단한다.

드라이브 문자가 있는 절대 경로만 받고, `GetDriveTypeW`로 입력 드라이브 루트가 `DRIVE_FIXED`인지 확인한다. 최종 실행 파일 자체는 non-reparse이며 열린 파일의 안정된 identity를 확인한다. 직접 UNC/device namespace 입력, 상대 경로, PATH 검색, 최종 실행 파일 자체가 reparse인 경로, 8 MiB 초과 파일, console subsystem(3), DLL, CLR 이미지, shell/Node/Python/WSL은 지원하지 않는다. 승인 해시와 읽은 바이트가 다르면 실행하지 않는다. 원본을 읽는 동안 쓰기·삭제 공유를 막고, 검증한 바이트를 새 private stage에 복사한다. 이후 원본 교체가 실행 내용을 바꾸지 않는다. 드라이브 루트 및 최종 파일 검사는 모든 상위 디렉터리의 junction/symlink를 배제하거나 그 해결 대상까지 고정 볼륨임을 증명하지 않는다.

지원은 운영체제 이름만으로 결정하지 않는다. 아래 API·토큰·접근 검사·Job·mitigation 중 하나라도 성립하지 않으면 `IsolationUnavailable`로 끝낸다. 직접 실행이나 약한 프로필로 재시도하지 않는다.

## 실행 경계

1. OS 난수로 고유 이름/SID를 만들고 **이번 호출에서 새로 만든** disposable AppContainer profile만 소유한다. 기존 profile 충돌은 오류이며 기존 profile을 열거나 고치거나 지우지 않는다.
2. OS known-folder의 LocalAppData/Temp 아래 새 stage에 owner/System 전체 권한과 고유 SID의 읽기·실행 권한을 원자적으로 부여한다. 기존 사용자 디렉터리, Vault, 시스템 ACL은 변경하지 않는다. image와 입력 파일의 쓰기 핸들은 실행 전에 닫고 읽기 잠금을 유지한다.
3. LPAC opt-out, 빈 capability 목록, child-process 제한, Win32k 차단, stdio 3개 핸들 allowlist, creation-time Job 목록을 동시에 설치하고 suspended 상태로 생성한다. stdin은 읽기 전용 입력 파일 핸들, stdout/stderr는 child 측 쓰기 pipe 핸들이다. 부모·Job·파일 쓰기 핸들을 상속하지 않는다.
4. Resume 전에 AppContainer 토큰 여부, 정확한 고유 SID, capability 수 0, Job 소속/활성 프로세스 수 1, child/Win32k mitigation을 검사한다. impersonation-token 사본에 대한 `AccessCheck`에서 AAP 권한은 거절되고 고유 SID 권한만 허용되는 양·음성 대조를 확인한다. 문서화가 불확실한 token information class 숫자를 성공 조건으로 삼지 않는다. API 실패를 접근 거절 성공으로 바꾸지도 않는다.
5. 실행 종료·실패·시간 초과 뒤 owned Job 종료를 관찰하고 활성 프로세스 수 0을 확인한다. stage와 profile 정리 성공을 확인한 뒤에만 결과를 반환한다. 정리 실패도 게시 가능한 성공으로 바꾸지 않는다. Job이 빈 뒤 stage 삭제의 sharing/lock 오류(32/33)만 최대 2초 제한 재시도하며, 실제 열린 파일 제어군으로 이 경계를 검증한다.

[Microsoft AppContainer 지침](https://learn.microsoft.com/en-us/windows/win32/secauthz/implementing-an-appcontainer)은 LPAC opt-out, 토큰/객체 권한의 교집합, profile 및 환경 경계를 설명한다. [UpdateProcThreadAttribute](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute)의 security capabilities·handle/job 목록·mitigation을 사용하고, 실제 접근 의미는 [AccessCheck](https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-accesscheck)로 확인한다.

## 환경과 자원

| 항목 | 계약 |
| --- | --- |
| 입력 | 검증을 통과한 공유본 JSON, 최대 1 MiB |
| 환경 | OS에서 구한 `LOCALAPPDATA`, `SystemRoot`, 새 stage의 `TEMP`·`TMP`만 명시. 부모 환경·PATH·HOME·토큰을 상속하지 않음 |
| profile 저장소 | Windows가 LocalAppData/Temp를 새 profile 전용 위치로 재지정할 수 있음. 이 새 profile은 child가 쓰는 별도 임시 저장소임 |
| 실행 시간 | 정책 timeout과 60초 중 작은 값. 0은 실행 없이 TimedOut |
| 프로세스 | private Job의 active-process limit 1, kill-on-close, breakaway 없음 |
| 메모리 | Job 합계 메모리 제한 128 MiB |
| 출력 | stdout 원시 앞 4096 bytes만 보관, stderr는 보관하지 않고 drain. UTF-8 손실 변환 뒤 메시지도 문자 경계에서 최대 4096 bytes |
| 출력 폭주 | tick당 제한된 drain 뒤 deadline을 다시 확인. 무한 출력도 timeout/Job 정리 |
| 파일 공간 | 새 writable profile에 별도 디스크 quota는 없음. 이 기능을 디스크 quota가 있는 VM으로 설명하지 않음 |

`LOCALAPPDATA`는 profile 실행에 필요한 OS known-folder 값이며, 그 경로를 전달하는 것이 기존 host 디렉터리에 SID 권한을 부여하지 않는다. 실행 시간이 끝났다는 사실과 profile 삭제 API의 소요 시간을 같은 제한으로 약속하지 않는다.

정상 반환은 Job-empty와 정리 완료를 확인한다. 프로세스 강제 종료나 host crash 때는 Job handle 종료가 실행 프로세스를 정리하지만, 이때 host의 Drop/삭제 코드가 실행되지 않아 **이번 실행의 passive stage/profile 잔재가 남을 수 있다**. 재시작 시 기존 profile을 임의 정리하는 기능은 추가하지 않았다. [DeleteAppContainerProfile](https://learn.microsoft.com/en-us/windows/win32/api/userenv/nf-userenv-deleteappcontainerprofile) 실패는 문서에 따라 제한적으로 재시도하고, 실제 디렉터리 부재까지 확인한다.

## 실제 Windows 검증

검증 환경: Windows x64 `10.0.26200.0`, Rust `1.95.0-x86_64-pc-windows-msvc`. fixture는 외부 crate 없이 native PE로 컴파일한 synthetic 프로그램이며 실제 사용자 Vault·자격 증명·모델·외부 게시를 사용하지 않는다.

- production Windows 모듈로 정상 `Passed`를 얻은 뒤, owner-only synthetic Vault 파일과 AAP-readable 파일의 읽기·쓰기 거절을 확인한다. host에서는 같은 canary를 읽을 수 있고 실행 뒤 바이트가 같다.
- 같은 native fixture의 비격리 loopback 연결은 성공한다. 격리 실행에서는 **WSAStartup 반환 10107**, listener 연결 0을 관찰했다. 이 관측은 startup 단계 거절이며, 직접 connect의 WSAEACCES와 구분한다. [WSAStartup](https://learn.microsoft.com/en-us/windows/win32/api/winsock2/nf-winsock2-wsastartup)은 오류를 반환값 자체로 제공하고, [10107](https://learn.microsoft.com/en-us/windows/win32/winsock/windows-sockets-error-codes-2)은 일반적인 system-call 실패다. 특정 정책 하나가 단독 원인이라고 단정하지 않는다. 회귀는 이 관측 또는 실제 connect의 10013 거절만 인정한다.
- 일반·detached/breakaway 자식 생성 거절, 제한 환경, 거부/비정상 종료, timeout/출력 폭주, UTF-8 비정상 바이트의 메시지 제한을 실제 프로세스로 확인한다.
- 잘못된 expected SID의 suspended token 검증 실패 뒤 fixture 출력 없음, Job active 0, stage/profile 부재를 확인한다. 기존 profile 이름 충돌을 새 소유권으로 오인하지 않는 것도 확인한다.
- child가 자기 profile에 일반 payload를 쓴 뒤 정리되는 것을 확인한다. 외부 disposable 디렉터리를 가리키는 junction 생성은 같은 fixture·버퍼·최소 FILE_WRITE_ATTRIBUTES 권한의 host 제어군에서 성공했다. LPAC에서는 해당 디렉터리 open은 성공하고 **FSCTL_SET_REPARSE_POINT가 ERROR_ACCESS_DENIED(5)**로 거절됐다. profile/stage 부재와 외부 canary의 바이트·volume/file-index 보존을 확인했다. 성공한 junction을 삭제했다고 주장하지 않으며, 거절을 피하려고 권한을 넓히지 않는다. [Mount-point FSCTL 규약](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/4aeefef8-92c3-4abc-af7a-a610caf8a165)을 따른다.
- 기존 `gate_issue_publish_with_code_validators`와 실제 production runner를 연결해 Passed만 허용하고 Rejected/Crashed/TimedOut/HashMismatch/IsolationUnavailable을 게시 차단으로 확인한다.

2026-11-27에 `prerelease@fa72e69a868826e6b61ec0f935260d1e5b9344a8`의 Windows runner와 fixture를
변경하지 않은 상태에서 Windows `10.0.26200` x64, Rust 1.95.0으로 native 모듈을 다시 실행했다.
PowerShell Full의 해당 selector는 **12 passed / 0 failed / 0 ignored**, 1.80초였다.
`native_windows_validator_user_cancel_ends_the_owned_job_and_cleans_up`도 실제 LPAC 실행과
취소 뒤 owned stage/profile 정리까지 통과했다. 이 모듈 결과와 전체 Full 결과는 구분한다.
전체 실행 중 Propose fixture의 플랫폼별 lease 파일명 오류를 발견한 기록은
[Propose 검증 기록](../validation/issue-automation-propose-flow.md)에 있다.
기존 PowerShell inventory에만 있던 Windows validator selector는 CI Bash inventory에도 추가하고
두 경로에서 정확히 한 번 포함되는지 정책 시험으로 검사한다.

초기 profileless 생성은 오류 2, profile만 만든 제한 환경은 오류 203을 반환했다. profile과 필수 known-folder 환경을 준비한 뒤에도 console subsystem fixture는 초기화에 실패했다. 최종 지원 프로필은 실제 통과한 headless Windows-subsystem PE로 제한하며, 이 조사 과정에서 network/registry capability나 Win32k 예외를 production에 추가하지 않았다.

## 사용자 취소

게시 요청(`akra/issue/publish`, `akra/issue/create/publish`)에 클라이언트가 정한 `validationId`를 주면, 실행 중에 `akra/issue/validation/cancel { validationId }`로 그 게시의 코드 검증을 취소할 수 있다. Gateway는 두 요청을 따로 처리하므로 취소가 실행 중인 검증에 닿는다.

- 모든 runner(macOS, Linux, Windows)는 제한 시간을 확인하는 자리에서 취소도 확인한다. 취소를 받으면 제한 시간과 같은 정리 경로로 프로세스 트리를 끝낸다: macOS 프로세스 그룹 SIGKILL, Linux contained-child 정리, Windows owned Job 종료와 Job-empty 확인. 결과는 `NotRun(Cancelled)`이다.
- 서비스는 검증기 실행 뒤 결과를 확정한다. 확정 전에 받은 취소는 늦게 끝난 성공을 포함한 모든 결과를 버리고 `IssuePublishBlocked::Cancelled`로 게시를 막는다. 필수·선택 검증기 모두 같다. 미리보기 오류는 `{ field: "code_validator", kind: "cancelled" }`이다.
- 결과 확정 뒤의 취소와 모르는 ID는 `{ cancelled: false }`이다. 이미 확정한 게시의 원격 쓰기는 이 취소로 멈추지 않는다.
- 취소한 검증의 남은 검증기는 시작하지 않는다. 격리 없는 fallback은 없다.
- `validationId`가 없는 요청은 취소할 수 없다. Desktop의 이슈 수정·신규 생성 화면은 게시 요청마다 ID를 보내며, 진행 중인 요청에 「검증 취소」 단추를 표시한다 (#2858 / PR #2899). 같은 ID가 실행 중이면 새 게시 요청을 거절한다.

일반 interpreter 지원, WSL 우회, 기존 Vault helper 승인 권한 재사용은 포함하지 않는다. Linux/Windows 실제 호스트 근거(#2703)는 이 문서의 실행 기록 범위만 주장한다. 취소 경로의 실제 Windows 실행은 native gate의 `native_windows_validator_user_cancel_ends_the_owned_job_and_cleans_up`이 맡는다.

## 실행 명령

```powershell
cargo test --locked --all-features --lib --target x86_64-pc-windows-msvc adapter::outbound::issue_code_validator::windows:: -- --nocapture --test-threads=2
powershell -NoProfile -File scripts/check_native_pr.ps1 -Mode Full
```

Windows native gate의 selector에도 위 모듈을 포함해 지원 형식, 격리, 정리, 실제 게시 관문을 반복 검증한다. 실행 SHA·최종 Full 결과는 PR 검증 기록에 남긴다.
