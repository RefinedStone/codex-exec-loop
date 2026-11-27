# Linux 사용자 검증기 native 증거 (#2722)

## 검증 대상

기준 소스는 `48b6d49a344890fa53fc2c046694e55484b00156`이다. production의 일반 사용자·신뢰된 non-setuid `/usr/bin/bwrap`·강제 namespace·읽기 전용 mount·해시 승인·제한 시간 조건은 그대로 사용한다. WSL과 격리를 만들 수 없는 환경의 직접 실행 fallback은 없다.

## 실제 native 회귀

기존 filesystem 회귀와 새 네 가지 회귀는 `--ignored`를 명시한 지원 Linux에서만 실행한다. 필요한 환경이 없으면 assertion이 실패하며 skip이나 Passed로 바꾸지 않는다.

| 회귀 | 실제 관찰 |
| --- | --- |
| 기존 파일시스템 경계 | 외부 파일·HOME·/run·/proc 미노출, /validator와 /dev·/dev/shm 쓰기 거절, /tmp와 /dev/null 사용 |
| `native_profile_blocks_private_files_writes_and_host_network` | 합성 Vault 파일 읽기·외부 파일 쓰기 거절, 호스트 TCP와 abstract Unix socket의 외부 positive control 뒤 sandbox 접근 거절, 정확한 stdin 공유본 |
| `native_profile_bounds_output_and_rejects_changed_hash` | 실제 2 MiB 출력의 보관 4 KiB 제한, 승인 후 파일 내용 변경 거절 |
| `native_profile_timeout_removes_observed_detached_descendants` | 실제 fork·setsid 자식이 관측된 뒤 timeout 발생, 같은 PID+birth의 생존 프로세스가 남지 않음 |
| `native_profile_real_service_blocks_required_failure_and_allows_only_pass` | 실제 application 게시 관문과 sandbox adapter로 통과만 허용하고 거절·비정상 종료·timeout·해시 불일치를 차단 |

프로세스 관측은 시험을 호출한 thread의 자손만 읽는다. 한 번 확인한 validator·분리 자식의 증거는 zombie 전이 뒤에도 보존한다. 종료 판정은 /proc stat의 birth/state를 사용하며, 읽기 오류를 종료 성공으로 해석하거나 추측한 PID에 signal을 보내지 않는다.

## 실행

```sh
cargo test --locked --lib adapter::outbound::issue_code_validator::linux::
cargo test --locked --lib adapter::outbound::issue_code_validator::linux:: -- --ignored --test-threads=1 --nocapture
```

`.github/workflows/linux-validator-native.yml`은 Ubuntu 22.04 disposable CI에 upstream bubblewrap 0.13.1을 고정 설치하고 위 실제 native suite를 실행한다. 공식 release archive의 SHA-256 `0e85c8702248b8dd64144f594147553650cdd0e4c951690345faace79c217c4d`를 확인한 뒤 빌드하며, `/usr/bin/bwrap`는 root 소유 0755 non-setuid다. Python과 빌드 도구는 배포판 패키지를 사용한다. 제품은 설치·sudo를 호출하지 않는다. CI도 sysctl, AppArmor profile, namespace 옵션을 완화하지 않는다. 환경이 제공하지 못하는 격리는 실패로 남긴다.

## 이번 확인 상태

- 제한된 로컬 Linux: UID 1000, kernel 6.18.57, bubblewrap 0.12.0. namespace 사전 확인이 exit 1로 실패했다: `Failed to create NETLINK_ROUTE socket: Operation not permitted`. 이 환경을 지원 프로필의 통과 증거로 사용하지 않는다.
- WSL은 production에서 의도적으로 미지원이다. WSL의 컴파일·일반 회귀와 native 격리 통과를 구분한다.
- 최초 WSL lib test 컴파일은 guest 디스크 공간 부족으로 중단됐다. 이번 실행이 생성한 실패 incremental만 정리하고 C:의 별도 출력 경로에서 재실행했다. Rust lib test 컴파일과 일반 회귀 **4 passed**가 통과했고, native capability 시험 **5개는 ignored**로 남았다. 이것은 native 격리 성공이 아니다.
- 독립 읽기 리뷰에서 관측 증거 보존과 종료 판정을 보완했다. `cargo fmt --all`, `git diff --check` 통과.
- 검증 코드 `86831e4235459b106a804aa4599b515f11947934`에서 planner 지정 Windows Full gate가 **exit 0**으로 완료됐다. Windows 계약 회귀·build/check/clippy와 Desktop check/build가 통과했고, Desktop 시험은 **1205 passed / 7 existing skipped**였다.
- 같은 코드의 Linux cfg를 WSL에서 `cargo clippy --locked --all-targets --all-features -- -D warnings`로 확인해 **exit 0**을 얻었다. 이 컴파일/정적 검사도 native 격리 성공 증거와 구분한다.
- 아래 Ubuntu 22.04 / bubblewrap 0.13.1 프로필의 실제 native CI가 성공했다. 지원 범위는 이 정확한 환경으로 한정한다.

이 변경만으로 모든 Linux 배포판이나 cgroup 전체 자원 quota를 지원한다고 선언하지 않는다. 실제 native suite가 성공한 정확한 환경만 검증된 프로필로 기록한다.

### Ubuntu 24.04 기본 호스트의 실제 차단

첫 [native 실행](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45656033639)은 kernel `7.0.0-1017-azure`, 배포판 bubblewrap `0.9.0-1ubuntu0.3`, 일반 사용자에서 **0 passed / 5 failed / 0 ignored**였다. 모두 `NotRun(IsolationUnavailable)`였으며 필수 검증기의 service 게시 관문도 차단됐다.

후속 [고정 no-op 진단](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45658718652)은 kernel `7.0.0-1018-aws`에서 같은 강제 namespace와 no-new-privileges 조건을 사용했고 `bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted`로 종료됐다. 읽기 전용 확인에서 `apparmor_restrict_unprivileged_userns=1`, `unprivileged_userns_clone=1`이 관찰됐다. 이 값만으로 특정 정책이 원인이라고 단정하지 않고, 해당 기본 호스트에서 필수 네트워크 격리를 구성하지 못한 결과로 기록한다.

호스트 보안 정책을 바꾸지 않고 지원 후보를 Ubuntu 22.04와 고정 upstream bubblewrap 0.13.1 조합으로 분리해 확인한다. Ubuntu 24.04 결과를 통과로 바꾸거나 미지원 실행을 우회하지 않는다.

### 검증된 native 프로필 — Ubuntu 22.04 / bubblewrap 0.13.1

[실제 native CI 45659579634](https://github.com/RefinedStone/codex-exec-loop/actions/runs/45659579634/job/139365464089)가 **5 passed / 0 failed / 0 ignored**로 성공했다. 실행 시간은 3.86초이며 skip이나 모의 runner를 성공 증거로 사용하지 않았다.

| 항목 | 실제 기록 |
| --- | --- |
| 제안 branch head | `454d89da024d2db7774da15eb9e0b42ceeee0324` |
| CI가 체크아웃한 PR merge commit | `4b33614a12c3e27f4932999411d66218eab41c8a` — 위 head와 prerelease `f4aef3c05c2942f0789b88c2c78a9748a80e8a5e`의 조합 |
| 호스트 | GitHub hosted Ubuntu 22.04, UID 1001, kernel `6.8.0-1070-azure` |
| 실행기 | 공식 archive 해시를 확인해 빌드한 bubblewrap 0.13.1, `/usr/bin/bwrap`, root:root 0755, non-setuid |
| 호스트 정책 | 기본 상태를 읽기만 함. `apparmor_restrict_unprivileged_userns=0`, `unprivileged_userns_clone=1`을 관찰했고 변경 명령은 실행하지 않음 |
| 경계 시험 | 기존 filesystem, 외부/private 파일·쓰기·TCP·abstract Unix socket, stdin 공유본, 출력 보관·해시 변경, timeout·관측된 detach 자식 종료, 실제 service 게시 관문 모두 통과 |

후속 commit은 이 결과와 지원 범위를 문서화하며, 검증한 Rust 코드와 workflow는 변경하지 않는다. 검증된 범위는 위 정확한 프로필이다. Ubuntu 24.04 기본 호스트의 앞선 차단 결과와 WSL 미지원은 유지하며, 다른 배포판·cgroup 집계 quota·실제 사용자 Vault 원문 접근 시험을 수행했다고 주장하지 않는다.

### 실행 중 사용자 취소 (#2703)

지원 프로필에서 아래 시험은 실제 runner에 무해한 셸 스크립트를 전달한다. 호스트의
읽기 전용 프로세스 관측으로 셸과 대기 중인 자식을 확인한 뒤 취소한다. `Cancelled`
결과, 관측한 프로세스의 종료, 실행 중 실제로 존재했던 staging 디렉터리의 삭제를 확인한다.
Ubuntu 24.04 기본 호스트나 WSL은 이 시험의 지원 프로필이 아니다.

```sh
cargo test --locked --lib adapter::outbound::issue_code_validator::linux::native_tests::native_profile_cancel_stops_live_fixture_and_removes_staging -- --ignored --exact --nocapture
```
