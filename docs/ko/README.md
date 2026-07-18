# Akra

[English](../../README.md)

Akra는 `codex app-server`를 사용하는 네이티브 Rust 운영 클라이언트입니다. 저장소 이름은
`codex-exec-loop`, crate와 기존 바이너리 이름은 `codex-exec-loop-native`, 운영 명령은 `akra`입니다.

Akra는 시작 진단, 세션 재개, 프롬프트 스트리밍, 승인된 planning, 큐 기반 연속 실행, 병렬 작업자
전달을 하나의 장시간 터미널 흐름으로 묶습니다. TUI가 기본 화면이며 CLI, Admin, Telegram,
자동화 adapter도 같은 application service를 사용합니다.

## 구현된 화면

| 화면 | 현재 역할 |
| --- | --- |
| 네이티브 TUI | Ratatui/Crossterm inline main-buffer shell. 세션, planning, queue, review, activity, parallel overlay와 host scrollback을 제공합니다. |
| Core runtime | 시작, 세션, turn 제출, stream, 완료, post-turn 평가를 위한 headless command/effect/completion/snapshot 흐름입니다. |
| Planning | SQLite 권한 저장소와 `.codex-exec-loop/planning/` 아래의 staged planning 산출물을 사용합니다. |
| Parallel mode | 보호된 worktree 3개, worker lease, 공식 완료 갱신, 검토된 GitHub 전달, 통합, 정리를 관리합니다. |
| CLI/자동화 | planning 상태·queue·reset, 구조화 planning mutation, 수동 distributor tick을 제공합니다. |
| Admin/Telegram | 같은 application service 위에서 loopback Admin UI/API와 allowlist 기반 Telegram 제어를 제공합니다. |
| 배포 | 네이티브 archive, npm 플랫폼 패키지, 검증 캡처 도구, tag 기반 release workflow를 제공합니다. |

정확한 운영 계약은 [현재 제품 계약](reference/current-product.md), 구조와 상태 소유권은
[런타임 아키텍처](reference/architecture.md)를 참고하세요.

## 설치

사전 조건:

- 신뢰할 수 있는 절대 `PATH`에서 찾을 수 있는 공식 Codex CLI
- 완료된 `codex login`
- 대상 workspace 접근 권한
- Linux에서는 동작하는 Codex sandbox helper(Codex 설치본 또는 시스템의 `bwrap`)

npm 설치:

```bash
npm install -g @refinedstone/akra
cd /path/to/workspace
akra
```

소스 빌드:

```bash
. "$HOME/.cargo/env"
cargo build --release
cargo run
```

네이티브 패키징과 배포 계약은 [네이티브 패키징과 배포](reference/release.md)에 있습니다.

## 운영 빠른 시작

작업할 저장소에서 `akra`를 실행하세요. 시작 진단은 즉시 실행됩니다. 진단 중에도 입력을 작성할 수
있지만 실제 제출은 준비 상태가 된 뒤 진행됩니다.

주요 shell 명령:

| 명령 | 용도 |
| --- | --- |
| `:sessions` | Codex 세션 검색과 재개 |
| `:queue` / `:q` | 승인된 작업과 제안 작업 확인 |
| `:planning` | planning 작성 시작 또는 재개 |
| `:directions` | planning 방향과 idle 정책 관리 |
| `:reviews` | review center 확인 |
| `:activity [all\|diff\|output\|command\|…]` | progressive activity 카드 목록과 선택 상세 확인 |
| `:parallel` / `:pa` | parallel board 활성화 또는 갱신 |
| `:parallel off` | worktree를 삭제하지 않고 로컬 병렬 자동화 중지 |
| `:turns <positive\|infinite\|off>` | 단일 세션 auto-follow 제어 |
| `:model`, `:think ...`, `:view`, `:language` | 모델, 추론, transcript 상세도, TUI 언어 선택 |
| `:doctor`, `:diag`, `:help` | planning 상태, 시작 진단, 명령 도움말 확인 |

전체 명령은 `:help`로 확인할 수 있습니다. 키, planning 규칙, 복구 동작, 병렬 전달 불변식은
[현재 제품 계약](reference/current-product.md)에 정리되어 있습니다.

## CLI

```text
akra doctor [workspace_dir]
akra status [workspace_dir]
akra queue [workspace_dir]
akra reset <queue|directions|all> [workspace_dir]
akra planning-tool <contract|run> [workspace_dir]
akra parallel-tick [workspace_dir]
akra admin [--port <port>]
akra telegram [options]
```

`planning-tool run`은 stdin에서 JSON 요청 하나를 읽습니다. `admin`은 `127.0.0.1`에만 bind하며
capability 기반 login/CSRF 경계를 사용합니다. Telegram은 bot token과 명시적인 chat allowlist가
필요하고, group 명령은 허용된 발신자도 필요합니다.

## 상태와 설정

- Planning 권한은 `${AKRA_HOME:-~/.akra}/projects/<project>/runtime/` 아래에 저장됩니다.
- Git 저장소는 canonical Git common directory의 owner-private incarnation marker로 식별됩니다.
  따라서 linked worktree는 권한을 공유하고 새 clone은 분리됩니다.
- `.codex-exec-loop/planning/`에는 운영자가 작성한 상세 문서, prompt, staged draft, 거부 기록이
  들어갑니다. task 권한 DB는 아닙니다.
- `AKRA_APP_SERVER_PROMPT_LOG=1`은 제한된 prompt/response 진단을 활성화합니다. Trace JSONL은
  별도 opt-in이며 body를 기록하지 않습니다.
- GitHub 쓰기는 설정된 login과 API credential, 대상 저장소가 일치해야 합니다.
  `bash scripts/gh-akra.sh auth write-status`로 검증합니다.

보안과 영속성 상세는 중복된 명령 문서가 아니라 [런타임 아키텍처](reference/architecture.md)에
유지합니다.

## 개발

```bash
. "$HOME/.cargo/env"
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets --all-features -- -D warnings
```

범위가 넓은 native/TUI 변경은 다음을 실행합니다.

```bash
bash scripts/check_native_pr.sh
```

저장소 구조, architecture gate, worktree 규칙, GitHub 전달 순서는
[개발 및 전달 가이드](reference/development.md)에 있습니다.

## 문서

- [문서 지도](docs-map.md)
- [현재 제품 및 운영 계약](reference/current-product.md)
- [런타임 아키텍처](reference/architecture.md)
- [개발 및 전달 가이드](reference/development.md)
- [TUI 시각 계약](reference/tui-contract.md)
- [검증 가이드](reference/validation.md)
- [경쟁 앱 조사 원문](../competitive/README.md)

현재 구현 참조 문서는 compact하게 유지하고 `docs/ko/`에서 한국어 번역을 관리합니다. 미래 계획과
재현 가능한 경쟁 앱 조사는 원문을 축약하지 않습니다.
