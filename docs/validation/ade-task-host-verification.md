# ADE 호스트 검증 화면 검증 (#2718)

## 동작 경계

작업자 보고와 호스트 실행 사실을 별도 표시한다. 현재 커널 조회의 `host_verification`만 현재 검증 상태의 근거로 사용한다. 검증 요청의 접수 응답은 통과 결과로 바꾸지 않는다.

- 고정 recipe `git_diff_check_v1`만 현재 실행 lease에서 요청한다.
- task ID, criterion ID, scope ID, acceptance revision은 조회한 좌표 그대로 전송한다.
- pending/running/passed/failed/not_run/interrupted/stale/unavailable를 구분한다.
- 실행 시간, 종료 코드, 수집 완전성, hash/크기만 표시한다. stdout/stderr 원문과 artifact 경로는 노출하지 않는다.
- Git 공백 검사가 빌드·기능 검증을 보장하지 않음을 화면에 명시한다.
- 완료 event는 재조회만 유발한다. 늦은 event의 receipt를 현재 상태에 합치지 않는다.
- 응답 유실·재접속은 자동 재전송하지 않는다. 명시적인 재확인은 같은 request ID/idempotency key를 쓴다.
- 원 attempt ID가 빠진 replay는 미확정으로 남긴다. 다른 창의 완료 attempt로 자기 요청의 완료를 추정하지 않는다.

## Windows에서 수행한 검증

기준 commit: `48b6d49a344890fa53fc2c046694e55484b00156`. 실제 모델 요청 없이 fake Gateway를 사용했다.

- `npm run check`: 통과.
- 변경 회귀 및 기존 Git 두 파일: `npx vitest run test/taskVerificationActions.test.ts test/kernelConnection.test.ts test/gitPublish.test.ts test/gitStatus.test.ts --maxWorkers=1` — 95 passed, 1 skipped.
- 빌드한 Electron의 `test/e2e/kernel-panel.mjs`: 40 assertions 통과. 요청 좌표, 접수/실행 결과 분리, 완료 event 재조회, 미실행/중단/오래된 결과/누락 artifact를 확인했다.
- 전체 Electron E2E: 자동 선정 30개 spec을 두 shard로 실행하여 모두 exit 0. Windows terminal 내부 검사는 기존 정책에 따라 skip됐다. 이번 화면 변경의 kernel-panel 40개 단언은 실제 Electron·fake Gateway로 수행했다.
- `git diff --check`: 통과.
- planner가 지정한 `powershell -File scripts/check_native_pr.ps1 -Mode Admin`을 실행했다. 전체 Desktop 테스트 중 기존 실제 Git fixture가 병렬 실행에서 5초 제한을 간헐적으로 초과했다. 같은 실패 파일은 위 직렬 실행에서 통과했다. 이 결과를 전체 gate 성공으로 표시하지 않는다.
- 초기 환경의 `rg` PATH 누락은 전용 도구 경로로 해결했다. 저장소 검사 스크립트와 테스트 제한 시간은 변경하지 않았다.

## 후속 확인

PR CI의 전체 검증 결과를 별도로 확인한다. 실제 모델 실행, 실제 OS 복원 인증, task의 delivery/merge 판정은 이 화면 회귀의 검증 범위가 아니다.

## 작업 작성 PR과의 통합 검증

#2727의 작업 작성 기능 위로 이 PR을 재배치했다. `App.tsx`와 `KernelPanel.tsx`는 작업 편집 진입과 호스트 검증 요청을 모두 전달하며, `onEditTasks`의 연결 및 기존 attached 조건을 보존한다. #2738의 결과 조회와 #2736의 이슈 완료 기준 채택을 함께 넣는 merge preview도 충돌 없이 통과했다.

검증한 코드 head는 `848060e1548db5182a2f18eebc05ff45f911a614`이다. 재배치 과정에서의 코드 차이는 두 화면의 prop 합성에 한정되며 독립 리뷰를 받았다.

- Windows Admin gate의 typecheck와 변경 회귀는 통과했으나 전체 Vitest는 기존 Git fixture의 5초 timeout 및 연쇄 실패로 exit 1이었다. 최초 결과는 1218 passed / 5 failed / 7 skipped, 같은 소스의 직렬 재시도는 1217 passed / 6 failed / 7 skipped였다. 실패는 Git 파일 동작·hunk·branch·publish fixture이며 변경한 화면 모듈의 실패는 없었다. timeout이나 기존 fixture는 바꾸지 않았다.

- 같은 코드의 `npm run e2e`: 전체 기본 31개 spec의 runner exit 0. `kernel-panel.mjs`와 `task-authoring.mjs`도 통과했으며 기존 Windows terminal 내부 검사는 skip됐다. 보고서: `desktop/test-results/e2e/restack-20261125/shard-1-of-1-PQooce/report.json`.
- Desktop production build와 Admin offline asset policy가 통과했다. 실제 모델 호출은 없었다.
