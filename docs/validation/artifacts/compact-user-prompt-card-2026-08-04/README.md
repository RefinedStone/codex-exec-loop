# Compact User Prompt Card Evidence

This focused artifact records the production-renderer proof for the compact submitted-prompt
surface.

## Result

| Contract | Result |
| --- | --- |
| Role copy | no separate `You:` row |
| Prompt hierarchy | one `›` marker inside a low-luminance full-row surface |
| Density | prompt label and body collapse from two rows to one |
| Wrapping | explicit and soft-wrapped rows retain the same surface; marker appears once |
| Interaction | submitted prompts remain non-selectable application chrome |
| Existing layout | assistant/tool hierarchy and fixed status/composer tail remain unchanged |

## Evidence

- [Exact 100×30 TestBackend text frame](frame-100x30.txt)
- [Rendered PNG](compact-user-prompt-card-100x30.png)
- [Reproducible PNG renderer](render_capture.py)

The text frame comes from
`reference_capture_is_rendered_by_the_real_fullscreen_testbackend`, which enters the same
`draw_projected` production renderer used by the native TUI. The PNG reconstructs the exact cell
geometry and `AkraTheme` colors for review. TestBackend buffer assertions remain the executable
style authority.

## Automated Proof

- `user_prompt_projects_as_one_compact_surface_without_a_role_label`
- `submitted_prompt_is_visible_but_does_not_capture_a_drag`
- `wrapped_user_prompt_keeps_the_surface_across_every_visual_row`
- `reference_capture_is_rendered_by_the_real_fullscreen_testbackend`
- `cargo test --test architecture_boundaries -- --test-threads=1`
- `cargo clippy --all-targets --all-features -- -D warnings`

## Scope Note

This is presentation-only. It changes no terminal mode, mouse primitive, resize transaction, or
clipboard delivery behavior, so deterministic production-renderer evidence is proportionate to the
change under the terminal UI validation methodology.
