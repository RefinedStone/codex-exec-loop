# 리포트

기준: `fix/native-platform-security-flow-hardening` / `docs/bug-report.md` / 2026-07-10

2026-06 감사 본문은 당시 증거를 보존합니다. 현재 구현 상태는
`docs/bug-report.md` 상단의 2026-07-10 재검증 표가 기준입니다.

## 구현 상태

아래 항목은 현재 브랜치의 코드/문서 계약과 로컬 Linux 통합 검증 결과입니다. 지원 OS
실기기, 실제 터미널, 브라우저 스크린샷, PR CI는 별도 증거로 구분합니다.

- release/npm: `GITHUB-001..004`, `TESTS-001..002`, `DOCS-003`
- operator scripts/prompts: `NPM-002..003`, `SCRIPTS-001`, `SCRIPTS-002`, `SCRIPTS-004`, `EXAMPLES-001..003`,
  `CODEX-LOOP-003`, `GEMINI-001..003`
- protocol/admin/runtime: `SCHEMA-001..003`, `TEMPLATES-001..003`, `SRC-002..003`
- app-server security: 기본 `on-request` / `workspace-write`, main TUI one-shot approval,
  unattended hidden-worker decline, hidden planning `read-only` + local interrupt, scrubbed child
  environment, narrow `api-key-auth=1`과 explicit `process-environment=all` opt-in, prompt/trace
  opt-in과 bounded private retention. Approval은 명시적 `Y`만 수락하고 `Enter`는 inert이며,
  receipt deadline/interrupt/disconnect/invalid payload는 응답 직전 재검사까지 fail-closed
- admin security: 256-bit capability, 매 실행마다 무작위 exact `*.localhost` origin, strict
  Host/Origin 검증, 30분 idle/8시간 absolute session 한도, logout 폐기
- planning safety: admin/runtime mutation guards, stale file-sync와 protected restore CAS, v7/v8 ->
  v9 atomic authority-store migration, descriptor-anchored Unix I/O, bounded quarantine
  preservation, no-clobber Git common-dir incarnation, Windows Git/non-Git private SQLite authority
- parallel delivery: configurable integration branch/push remote, public/autonomous delivery의
  parent-process 명시 opt-in, immutable remote/base/source proof, stable tracking observation,
  detached integration, post-turn generation + epoch commit linearization, process-start-aware
  official refresh ownership과 cancel-without-advance
- process/release: PID/start-time fenced Telegram ownership, platform subprocess containment,
  exact 7-key `VERSION.txt`, complete canonical `SHA256SUMS.txt`, streamed archive/extracted-byte
  reverification
- validation status disclosure: `DOCS-002`
- architecture and repository hygiene: `TESTS-003`, `ASSETS-002..003`, `ARTIFACTS-001..003`,
  `TMP-001..003`
- time-sensitive copy safety: `DOCS-004`의 과거 지표는 archival snapshot으로 격리했고,
  metric-bearing draft 문구를 제거했습니다.

### 신규 보안 구현과 로컬 통합 검증

- `SECURITY-010`: Codex는 trusted absolute path의 native binary 또는 제한된 npm launcher만
  허용합니다. Unix npm script/Windows standard npm `.cmd`의 Codex target과 native Node를
  따로 pin하며, `git`/`gh`/`curl`/`bash`도 native executable을 pin합니다. hostile/relative
  `PATH`, repository/pool-controlled path, unsafe owner/ACL/interpreter는 fail-closed입니다.
- `SECURITY-011`: prompt capture가 비활성인 모든 production composition 기동은 이전 opt-in
  실행에서 남은 아직 만료되지 않은 본문까지 대상으로 all-row/metadata clear를 호출합니다.
  authority SQLite는 `secure_delete=ON`이며 cleanup 실패는 본문 없는 경고를 남기고 capture를
  켜지 않습니다. capture가 활성일 때만 7일/100 interaction 한도와 body/item bound를 적용해
  보존합니다.
- `SECURITY-012`: 모든 pool mutation은 private pool root의 persistent repo-scoped OS lock과
  root/lock identity 재검사를 거칩니다. 새 slot lease generation은 OS CSPRNG의 32 bytes를
  정확한 64 lowercase hex로 저장하고 lifecycle/session/distributor/cleanup mutation은 exact
  generation CAS/event fence를 사용합니다. Unix JSON mirror는 descriptor-anchored
  non-authoritative projection이고, missing/stale same-generation mirror install과 rollback은
  authority CAS 및 exact observed body에 결합됩니다. Lock file만 생긴 첫 초기화는 기존 pool로
  오인하지 않습니다. Windows는 SQLite-only입니다.
- `SECURITY-013`: host-owned worktree/checkout/reset/cherry-pick/commit mutation 전에 effective
  repository-local/worktree Git executable config key를 bounded/NUL-safe하게 감사하고 filter,
  merge/diff driver, textconv, alternate-ref, unsafe `core.worktree`, archive/mergetool/difftool
  command가 있으면 값을 실행하거나 노출하지 않고 차단합니다. 정상 submodule의 검증된
  `core.worktree`만 pinned worktree와 일치할 때 허용하며 nested config도 별도 감사합니다. Remote
  Git/GitHub write는 frozen credential-free HTTPS target의 owner-private isolated
  network/config/object context에서만 실행합니다.
- `SECURITY-014`: app-server approval은 explicit `Y` only이며 `Enter`는 no-op입니다. receipt
  deadline, timeout, interrupt, disconnect, channel saturation, invalid request는 모두 decline하고
  accept 직전에도 interrupt/deadline을 다시 확인합니다.
- `SECURITY-015`: GitHub credential discovery는 explicit token 또는 trusted `gh auth token`으로
  제한되며 legacy credential scan 변수의 존재 자체가 fail-closed입니다. Review HTTP는 90초
  aggregate deadline, 20 page/2,000 item/32 MiB aggregate/8 MiB response 한도와 HTTPS/no-redirect
  계약을 가집니다. Hostile PATH 테스트는 process-global 환경을 바꾸지 않고 resolver에 명시
  입력되며 기본 병렬 library test 2,098개가 통과했습니다.

검증 결과:

- `cargo test --locked --lib -- --quiet`: 2,098 passed, 기본 병렬 실행
- `cargo test --locked --lib --no-fail-fast -- --test-threads=1 --quiet`: 2,098 passed
- architecture 42, binary entrypoint 3, native validation script 49, repository hygiene 5 passed
- `cargo clippy --locked --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`,
  `git diff --check`, shell syntax, TUI layering passed
- npm launcher/release 65 tests, admin TypeScript/build/offline-asset contract,
  `cargo audit --deny warnings` passed
- admin graphic contract passed; 지원 브라우저 실행 파일이 없어 screenshot capture는 skipped
- Windows GNU/macOS x64 `cargo check`는 각각 MinGW GCC와 Apple C cross toolchain 부재로
  `libsqlite3-sys` 빌드 단계에서 중단됐습니다. Akra crate의 지원 OS 검증 증거로 계산하지 않습니다.

## 남은 검증과 한계

- `DOCS-001`: 실제 terminal-baseline required row가 아직 `0/4 pass`입니다. 코드나 문서 수정으로
  pass를 만들지 말고, 해당 터미널에서 실측 캡처해야 합니다.
- 이 브랜치에서 추가한 Windows ACL/standard npm `.cmd`, native executable/interpreter pin,
  Windows pool lock은 Linux 테스트나 pure parser 테스트만으로 지원 OS 검증 완료라 할 수
  없습니다. Windows와 macOS 지원 host/PR CI 결과를 별도로 확인해야 하며 cross-compile은
  실기기/실제 터미널 증거를 대체하지 않습니다.
- `DOCS-004`: 문서가 stale 값을 현재값으로 표시하지는 않지만, 공개 직전 실제 GitHub/npm
  결과를 날짜와 함께 다시 캡처해야 합니다.
- Windows는 Git/non-Git workspace authority를 private SQLite로 지원합니다. 다만 direct planning
  artifact filesystem, candidate inspection, external file export/apply는 NT relative-handle 구현
  전까지 fail-closed입니다. Parallel worker의 lease/distributor/session 상태도 SQLite authority를
  사용하며, pool-local JSON mirror는 NT relative-handle 구현 전까지 생성하거나 읽지 않습니다.
  mirror write/remove는 non-authoritative no-op projection이고 read/list는 cache miss라서 insecure
  Windows path fallback 없이 worker flow를 유지합니다.
- repository authority namespace는 canonical Git common-dir 안의 owner-private 256-bit 세대
  marker를 기준으로 합니다. 같은 repository의 linked worktree와 이동된 common-dir은 공유하지만,
  같은 경로에 다시 생성한 repository나 새 clone은 새 namespace입니다. marker가 unsafe하면
  경로 기반 fallback 없이 중단합니다.
- 권한/handle/identity 재검증은 다른 사용자와 우발적 경합을 줄일 뿐, 같은 UID의 악성 process가
  로컬 파일/ref/sidecar를 경합하는 위협을 격리하지 않습니다. 이 위협은 별도 OS 계정 또는 sandbox가
  경계입니다.
- Linux에서 marker를 지우고 모든 관측 가능한 parent 종료 뒤 완전히 daemonize한 process는
  userspace sweep를 벗어날 수 있어 cgroup/PID namespace/VM이 필요합니다. macOS 및 기타 Unix는
  process-group-only라 `setsid` 이탈이 잔여 한계입니다.
- `AKRA_APP_SERVER_PROCESS_ENVIRONMENT=all`은 모든 parent credential을 app-server에 노출하는 명시적
  고위험 opt-in입니다. 기본 scrubbed 정책과 동일한 보장을 제공하지 않습니다.
- release mutation 직전/직후 tag target을 검증해도 작은 check-to-mutation race는 남습니다.
  protected `v*` tag ruleset이 서버 경계이고, publish된 npm version은 되돌릴 수 없으므로 이후
  GitHub Release 실패는 동일 tag 재실행으로 복구해야 합니다.
