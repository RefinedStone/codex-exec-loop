# 06. 인계 가이드

상태: Proposed · 기준일: 2026-11-05

이 문서는 이 계획을 이어받는 작업자가 처음 읽는 실무 안내이다.

## 1. 먼저 읽을 것

1. [README](README.md) 의 요약, 결정 요약, 단계 표
2. [03 결정 기록](03-decisions.md): 특히 상태가 "권장"과 "제안"인 항목. 이 항목은 사용자 확인이 필요하다
3. [04 목표 구조](04-target-architecture.md)
4. 다음 단계 문서: 현재는 [P1](phases/P1-gateway-daemon.md)
5. Akra 저장소 규칙: [AGENTS.md](../../../AGENTS.md),
   [development.md](../../reference/development.md)

## 2. 작업 환경 (2026-11-05 확인값)

| 항목 | 값 |
| --- | --- |
| OS | macOS (Apple Silicon), Windows 11 (D-15) |
| Rust | 1.95.0 (`rust-toolchain.toml` 이 고정) |
| Node, npm | Node 24.3.0, npm 11.4.2. pnpm 은 설치되어 있지 않다 |
| codex CLI | macOS: 0.149.1 (`/opt/homebrew/bin/codex`). Windows: 0.159.0 (npm 전역 설치). Akra 와 Gateway 는 PATH 의 codex 를 쓴다 |
| Codex 로그인 | CLI 는 ChatGPT 계정으로 로그인되어 있다. 플랜은 free 이다 |
| 공식 데스크톱 앱 | ChatGPT 앱 26.1029.61003 (번들 codex 0.166.0-alpha.6). 비교용으로만 쓴다 |
| Akra 전역 설정 | `$AKRA_HOME` (기본 `~/.akra`) |

## 3. 저장소와 신원

| 항목 | 값 |
| --- | --- |
| 저장소 | `RefinedStone/codex-exec-loop` (**공개**) |
| 통합 체크아웃 | `~/Documents/dev/akra/codex-exec-loop` (`prerelease` 브랜치) |
| 작업 worktree 위치 | `~/Documents/dev/akra/codex-exec-loop-worktrees/<이름>` |
| 이 계획의 반영 | `prerelease` 에 직접 push (2026-11-05, 사용자 지시) |
| 커밋·원격 신원 | repo-local `RefinedStone` (D-14) |

원격에 쓰기 전에 신원을 확인한다.

```bash
git config --get akra.githubLogin          # RefinedStone 이어야 한다
bash scripts/gh-akra.sh auth write-status   # RefinedStone 이어야 한다
```

- 전역 `gh` 로그인이나 `GITHUB_TOKEN` 은 다른 계정일 수 있다. 저장소 규칙대로 repo-local 신원을 쓴다.
- 자격 증명 값은 출력하지 않는다.
- 회사 계정, 회사 저장소, 회사 데이터를 이 저장소에 넣지 않는다.

## 4. 작업 절차 (Akra 저장소 규칙 요약)

1. 읽기, 진단, 계획은 현재 체크아웃에서 한다.
2. 추적 파일을 처음 바꾸기 전에 `git fetch origin` 을 하고, `origin/prerelease` 에서 전용 worktree 를 만든다.

   ```bash
   git worktree add ../codex-exec-loop-worktrees/<이름> -b <브랜치> origin/prerelease
   ```

3. 결과물 하나에 worktree 하나, 브랜치 하나, PR 하나를 쓴다. 커밋은 작게 나눈다.
4. 전달 전에 `node scripts/agent-plan.mjs` 를 실행하고, 출력된 검사를 모두 실행한다.
5. 전달 순서: commit → push → PR(`prerelease` 대상) → CI Gate → rebase merge → worktree 정리.
6. 원격 쓰기(push, PR) 전에 사용자에게 확인한다. 사용자가 로컬 보류를 요청하면 따른다.
7. `akra-agent/slot-*` worktree 는 Akra 런타임이 소유한다. 정리하지 않는다.

## 5. 자주 쓰는 명령

| 목적 | 명령 | 소요 시간 (이 Mac) |
| --- | --- | --- |
| 릴리스 빌드 | `cargo build --release` | 약 2분 19초 |
| 작업공간 진단 | `./target/release/akra doctor` | 즉시 |
| 전체 테스트 | `cargo test --release` | 약 12분. 불안정 테스트 있음 |
| 일부 테스트 | `cargo test --release --lib -- <경로 필터>` | - |
| 전달 전 검사 목록 | `node scripts/agent-plan.mjs` | 즉시 |
| app-server 핸드셰이크 (macOS·Linux) | `python3 -I docs/plan/17-akra-desktop/scripts/appserver_handshake.py "$(command -v codex)"` | 수 초 |
| 다중 접속 시험 (macOS·Linux) | `python3 -I docs/plan/17-akra-desktop/scripts/multi_client_check.py --codex "$(command -v codex)"` | 수 초 |
| 다중 접속 시험 (Windows 포함) | `node docs/plan/17-akra-desktop/scripts/proxy_ws_check.mjs <codex 실행 파일> 3 [소켓 경로]` | 수 초 |
| 타입 생성 (P2 이후) | `codex app-server generate-ts --out <디렉터리>` | 수 초 |

코딩 에이전트의 샌드박스 안에서 git 이나 소켓 명령이 실패하면, 먼저 샌드박스 제한인지 확인한다.

## 6. Codex 사용량 정책 (D-13)

| 구분 | 예 |
| --- | --- |
| 허락 없이 해도 된다 | 빌드, 단위 테스트, 가짜 서버 시험, 녹화 재생, `initialize`, `account/read`, `thread/list`, `thread/read`, `project/list`, `model/list` |
| 사용자 허락이 필요하다 | `turn/start`, `turn/steer`, `thread/compact/start`, 그 밖에 모델을 호출하는 모든 요청 |
| 허락받은 실측 | 목적과 횟수를 먼저 알린다. 짧은 프롬프트, 낮은 추론 강도, 가능하면 `ephemeral` 스레드를 쓴다. 녹화해서 재사용한다. 시험 스레드는 끝나면 archive 한다 |

## 7. 금지 사항

- 공식 Codex 앱, Claude 앱의 코드를 복사하지 않는다. 구조만 참고한다. (dots 비공개 백엔드 연결은 D-19 로
  범위 안으로 들어왔다. 단 실계정 쓰기는 매번 허락 뒤에 한다.)
- AGPL-3.0, SUL-1.0 코드를 차용하지 않는다 ([alternatives.md](reference/alternatives.md) §1).
- 자격 증명을 출력하거나 커밋하지 않는다.
- 녹화 파일, 실제 대화 내용, 회사 정보를 커밋하지 않는다.
- 사용자 허락 없이 턴을 실행하지 않는다.
- 계획 문서의 미래 내용을 `docs/reference/` 에 구현 완료처럼 적지 않는다.

## 8. 작업을 시작할 때 확인 목록

1. `git fetch origin` 후 이 계획의 기준 커밋과 현재 `origin/prerelease` 를 비교한다. 바뀐 부분이 이 계획과
   충돌하는지 본다.
2. [05](05-risks-and-open-questions.md) 에서 "확인 필요" 상태의 질문을 사용자에게 묻는다. 현재는 Q-10,
   Q-11 이다.
3. 다음 단계 문서의 "선행 조건"을 확인한다.
4. `codex --version` 이 이 문서의 값과 다르면 P0 의 핸드셰이크와 다중 접속 시험을 다시 실행한다.
5. 작업이 끝나면 단계 문서의 상태, [README](README.md) 의 단계 표, 결정과 질문의 상태를 고친다.
