# 실행 뒤 diff 판정과 Delivery 승인

관련: #2558. #2557의 결정 기한·재알림 계약을 사용하는 연계 변경이다.

## 판정 범위

호스트는 현재 work의 task·lease generation·완료 queue가 가리키는 worktree를 확인하고,
고정된 integration base부터 실제 전달할 source commit까지 검사한다. 마지막 커밋만
비교하거나 현재 원격 HEAD를 누락된 base 대신 사용하지 않는다. 검사 전후 queue의
task/session/worktree/base/source가 같아야 하며, 발급 직전에도 같은 범위를 확인한다.

Git은 전체 OID와 조상 관계를 검사한 뒤 `--numstat -z --no-renames --no-ext-diff
--no-textconv`로 변경 경로와 추가·삭제 줄 수를 읽는다. rename은 이전 경로 삭제와 새
경로 추가로 계산하여 두 경로 모두 기존 정책의 보호 경로 검사에 포함한다.

| 조건 | Delivery 처리 |
| --- | --- |
| 보호 경로와 겹침 | `protected_path`, 사람 승인 |
| 변경 파일 50개 초과 또는 추가+삭제 1,000줄 초과 | `diff_too_large`, 사람 승인 |
| 바이너리 포함 | `binary_diff`, 사람 승인; 전체 줄 수는 미확정 |
| 위 조건 없음 | 새 호스트 diff 판정에 근거해 기존 전달 흐름 유지 |
| Git 읽기·파싱·OID·경로·오버플로 검증 실패 | 제출 허가 없음 |

50개/1,000줄은 포함 상한이다. rename은 보수적으로 두 경로와 양쪽 줄 수를 센다.
전체 결과 수집 상한은 4,096개 파일, stdout 1 MiB, 경로당 1,024 bytes다. 상한 초과나
비 UTF-8 경로 등 처리할 수 없는 결과를 잘라서 저위험으로 판정하지 않는다.

보호 경로는 기존 정책의 내장 목록과 사용자·저장소 추가 목록을 합성한 결과를 그대로
사용한다. 저장소 설정으로 내장 보호를 제거할 수 없다.

## 승인과 철회

- Start 승인과 작업 상태는 보존한다. 별도 Delivery assessment와 증가하는 revision에
  범위·정책·이슈 원문 hash·authorization epoch·판정 결과를 고정한다.
- 위험한 결과는 work에 연결된 `RiskException / Delivery` 결정을 열며 항상 Human과
  `on_timeout: wait`를 사용한다. `deliver`에 대한 검증된 사람 확인만 제출을 허용한다.
- native 확인창과 Desktop 카드는 해당 결정에 저장된 전체 base/source SHA, 파일·줄 수,
  검토 사유를 표시한다. 이력은 이후 checkpoint로 덮어쓰지 않는다.
- source나 기준이 바뀌면 허가를 회수한다. A → B → A로 돌아와도 이전 답을 재사용하지
  않도록 assessment를 지운 뒤에도 Delivery revision을 보존한다.
- 수집 Off, 정책 조회 실패, 실행 조회 실패에서도 감권 전용 CAS로 제출 판정을 지운다.
  이미 발급 후 철회된 같은 source를 정책 읽기 회복만으로 새 허가로 승격하지 않는다.
- PR이 아직 없는 Queued 상태의 frozen source도 로컬 추적으로 발견한다. 사람의 답은
  원격 수집 주기와 독립적으로 기존 재확인·제출 경계에 전달한다.
- planning의 Push·PR·Integrate는 정확한 base/source/revision의 호스트 검토 증거를
  요구한다. 기존 payload는 읽을 수 있지만, 업그레이드 전 Start 승인만으로 발급된
  이슈 전달 permit은 coordinator 재평가 전에도 거절한다. 통합의 사람 리뷰는 유지한다.

## 검증 경계

| 계층 | 회귀 |
| --- | --- |
| domain / 실제 Git | 크기 경계, 내장·추가 보호 경로, 이전 커밋의 변경, rename/delete, 바이너리, 잘못된 결과 |
| SQLite / 결정 service | Human 강제, 현재 범위·정책·epoch, stale proof, revision 보존, Off 감권, 이력 불변 |
| coordinator | 승인 대기, 정확한 Human 답 뒤 1회 발급, source 변경·A/B/A·읽기 실패, Start 보존, Queued 추적 |
| planning / 실제 bare 원격 | 구형 permit으로 push·PR 0회 및 ref 불변, 정확한 검토 뒤 전달, 승인 뒤 실제 HEAD 변경 차단 |
| kernel / Desktop | 결정 조회·명령의 근거 보존, 카드 표시, 같은 범위 초안 유지, 범위 변경 시 초안 폐기 |

시험은 격리된 SQLite와 로컬 Git, 가짜 공급자·실행 경계를 사용한다. 실제 모델 턴과
외부 이슈·PR 쓰기는 수행하지 않는다. 전체 완료 기준 증거 하네스(plan 24)는 이 범위에
포함하지 않는다.

## 실행 기록

Rust 및 원격 CI 결과는 PR의 검증 표에 연결한다. 로컬 Desktop 생산 번들은 성공했다.
소켓을 사용하는 기존 단위/E2E 시험은 이 실행 환경의 `listen EPERM`으로 제한되므로,
해당 경계의 최종 결과는 원격 CI에서 확인한다.
