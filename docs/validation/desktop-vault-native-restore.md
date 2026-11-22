# Desktop 원문 보기의 native 연결 (#2414)

## 목적과 범위

기존 Desktop의 전용 복원 창은 복원 서비스가 연결되지 않았고 이슈 상세에서 여는 진입점도 없었습니다. 이번 변경은 기존에 암호화해 보관한 GitHub 이슈의 원문을 사용자가 확인한 뒤 전용 읽기 화면에서 한 번 여는 경로를 연결합니다.

이 문서는 이 후속 구현의 검증 범위를 기록합니다. [기존 게시·복원 검증](issue-publish-restore-flow.md)의 미완료 항목 전체를 완료로 바꾸지 않습니다.

| 환경·기능 | 이번 범위 |
| --- | --- |
| macOS | 시스템 native 확인과 OS 키 저장소를 사용하는 기존 레코드의 읽기 |
| Linux | 신뢰할 수 있는 시스템 Zenity 및 Secret Service가 있는 로컬 환경의 기존 레코드 읽기 |
| Windows | 키·레코드를 읽기 전에 지원하지 않는 환경으로 보류 |
| 원격 Gateway의 Vault | 지원하지 않음. Desktop과 같은 호스트의 저장소만 대상 |
| 재연결·게시·새 키 만들기 | 제외. 이번 채널은 읽기만 하며 재연결 버튼을 제공하지 않음 |

## 사용자 흐름

1. GitHub 이슈 상세의 **원문 보기**를 누르면 저장소와 이슈 전체 좌표에 묶인 전용 창이 열립니다.
2. main은 자동화·개발 renderer·에이전트 작업 상태 등 기존 열람 제한을 확인합니다.
3. 전용 helper가 대상 URL과 안정 ID, 대표 checkout을 대조한 뒤 직접 OS 확인 창을 띄웁니다. 기본 선택은 취소입니다.
4. 확인 성공 후 main은 창 세대와 실행 프로필을 다시 확인하고 같은 helper에 읽기를 요청합니다.
5. helper가 최신 공유본과 기존 암호화 레코드를 대조해 복원한 경우에만 전용 창에 제목과 본문을 표시합니다.
6. 프로젝트 전환, 창 닫힘, OS 잠금, 에이전트 작업 시작, 열람 시간 만료는 열람과 진행 중인 helper를 종료합니다. 늦게 도착한 결과는 표시하지 않습니다.

## 확인과 데이터 경계

일반 Gateway 명령이나 kernel 이벤트에 복원 평문을 추가하지 않습니다. 사용자 확인은 helper 자신이 수행하며 JSON 요청에는 승인 여부를 대신하는 값이 없습니다. CLI에서 같은 helper를 직접 실행해도 확인 창을 건너뛰는 제품 경로가 없습니다. 시험용 native 확인은 테스트 의존성으로만 주입합니다.

전용 프로세스의 상속된 stdin/stdout으로만 `prepare → prepared → restore`를 주고받습니다. 준비 단계의 대상과 읽기 단계의 대상은 동일하며 두 번째 요청으로 대상을 바꿀 수 없습니다. 요청은 16 KiB, 출력은 8 MiB로 제한하고 요청 ID·버전·필드 목록·UTF-8·정상 종료를 확인합니다. 원시 오류와 stderr는 화면이나 일반 로그로 보내지 않습니다.

최초 키 생성·저장과 디렉터리 생성이 없는 읽기 조립을 사용합니다. 취소·EOF·만료·잘못된 좌표에서는 원문을 반환하지 않습니다. 일반 domain의 평문과 키 타입에는 직렬화 구현을 추가하지 않습니다.

## 검증 결과

Windows 작업 호스트에서 실행한 결과와 CI에서 확인할 범위를 구분합니다. 모든 fixture는 합성 데이터이며 실제 모델 턴을 시작하지 않습니다.

| 검증 | 결과 |
| --- | --- |
| Rust helper·읽기 조립·직접 port 상태 전이 회귀 | Windows 21개 통과 |
| Native prompt 인자·환경 회귀 | 2개 통과 |
| 전체 architecture 경계 회귀 | 144개 통과 |
| Windows all-features/all-targets clippy (`-D warnings`) | 구조 수정 후 통과 |
| Unix 실제 암호화 파일 → 전용 pipe 복원 및 파일 불변 회귀 | Windows에서 `cfg(unix)` 제외; Linux CI 확인 대상 |
| Desktop 타입 검사 및 전용 source/session 회귀 | 타입 검사 통과, 54개 통과 |
| Desktop 전체 Vitest | 통합 기준 1,000개 통과, 기존 7개 제외 |
| 통합 후 가짜 Gateway/합성 데이터 기반 원문 화면·이슈 진입점 | 전용 화면 20개 및 issue-interventions 10개 통과 |
| 전체 Electron E2E | 기본 25개 spec 중 24개 통과, Windows terminal 1개 제외 |
| 실제 Windows helper 바이너리 smoke | prepare → platform_unsupported, 종료 코드 0, stderr 0바이트 |
| planner 지정 Windows Full gate | 구조 수정 후 실제 prerelease 기준 통과 (종료 코드 0) |
| CI Gate | 최종 PR head의 Checks에 결과 기록 |

전체 Electron E2E는 기능 구현 기준에서 수행했고, 이후 #2643·#2646의 이슈 상세/실행 제어와 #2649의 알림 회귀가 포함된 통합 후보에 반영한 뒤 Windows Full과 관련 두 화면 spec을 다시 통과했습니다. 최종 PR의 전체 CI는 Checks에 남습니다.

Windows Vitest 기본 제외는 실행 권한·LFS 필터·FIFO·symlink 관련 3개·실제 PTY의 7개입니다. 별도 수동 화면 시험인 `browserAutomation.mjs`와 `browserBridge.mjs`는 기본 E2E 실행 대상이 아닙니다.

실제 사용자 키 저장소의 비밀, 실제 GitHub 이슈 원문, 실제 모델 턴을 검증 데이터로 사용하지 않습니다. 합성 화면 검증과 서비스/프로세스 시험은 실제 OS 확인 창·실제 키 저장소를 함께 사용한 수동 성공 검증과 구분합니다.

## 검증 중 발견한 구조 문제와 수정

초기 사전 CI에서는 Linux lib 5,036개와 실제 암호화 파일의 pipe 복원이 통과했지만, architecture 144개 중 `production_driving_adapters_depend_on_inbound_ports_not_application_services` 한 개가 실패했습니다. helper가 application service를 직접 참조한 것이 원인이었습니다.

전용 `VaultRestoreSessionPort`와 `DesktopVaultRestoreService`를 두어 대상 검증·native 확인·권한 수명을 application이 소유하도록 고쳤습니다. 파일시스템의 대표 checkout 조회는 별도 읽기 port로 분리했습니다. architecture 예외를 추가하거나 검사를 완화하지 않았습니다. 직접 port 호출에서도 prepare 생략·다른 대상으로 재준비·중복 복원·취소 후 재진입·만료·늦은 확인 응답을 거부하는 회귀를 추가했고, 로컬 전체 architecture 144개가 통과했습니다. 최종 CI 결과는 PR의 최종 head에 기록합니다.

## 남은 범위

- Windows에서 안전하게 읽고 게시하는 Vault·게시 일지·생성 일지 저장소 지원.
- 프로세스 사이에서도 원자적으로 동작하는 재연결 저장과 그 native 승인.
- 실제 OS 확인 창과 실제 키 저장소를 결합한 수동 검증.
- 기존 문서에 남은 게시 중 프로세스 종료·디스크 부족·최초 키 생성 경합 등은 이 변경의 완료 주장에 포함하지 않습니다.
