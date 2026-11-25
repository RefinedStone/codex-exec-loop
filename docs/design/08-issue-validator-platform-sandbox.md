# 사용자 이슈 검증기의 플랫폼 격리 결정안 (#2703)

상위 #2414. 플랫폼별 지원 범위와 실제 검증 근거는 아래 표 및 검증 기록에 구분한다.

기존 macOS sandbox-exec 계약을 변경하지 않는다. 모든 필수 검증기는 Passed 외의 결과를 게시 차단으로 해석한다. 검증기 입력은 승인된 공유본이고 Vault 복원본이나 사용자 토큰은 주입하지 않는다.

## 선택지와 경계

| 환경 | 방안 | 결정 |
| --- | --- | --- |
| macOS | 기존 sandbox-exec | 기존 계약 유지 |
| Native Linux, 일반 사용자 | root-owned non-setuid /usr/bin/bwrap | Ubuntu 22.04 + upstream 0.13.1의 제한된 시스템 런타임을 실제 검증 |
| Linux에서 user/PID/network namespace를 제공하지 못함 | 직접 실행 또는 --unshare-*-try | 금지 |
| Windows | AppContainer + 최소 ACL + Job + 네트워크 capability 거부 | headless AMD64 PE32+ Windows-subsystem 프로필. 실제 LPAC·Job·접근 의미 확인 뒤 실행 |
| Windows | WSL, Windows Sandbox, Hyper-V | 일반 앱의 권한 격리와 동일시하지 않음. 설치, VM 및 host 공유 정책이 추가된다 |

Linux 구현은 절대 경로의 신뢰된 바이너리와 일반 사용자만 허용한다. WSL은 Windows interop과 호스트 공유 경계가 따로 검증되어야 하므로 차단한다. bwrap 설치, sudo, host 정책 변경은 자동 수행하지 않는다.

- 사용자·PID·network·IPC·UTS·cgroup namespace는 강제 생성하고 추가 user namespace와 capabilities를 막는다. 실패 시 직접 실행하지 않는다.
- 새 mount root에 관리자가 제공하는 제한된 /usr/bin, /usr/lib 및 동등 경로만 읽기 전용으로 제공한다. HOME, 작업 저장소, /etc, /usr/local, /run, /sys, /proc, Windows 공유 드라이브를 연결하지 않는다. 이 시스템 경로와 배포판 패키지는 신뢰 기반이며 관리자 설치 파일 자체의 악성을 검출하는 기능이 아니다.
- 실행 파일을 일반 파일로 제한하고 승인 SHA-256 및 최대 8 MiB를 확인한 후 새 private 임시 위치에 복사한다. 검증기는 읽기 전용 파일과 stdin JSON만 받는다. `/dev` tmpfs와 그 안의 `/dev/shm`은 읽기 전용으로 두고 장치 bind mount만 유지한다. 쓰기 가능한 작업용 tmpfs는 `/tmp` 16 MiB이며, per-process resource limit, 상속 환경 제거, 출력 보관 4 KiB, 제한 시간과 contained-child 정리를 적용한다.
- sandbox 초기화 성공 이후의 고정 시작 프레임이 없으면 정상/거부 종료 코드라도 승인하지 않는다. 오류 출력은 전달하지 않는다.
- per-process limit은 cgroup 집계 CPU/RAM quota가 아니다. 검증된 native 프로필에서 파일·network·tmpfs·출력·timeout·detach 자식 종료를 확인했으며, 다른 배포판과 집계 자원 quota 검증으로 확대 해석하지 않는다.

## 검증과 보류

자동 테스트: 해시/스냅샷 경계, 실행 실패와 검증기 거부 분류, 사용 불가 환경에서 direct fallback 금지. 인자 순서 회귀로 `/dev` 생성 후 해당 mount를 읽기 전용으로 바꾸는지 확인한다. 별도 ignored native test는 실제 bwrap와 namespaces가 있어야 실행하며, `/dev` 일반 파일 생성 및 `/dev/shm`의 12 MiB 파일 두 개 쓰기 거절과 `/tmp`·`/dev/null`의 정상 사용을 함께 확인한다. Windows는 아래의 실제 검증된 제한 프로필만 실행하며 그 외에는 미지원으로 끝낸다.

Native Linux disposable 환경에서 다음을 실행한다.

```sh
cargo test --locked --lib adapter::outbound::issue_code_validator::linux::tests::
cargo test --locked --lib adapter::outbound::issue_code_validator::linux::tests::native_profile_executes_only_inside_the_declared_boundaries -- --ignored --exact
```

Ubuntu 22.04 + 고정 upstream bubblewrap 0.13.1에서 network namespace, root 밖 파일/쓰기 차단, timeout 및 detach 자식 제거와 실제 필수 검증기 service 게시 차단까지 통과했다. 다른 배포판별 경로/호스트 정책과 cgroup 집계 quota는 별도 검증 대상이다. Windows의 실제 AppContainer 실행/격리·네트워크·Job·해시 검증과 게시 관문 연결은 아래 구현을 따른다. 실행 중 사용자 취소 전파는 기존 경로에 없어 남은 작업이다.

참고: [bubblewrap upstream man page](https://github.com/containers/bubblewrap/blob/main/bwrap.xml), [Debian bwrap(1)](https://manpages.debian.org/trixie/bubblewrap/bwrap.1.en.html).

## Native 증거의 후속 검증

#2722의 추가 native canary, 실제 게시 관문 회귀와 환경별 실행 상태는 [Linux native 검증 기록](../validation/issue-validator-linux-native.md)에 구분한다. 실제 native 5개를 통과한 정확한 source/환경과 Ubuntu 24.04의 차단 결과를 함께 기록한다. WSL 및 필요한 namespace를 제공하지 못하는 환경의 미지원 판정은 유지한다.

## Windows 구현 (#2723)

고유 disposable profile의 LPAC, capability 0, creation-time Job, stdio handle allowlist, child/Win32k mitigation을 검증한 뒤에만 headless AMD64 PE32+ Windows-subsystem 이미지를 Resume한다. 승인 바이트 복사와 드라이브 루트·최종 파일 검사 및 형식 제한, 제한 환경, 시간/출력/메모리 제한, Job-empty 및 profile/stage 정리 후 결과 반환을 적용한다. 실패 시 직접 실행하지 않는다.

실제 Windows 10.0.26200.0 x64에서 정상 Passed, 외부 synthetic 파일 차단, loopback host 제어군과 격리 WSAStartup=10107·연결 0, child 생성 거절, timeout/output flood, 사용자 payload 정리와 외부 canary 보존을 확인했다. console subsystem이나 일반 interpreter 지원으로 확대하지 않는다. 동기 게시 경로의 실행 중 사용자 취소 API는 추가하지 않았으므로 #2723 전체 완료 표시는 보류한다. 세부 계약·근거·제한은 [Windows 검증기 실행 계약](../reference/issue-validator-windows.md)에 기록한다.
