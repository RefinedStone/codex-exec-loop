# Windows 기존 이슈 공유본 게시 검증

대상: #2660. Windows의 기존 이슈 편집기에서 기본 정책의 공유본을 승인·게시하고, 응답 유실이나 프로세스 재시작 뒤 재전송 없이 결과를 확인한다.

## 변경과 유지한 경계

- Windows는 private SQLite journal과 별도 publication lease DB를 사용한다. lease는 열린 연결의 BEGIN IMMEDIATE로 보유하며 30초는 획득 대기 한도다.
- Sent를 durable commit한 뒤 원격 쓰기를 시작한다. 완료 상태를 저장하지 못하면 Sent를 유지하고 이후 원격 조회로만 복구한다.
- intent 참조·hash·상태만 저장하며 원문이나 키를 journal에 추가하지 않는다. 삽입 순서, 제자리 교체, sequence=0 및 직렬화 누계 8 MiB 제한을 유지한다.
- 알 수 없는 schema/손상 기록과 legacy JSONL/lock은 거부한다. 자동 가져오기·삭제는 하지 않는다.
- 기존 Windows DB의 ACL은 검증만 한다. 원자적 create_new로 생성한 새 DB에만 초기 ACL을 설정한다.
- Unix JSONL과 서비스·승인·UI 계약, 기본 제목/required_sections 검사, Windows 템플릿/필수 외부 validator 차단은 유지한다.

## 검증 기록

변경 전 Red는 #2660의 2026-11-23 08:58 KST 기록과 보존된 Windows fixture에서 확인했다. 실제 Gateway open은 -32000 / the issue persistence lease is unavailable on this platform으로 실패했다. 기존 터널 세션은 현재 재조회할 수 없으며 이를 이번에 새로 실행한 Red로 표시하지 않는다.

최종 Windows Full: 2026-11-23 18:08:54–18:12:38 KST, 224.007초, exit 0.
명령: powershell -File scripts/check_native_pr.ps1 -Mode Full.

| 검사 | 결과 |
| --- | --- |
| Windows 지정 계약 필터 | 22개 필터, 111개 통과 |
| 실제 Windows Gateway 공유본 흐름 | 9개 통과 |
| Windows publication journal | 10개 통과 |
| 공통 private SQLite | 10개 통과 |
| automation hot-journal 복구 | 3개 통과 |
| Desktop 단위 검사 | 1,068개 통과, 기존 7개 skip |
| Node/Admin, typecheck, audit, build, format, all-targets clippy -D warnings | 통과 |

위 Gateway/journal/private SQLite/hot-journal 개수는 Windows 필터 111개의 일부이며 별도 합산하지 않는다.

Gateway 회귀는 기본 공유본과 명시적 shared-copy-only에서 key/Vault 호출 0, Held 이후 전환의 추가 호출 0, 정책 차단, 재조립·재조회와 publication 격리를 확인한다. 실제 SQLite COMMIT 실패를 주입해 Sent 저장 실패 시 원격 쓰기 0, 완료 저장 실패 시 durable Sent 유지와 조회 복구를 확인한다.

독립 worker를 원격 요청 수신 뒤 강제 종료하고 TCP 서버의 지연 요청을 유지했다. 다음 worker는 새 게시 없이 NeedsConfirmation을 유지하고, 늦은 적용 뒤 조회로 Confirmed가 된다. 전 과정의 원격 요청 수는 1이다.

처음 Full의 기존 Git 테스트 2개는 동시 빌드 부하에서 5초 제한에 걸렸다. 기본 제한을 바꾸지 않은 재실행에서 통과했다. 새 ACL 검증이 crash fixture의 복사본 권한 문제를 드러내어 시험이 새로 만드는 파일에만 owner ACL을 명시했으며 제품의 기존 DB 자동 수선은 허용하지 않았다.

실제 workflow가 호출하는 scripts/check_windows_portable.sh와 Windows 로컬 gate 모두 신규 회귀 필터를 포함한다. 최종 PR head의 원격 CI 결과는 PR의 CI Gate를 기준으로 확인한다.

## 증거와 한계

- 전체 로컬 로그: C:/dev/_tmp/issue-2660-validation-20261123/windows-native-full.log
- 기존 Red fixture: C:/dev/_tmp/windows-issue-publish/red-fixture
- 합성 GitHub/TCP 원격, 실제 Windows Gateway·SQLite·자식 프로세스를 사용했다. 실제 GitHub 이슈 게시, 실제 모델 turn, OS 키링 프롬프트, 전원 중단은 실행하지 않았다.
- Linux 로컬 실행은 하지 않았다. Unix 유지 회귀와 다른 플랫폼 검사는 원격 CI로 확인한다.
- lease 해제가 이미 전송된 원격 요청을 취소하지 않는다. 명시적 abandon과 새 승인까지 포함한 원격 exactly-once를 주장하지 않는다.
- Windows Vault/native 복원/새 이슈 생성/템플릿 reader/외부 validator 지원은 이번 범위가 아니다.
