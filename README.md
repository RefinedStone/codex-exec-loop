# codex-exec-loop

`codex-exec-loop` 는 이제 `codex app-server` 기반 Rust native client 를 메인 제품으로 삼습니다.

Python CLI 는 이전 migration/compatibility 경로로만 유지합니다. 새 기능 기준의 기본 설명과 운영 경로는 모두 native 쪽을 우선합니다.

## Native Quick Start

필수 조건:

- Codex CLI 설치
- Codex 로그인 완료
- Rust toolchain 사용 가능

실행:

```bash
cd /path/to/codex-exec-loop/native
. "$HOME/.cargo/env"
cargo run
```

frontend 선택:

- `CODEX_EXEC_LOOP_FRONTEND=inline`: 기본 inline main-buffer mode
- `CODEX_EXEC_LOOP_FRONTEND=alternate`: fullscreen alternate-screen mode
- `CODEX_EXEC_LOOP_ALT_SCREEN=1`: legacy alternate-screen fallback

선택 사항:

- `CODEX_EXEC_LOOP_GITHUB_PR=owner/repo#123`
- `CODEX_EXEC_LOOP_GITHUB_POLL_INTERVAL_SECS=60`

## Current Native Capability

- startup diagnostics 와 draft shell 진입
- recent session browse, search, paging, current-project filter
- existing thread resume, new thread start, `turn/start` streaming
- startup 진행 중 manual prompt queue 후 자동 제출
- inline shell inspections for diagnostics, sessions, and follow-up templates
- builtin/workspace follow-up templates, reload, editable max turns, stop rules
- approval, tool activity, runtime warning, GitHub review-change notice visibility
- packaging, checksum, verification helper scripts

세부 제품/설계 문서는 아래를 우선 보세요.

- [native/README.md](./native/README.md)
- [native/docs/design/01-current-product-state.md](./native/docs/design/01-current-product-state.md)
- [native/docs/README.md](./native/docs/README.md)

## Packaging And Validation

배포 번들 생성:

```bash
cd /path/to/codex-exec-loop
./scripts/package_native_release.sh
```

생성물 검증:

```bash
./scripts/verify_native_release.sh \
  --archive dist/native/codex-exec-loop-native-<version>-<target>.tar.gz \
  --bundle-dir dist/native/codex-exec-loop-native-<version>-<target>
```

validation 결과 템플릿 캡처:

```bash
./scripts/capture_native_validation.sh \
  --frontend inline \
  --result pass \
  --output-dir native/docs/validation
```

운영자 runbook 과 플랫폼 검증 기준:

- [native/docs/plan/13-native-packaging-and-operator-runbook.md](./native/docs/plan/13-native-packaging-and-operator-runbook.md)
- [native/docs/plan/12-platform-validation-matrix.md](./native/docs/plan/12-platform-validation-matrix.md)

## Repository Guide

- `native/`: main product crate
- `native/docs/`: current native design, plan, validation, packaging notes
- `scripts/`: native packaging / verification helpers
- `examples/`, `.codex-exec-loop/followups/`: sample prompts and follow-up templates
- `src/codex_exec_loop/`: legacy Python CLI compatibility path

## Legacy Python CLI

Python CLI 는 native migration 이 끝날 때까지 compatibility 용으로만 남겨둡니다. 새 기능 작업은 이 경로에서 시작하지 않습니다.

설치:

```bash
cd /path/to/codex-exec-loop
python3 -m venv .venv
. .venv/bin/activate
PYTHONPATH=/usr/lib/python3/dist-packages python -m pip install --no-build-isolation -e .
```

최소 사용 예:

```bash
codex-exec-loop [PROMPT]
codex-exec-loop sessions --limit 20
codex-exec-loop verify --summary logs/demo-run/summary.json
```

legacy follow-up examples and placeholders:

- [examples/followup_prompt.txt](./examples/followup_prompt.txt)
- [examples/followups/](./examples/followups/)
- [.codex-exec-loop/followups/10-review-queue.md](./.codex-exec-loop/followups/10-review-queue.md)
- [.codex-exec-loop/followups/20-docs-and-verify.md](./.codex-exec-loop/followups/20-docs-and-verify.md)
