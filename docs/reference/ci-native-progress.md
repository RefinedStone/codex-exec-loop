# Native contract progress records

#2930의 근거는 [Windows 실행 45936932223](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45936932223/job/140302241610)이다.
126필터 중 80개를 시작하고 79개를 성공 완료한 뒤 60분 job 제한에 도달했다.
종전 실행기는 그룹 stdout을 종료 후 출력해 중단된 그룹 내부의 진행을 잃었다.

계약 실행기는 stdout/stderr를 자식 실행 중에 계속 출력한다. 각 그룹 전후에
`AKRA_NATIVE_PROGRESS` JSON 행을 출력하고
`artifacts/ci-performance/native-contracts-<pid>.jsonl`에도 즉시 추가한다.
기존 `always()` performance artifact 업로드에 이 파일이 포함된다.
경로는 `AKRA_NATIVE_PROGRESS_FILE`로 지정할 수 있다.

각 행은 schemaVersion, sourceSha, 시각, filter, 시작/종료 phase를 담는다.
종료 행은 passed/failed/cancelled, 경과 시간과 성공 시험 수를 기록한다.
시작만 있고 종료가 없는 그룹은 **미완료**다. 이미 완료한 그룹만 성공으로 센다.
SIGINT/SIGTERM을 받은 실행기는 자식을 종료하고 취소를 실패로 전달한다.
job 강제 종료나 전원 손실은 종료 행·artifact 업로드 자체를 막을 수 있으므로
실행 로그의 마지막 시작/완료 행을 기본 근거로 사용한다.

시험 선택, 필터별 독립 프로세스, Cargo 옵션, loader 경로와 60분 제한은 그대로다.
필수 필터 0개 일치, 성공 시험 0개, 자식 실패·signal 종료는 계속 실패한다.
진행 계측은 속도 개선 결과가 아니며 실제 시험 단축은 #2864/#2865에서 판단한다.

검증: `node --test scripts/native-rust-contracts.test.mjs scripts/native-contract-progress.test.mjs`.

취소 정리는 직접 자식 하나에만 신호를 보내지 않는다. POSIX에서는 이 실행이 만든 별도 process group을 종료한다.
Windows에서는 직접 소유한 자식이 살아 있는 동안 부모 환경의 `SystemRoot/System32/taskkill.exe`를 절대경로로
실행해 `/T /F`로 트리를 종료하고 그 결과까지 확인한다. 이미 Windows 자식이 끝나 후손의 신원을 증명할 수 없으면
숫자 PID만으로 다른 트리를 겨냥하지 않고 정리 실패를 보고한다.

기본 정리 유예 5초와 출력 배출 1초 이후에도 상속 pipe가 열려 있으면 스트림을 닫고 명시적 실패로 끝낸다.
정리 실패를 성공이나 정상 취소로 승격하지 않는다. 직접 자식·pipe를 상속한 후손의 실제 종료와,
부모 종료 뒤 group을 벗어난 writer의 제한된 실패 처리를 Windows와 Linux의 작은 Node fixture로 검증했다.
