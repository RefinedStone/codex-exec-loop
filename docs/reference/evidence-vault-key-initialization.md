# Vault 최초 키 생성의 프로세스 간 조정

#2658은 같은 OS 사용자의 여러 Akra 프로세스가 처음 Vault 키를 만들 때 서로 다른 키를
저장하지 않도록 생성 진입을 조정합니다. 키는 기존과 같이 OS 저장소의
`akra-evidence-vault / device-key-v1` 항목에 있으며, 암호문과 레코드의 키 식별자 형식은
바뀌지 않습니다.

## 읽기와 생성 요청

`VaultKeyStorePort::load_key()`는 기존 키를 읽는 계약입니다. Native 열람의
`unlock/access`는 이 경로만 사용합니다. 키가 없으면 새 키나 초기화 기록을 만들지 않습니다.
어댑터 생성자도 디렉터리, DB, 키를 생성하지 않습니다.

`get_or_create_key()`는 새 암호문을 만들기 위한 계약입니다.
`EvidenceVaultUnlockService::sealing_key()`는 이 계약을 호출하고 기존 레코드 키 형식으로
변환합니다. 공개 교체용 `store_key()`는 없습니다. 실제 OS 저장 호출은 어댑터 안에서
초기화 시도 한 번에 최대 한 번만 수행합니다.

정상 키는 OS 난수로 만든 32바이트입니다. OS 조회에서 길이가 다른 값이나 손상된 값을
받으면 새 키로 덮지 않고 실패합니다. 키 바이트는 초기화 DB, 이벤트, 로그, Gateway 응답에
기록하지 않습니다.

## 범위와 영속 기록

초기화 기록은 저장소별 `AKRA_HOME`과 분리된 OS 사용자 데이터 경로를 사용합니다.

| 플랫폼 | 데이터 root의 출처 |
| --- | --- |
| Unix | 현재 effective UID의 OS 사용자 정보에 있는 home 아래 `.akra` |
| Windows | 현재 사용자 token으로 조회한 `FOLDERID_LocalAppData` 아래 `Akra` |

그 아래 `global/vault-key/runtime/key-initialization.db`를 사용합니다. `HOME`,
`AKRA_HOME`, XDG 환경변수, 현재 작업 디렉터리, 프로젝트 경로는 이 범위를 바꾸지 않습니다.
시험은 별도 임시 root와 합성 credential backend를 주입하며 실제 사용자 root에 기록하지
않습니다.

공용 private SQLite 경계는 디렉터리와 DB handle을 트랜잭션이 끝날 때까지 유지하고,
소유권·권한·링크·파일 identity·rollback journal을 확인합니다. 최초 생성도 검사한 부모
아래에서 한 디렉터리씩 진행합니다. 기존 issue automation DB는 기존 파일명과
`synchronous=FULL` 설정을 유지합니다.

새 초기화 DB는 `DELETE` journal과 `synchronous=EXTRA`를 요청하고 실제 설정값을
확인합니다. SQLite는 EXTRA에서 rollback journal 삭제 뒤 그 디렉터리의 동기화를
추가합니다. 이 설정은 [SQLite 공식 계약](https://www.sqlite.org/pragma.html#pragma_synchronous)을
따르며, 실제 디스크·운영체제의 전원 중단 내구성을 이번 시험에서 측정한 것은 아닙니다.

기록에는 schema, service/account 범위, 단조 증가 generation, `Pending/Settled`와 후보의
전체 SHA-256 fingerprint만 둡니다. fingerprint에는 별도 domain을 사용합니다.
기존 레코드의 64비트 키 식별자를 초기화 일치 판정에 재사용하지 않습니다. 키 원문이나
다시 전송할 후보는 저장하지 않습니다.

## 상태별 결과

| 기록 | OS 조회 | 생성 요청의 결과 |
| --- | --- | --- |
| 없음 또는 Settled | 정상 키 | 기존 키를 반환하고 저장하지 않음 |
| 없음 또는 Settled | 키 없음 | 새 Pending을 commit한 뒤 최초 저장 한 번 |
| Pending | fingerprint가 같은 정상 키 | 재저장 없이 같은 generation을 Settled로 회복 |
| Pending | 키 없음 | 보류, 새 후보나 저장 요청 없음 |
| Pending | 다른 fingerprint | 보류, 그 키로 새 암호문을 만들지 않음 |
| Pending | 실패·손상 | 보류, Pending 유지 |

정상 키가 조회되더라도 생성 경로는 먼저 Pending과 대조합니다. 아직 처리 중인 이전 저장이
늦게 다른 키를 덮어쓸 수 있으므로, 정상 키가 있다는 이유만으로 이 검사를 건너뛰지 않습니다.

외부 키 조회는 SQLite 쓰기 트랜잭션 밖에서 수행합니다. 먼저 기록의 generation/state를 읽고,
키를 조회한 다음 `BEGIN IMMEDIATE` 안에서 같은 기록인지 다시 확인합니다. 다른 프로세스가
그사이에 기록을 바꾸었으면 저장하지 않고 새 상태를 조회합니다. 이 반복은 제한되어 있으며,
계속 바뀌거나 DB가 잠겨 있으면 실패로 반환합니다. 오래된 “키 없음” 응답이 다음 생성을
열지 못합니다.

새 Pending의 commit이 성공하기 전에는 OS 저장을 호출하지 않습니다. 저장 뒤에는 정상 키를
다시 읽고 같은 fingerprint인지 확인합니다. 확인하지 못하면 후보를 반환하지 않고 Pending을
보존합니다. 완료 기록도 generation을 확인하여 늦게 돌아온 호출이 새 시도를 바꾸지 못하게 합니다.

## 중단과 재조회

프로세스 잠금은 호출 프로세스가 종료되면 해제될 수 있습니다. 그러나 이미 서버가 받은
저장 요청까지 취소됐음을 의미하지는 않습니다. 따라서 PID 종료, 시간 경과, 잠금 회수는
Pending을 지우거나 다른 후보를 만드는 근거가 아닙니다.

- 저장은 적용됐지만 응답을 잃었다면, 같은 fingerprint를 읽는 다음 요청이 추가 저장 없이
  회복합니다.
- 저장 요청이 아직 적용되지 않아 키가 없다면 후속 요청은 보류합니다. 지연 요청이 나중에
  적용되면 같은 방식으로 회복할 수 있습니다.
- Pending commit 직후 실제 저장 요청 전에 종료되면 요청이 전혀 없어도 보류가 남을 수
  있습니다. SQLite와 OS credential API를 하나의 트랜잭션으로 묶을 수 없어 생기는 가용성
  제한입니다. 자동 timeout 재생성은 제공하지 않습니다.
- Settled 뒤 정상적으로 사용하던 키가 상실된 경우에는 새 generation에서 키를 만들 수
  있습니다. 새 키로 이전 암호문을 복원할 수 있다는 뜻은 아닙니다.
- 초기화 DB나 Pending을 수동 삭제하는 것은 안전한 복구 절차가 아닙니다.

동시 호출이 모두 성공한다고 보장하지 않습니다. 성공한 호출은 같은 키를 사용하고,
미확정 호출은 명시적으로 보류됩니다.

## 제품에서 보이는 보류와 복구

미확정 초기화는 기존 `KeyStoreUnavailable` 사유를 사용합니다.
공유본 게시로 자동 전환하지 않습니다.

| 경로 | 보류 뒤 가능한 동작 |
| --- | --- |
| 기존 이슈 게시 | 명시적으로 공유본만 게시하거나, 보류 상태를 닫고 다시 게시하여 키 재조회 |
| 새 이슈 최초 생성 | 기록된 공유본만 명시적으로 게시하거나 새 초안에서 원문을 다시 입력·승인 |
| 이미 원격 게시·생성이 확정된 복원 연결 | 연결 재확인에서 키를 다시 조회하며 원격 쓰기는 재전송하지 않음 |
| 백업 export/import | 키 준비 단계에서 Locked로 종료, 서비스 재호출로 키 재조회 |
| Native 열람 | 기존 키만 조회하고, 실패하면 공유본 상태에서 다시 열기 |

새 이슈 생성의 Held 의도에서 이미 폐기한 원문을 자동으로 되살리거나 staging을 재개하지
않습니다. 백업 서비스는 아직 Desktop/Gateway의 사용자 화면에 연결되지 않았습니다.

## 보장 범위와 검증

이 계약은 같은 OS credential 항목을 사용하는 모든 생성 참여자가 새 초기화 계약을 따를 때
성립합니다. 이전 `load → store` 구현의 바이너리나 외부 프로그램은 이 기록을 따르지
않습니다. 최초 생성 또는 키 상실 뒤 생성 전에 이전 Akra 프로세스도 종료·갱신해야 합니다.
같은 사용자 권한으로 키나 초기화 DB를 변경하는 프로그램은 기존 Vault 보호 범위 밖입니다.

프로세스 간 회귀는 독립된 합성 credential 서버와 여러 worker로 검사합니다. worker가
종료되어도 서버가 받은 요청을 유지한 채, 부모 시험이 적용 시점을 제어합니다.
실제 사용자 keyring, 사용자 OS 승인, 실제 모델 turn을 호출하지 않습니다.
실행 결과와 확인한 한계는 [검증 기록](../validation/vault-key-initialization.md)에 남깁니다.
