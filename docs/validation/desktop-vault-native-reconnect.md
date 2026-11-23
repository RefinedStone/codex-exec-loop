# Desktop native 표식 재연결 검증 (#2656)

> 이 문서의 실행 결과는 당시 구현 기준입니다. #2688의 Windows Vault 지원과 후속 검증은
> [Windows Vault 원문 복원·재연결](windows-vault-restore.md)에 별도로 기록합니다.

상위: #2414. 저장 경계는 #2654 / #2655의 [binding transaction](../reference/evidence-vault-bindings.md)을 사용합니다.
기존 읽기 경로의 검증은 [#2650 기록](desktop-vault-native-restore.md)에 남깁니다.
선행 PR #2655의 실제 merge 기준은 `3239038a04049ec905f22845a8f00377bd171cb1`입니다.

## 사용자 결과와 지원 범위

외부 수정으로 복원이 보류된 GitHub 이슈를 전용 창에서 검토하고, 선택한 표식의 위치를 별도로
확인한 뒤 로컬 연결을 한 번 갱신합니다. 원격 제목·본문을 수정하지 않습니다. 선택하지 않은
표식은 최종 복원본에서도 표식으로 남습니다.

macOS/Linux의 기존 native 확인·OS 키 저장소·private 파일 지원 조건을 유지합니다. Windows는
provider·키·Vault 접근 전 `platform_unsupported`로 보류합니다. 새 키나 Vault root를 만들지 않고,
원격 Gateway 호스트의 Vault를 읽지 않습니다.

## 전용 프로토콜 v2

| 단계 | 요청 | 결과 |
| --- | --- | --- |
| 처음 | `prepare` | 전체 이슈 좌표·URL·대표 checkout 검증과 native 열람 확인 후 `prepared` |
| 준비 | `restore` | 성공이면 최종 복원. 게시본/표식 변경 보류만 `can_reconnect: true` |
| 변경 보류 | `review` | helper에 원래 검토를 보관하고 공개 공유본·표식 후보 projection을 반환 |
| 검토 | `reconnect` | 원 검토의 hash·mapping 검증, 별도 native 쓰기 확인, 최신 연결 재검증·원자 저장 |
| 저장 완료 | `restore` | 최종 복원 또는 `can_reconnect: false` 보류 후 종료 |

준비 응답의 capability와 개별 보류 응답의 `can_reconnect`를 함께 확인합니다. 첫 보류에서만
재연결을 허용하고, 최종 복원이나 검토·저장 오류 뒤에는 새 열람 세션이 필요합니다.

- 요청 ID와 v2를 모든 단계에서 대조합니다. prepare 뒤에는 대상 좌표를 다시 받지 않습니다.
- 요청은 16 KiB, 세션의 출력 합계는 8 MiB로 제한합니다. 추가·알 수 없는 필드, 잘못된 UTF-8,
  잘린 JSON, 미리 보낸 다음 요청을 거절합니다.
- 검토 해시는 `u32 버전:64자리 소문자 hex`이며 버전에 불필요한 앞자리 0을 허용하지 않습니다.
- 위치는 UTF-8 바이트 위치입니다. 이전 위치는 이전 판의 값이므로 현재 게시본 길이로 거절하지 않습니다.
- 표식 문자열·후보·mapping의 중복과 범위·실제 위치를 검증합니다. 빈 mapping은 모든 표식을
  선택하지 않은 기존 계약을 유지합니다.
- 승인 결과, grant, 원래 검토 basis, 로컬 저장 token을 JSON으로 받지 않습니다. 원 검토와
  승인은 application service가 소유하며, application port에는 자체 비직렬화 projection만 둡니다.
- 최종 응답은 helper의 정상 종료 코드 0까지 확인한 뒤 전용 창으로 전달합니다. 원시 오류·stderr,
  검토 내용과 복원 평문을 일반 Gateway나 대화 IPC·일반 로그에 추가하지 않습니다.
- 구형 helper와 버전이 맞지 않으면 보류합니다. 이전 승인 방식으로 자동 전환하지 않습니다.

## 별도 쓰기 확인과 취소

열람 확인은 재연결 쓰기 확인을 대신하지 않습니다. helper는 `RewriteBinding` 목적으로
별도의 실제 native 확인을 수행하며 기본 선택은 취소입니다. 합성 확인은 시험 의존성으로만 주입합니다.

기존 재연결 서비스가 최신 공유본·원래 source bundle·전체 mapping·grant를 검사한 뒤,
최종 grant 검사와 `binding.commit` 사이에 단회 gate를 둡니다. commit 내부에서 기대 binding
token·불변 generation을 검사하고 CAS합니다. gate와 cancel은 같은 짧은 세션 상태 잠금 안에서
기한·취소·단계를 확인합니다. 외부 I/O 동안에는 세션 상태 잠금을 잡지 않습니다.

| 종료 시점 | 저장과 화면 |
| --- | --- |
| 저장 gate 이전 EOF·취소·만료 | 새 binding을 쓰지 않음 |
| 저장 gate 이후 창 종료·취소 | 원자 저장이 끝났을 수 있으며, 늦은 평문과 후속 요청을 차단 |
| pointer 교체 후 결과 불명 | `write_outcome_unknown`으로 종료. 자동 재전송하지 않고 다시 열어 현재 연결 확인 |
| reconnect 요청을 보낸 뒤 응답 유실·잘못된 응답·비정상 종료 | 저장 여부를 추정하지 않고 `write_outcome_unknown`으로 종료 |
| 검토 후 원격 게시본 또는 로컬 연결 변경 | `changed_during_review`로 종료. 같은 승인 재사용 금지 |

창 닫힘·탐색·재로드·renderer 종료는 새 ready를 기다리지 않고 세션을 취소합니다. 원래 창의
프로젝트·수명 변경도 대상을 비활성화합니다. 현재 전체 좌표로 새로 여는 요청만 대상을 다시
활성화할 수 있습니다. 기존 OS 잠금·절전·실행 프로필·에이전트 작업·5분 제한을 유지하고,
검토와 쓰기 확인으로 기한을 연장하지 않습니다.

## 수정 전 실제 실패

기준: `4bd64b600ef7f9dfab6743c55ff0229288c24bec`.
기존 source에 v2의 준비 → 변경 보류 → 검토 → 재연결 → 최종 복원 회귀만 추가해 실행했습니다.

```text
VITEST_MAX_WORKERS=2 npx vitest run test/vaultRestoreSource.test.ts -t 'keeps one native helper through changed copy review'
```

Windows 실행 결과: **exit 1, 1개 실패 / 나머지 25개 선택 제외**.
첫 v2 `prepared`를 기대했지만 기존 source는
`{ kind: "held", reason: "source_unavailable" }`를 반환했습니다.
실행 세션: `88a191a3-ea89-48c0-8c07-5796d1d40f5f`.

## 검증 기록

합성 회귀·실제 파일·플랫폼 실행 결과를 구분합니다. CI와 실제 OS 수동 확인은 별도로 기록합니다.

| 검증 | 현재 기록 |
| --- | --- |
| Rust service·전용 pipe·단회 상태·취소 | `desktop_restore` **44개**, commit gate **3개 통과** |
| 전용 pipe v2·엄격한 입력·출력 한도·최종 종료 | 위 44개에 포함하여 통과 |
| 실제 Unix 파일과 pipe의 재연결·취소·재개방 | Unix 전용 실제 암호화 파일·pipe 회귀 1개를 Linux CI에 포함. 최종 결과는 구현 PR에 기록 |
| Native 확인 purpose·기본 취소·argv/환경 경계 | 합성 회귀 **3개 통과** |
| Desktop source/session/window 회귀와 타입 검사 | 3개 파일 **113개 통과**, TypeScript 검사 통과 |
| 전체 Desktop Vitest 및 build | **64개 파일·1,059개 통과**, 기존 1개 파일·7개 제외, 타입 검사·build 통과 |
| 가짜 Gateway·합성 데이터의 전체 Electron E2E | **25개 시나리오 통과**, Windows 기본 터미널 1개 제외, 전체 실행 exit 0 |
| 실제 Windows helper의 미지원 응답 smoke | v2 `platform_unsupported`, `can_reconnect:false`, exit 0·stderr 0바이트, 격리 홈 생성 없음 |
| architecture·clippy·planner 지정 gate | architecture **144/144**, planner **Full gate exit 0**·Windows 계약 47개·clippy·fmt 통과 |
| 최신 head 리뷰와 CI Gate | 사전 독립 검토 완료. 최신 head Codex 리뷰·CI Gate는 구현 PR에 기록 |

전체 Electron E2E 세션: `f2081028-d56b-4b41-ae70-c9158d03bf9a`, exit 0.
`AKRA_DESKTOP_HIDDEN=1 npm --prefix desktop run e2e`로 기본 목록 26개 중 25개를 실행했습니다.
Windows의 기존 터미널 제외 정책을 유지했습니다. 합성 review의 UTF-8 위치는 46바이트이며,
후보 선택·중복 클릭 차단·쓰기 확인 중 화면·결과 미확정·전용/원래 창 reload의 평문 삭제가 통과했습니다.

Full gate 세션: `c913377c-9e51-4384-a708-089109a60ac2`, exit 0.
실제 Windows helper build 세션: `fc501e4a-7cc7-467f-adf3-573afd650d83`, exit 0.
격리된 합성 prepare smoke 세션: `b4d9f71b-b232-466e-9fe9-c3bd65effd60`, exit 0.

실행한 주요 명령:

```text
cargo test --locked --all-features --target x86_64-pc-windows-msvc --lib desktop_restore -- --nocapture
cargo test --locked --all-features --target x86_64-pc-windows-msvc --lib evidence_vault_restore::tests::native_commit_gate -- --nocapture
cargo test --locked --all-features --target x86_64-pc-windows-msvc --lib native_vault_confirmation -- --nocapture
cargo test --locked --all-features --target x86_64-pc-windows-msvc --test architecture_boundaries -- --nocapture
node scripts/agent-plan.mjs
powershell -File scripts/check_native_pr.ps1 -Mode Full
cargo build --locked --all-features --target x86_64-pc-windows-msvc --bin akra-gateway
```

Windows Rust 집중·구조 검사 세션: `4400ed62-2944-4726-8fc1-027eedb5a9df`, exit 0.

Desktop 집중 검증 세션: `08ff2b90-e026-4d77-a9d9-7b26148c9c46`, exit 0.
독립 리뷰에서 발견한 idle helper 종료 뒤 남는 검토 화면은 `onClosed` 수명 알림과
실제 source+Session 결합 회귀로 수정했으며, 수정 코드의 재검토를 완료했습니다.

Windows의 전체 Full gate는 Desktop 의존성·타입 검사·Vitest·build를 포함합니다. 같은 기준에서
완료한 전체 단위 시험을 중복 실행하지 않고, UI 변경에 필요한 E2E를 추가합니다. 실패를 고치거나
필수 gate가 요구하는 경우에는 해당 검증을 다시 수행합니다.

## 검증 경계와 남은 작업

모든 키·이슈·native 확인 fixture는 합성 데이터입니다. 메모리 stream, 서비스 조립, 실제 private
파일/pipe, hidden Electron 화면 검증을 구분합니다. 화면 fixture는 실제 OS 확인 성공을 대신하지 않습니다.

실제 사용자 키 저장소의 비밀·실제 이슈 원문을 입력으로 사용하거나 실제 모델 turn을 시작하지 않습니다.
실제 OS 확인 창과 사용자 키 저장소를 결합한 수동 성공 검증, Windows Vault·게시/생성 저장소,
최초 OS 키 생성의 프로세스 간 경합은 이번 결과의 완료 주장에 포함하지 않습니다.
