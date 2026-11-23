# Windows Vault 원문 복원과 표식 재연결 (#2688)

상위: #2414. 이 문서는 Windows에서 사용하는 파일·native 확인 경계와 재현 절차를 설명합니다.
기존 Unix 실행 기록은 [최초 읽기 연결](desktop-vault-native-restore.md)과
[native 재연결](desktop-vault-native-reconnect.md)에 남습니다.

## 사용자 결과와 지원 범위

같은 호스트의 기존 Vault에 저장된 GitHub 이슈를 OS 확인 후 Desktop의 전용 원문 창에서
열람합니다. 게시본이 달라졌다면 기존 표식 검토를 사용하고, **별도의 저장 확인**을 거쳐
해당 이슈의 현재 게시본에 다시 연결합니다. 일반 대화·Gateway RPC에는 복원 평문을 보내지 않습니다.

| 경계 | Windows 동작 |
| --- | --- |
| 파일시스템 | 드라이브 문자 경로의 로컬 NTFS와 persistent ACL이 확인되는 실제 디렉터리만 허용합니다. ReFS·네트워크 공유·판별 불가 volume은 거절합니다. |
| 저장소 권한 | 기존 owner·ACL·reparse·hardlink·파일 신원을 검증합니다. 안전하지 않은 기존 ACL을 자동 수리하지 않습니다. |
| 키와 형식 | 기존 `akra-evidence-vault / device-key-v1`, record format 1, AEAD, scope와 key ID를 유지합니다. |
| 읽기 | 없는 root·records 디렉터리·키를 새로 만들지 않습니다. 암호문은 8 MiB와 UTF-8 제한으로 읽습니다. |
| OS 확인 | 열람과 연결 저장을 구분하며 기본 선택은 취소입니다. 신뢰할 수 있는 Gateway 실행 파일의 별도 프로세스를 사용합니다. |
| 실행 프로필 | 자동화·개발 renderer·숨긴 시험 창·앱에서 진행 중인 agent turn의 기존 복원 제한을 유지합니다. |
| 백업 | 기존 서비스가 같은 파일·binding 경계를 사용합니다. 복구 키·archive를 고르는 새 화면은 추가하지 않습니다. |

복원 대상은 기존과 같은 canonical primary checkout입니다. Windows 경로의 대소문자나
구분자를 scope 계산 전에 다시 쓰지 않습니다. `C:\`와 `\\?\C:\` 표기는 기존 게시 경로와
같은 `fs::canonicalize` 결과로 수렴해야 합니다. 대상 조회는 Git과 파일 메타데이터를 읽기만
하며 저장소 incarnation marker·lock·authority database를 만들지 않습니다.

## 암호문과 binding 저장

`vault_fs`는 Vault 레코드의 파일 연산만 제공합니다. Unix에서는 기존 `secure_fs`와
private runtime lease에 위임하고, Windows에서는 다음 순서를 사용합니다.

1. 기존 경로 handle로 지원 volume을 확인합니다. 새 디렉터리·lock·임시 파일을 만들기 전입니다.
2. 조상 디렉터리 handle을 고정하고 기존 owner·ACL·신원을 검증합니다.
3. 같은 이슈의 binding lease를 잡고, 파일 변경 때 root mutation lease를 추가로 잡습니다.
   게시 경로의 publication lease가 필요하면 그것을 먼저 잡습니다.
4. 새 private 임시 파일에 암호문을 쓰고 flush합니다. generation은 no-replace,
   pointer는 기대 암호문·신원과 현재 대상을 다시 대조한 뒤 설치합니다.
5. 고정한 부모 handle에서 Volume GUID 절대 경로를 얻고 검증한 파일명을 붙입니다.
   부모와 조상 handle의 고정을 유지한 채 `RootDirectory = NULL`인
   `SetFileInformationByHandle(FileRenameInfo)`를 source handle에 한 번 호출합니다.
   설치 후 flush, 대상 신원, 암호문 readback이 모두 성공해야 확정합니다.

파일명·scope·AAD·generation HMAC 및 `AlreadyCommitted`·`Conflict` 정책은 바꾸지 않습니다.
pointer 설치 이후 결과를 확정할 수 없으면 `WriteOutcomeUnknown`으로 반환하며, 자동 재전송이나
되돌리기를 하지 않습니다. 불확실한 설치의 source를 임시 파일로 간주해 삭제하지 않습니다.
generation 준비 실패는 현재 pointer와 도달 가능한 기존 묶음을 보존합니다.

Windows의 파일 flush와 write-through는 문서화된 NTFS 동작에 의존합니다. 디렉터리 handle
검증을 Unix directory fsync나 모든 장치의 전원 손실 보장으로 해석하지 않습니다.
API 근거와 상세 계약은 [binding 문서](../reference/evidence-vault-bindings.md)에 있습니다.

### Windows 이름 변경 인자 확인

실제 Windows 호스트에서 non-NULL `RootDirectory`는 상대명·절대명 모두 오류 87로
거절됐습니다. 동일한 source handle과 부모 고정을 유지한 `RootDirectory = NULL` +
절대명은 성공했습니다. 구조체 offset, 버퍼 길이, 부모 handle 권한을 분리해 인자 경계를
확인했으며, 구현은 처음부터 절대명을 한 번 사용합니다. 오류 뒤 다른 이름 변경 방식으로
자동 재시도하지 않습니다.

부모 handle에서 얻은 Volume GUID 경로에서도 source·destination 파일 신원과 내용이
일치했습니다. 한글을 포함한 487~490 UTF-16 단위 경로에서 신규 설치와 교체가 성공했고,
no-replace 충돌은 오류 183을 반환하며 기존 목적지와 source를 보존했습니다. 시험의
파일 신원 구조체는 실제 Windows ABI와 같은 52바이트 배치임을 확인한 뒤 관측했습니다.

## 확인 프로세스와 취소

Gateway의 고정 `vault-native-confirmation` 입구는 진단 초기화 전에 분기합니다.
대상·목적·nonce는 argv나 승인 환경변수가 아닌 inherited private stdin으로 전달됩니다.
요청은 최대 16 KiB, 영수증은 최대 1 KiB이며 한 요청·한 응답만 허용합니다.

native 확인용 프로세스는 `CREATE_SUSPENDED | CREATE_NO_WINDOW`로 만들고 Job에 넣은 뒤
재개합니다. pipe에 올바른 영수증이 왔더라도 목적·nonce·형식과 정상 종료를 모두 확인해야
승인합니다. 알 수 없는 필드, 추가 응답, 다른 목적·nonce, 실패 종료, 계속 살아 있는 helper는
승인하지 않습니다.

helper는 확인 창을 기다리는 동안 부모 EOF·기한·추가 입력을 감시합니다. 부모 취소·창 수명
무효화·기한 경과·강제 종료 시 승인과 늦은 결과를 폐기합니다. 부모의 cleanup과 pipe writer
대기는 2초로 제한하며, 정리를 확인할 수 없으면 `Unavailable`입니다. 재연결 쓰기가 이미
admission을 통과했다면 저장이 끝날 수 있지만, 닫힌 창으로 평문을 다시 보내지 않습니다.

## 회귀 시험의 범위

- **실제 Windows 파일:** native handle no-replace/replace, source 신원과 readback, 없는 경로
  무생성, 크기·UTF-8 상한, 안전하지 않은 ACL 비수리, hardlink·junction 거절, 읽기 중 변경.
- **실제 Windows 프로세스:** 두 프로세스 CAS의 단일 승자, lease owner 종료·재수집 후 같은
  lock 객체 재획득, 부모만 강제 종료했을 때 inner Job의 helper·손자 정리.
- **실제 private pipe와 가짜 확인 함수:** 정상 종료 전 영수증 미수락, 늦은 승인·EOF·기한,
  잘못된 응답과 초과 출력, 읽지 않는 child, 지연된 cleanup·writer. child 안에서 콘솔 부재도
  확인합니다.
- **기존 암호화 계약:** 고정 format 1 암호문, Windows workspace 원문 유지, 실제 파일을
  사용한 복원→재연결→권한 만료, 늦은 publish finalize 거절, journal ACK 유실 재시도,
  백업의 readable binding 보존과 손상 pointer의 명시적 복구.
- **실제 Git fixture:** 일반·정규화·linked 경로 조회 전후 파일 목록과 marker 부재.

실제 OS 확인 창, 사용자 키 저장소, 실제 GitHub/Jira/Linear 데이터, 모델 turn은 이 시험에서
사용하지 않습니다. 따라서 합성 시험의 성공을 실제 OS 대화상자와 사용자 키 저장소를 결합한
수동 성공 검증으로 계산하지 않습니다.

## 재현 명령

저장소가 지정한 Rust toolchain과 Windows Node 환경에서 실행합니다.

```powershell
node scripts/agent-plan.mjs
git diff --check
cargo fmt --all
powershell -File scripts/check_native_pr.ps1 -Mode Full
```

Windows Full/Portable과 Bash portable 스크립트에 파일·binding·backup·restore·native pipe
시험을 연결했습니다. Linux는 전체 Rust suite로 확인하고, macOS portable CI도 Vault 저장과
복원 회귀를 실행합니다. CI Gate는 PR의 최신 head를 기준으로 확인합니다.

## 최종 로컬 검증 결과

2026-11-24 KST, Windows 호스트의 전용 작업 디렉터리에서 실행했습니다.

| 검사 | 관측 결과 |
| --- | --- |
| Vault 집중 시험 | 92개 통과, 실패 0개 |
| Windows 제품 빌드와 아키텍처 검사 | 144개 통과, 실패 0개 |
| 저장소 지정 Windows Full gate | 종료 코드 0 |
| Full gate의 Windows Rust 계약 시험 | 33개 실행 묶음에서 299개 통과, 실패·제외 0개 |
| Desktop Vitest | 1,157개 통과, 7개 제외; 시험 파일 74개 통과, 1개 제외 |
| Desktop 타입 검사·빌드, Rust Clippy | 통과 |
| `cargo fmt --all`, `git diff --check` | 통과 |

초기 실제 파일 시험에서 발견한 이름 변경 오류 87은 위의 Volume GUID 절대 경로 방식으로
수정했습니다. 이후 긴 한글 경로를 포함한 파일 경계 21개가 모두 통과했습니다. 다른 scope에
암호문을 복사하는 기존 시험 3개는 Windows ACL을 갖춘 private 파일로 같은 바이트를 설치하도록
준비 방식을 보정했습니다. 기대하는 `IntegrityFailure`와 재암호화 검증은 유지했습니다.

Desktop 소스와 화면은 변경하지 않았습니다. 이번 기록은 기존 전용 창의 Windows native 연결과
실제 파일·프로세스 경계 검증이며, 앞서 명시한 실제 OS 확인창·사용자 키 저장소의 수동 통합
시험을 대신하지 않습니다. PR의 Linux·macOS·Windows 결과와 `CI Gate`는 최신 head의 Checks에 기록합니다.
