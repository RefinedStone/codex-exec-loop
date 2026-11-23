# Vault 연결 묶음의 원자적 갱신

이 문서는 #2654의 저장 계약을 설명합니다. 게시 확정, 서비스 재연결, 백업 가져오기는
`EvidenceVaultStorePort::begin_binding`을 통해 같은 이슈의 연결을 갱신합니다. Native helper와
Desktop 전용 창은 #2656의 [별도 승인·세션 경계](../validation/desktop-vault-native-reconnect.md)를
거쳐 이 저장 계약을 사용합니다.

## 이슈별 저장 경계

트랜잭션은 전체 이슈 좌표(프로젝트, 공급자, 서버, 저장소, 이슈)와 한 키에 고정됩니다.
Unix의 기존 private runtime lease와 Windows의 Vault 전용 파일 lease가 같은 이슈의
참여 프로세스를 직렬화합니다. 게시 경로는 publication lease, binding lease 순서로 획득하고,
개별 파일 변경은 그 안에서 root mutation lease를 사용합니다. 사용자 확인을 기다리는 동안
이 잠금을 보유하지 않습니다.

`snapshot`은 현재 포인터의 부재 또는 실제 암호문 지문을 담은 token과
`Missing | Readable | Unreadable` 상태를 제공합니다. `Unreadable`은 포인터 자체를 열 수 없는
경우입니다. 안전하지 않은 경로, symlink/hardlink, 소유권 문제, 파일 읽기 오류를
복구 가능한 unreadable 상태로 바꾸지 않습니다. 참조 필드의 손상 역시 readable pointer를
자동으로 교체할 근거가 되지 않습니다.

`load_version_fields`는 같은 트랜잭션에서 제목·본문의 존재와 내용을 함께 읽습니다.
기존 store의 `load`를 다시 호출하는 재진입은 요구하지 않습니다. `save`는 미확정
`PublishIntent` staging에만 사용하며, `ConfirmedVersion`과 `CurrentVersion`의 운영 쓰기는
binding commit을 거쳐야 합니다. 이전 형식 시험용 seed는 `cfg(test)`로 제한합니다.

## 불변 generation과 확정

새 제목·본문은 새 generation에 준비됩니다. generation identity는 다음 전체 값을
장치 키의 HMAC-SHA256에 묶습니다.

- 전체 이슈 좌표, key identifier, 기대 포인터의 부재 또는 암호문 지문
- 안정적인 operation identifier, 원본 base version, 승인 지문, 교체 정책
- 제목과 본문의 명시적인 부재 또는 전체 내용

일반 평문 해시를 파일명에 노출하지 않습니다. 같은 generation의 기존 필드는 같은 내용일
때만 재사용하며 다른 내용으로 덮지 않습니다. 두 필드가 준비된 뒤 현재 포인터를
기대 암호문에 대한 CAS로 교체합니다.

동일한 전체 요청이 이미 current이면 실제 두 필드도 대조하고 `AlreadyCommitted`를
반환하며 추가 쓰기는 하지 않습니다. 같은 게시본 해시라도 mapping이나 기대 연결이
다르면 동일 요청이 아닙니다. 다른 현재 연결을 발견하면 `Conflict`입니다.

| 결과 | 의미 |
| --- | --- |
| `Committed` | 새 bundle과 current pointer를 확정했습니다. |
| `AlreadyCommitted` | 같은 요청의 같은 bundle이 이미 current입니다. 쓰지 않았습니다. |
| `Conflict` | 기대 연결 또는 기존 immutable generation과 맞지 않습니다. |
| pointer 이전 오류 | 이전 current와 도달 가능한 bundle은 보존됩니다. 참조되지 않은 staging이 남을 수 있습니다. |
| `WriteOutcomeUnknown` | pointer 교체 요청 이후 동기화·검증 실패 등으로 설치 결과를 확정할 수 없습니다. 이전 상태로 되돌아갔다고 가정하지 않습니다. |

이 경계는 이슈 하나에 적용됩니다. 여러 이슈를 포함하는 백업 전체를 하나의 트랜잭션으로
확정하지 않으며, 원격 공급자의 편집과 로컬 파일 저장을 분산 트랜잭션으로 만들지 않습니다.

## 재연결 승인과 재시도

`VaultReconnectApproval::from_review`는 검토의 전체 이슈, 로컬 token, 원래 source version,
최신 게시본 해시, 정규화한 mapping 전체를 고정합니다. 이 타입과 키·평문·transaction 요청은
직렬화 가능한 일반 도구 입력이 아닙니다.

확정 시 최신 게시본과 살아 있는 권한을 다시 확인합니다. bundle은 승인 당시 source version에서
다시 구성합니다. 첫 승인에서 일부 표식만 연결했더라도 재시도는 축소된 현재 판에서
다른 내용을 만들어내지 않습니다. 다른 프로젝트·서버·저장소·이슈의 승인과 권한을
섞을 수 없습니다. 게시본 또는 로컬 연결이 검토 이후 바뀌면 `ChangedDuringReview`로 보류합니다.

## 게시 일지와 지연된 확정

새 게시 작업은 원격 쓰기 전에 기대 binding token을 비평문 일지 descriptor로 저장합니다.
이미 unreadable인 pointer 때문에 복원 연결을 확정할 수 없다는 사실이 확인되면 게시를
Held로 보류합니다. 사용자가 명시적으로 공유본만 게시한 경로는 기존대로 동작합니다.
확정 때 새로 관측한 연결을 원래 기대값으로 바꾸지 않습니다. 따라서 게시 결과가 확인된 뒤
사용자가 재연결하거나 백업이 연결을 설치했다면 늦은 finalize는 그 결과를 덮지 않습니다.

pointer 확정 뒤 완료 일지 저장이 실패해도 동일 descriptor와 동일 intent를 다시 사용합니다.
이전 일지에 descriptor가 없는 경우는 연결 부재에서 설치하거나, 해당 intent의 기존 판과
필드 전체가 이미 정확히 일치할 때 완료 사실만 보완합니다. 다른 readable/unreadable 연결은
안전하게 보류하며 임의로 새 기대값을 부여하지 않습니다.

새 이슈 adoption은 보이지 않는 PublishIntent 필드를 준비한 뒤 같은 binding guard 아래에서
연결 부재와 원격 상태를 확인하고 일지에 그 기대값을 남깁니다.

## 백업과 형식 호환

백업은 전체 archive의 인증·복호화를 쓰기 전에 끝냅니다. 그 뒤 승인한 이슈별로 같은
binding transaction을 사용합니다. 읽을 수 있는 current는 잠금 안에서 확인하고 보존합니다.
없는 연결만 설치하며, unreadable pointer 교체는 명시적인 `RecoverUnreadable` 승인이 필요합니다.

pointer 이전에 중단된 같은 import는 동일 operation과 기대값으로 partial generation을
검증하고 이어갈 수 있습니다. pointer가 이미 설치된 경우 재시도는 readable local 정책에 따라
보존합니다. 다른 프로세스가 새 연결을 확정했다면 그 연결도 보존합니다.

저장 계약은 4, 백업 계약은 3입니다. 암호화 레코드와 archive의 파일 형식은 각각 1을 유지합니다.
기존 hash-only와 `:intent:<64 hex>` 판은 계속 읽으며 새 `:binding:<64 hex>` suffix도
엄격하게 비교합니다. 저장 계층은 base version을 불투명한 문자열로 취급합니다.

## 플랫폼과 검증 범위

Vault root는 기존 저장 계약대로 미리 준비된 사용자 전용 실제 디렉터리여야 합니다.
가져오기에서 미존재 root를 자동 생성하거나 안전하지 않은 root의 권한을 변경하지 않습니다.

Vault 전용 `vault_fs`가 Unix에서는 기존 `secure_fs`와 private runtime lease에 위임하고,
Windows에서는 드라이브 문자 경로로 지정한 로컬 NTFS의 파일·잠금 경계를 제공합니다. 읽기와 가져오기는 없는 root를
생성하지 않습니다. 쓰기 조립에서 새 root를 만들 때만 private ACL을 설정하며, 기존의
안전하지 않은 ACL을 자동으로 수리하지 않습니다.

Windows는 기존 private 경로의 조상 handle을 고정하고 reparse point, 여러 hardlink,
owner·ACL·파일 신원 불일치를 거절합니다. 지원 volume 확인은 새 디렉터리·lock·임시 파일을
만들기 전에 수행합니다. ReFS와 네트워크 공유는 이번 지원 범위에 포함하지 않습니다.

암호문은 같은 부모 안의 새 private 파일에 준비합니다. 부모 handle에서 조회한 Volume GUID
절대 경로와 검증한 파일명으로 목적지를 구성하고, `RootDirectory = NULL`인
`SetFileInformationByHandle(FileRenameInfo)`를 source handle에 한 번 호출합니다. 호출 중에도
부모와 조상 handle의 삭제 공유를 금지한 고정을 유지합니다. 드라이브 문자나 현재 디렉터리를
다시 해석하는 fallback은 없습니다. 불변 generation은 no-replace, 현재 pointer는 기대
암호문 비교 후 교체하며, 설치 이후 flush와 신원·내용 확인이 필요합니다.
`ReplaceIfExists` 자체가 암호문 CAS를 수행하는 것은 아닙니다.

Windows의 write-through와 파일 flush는 NTFS의 문서화된 파일·메타데이터 처리에 의존합니다.
디렉터리 handle 검증을 Unix의 directory fsync나 모든 저장장치의 전원 손실 보장으로
해석하지 않습니다. Microsoft의 [rename 구조](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info),
[handle 변경 API](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle),
[CreateFileW 캐시·write-through 계약](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew)을 따릅니다.

실제 OS 사용자 확인, 실제 키 저장소 round trip, native UI 재연결, 실제 모델 turn은 이 저장
변경의 검증 결과에 포함하지 않습니다. 실행 명령과 관측 결과는
[기존 검증 기록](../validation/vault-binding-transactions.md)과
[Windows Vault 검증](../validation/windows-vault-restore.md)에 분리합니다.
