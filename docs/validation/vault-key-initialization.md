# Vault 키 초기화 검증 기록

대상: #2658, 기준 `38687dd742d69d92930779d3798b1baf95c0417d`.

## 수정 전 Red

제품의 기존 `EvidenceVaultUnlockService::sealing_key()`와 `load_key/store_key` 계약을
그대로 둔 상태에서 `process_initialization_tests`를 추가했습니다. 공용 SQLite 추출은
병행했지만 이 시점의 제품 키 생성 경로에서는 사용하지 않았습니다.

별도 credential server 프로세스와 worker 두 개를 실행했습니다. 부모 제어 채널과 worker
RPC 채널을 분리하고, 두 worker의 최초 조회가 모두 키 없음 상태에 도달한 것을 확인한 다음
조회 응답을 해제했습니다. 각 worker가 반환받은 키로 합성 AEAD 암호문을 만들고, 최종 서버
키로 이를 다시 열어 보았습니다.

```text
cargo test --locked --all-features --lib --target x86_64-pc-windows-msvc application::service::evidence_vault_unlock::process_initialization_tests::two_processes_share_one_durable_initialization_key -- --exact --nocapture
```

2026-11-22 Windows 실행 결과:

| 항목 | 관측 |
| --- | --- |
| 관리 실행 session | `d287fc38-bead-4ff5-be85-8608ebbb7b9c` |
| exit code | 101 |
| credential 저장 | 2회 |
| worker의 성공 반환 | 서로 다른 키 ID 두 개 |
| 최종 서버 키로 합성 암호문 복원 | `[true, false]` |
| 실패 assertion | `expected stores=1, actual=2` |
| 시험 실행 시간 | 0.05초 |

이는 실제 사용자 OS keyring에서 발생시킨 사고가 아니라, 제품의 생성 서비스와 합성 공유
credential 경계를 사용한 재현입니다. 난수 키 원문은 출력하지 않았습니다.

첫 하네스 실행 `274034a8-91d7-4866-86e3-6cd4baf65d2c`는 Windows accepted socket의
nonblocking 모드를 명시적으로 해제하지 않아 `WSAEWOULDBLOCK`으로 실패했습니다.
이 하네스 오류를 수정한 뒤 위 제품 Red를 관측했으며, 첫 실패를 경합의 증거로 세지 않습니다.

## 수정 후 검증

Windows 집중 회귀와 planner가 지정한 전체 로컬 검증이 모두 통과했습니다.

| 그룹 | 결과 | 실행 session |
| --- | --- | --- |
| Unlock 서비스와 독립 프로세스 회귀 | 18 통과, 새 프로세스 시나리오 5개와 하네스 helper 2개 포함 | `324ec54a-b10b-45d6-9d34-119ceec5aef0` |
| 키 초기화 coordinator | 11 통과 | 같은 실행 |
| 공용 private SQLite | 9 통과 | `aadb263c-7dc2-4c2f-bf37-d33f8f7730c1` |
| 안정적인 OS 사용자 경로 | 3 통과 | `af1a2b1c-d42f-4166-a2ee-728656e04f8e` |
| 기존 issue automation private store | 3 통과 | 같은 실행 |
| Native restore bridge | 43 통과 | `6b2c0d0d-6039-46a3-aa50-d6e73e81fb98` |
| 아키텍처 검사 | 144 통과 | 같은 실행 |
| Windows Full gate | exit 0, Windows 계약 20그룹 91개 통과, Rust check/clippy·Node·Desktop 검사 통과 | `866dfbda-262a-45ac-acc0-3356713c9256` |

Full gate는 키 생성·경로·공용 SQLite와 기존 private store 그룹을 최신 소스로 다시
검사했습니다. Desktop은 1,059개 통과·기존 7개 건너뜀, typecheck와 build 성공입니다.
패키지 audit에는 차단하는 high/critical advisory가 없었습니다.

두 None 조회 회귀의 수정 후 결과는 저장 요청 1회, 적용 1회, 동일한 키 ID 반환, 합성 암호문
복원 `[true, true]`입니다. 지연 요청을 보유한 서버를 살려 둔 worker 종료와 RPC 직전 종료,
응답 유실의 추가 저장 없는 회복도 통과했습니다.

### 시험 중 확인하고 해결한 경계

- Windows verbatim 경로에서 `PathBuf::join("..")`가 입력을 먼저 정상화했습니다. 잘못된
  경로 시험을 raw `OsString`으로 조립하고 실제 `ParentDir`/NUL 포함을 확인하여 고쳤습니다.
  이때 제품의 경로 검사 코드는 변경하지 않았습니다.
- 환경변수를 바꾼 child에서 null-token `SHGetKnownFolderPath`가 잘못된 profile 경로를
  사용했습니다. 독립 Rust probe에서 명시적인 현재 사용자 token이 기존 LocalAppData를
  유지함을 확인하고, 실제 서비스가 사용하는 단일 helper를 수정했습니다. 기존 flags를
  유지하며 현재 사용자 token과 COM 수명을 관리합니다. 환경변수 격리 및 기존 STA/MTA
  수명 회귀를 포함한 경로 시험 3개가 통과했습니다.
- Native fixture의 암호문 준비가 생성 금지용 key port를 호출하던 문제를 분리했습니다.
  준비 단계만 별도 합성 key store를 사용하며, 실제 native 경로의 생성 금지 panic과
  호출 횟수 검증은 유지했습니다. 수정 후 native 회귀 43개가 통과했습니다.
- 최종 planner는 27개 경로를 `full`로 판정했습니다. 지정 명령은
  `powershell -File scripts/check_native_pr.ps1 -Mode Full`이며 실제 실행은 성공했습니다.
- Full gate가 다시 만든 Admin game asset은 HEAD와 줄바꿈을 정규화한 내용이 같음을
  확인한 뒤 해당 파일만 복구했습니다. 제품 변경에 생성물의 줄바꿈 차이는 포함하지 않습니다.

PR의 최신 head 리뷰와 Linux/macOS/Windows CI 결과는 PR에 별도로 기록합니다.

## 실제 환경 검증의 범위

- credential server와 worker는 모두 합성 시험 프로세스입니다.
- worker 강제 종료와 별개로 서버의 요청 수·적용 수·대기 요청을 확인합니다.
- 실제 사용자 OS 키 생성·교체·조회, 실제 사용자 승인, 실제 모델 turn을 사용하지 않습니다.
- 실제 전원 중단을 시험하지 않습니다. SQLite의 DELETE/EXTRA 설정 확인과 프로세스 종료
  회귀를 전원 중단 시험으로 표현하지 않습니다.
- Windows Vault 파일·native 승인 전체 지원은 이번 범위가 아닙니다.
