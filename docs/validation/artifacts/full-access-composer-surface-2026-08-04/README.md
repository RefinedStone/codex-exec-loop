# Full-access And Composer Surface Evidence

This focused artifact records the deterministic production-renderer validation for the default
app-server permission profile and the fullscreen shell-tail readability change.

## Result

| Contract | Result |
| --- | --- |
| Default execution policy | `approval=never`, `reviewer=none`, `sandbox=danger-full-access` |
| Main, resume, planning, and parallel policy reuse | covered by adapter request assertions |
| Healthy default startup | policy remains visible in diagnostics without permanent `DEGRADED` chrome |
| Status/composer separation | low-contrast status surface plus lighter complete rounded composer frame |
| Tail height and cursor geometry | unchanged two-row chrome budget; inner width accounts for both borders |

## Evidence

- [Exact 100×30 TestBackend text frame](frame-100x30.txt)
- [Rendered PNG](composer-surface-100x30.png)
- [Reproducible PNG renderer](render_capture.py)

The text frame comes from
`reference_capture_is_rendered_by_the_real_fullscreen_testbackend`, which enters the same
`draw_projected` production renderer used by the native TUI. The PNG reconstructs that exact cell
geometry and the semantic colors defined by `AkraTheme`; it is a review aid, while the TestBackend
buffer assertions are the executable authority.

## Automated Proof

- `execution_policy_defaults_to_full_access_without_approval_prompts`
- `restrictive_approval_override_restores_the_user_reviewer`
- `startup_policy_summary_is_informational_and_only_elevated_risks_warn`
- `focused_composer_uses_a_complete_frame_and_distinct_tail_surfaces`
- `reference_capture_is_rendered_by_the_real_fullscreen_testbackend`
- `cargo test --test architecture_boundaries -- --test-threads=1`
- `cargo clippy --all-targets --all-features -- -D warnings`

## Platform Note

The full Windows `cargo test --lib` run reached 2,255 passing tests and 461 known platform-bound
failures. Representative failures were reproduced independently: Windows intentionally disables
external planning-file sync, and Git for Windows rejects the `\\?\` temporary worktree path used by
Linux-oriented pool fixtures. The focused policy/TUI tests and architecture/lint gates passed.
