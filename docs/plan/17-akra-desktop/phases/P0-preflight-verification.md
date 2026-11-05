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
