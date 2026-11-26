# 고정 커널 시나리오 저장·재생 (A11)

기존 seed 시뮬레이션은 난수가 고르는 경로를 넓게 검사한다. 고정 재생은 명령, 효과 응답,
가짜 시간, 기대 사건을 JSON에 적어 두고 같은 실제 `KernelHostLoop`와 가짜 효과 실행기에
넣는다. 재생 경로는 seed, 난수 호출 순서, 실제 모델이나 네트워크를 사용하지 않는다.

## 실행과 저장

기본 사례는 `src/composition/kernel_host/simulation/replay/fixtures/manual-turn-v1.json`이다.
수동 턴 하나를 시작·완료한 뒤 평가기가 자동 후속을 멈추는 경로를 고정한다.

```bash
cargo test --locked kernel_simulation_replay_fixture_or_file
AKRA_SIM_SCENARIO=/absolute/path/scenario.json cargo test --locked kernel_simulation_replay_fixture_or_file
AKRA_SIM_SCENARIO_SAVE=/absolute/path/new-scenario.json cargo test --locked kernel_simulation_replay_fixture_or_file
```

PowerShell에서는 환경변수를 먼저 설정한다.

```powershell
$env:AKRA_SIM_SCENARIO = 'C:\temp\scenario.json'
cargo test --locked kernel_simulation_replay_fixture_or_file
Remove-Item Env:AKRA_SIM_SCENARIO
```

`AKRA_SIM_SCENARIO_SAVE`는 재생·기대 결과 검증을 통과한 시나리오를 새 파일에 저장한다.
기존 파일은 덮어쓰지 않으며, 읽기 실패·저장 실패·잘못된 형식은 시험 실패로 보고한다.
`AKRA_SIM_SEED`와 `AKRA_SIM_SEEDS`는 기존 무작위 시뮬레이션 설정으로 남는다.

## 버전 1 계약

최상위 필드는 `version`, `name`, `max_auto_turns`, `steps`, `expected`다. 버전은 정수 `1`만
허용하고 알 수 없는 버전·단계·입력 필드는 거절한다. 파일은 1 MiB, 단계는 10,000개,
누적 가짜 시간은 7일 이내로 제한한다.

- `command`: `open_client`, `submit_turn`을 지원한다. 수동 입력의 `prompt`와
  `continue_viewed`를 명시한다. 새 대화를 여는 동작은 기존 시뮬레이션과 같은 경로를 쓴다.
- `effect_response`: session catalog의 `generation`, 턴의 `submission`, 평가의
  `generation`·`thread_id`·`completed_turn_id`로 실제 대기 중인 가짜 효과와 대응한다.
  턴 시작·완료·실패와 평가의 자동 후속 여부를 명시한다. 모르는 효과나 완료 전 시작 누락은
  해당 단계 번호와 함께 거절한다.
- `advance_time`: `milliseconds`만큼 시간을 옮기고 커널 주기를 한 번 실행한다. `0`도
  입력함을 처리하는 명시적인 주기다. 자동 settle이나 숨은 시간 진행은 없다.
- `expected`: 누적 `elapsed_ms`, 최종 `quiet`, 순서가 있는 전체 커널 판단 `events`를
  비교한다. writer의 실제 시각은 제외한다. 기존 커널 불변식 검사도 그대로 수행한다.

응답 뒤에는 필요한 `advance_time`을 직접 둔다. 입력함을 처리하지 않고 너무 많은 응답을
쌓으면 bounded 채널에서 멈추는 대신 시나리오 오류로 거절한다. 효과를 자동 성공시키거나
기대값을 실제 결과로 자동 갱신하는 기능은 없다.

## 첫 범위와 후속 경계

이번 형식은 공통 재생 기반과 고정 수동 턴 사례 하나다. 기존 승인·재시작·응답 유실 시험은
그대로 유지한다. 모든 seed 실행을 자동 녹화하는 기능, 승인·재시작의 파일 단계,
제품 UI와 실모델 재생은 후속 범위다. 시나리오 형식의 의미가 바뀌면 버전을 올리고
이전 파일을 묵시적으로 다른 의미로 실행하지 않는다.

시나리오는 개발용 합성 데이터만 담는다. 실제 대화나 민감 로그를 가져오는 수집 경로는 없다.
