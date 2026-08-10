# 개발 및 전달 가이드

[English](../../reference/development.md)

이 문서는 저장소 지도, 코딩 규칙, 테스트, worktree 조정, GitHub 전달 workflow를 통합합니다.
`AGENTS.md`가 compact한 instruction entrypoint입니다.

## 저장소 지도

- `src/core/`: headless app command/effect/completion/snapshot runtime
- `src/domain/`: 순수 conversation, session, planning, parallel, terminal, review model
- `src/application/service/`: use-case orchestration과 control-plane service
- `src/application/port/`: adapter와 독립적인 application 경계 계약
- `src/application/port/inbound/`: adapter-facing use-case interface와 request/response 계약
- `src/application/port/outbound/`: application 소유 integration capability 계약
- `src/adapter/inbound/tui/`: alternate-screen fullscreen Ratatui/Crossterm shell
- `src/adapter/inbound/{cli,admin_api,telegram_bot}/`: 다른 운영 adapter
- `src/adapter/outbound/{app_server,db,filesystem,git,github,telegram}/`: concrete boundary
- `src/composition/`: production dependency wiring
- `schema/`: app-server protocol snapshot과 provenance
- `templates/admin/`, `assets/admin/`: Admin UI template과 packaged asset
- `npm/`: launcher, platform package, Node test
- `scripts/`: validation, packaging, GitHub identity, planning, cleanup helper
- `tests/`: cross-layer integration과 architecture gate
- `docs/`: 현재 reference, executable validation contract, 명시적 미래 계획, 근거, 최신 pin을
  유지하는 compact 경쟁 분석

## 코딩 규칙

- Spring Boot/Kotlin 개발자가 빠르게 읽을 수 있는 명시적인 Rust를 작성합니다.
- 작은 단일 목적 함수와 일관된 `Service`, `Port`, `Adapter`, `Request`, `Response`, `State` 이름을
  사용합니다.
- Macro 중심 또는 추측성 abstraction보다 단순한 struct와 method를 우선합니다.
- Mapping은 adapter, 순수 decision은 domain에 둡니다.
- Inbound adapter는 좁은 inbound port에 의존하고 service/use case가 해당 port를 구현하게 합니다.
- 실패 가능한 boundary는 `Result`를 반환하고 test 밖의 `panic!`은 피합니다.
- 실제 integration boundary가 있을 때만 outbound port를 추가합니다.
- Composition wiring은 entrypoint 가까이에 두고 feature code가 concrete leaf adapter를 직접
  import하지 않게 합니다.
- App runtime/controller/presentation/planning module은 명시적으로 import하며 parent wildcard
  import를 피합니다.

TUI 작업은 수정 전에 책임 계층을 선택합니다. Context-facing 새 module은 약 600 LOC에 가깝게
유지하고, 여러 책임이 섞인 file은 약 800 LOC를 넘기 전에 분리합니다.

## 명령과 테스트

```bash
. "$HOME/.cargo/env"
cargo run
cargo build
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets --all-features -- -D warnings
```

Unit test는 module 옆에, integration test는 `tests/` 아래에 둡니다. 시작 진단, app-server parsing,
stream reduction, session mapping, planning authority mutation, parallel recovery boundary를 우선합니다.

범위가 넓은 native/TUI 작업:

```bash
bash scripts/check_native_pr.sh
```

이 gate는 TUI layering, Node surface, rustfmt, Rust test, clippy를 실행합니다. Primitive-sensitive
terminal 변경은 [검증 가이드](validation.md)도 따르고 필요한 실제 terminal evidence를 첨부합니다.

## Worktree lane

모든 변경은 최신 `origin/prerelease`에서 만든 전용 worktree에서 진행합니다.

```bash
git fetch origin
git worktree add ../codex-exec-loop-worktrees/docs-native-platform-reference \
  -b docs/native-platform-reference origin/prerelease
```

Worktree 하나, branch 하나, review 가능한 slice 하나, PR 하나를 유지합니다. Lane을 고르기 전에
`git worktree list`, local branch, open PR을 확인합니다. 서로 다른 file boundary를 우선하고 의도적인
overlap은 충돌 파일을 정확히 기록합니다.

```text
feature/native-<lane>-<zone>-<slice>
fix/native-<lane>-<zone>-<slice>
docs/native-<lane>-<zone>-<slice>
chore/native-<lane>-<zone>-<slice>
```

Local `prerelease`는 integration checkout 하나에서만 checkout합니다. Feature worktree는
`origin/prerelease`로 rebase하며 명시된 의존성이 없는 한 진행 중인 feature branch에서 새 branch를
만들지 않습니다.

## GitHub identity

이 저장소는 repo-local `RefinedStone` 전달 identity를 사용합니다. 첫 remote write 전:

```bash
git config --get akra.githubLogin
bash scripts/gh-akra.sh auth write-status
```

Global `GITHUB_TOKEN`, connector identity, `gh auth status`가 다르다는 이유만으로 중단하지 않습니다.
먼저 repo-local login과 credential helper를 확인하고 credential 값은 출력하지 않습니다.

Repository-scoped Git credential은 `git push`용입니다. GitHub API write는 wrapper를 통해 명시적
token 변수 또는 신뢰하는 `gh auth token`을 사용합니다. 제거된 legacy credential scan은 지원하지
않으며 fail-closed입니다. Intended login, repository, token identity를 검증할 수 없으면 remote write를
하지 않습니다.

## Review와 통합

완료된 slice의 기본 흐름:

```text
commit -> push -> prerelease 대상 PR -> CI Gate -> rebase merge -> cleanup
```

`prerelease`는 repository Ruleset으로 보호됩니다. 모든 update는 PR을 거치고 stable `CI Gate`가
통과해야 하며 linear history를 유지합니다. GitHub auto-merge와 branch deletion은 활성화되어
있습니다. Rebase merge를 사용하고 integration checkout을 직접 push하지 않습니다.

Workflow는 선택된 policy check를 집계한 측정용 `Fast Gate`도 발행합니다. 긴 Rust test와 portable
job은 병렬로 계속 실행하며 `CI Gate`와 push 전용 `Post-Merge Gate`가 집계합니다. 사용자가
[운영 전환 runbook](pr-validation-rollout.md)의 증거를 보고 별도 승인하기 전까지 required Ruleset
context는 `CI Gate`입니다. 일반 코드 변경은 bypass actor를 추가하거나 protection을 약화하지
않습니다.

Review thread를 확인하고 올바르며 범위에 맞는 feedback만 반영합니다. 실제 conflict, base refresh
요청, 필요한 dependency가 있을 때만 기존 PR branch를 rebase하며 그 경우에만
`--force-with-lease`를 사용합니다. 관련 없는 사용자 작업을 reset하지 않습니다.

## 정리

Branch 통합 후 integration checkout에서 실행합니다.

```bash
bash scripts/cleanup_merged_worktrees.sh --apply \
  --branch docs/native-platform-reference
```

Helper는 clean하고 merge된 non-root worktree만 제거합니다. `--force-dirty`는 남은 churn을 버려도 되는
완료 branch 하나에만 명시적으로 사용합니다. `akra-agent/slot-*`에는 사용하지 않습니다. 해당
worktree, lease, session detail은 parallel runtime이 소유합니다.
