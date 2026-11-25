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

`.github/workflows/linux-validator-native.yml`은 Ubuntu 24.04 disposable CI에 배포판 bubblewrap/Python을 설치하고 위 실제 native suite를 실행한다. 제품은 설치·sudo를 호출하지 않는다. CI도 sysctl, AppArmor profile, namespace 옵션을 완화하지 않는다. 환경이 제공하지 못하는 격리는 실패로 남긴다.

## 이번 확인 상태

- 제한된 로컬 Linux: UID 1000, kernel 6.18.57, bubblewrap 0.12.0. namespace 사전 확인이 exit 1로 실패했다: `Failed to create NETLINK_ROUTE socket: Operation not permitted`. 이 환경을 지원 프로필의 통과 증거로 사용하지 않는다.
- WSL은 production에서 의도적으로 미지원이다. WSL의 컴파일·일반 회귀와 native 격리 통과를 구분한다.
- 최초 WSL lib test 컴파일은 guest 디스크 공간 부족으로 중단됐다. 이번 실행이 생성한 실패 incremental만 정리하고 여유 있는 별도 출력 경로로 다시 검증 중이다.
- 독립 읽기 리뷰에서 관측 증거 보존과 종료 판정을 보완했다. `cargo fmt --all`, `git diff --check` 통과.
- planner 지정 Windows Full gate 및 실제 Ubuntu CI 결과는 진행 중이며, 완료 결과를 이 문서와 PR에 추가한다.

이 변경만으로 모든 Linux 배포판이나 cgroup 전체 자원 quota를 지원한다고 선언하지 않는다. 실제 native suite가 성공한 정확한 환경만 검증된 프로필로 기록한다.
