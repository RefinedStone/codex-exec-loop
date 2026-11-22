# Desktop 이슈 실행 추적 검증 (#2554)

## 기준과 실행 환경

- 제품 코드 기준: `618aa7421eb7783ad7092d1a043d46c52d1dd364`
- 화면 검증 시 선행 #2553 기준: `67af6ec2257ae4926fad8f42b00481d5ddf8074f`
- 게시 전 선행 PR의 포트 DTO 배치 정리(`4a1824bbd3823e4beb6639a1341cb76854ac1212`) 위로 재배치했다. 실행 추적과 Desktop 코드는 변경하지 않았다.
- 실행일: 2026-11-22 (Asia/Seoul)
- Windows 11 Pro x64, OS 10.0.26200
- Node.js v24.13.0, Rust/Cargo 1.95.0, Git 2.53.0.windows.1

이 문서와 화면 증거는 위 제품 코드에 대한 기록이다. 숨김 Electron과 가짜 Gateway를 사용했으며 실제 모델 턴, GitHub 게시, 이슈 닫기를 실행하지 않았다. 전체 Desktop E2E도 통과했다(25개 spec, Windows terminal 기존 제외 1개). 게시 head의 CI 결과는 이 변경의 PR에서 추적한다.

## 조회 계약과 화면

`issue` 상세는 저장된 실행 상태, 실행·검증·게시 시도 횟수, 현재 정책의 실행 한도, 현재 validation 계보의 실제 PR URL, 종료 원인과 다음 행동을 제공한다. 실행 기록 없음, 읽기 실패, 작업 ID 불일치를 서로 구분한다. 정책 한도를 알 수 없을 때 renderer가 기본값을 추정하지 않는다.

통합 완료, 외부 Open/Closed/Unknown, 계약 복구와 게시 결과 미확정은 독립된 값이다. 통합 뒤 계약 확인이 실패해도 Done과 통합 증거를 지우지 않으며, 시도 한도 도달이 원래의 종료 원인을 가리지 않는다. UI는 서버의 상태·이유·다음 행동을 표시하고 실행 자격이나 재시도 정책을 계산하지 않는다.

전체 이슈 좌표가 일치하는 최근 기록을 sequence 순서로 최대 20개 표시한다. 저장된 기록과 아직 저장되지 않은 최근 runtime 기록을 합치고 중복 sequence를 제거한다. 같은 sequence의 내용이 충돌하거나 일부 조회가 실패하면 완전한 기록처럼 표시하지 않는다. 21번째 이전 기록의 존재만 알리며 이번 범위에는 페이지 이동을 추가하지 않는다.

최근 기록 조회는 기존 DB를 읽기 전용으로 열고 migration, DB 생성, repository marker 생성, 권한 변경을 하지 않는다. 쓰기 없이 안전하게 읽을 수 없는 WAL 상태는 unavailable로 반환한다. 상세 조회도 실행 관찰, provider 호출, 게시, 복구 명령을 시작하지 않는다.

상세 조립 중 catalog와 feedback intent revision이 바뀌면 한 번 다시 읽는다. 계속 바뀌면 불완전한 상세를 ready로 반환하지 않는다. 이것은 실행·피드백 조회의 제한된 일관성 경계이며, 별도 authority를 가진 모든 planning 제어 상태를 하나의 원자적 snapshot으로 보장한다는 뜻은 아니다.

## 재조회와 이동

- “다시 확인”은 조회 요청만 보내며 unavailable, stale 또는 조회 오류에서도 접근할 수 있다.
- 실행·통합·외부 닫힘·게시 이벤트는 진행 중인 이전 상세 응답을 무효화하고 새 조회를 합쳐 실행한다.
- 서로 다른 장부의 feedback revision과 catalog revision을 비교하지 않는다.
- PR과 외부 이슈 열기는 서버 URL을 그대로 전달한다.
- 대화 열기는 새 conversation 조회에서 받은 실제 다른 thread로 이동하며 turn/start, turn/steer, take_over를 보내지 않는다.

## 수행한 검증

| 명령/범위 | 결과 |
| --- | --- |
| `powershell -NoProfile -File scripts/check_native_pr.ps1 -Mode Full` | 통과. Desktop 950개 통과, 기존 skip 7개. 타입·빌드·Windows 계약·all-target/all-feature clippy 포함 |
| Rust 실행 상세·wire·PR 계보·legacy·최근 기록·schema 회귀 | 최종 기준에서 37개 통과 |
| Rust coordinator·collection·triage를 포함한 확장 회귀 | 최종 restack 전 같은 변경에서 154개 통과 |
| Desktop 연결·실행 표시·알림 회귀 | 최종 restack 후 130개 통과 |
| `cd desktop && node --experimental-strip-types --disable-warning=ExperimentalWarning test/e2e/issue-execution.mjs` | 6개 시나리오 통과 |
| `npm --prefix desktop run e2e` | 전체 통과. 25개 spec 실행, Windows terminal 기존 제외 1개 |
| `cargo fmt --all`, `git diff --check` | 제품 코드 commit 전 통과 |

전용 Electron E2E는 다음을 검증했다.

1. 실행 2/3, 검증 4회, 게시 5회의 독립된 값과 최근 기록 20개의 서버 순서를 보존한다.
2. 실행 이벤트 후 새 상세를 읽고 PR/이슈의 정확한 URL을 연다.
3. Done, 외부 Open, contract recovery와 게시 결과 미확정을 함께 표시한다.
4. 다시 확인은 읽기만 수행하고, 이벤트의 서로 다른 revision을 섞어 비교하지 않는다.
5. 기록 읽기 실패와 상세 조회 오류를 명시하며 다시 확인을 유지한다.
6. 별도 실제 fake thread로 이동하되 새 모델 턴이나 직접 맡기 명령을 보내지 않는다.

Gateway와 링크를 여는 외부 경계는 가짜다. SQLite 읽기 전용 동작, 실제 임시 DB/파일 불변성, revision 변화와 schema 검증은 별도 native 회귀로 확인했다. 화면 데이터와 시도 횟수는 검증 fixture이며 실제 사용자 이슈의 실행 결과가 아니다.

## 화면 증거

아래 5개 이미지는 최종 제품 코드의 전용 E2E 실행에서 캡처했다. 일반 폭과 최대화, 통합/외부 상태 동시 표시, 최근 기록의 끝, 조회 오류를 직접 확인했으며 해당 내용의 잘림이나 겹침은 없었다. 긴 상세는 패널 안에서 세로로 스크롤한다.

| 화면 | 증거 |
| --- | --- |
| 일반 폭: 실행 횟수·PR·종료 이유 | [일반 상세](issue-2554-execution-compact.png) |
| 최대화: 실행 추적 상세 | [최대화 상세](issue-2554-execution-maximized.png) |
| 통합 완료·외부 열림·게시 미확정의 동시 표시 | [독립 상태](issue-2554-execution-outcome.png) |
| 최근 20개 제한과 대화 연결 | [최근 기록](issue-2554-execution-timeline.png) |
| 상세 조회 실패 후 다시 확인 | [조회 오류](issue-2554-execution-unavailable.png) |
