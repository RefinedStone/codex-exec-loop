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
- `git diff --check`: 통과.
- planner가 지정한 `powershell -File scripts/check_native_pr.ps1 -Mode Admin`을 실행했다. 전체 Desktop 테스트 중 기존 실제 Git fixture가 병렬 실행에서 5초 제한을 간헐적으로 초과했다. 같은 실패 파일은 위 직렬 실행에서 통과했다. 이 결과를 전체 gate 성공으로 표시하지 않는다.
- 초기 환경의 `rg` PATH 누락은 전용 도구 경로로 해결했다. 저장소 검사 스크립트와 테스트 제한 시간은 변경하지 않았다.

## 후속 확인

PR CI의 전체 검증 결과를 별도로 확인한다. 실제 모델 실행, 실제 OS 복원 인증, task의 delivery/merge 판정은 이 화면 회귀의 검증 범위가 아니다.
