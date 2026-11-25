# ADE 완료 기준 사람 면제 연결 검증 (#2721)

## 변경과 권한 경계

완료 기준 화면에서 사유를 작성하면 기존 private confirmation utility가 Gateway에 helper를 등록하고 정확한 명령을 준비한다. 시스템 확인을 승인한 경우에만 기존 `waive_criterion`의 confirmed 경로로 보낸다.

- 현재 workspace, kernel, task, criterion, scope ID, acceptance revision과 사유를 하나의 요청에 묶는다.
- main/utility 경계에서 허용 필드, 식별자 길이, 64자리 scope, safe revision과 사유의 UTF-8 4,096바이트 한도를 검사한다.
- renderer는 proof, actor, by를 보내지 못하며 직접 waiver 명령을 전송하지 않는다.
- 기존 Rust의 native proof·digest·정확한 대상·revision 검증과 영속 면제 기록을 재사용한다. 새 면제 authority나 우회 경로는 만들지 않았다.
- 접수·처리 응답만으로 화면의 면제 상태를 만들지 않고 현재 kernel snapshot을 다시 읽는다. 면제는 작업자 보고와 호스트 검증을 덮어쓰지 않는다.
- 첫 확인 취소는 변경을 남기지 않고 입력 초안을 보존한다. 이미 결과가 불명인 요청의 재확인 취소는 기존 unknown 상태를 유지한다.
- 응답 유실 뒤에는 명시적인 재확인만 허용하고 동일 request ID, 멱등 키, 원 사유를 유지한다. 재접속 자동 재전송은 없다.
- 확정·거절 뒤에는 재시도용 메모리의 사유를 비운다. 공개 store는 상태 문구만 가지며 proof와 사유 원문을 기록하지 않는다.

## 검증

기준: #2726의 `1169fe2b27d8e21a240f94ace39599b379dbd22e`. 이 PR은 호스트 검증 화면 PR 위의 별도 작업 단위다.

- `npm run check`, Desktop build 통과.
- `npx vitest run test/issueConfirmationService.test.ts test/issueConfirmationProcess.test.ts test/issueConfirmationTransport.test.ts test/kernelConnection.test.ts test/gitStatus.test.ts --maxWorkers=1`: **151 passed, 1 skipped**.
- 새 `task-criterion-waiver.mjs`: 빈 사유 차단·native 취소, 확인 중 scope 변경, 현재 정확한 대상 승인 후 재조회의 3개 시나리오 통과.
- 전체 기본 Electron runner: 31개 spec을 두 shard로 실행하여 exit 0. 기존 Windows terminal 검사는 내부 skip.
- 위 화면 시험은 실제 Electron IPC와 utility process에 dialog/helper/Gateway 대역을 연결했다. 실제 사용자 native 승인, OS 인증 또는 모델 호출의 증거가 아니다.
- planner 지정 Windows Admin gate 실행: **1,227 passed, 7 skipped, 1 failed**. 실패는 기존 `gitStatus.test.ts` stage/unstage/discard fixture의 5초 timeout이다. 같은 파일과 변경 회귀는 위 최종 직렬 실행에서 통과했다. 전체 gate 성공으로 표시하지 않으며 시간 제한·기존 fixture는 변경하지 않았다.
- 독립 코드 리뷰에서 발견한 “결과 불명 요청의 재확인 취소”와 사유 수명 문제를 수정하고 최종 회귀에 포함했다.
- `git diff --check` 통과. PR CI 결과는 별도로 확인한다.


## 작업 작성·호스트 검증 PR과의 통합 검증

#2727 → #2726 → 이 PR 순서로 재배치했다. `App.tsx`와 `KernelPanel.tsx`는 작업 편집, 호스트 검증 요청, 사람 면제의 세 action을 모두 전달한다. `onEditTasks`의 선택성과 기존 attached 조건을 보존했으며 독립 리뷰를 받았다. #2738의 결과 조회 및 #2736의 이슈 완료 기준 채택과의 merge preview도 충돌 없이 통과했다.

검증한 실행 코드 head는 `fa9553f4ac813f1785cbfd0f22c7ecc94dc80be8`이다. 이후 #2726의 검증 기록 commit `cfc00a4643947442fd46d27bf88e41e5ea08be92` 위로 재배치했고, 변경은 검증 문서에 한정됨을 전체 tree diff로 확인했다.

- Desktop typecheck와 production build 통과.
- Windows Admin gate: **1,234 passed / 6 failed / 7 skipped**. 실패는 기존 Git hunk·branch·publish fixture의 5초 timeout과 후속 assertion이다. 직렬 재시도는 같은 환경에서 기존 Git 경합이 반복되어 실행 도중 중단했으며 완료나 통과로 기록하지 않는다. 기존 fixture와 제한 시간은 바꾸지 않았다.
- 같은 실행 코드의 `npm run e2e`: 전체 기본 **32개 spec 모두 runner exit 0**. 작업 작성·호스트 검증·사람 면제 시나리오가 함께 통과했다. 기존 Windows terminal spec 내부 검사는 skip이다.
- E2E 보고서: `desktop/test-results/e2e/restack-20261125/shard-1-of-1-SQ9GZk/report.json`.
- Admin offline asset policy와 `git diff --check` 통과.
- 실제 Electron IPC와 utility process를 사용했고 dialog/helper/Gateway는 대역이다. 실제 사용자 승인·OS 인증·모델 호출의 증거로 계산하지 않는다.
