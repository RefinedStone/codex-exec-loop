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
