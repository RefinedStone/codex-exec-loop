# 실제 OS 확인 + 키 저장소 + 원문 복원 검증 (#2704)

상위 #2414. **실행 전 검증 절차**. 합성 키/확인을 사용하는 기존 CI와 이 실제 OS 검증을 구분한다. Windows/macOS/Linux의 실제 성공 여부는 실행 후에만 기록한다.

## 준비 및 안전 경계

전용 폐기 가능한 대화형 OS 사용자/VM에서만 진행한다. 본인 운영 계정, 사용자 Vault, 실제 GitHub/LLM 계정, 실제 원문·토큰은 사용하지 않는다. OS마다 새 계정 또는 disposable VM을 준비하고 Akra의 기존 프로필을 복사하지 않는다. 시스템 확인 창을 사람이 직접 조작해야 하므로 headless/자동 승인/CI에서는 실행하지 않는다.

| 환경 | 사전 조건 | 아직 증명해야 하는 것 |
| --- | --- | --- |
| Windows | 대화형 desktop, Credential Manager, 이 revision의 akra-gateway.exe | production helper 프로세스/Job/pipe와 실제 확인 창, test namespace keyring |
| macOS | 대화형 로그인과 unlocked Keychain, 시스템 osascript | 취소/승인 및 Keychain 실제 roundtrip |
| Linux | 대화형 X11/Wayland, 신뢰된 /usr/bin/zenity, D-Bus Secret Service | 실제 confirmation과 secret-service roundtrip |

Test-only ManualCredentialFixture는 랜덤 신규 service 이름과 별도 임시 journal을 사용하며 제품 이름 akra-evidence-vault를 읽거나 쓰지 않는다. 같은 실제 OS credential backend·초기화 journal 계약을 사용하고, 기존 entry가 있으면 중단한다. 삭제도 자신이 생성한 namespace로 제한한다. 강제 종료하면 fixture entry가 남을 수 있으므로 해당 폐기 프로필 전체를 삭제한다. 키 값·실제 원문·credential 목록은 결과에 남기지 않는다.

## 실제 OS 실행

같은 revision에서 cargo build --locked --bin akra-gateway를 수행한다. Windows의 `AKRA_MANUAL_GATEWAY_EXE`에는 해당 빌드 원본의 절대 경로를 지정한다. 수동 fixture는 OS credential에 접근하기 전에 저장소 밖 private 임시 디렉터리에 원본을 복사한다. 공유 읽기만 허용한 원본 handle, 새 파일 생성, 현재 사용자 전용 ACL, SHA-256 동일성, 기존 실행 파일 신뢰 검사와 native 형식 검사를 모두 통과한 사본만 실행한다. 저장소 안 빌드 원본을 직접 실행하도록 제품의 신뢰 검사를 완화하지 않는다. 외부 임시 경로가 신뢰 검사에 실패하면 중단한다. 다른 OS는 실제 native adapter를 호출한다. 환경 변수를 지정하는 것은 실제 OS 승인 대신 사용되는 값이 아니다.

Windows PowerShell (폐기 사용자에서만):

```powershell
cargo build --locked --bin akra-gateway
$env:AKRA_MANUAL_VAULT_PROFILE = "isolated-disposable"
$env:AKRA_MANUAL_GATEWAY_EXE = (Resolve-Path target/debug/akra-gateway.exe).Path
cargo test --locked --lib adapter::inbound::desktop_restore_bridge::tests::manual_os::real_os_confirmation_and_credential_restore_roundtrip -- --ignored --exact --nocapture
Remove-Item Env:AKRA_MANUAL_VAULT_PROFILE, Env:AKRA_MANUAL_GATEWAY_EXE
```

macOS/Linux:

```sh
AKRA_MANUAL_VAULT_PROFILE=isolated-disposable cargo test --locked --lib adapter::inbound::desktop_restore_bridge::tests::manual_os::real_os_confirmation_and_credential_restore_roundtrip -- --ignored --exact --nocapture
```

테스트는 새 키의 재조회, 실제 암호화된 Vault binding, Desktop 전용 pipe 프로토콜과 실제 native 확인을 연결한다. 원격 스냅샷과 checkout resolver는 합성 fixture다. Windows는 동일성을 검증한 실제 production helper binary의 private 사본을 실행하고, libtest binary로 native 확인을 대체하지 않는다. 별도 자동 회귀는 credential·확인 창을 호출하지 않고 합성 native 실행 파일의 준비·신뢰 경계·덮어쓰기 거절만 검증한다. 첫 번째 창은 취소, 다음 열람 창은 승인, 외부 편집 후 열람 창과 별도 연결 저장 창은 승인한다. 실제 GitHub·모델을 호출하지 않는다.

## 별도 실제 Desktop UI 검증

위 시험은 Electron renderer나 실제 GitHub 조회를 포함하지 않는다. disposable profile에서 제품 빌드를 사용하여 다음 결과를 직접 확인하고 증거를 기록한다. 실제 원격 연결이 필요하면 명시적 승인 후 별도 샌드박스 GitHub 계정과 테스트 이슈로만 실시한다. 승인 전에는 임의의 실제 계정에 연결하지 않는다.

1. 이슈 원문 보기에서 OS 확인 취소와 승인; 승인 전 원문이 나타나지 않는지 확인.
2. 새 프로세스/창에서 다시 승인하여 복원; 다른 이슈/프로젝트에서는 복원 거절.
3. 외부 편집 이후 기존 링크 복원 보류, 재연결 검토, 별도 저장 승인, 저장 후 새 원문.
4. 창 닫기/재로드/프로젝트 전환/OS 잠금/5분 만료/agent 시작으로 grant 취소.
5. 지원하지 않는 실행 프로필에서 보류. 일반 Gateway/채팅/세션/로그에는 원문이 없는지 확인.

## 증거 표 (실행 전)

| OS | 실제 native+credential+private pipe | 실제 Electron 창/OS 잠금 | 결과 |
| --- | --- | --- | --- |
| Windows | 미실행 | 미실행 | 보류 |
| macOS | 미실행 | 미실행 | 보류 |
| Linux | 미실행 | 미실행 | 보류 |

실행한 revision, OS/지원 프로필, 각 단계의 성공/취소/차단 여부만 기록한다. 화면에 개인 정보가 포함되면 캡처하지 않는다. 합성 화면·프로세스 CI 또는 ignored test의 컴파일만으로 실제 OS 검증 완료를 주장하지 않는다.
