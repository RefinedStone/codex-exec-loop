# Windows 실제 Vault 수동 실행기 (#2724)

이 도구는 실제 제품 Electron 창과 production helper를 폐기 OS 계정에서 실행하고, 관측 결과를 남깁니다. 제품의 native 확인·keyring namespace·자동화 보류 조건을 변경하지 않습니다. 도구와 단위 시험이 준비되었다는 사실은 #2724의 실제 OS 검증 완료를 뜻하지 않습니다.

## 실행 전 조건

별도의 **폐기 가능한 Windows OS 사용자**로 로그인해야 합니다. Electron userData만 바꾸어도 Credential Manager의 제품 키는 같은 OS 계정에 남기 때문에, 현재 운영 계정에서 이 도구를 실행해서는 안 됩니다. 실행기는 명시한 SID와 현재 OS 토큰의 SID를 대조하지만, 계정이 실제로 폐기 가능한지는 작업자가 확인해야 합니다.

작업자는 sandbox GitHub 계정·기존 테스트 이슈·그 이슈의 실제 Git checkout을 준비합니다. 제품은 GitHub snapshot과 고정 akra-evidence-vault / device-key-v1 키를 사용합니다. 실행기가 원격 공급자나 credential namespace를 가짜로 바꾸지 않습니다. GitHub 접근과 게시 조작도 사용자가 제품 화면에서 직접 수행합니다. 운영 원문·키·credential·모델 턴을 사용하지 않습니다.

일반 대화형 터미널에서 실행합니다. CI, Playwright, renderer 개발 서버, hidden 창, 디버깅/Node 옵션, 다른 프로필·Gateway override, 명시적 API token 환경이 있으면 실행을 보류합니다. 실행기는 이런 값을 지워서 제품 검사를 통과시키지 않습니다.

## 같은 SHA의 제품 빌드

폐기 계정의 새 checkout에서 작업합니다. 빌드 후 tracked/untracked 변경이 없는 상태를 유지합니다. 설정 파일은 checkout 밖에 둡니다.

~~~powershell
git rev-parse HEAD
cargo build --locked --target x86_64-pc-windows-msvc --bin akra-gateway
Push-Location desktop
npm ci
npm run build
Pop-Location
whoami /user
~~~

현재 SID와 SHA, 실제 sandbox 경로를 다음 설정에 넣습니다. 실행기는 SHA와 파일 SHA-256을 기록하며, 빌드가 해당 SHA에서 이루어졌는지 확인하는 책임은 작업자에게 있습니다. 임의의 다른 바이너리를 같은 버전 문자열만 보고 사용하지 않습니다.

~~~json
{
  "expectedRevision": "0000000000000000000000000000000000000000",
  "expectedUserSid": "S-1-5-21-111111111-222222222-333333333-1001",
  "acknowledgement": "DISPOSABLE_WINDOWS_OS_ACCOUNT_AND_SANDBOX_ISSUE",
  "sandboxIssueUrl": "https://github.com/fixture-owner/fixture-repo/issues/7",
  "workspace": "C:\\fixture\\sandbox-checkout",
  "gatewayExecutable": "C:\\fixture\\akra\\target\\x86_64-pc-windows-msvc\\debug\\akra-gateway.exe",
  "electronExecutable": "C:\\fixture\\akra\\desktop\\node_modules\\electron\\dist\\electron.exe"
}
~~~

## 준비 확인과 실제 실행

~~~powershell
node scripts/windows-vault-live.mjs --dry-run C:\fixture\vault-live.json
node scripts/windows-vault-live.mjs --launch C:\fixture\vault-live.json
~~~

dry-run은 OS SID·환경·checkout·빌드 파일을 읽고 해시만 확인합니다. 제품 실행, 파일 작성, credential 호출은 하지 않습니다.

실제 실행은 OS 프로필 바로 아래의 새 무작위 폴더를 현재 사용자 전용 ACL로 만들고, 원본 helper를 쓰기/삭제 공유 없는 handle로 고정한 채 private 사본을 생성합니다. 사본 SHA-256이 원본과 같은지 확인하며 기존 파일이나 조상 ACL을 수리하지 않습니다. 제품 helper가 자체 실행 파일·조상·ACL·native 형식 검사를 통과해야 실제 복원이 가능합니다.

동일 폴더의 새 AKRA_HOME과 AKRA_DESKTOP_USER_DATA로 일반 visible Electron을 실행합니다. remote debugging, renderer 상태 주입, 메모리 transport, 승인 자동화는 없습니다. 앱 화면에서 sandbox checkout과 테스트 이슈를 선택합니다.

Windows Gateway 접속 주소는 OS 사용자 SID만으로 정해지고, 이전 실행의 Gateway는 Electron 종료 뒤에도 남을 수 있습니다. 그래서 실행기는 Electron을 열기 전에 현재 사용자의 Gateway pipe를 확인합니다. pipe 소유 프로세스의 실행 파일이 이 실행 폴더의 helper가 아니거나, 이 실행 AKRA_HOME의 `run\gateway.lock`이 잠겨 있지 않거나, 확인 자체가 실패하면 제품을 시작하지 않습니다. 그 akra-gateway를 종료한 뒤 `--resume`으로 다시 시작합니다. `--resume`은 같은 실행의 Gateway만 다시 사용합니다.

터미널에 표시한 이 실행 전용 합성 canary를 이슈 작성 화면의 **복원 가능한 가림 영역**으로 넣고, 공개 미리보기에서 가려졌음을 확인한 뒤 sandbox 이슈에 게시합니다. canary를 일반 공개문·대화·모델 입력으로 보내지 않습니다. 실행기는 게시를 자동 수행하지 않습니다.

다음을 사람이 실제로 조작합니다. Windows native 확인은 기본 취소인 제품 대화상자이며, 실행기가 대신 승인하지 않습니다.

| 기록 키 | 실제 수행 |
| --- | --- |
| cancel_view | 첫 열람 native 확인을 취소하고 원문 미표시 확인 |
| approve_view | 다시 열어 승인하고 전용 창에서 합성 canary 복원 확인 |
| restart_restore | 제품을 닫고 같은 실행 기록으로 다시 열어 동일 키·binding 복원 확인 |
| external_edit_hold | sandbox 공개문 외부 편집 뒤 기존 연결 복원이 보류되는지 확인 |
| separate_reconnect_approval | 현재 표식 검토와 별도 연결 저장 승인을 수행하고 새 판 복원 확인 |
| window_close | 대기 중/복원 후 창 닫기에서 늦은 원문이 재등장하지 않는지 확인 |
| window_reload | 일반/전용 창 reload 후 재가림과 이전 대상 재활성화 차단 확인 |
| project_issue_switch | A→B→A 전환에서 다른 이슈·창으로 원문이 넘어가지 않는지 확인 |
| os_lock | 실제 Windows 잠금 후 돌아왔을 때 재가림 확인 |
| five_minute_expiry | 실제 5분 기한 이후 재가림 확인 |

재시작은 처음 표시한 경로의 run.json을 그대로 사용합니다. 새로운 키를 만들거나 fixture 프로필을 초기화하지 않습니다.

~~~powershell
node scripts/windows-vault-live.mjs --resume C:\Users\fixture\akra-vault-live-RUN_ID\run.json
~~~

## 증거 기록과 누출 검사

생성된 operator-results.template.json을 별도 operator-results.json으로 복사한 뒤 실제 수행한 항목만 pass, fail, blocked로 기록합니다. 미실행 항목은 not_run입니다. 허용된 사유는 none, operator_not_available, disposable_profile_missing, sandbox_issue_missing, native_confirmation_unavailable, product_hold, unexpected_result입니다. 자유 텍스트·원문·스크린샷·키·proof는 결과 JSON에 넣지 않습니다.

~~~powershell
node scripts/windows-vault-live.mjs --report C:\Users\fixture\akra-vault-live-RUN_ID\run.json C:\fixture\operator-results.json
~~~

report.json은 실행 SHA, gateway/Electron 실행 파일과 main index.js·vaultRestore preload.js·vault-restore.html의 해시, SID 해시, 관측한 제품 프로세스 수명, **operator_reported** 단계, 자동 canary 파일 검사를 분리합니다. 이 해시 목록은 주요 진입 파일만 포함하며 모든 renderer chunk·의존성·전체 bundle의 출처 인증이 아닙니다. stdout/stderr는 화면에 재출력하지 않고 해당 실행의 전용 폴더에 최대 각 8 MiB만 보관합니다.

제품을 닫고 실행기가 종료 기록을 저장한 뒤 보고서를 만듭니다. 파일 검사는 해당 실행의 akra-home, electron-user-data, captures만 대상으로 UTF-8/UTF-16LE canary를 찾습니다. 각 디렉터리의 진입/종료 identity와 수정 시각도 비교하므로 검사 중 항목 추가·삭제·교체를 감지하면 incomplete로 남깁니다. 다른 프로필·OS credential 목록을 열지 않습니다. 링크·읽기 실패·파일/전체 예산 초과는 incomplete로 기록합니다. 로그가 잘리거나 검사하지 못한 파일이 있으면 깨끗한 종단 간 검증으로 판단하지 않습니다. 실행 중 계속 변하는 파일의 검사도 특정 시점 관측일 뿐이며, 원문 창/일반 화면·원격 게시 내용은 위의 수동 확인과 별개입니다.

기본 결과는 항상 실제 OS 검토가 필요한 상태입니다. 프로세스 정상 종료나 canary 미관측만으로 승인·복원·재잠금 성공을 자동 기록하지 않습니다. PR에는 비민감 report.json을 검토한 결과만 반영하고 raw 로그·원문 화면은 게시하지 않습니다. 종료 뒤에는 본인이 만든 fixture 프로세스가 모두 종료되었는지 확인하고 폐기 OS 계정 전체를 정리합니다. 도구는 Credential Manager 키나 운영 프로필을 자동 삭제하지 않습니다.

## 현재 실행 범위

`node --test scripts/windows-vault-live.test.mjs`는 입력/환경 차단, 다른 SID의 실제 dry-run 거부, 결과 분류, UTF-8/UTF-16LE canary의 청크 경계, 링크·누락 경로·검사 예산 처리를 검증합니다. Windows 파일 시험은 OS의 whoami.exe를 실행하지 않고 바이트만 새 전용 폴더에 복사하여 동일 해시, 소유자·보호 DACL, 폴더 재사용 거부를 확인합니다. 기존 프로필이나 키는 읽지 않습니다.

실제 Electron→production helper→native 확인→Credential Manager 복원은 폐기 OS 로그인과 작업자 입력이 준비될 때 실행합니다. 입력 거부 시험은 유효한 전체 제품 실행을 대신하지 않습니다. 이 PR의 준비·dry-run 결과만으로 #2724 또는 상위 #2704/#2414를 닫지 않습니다.
