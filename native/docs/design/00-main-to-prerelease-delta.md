# Main To Prerelease Delta

## Why This Comparison Matters
The current docs should not be based on `origin/main`. The latest native work is on `origin/prerelease`, and that branch already moved well beyond the older dashboard-only prototype.

## Merge Base
- common base: `dc0a862`

## Commit Themes Added On `prerelease`
- live conversation shell
- new thread creation flow
- explicit workspace path in startup diagnostics
- `Ctrl+C` back navigation and exit confirmation behavior
- clearer conversation input state handling
- conversation tail and rendering fixes
- auto follow-up loop and decision model
- follow-up strategy picker
- stop rules, including `AUTO_STOP` and no-file-change behavior
- workspace follow-up templates loaded from `.codex-exec-loop/followups/`

## Code-Level Delta
Relative to `origin/main`, `origin/prerelease` changes 16 native files with roughly 2.6k insertions. The biggest shifts are:

- `src/adapter/inbound/tui/app.rs`
  - shell state, input handling, live stream event reduction, auto follow-up controls
- `src/adapter/outbound/codex_app_server_adapter.rs`
  - `thread/read`, `thread/start`, `thread/resume`, `turn/start`, and stream notification mapping
- `src/application/port/outbound/codex_app_server_port.rs`
  - conversation snapshot and stream APIs added
- `src/application/service/conversation_service.rs`
  - conversation orchestration added
- `src/application/service/followup_template_service.rs`
  - builtin and workspace follow-up catalog loading added
- `src/adapter/outbound/filesystem_followup_template_adapter.rs`
  - workspace template discovery added
- `src/domain/conversation.rs`
  - conversation snapshot and stream event model added
- `src/domain/followup_template.rs`
  - follow-up template catalog model added

## Documentation Rule
Any design or roadmap document under `native/docs/` should assume the `prerelease` feature set above. If a document reads like the shell is still only a placeholder preview, it is outdated.
