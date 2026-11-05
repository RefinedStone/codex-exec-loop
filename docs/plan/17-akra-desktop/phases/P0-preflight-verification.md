# P0. 사전 검증

상태: **완료** (2026-11-05) · 이 문서는 검증 기록이다

## 1. 목적

설계를 정하기 전에 다음 세 가지를 확인한다. 모든 검증은 턴을 만들지 않는다 (D-13).

1. Akra 가 현재 환경에서 빌드되고 동작하는가
2. Akra 가 현재 Codex app-server 와 호환되는가
3. app-server 가 여러 접속을 직접 처리하는가

## 2. 결과

| 항목 | 결과 | 비고 |
| --- | --- | --- |
| 저장소 상태 | `prerelease` = `origin/prerelease` (`0732aea8`) | 미추적 파일은 `.DS_Store` 뿐 |
| 툴체인 | Rust 1.95.0 | `rust-toolchain.toml` 과 같다 |
| 릴리스 빌드 | 성공, 약 2분 19초 | `akra`, `akra-admin`, `akra-telegram` 생성 |
| `akra doctor` | 정상 | app-server 를 실행하지 않는 작업공간 검사이다 |
| 메서드 이름 호환성 | 호환 | Akra 가 쓰는 app-server 메서드 이름이 모두 현재 공개 프로토콜에 있다. 차이 나는 이름은 Akra 내부 이벤트 라벨과 테스트용 이름뿐이다 |
| 실제 핸드셰이크 | 성공 | `initialize` → `account/read` → `thread/list` 가 0.149.1 과 0.166.0-alpha.6 모두에서 성공 |
| 새 필드 대응 | 영향 없음 | 0.166 의 추가 필드를 Akra 는 무시한다 (`deny_unknown_fields` 미사용) |
| 다중 접속 | 지원 | unix 소켓 하나에 접속 2개, 접속마다 initialize 성공, 같은 요청 ID 를 각자 사용 |
| 테스트 전체 | 3,099 / 3,126 통과 | 실패 25개는 실행마다 바뀌는 불안정 테스트 ([akra-current.md](../reference/akra-current.md) §7) |
| CLI 로그인 계정 | ChatGPT, free 플랜 | 사용량 한도가 작다 (D-07, D-13) |

## 3. 재현 방법

모든 명령은 Akra 체크아웃의 루트에서 실행한다.

### 3.1 빌드와 진단

```bash
cargo build --release
./target/release/akra doctor
```

### 3.2 메서드 이름 비교

```bash
# Akra 가 쓰는 이름
grep -rhoE '"[a-zA-Z]+(/[a-zA-Z]+)+"' src/adapter/outbound/app_server | tr -d '"' | sort -u > /tmp/akra-methods.txt
# 공개 프로토콜의 이름 (openai/codex 체크아웃 필요)
grep -rhoE '"[a-zA-Z]+(/[a-zA-Z]+)+"' <codex>/codex-rs/app-server-protocol/src | tr -d '"' | sort -u > /tmp/oss-methods.txt
comm -23 /tmp/akra-methods.txt /tmp/oss-methods.txt
```

결과에 실제 app-server 메서드가 나오면 호환성 문제이다. 2026-11-05 결과에는 내부 라벨
(`turn/retrying`, `thread/prepared`, `item/agentMessage/completed` 등)과 테스트용 이름만 나왔다.

### 3.3 핸드셰이크

```bash
python3 -I docs/plan/17-akra-desktop/scripts/appserver_handshake.py "$(command -v codex)"
```

출력은 응답의 키 목록과 계정 종류, 플랜뿐이다. 이메일과 대화 내용은 출력하지 않는다.

### 3.4 다중 접속

```bash
# app-server 를 임시 소켓으로 직접 실행해서 시험한다
python3 -I docs/plan/17-akra-desktop/scripts/multi_client_check.py --codex "$(command -v codex)"
# 실행 중인 Gateway 를 시험한다 (P1 이후)
python3 -I docs/plan/17-akra-desktop/scripts/multi_client_check.py --socket ~/.akra/run/gateway.sock
```

주의: unix 소켓 경로는 104바이트를 넘으면 안 된다. 스크립트는 `/tmp` 아래의 짧은 경로를 쓴다.

## 4. 해석

- Akra 를 고치지 않고도 현재 Codex 와 함께 쓸 수 있다.
- app-server 가 접속마다 세션을 따로 관리한다. 그래서 Gateway 는 접속 1:1 중계로 충분하다 (P-03).
- 실패하는 테스트는 이 계획의 작업 영역과 관계없다. 다만 하네스 연동 전에 정리해야 한다 (K-02).

## 5. 남은 확인

턴이 필요해서 P0 에서 하지 않았다. P1 의 실측 단계에서 사용자 허락을 받고 확인한다.

- Q-03: 두 접속이 같은 승인 요청에 답할 때의 처리
- Q-09: 구독 접속이 모두 끊긴 스레드의 진행 중인 턴이 계속되는지

## 6. Windows 검증 (2026-11-05)

환경: Windows 11, codex-cli 0.159.0 (PATH, npm) 과 0.166.0 (Codex 관리형 데몬 패키지), Node v24.13.0.
턴은 만들지 않았다 (D-13).

| 항목 | 결과 |
| --- | --- |
| `codex app-server --listen unix://<경로>` | 동작한다. 소켓 디렉터리가 현재 사용자 전용 ACL 이 아니면 `Error: socket directory is not private to the current user` 로 시작하지 않는다 |
| Node `net.connect(<unix 소켓 경로>)` | 실패 (`EACCES`). Node 는 Windows 에서 경로를 named pipe 로 다룬다 |
| Python `socket.AF_UNIX` | 없다. §3 의 Python 스크립트는 Windows 에서 실행되지 않는다 |
| `codex app-server proxy` 에 줄 단위 JSON 전송 | 응답 없음. proxy 는 바이트를 그대로 전달하고, 소켓은 WebSocket 을 기대한다 |
| proxy 위 WebSocket 업그레이드 | `HTTP/1.1 101 Switching Protocols`. 최대 메시지 16 MiB (`x-codex-websocket-max-unfragmented-message-bytes`) |
| 직접 실행한 app-server 에 `proxy --sock` 3개 동시 접속 | 3개 모두 `initialize` 와 `thread/list` 성공. 같은 요청 ID 를 각자 사용 |
| proxy 프로세스 메모리 | 대기 중 약 6MB (private) |
| proxy 종료 | Node `child.kill()` 뒤에도 proxy 가 남은 경우가 있었다 |

재현:

```bash
node docs/plan/17-akra-desktop/scripts/proxy_ws_check.mjs <codex 실행 파일> 3 <소켓 경로>
```

- 소켓 경로를 생략하면 Codex 관리형 데몬의 control 소켓에 접속한다.
- Windows 에서는 소켓 디렉터리를 먼저 현재 사용자 전용으로 만든다. 예:
  `icacls <디렉터리> /inheritance:r /grant:r "%USERNAME%:(OI)(CI)F"`.
- 해석: Windows 에서 앱 → Gateway 구간은 named pipe, Gateway → app-server 구간은 proxy 를 쓴다
  (D-15, P-04, P-09). named pipe 위 WebSocket 구간은 아직 시험하지 않았다. P1 첫 작업에서 확인한다.
