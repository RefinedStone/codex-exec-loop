# Parallel Operations Visual Evidence

This supplemental artifact records the commercial-UX validation for the focused Parallel
Operations inspection board.

## Result

| Scenario | Geometry | Result |
| --- | --- | --- |
| Three-column lifecycle, lane, and selected detail | `160x16` | pass |
| Compact three-column board with every delivery gate | `120x16` | pass |
| Stacked lanes and selected lane | `80x16` | pass |
| Focused wide → narrow → passive close | frame-recorder sequence | pass |

The board stays in Akra's inline main buffer. It does not enter alternate-screen mode, does not put
panel chrome in host scrollback, and does not expose the hidden composer while focused.

## Evidence

- [Rendered wide/narrow comparison (PNG)](parallel-operations-capture.png)
- [Rendered wide/narrow comparison (SVG source)](parallel-operations-capture.svg)
- Golden text captures:
  - `parallel_operations_board_80.snap`
  - `parallel_operations_board_120.snap`
  - `parallel_operations_board_160.snap`

The PNG is rendered from the deterministic Ratatui `TestBackend` golden text. Semantic color accents
are reconstructed for readability; the golden `.snap` files remain the exact text authority.

## Automated Proof

- `parallel_operations_board_scales_across_80_120_and_160_columns`
- `focused_parallel_operations_survives_resize_and_close_without_chrome_in_scrollback`
- `parallel_projection_refresh_preserves_supersession_overlay_focus_and_selection`
- `supersession_v_opens_agent_view_without_mutating_the_buffered_prompt`
- `operations_lane_surfaces_projection_disagreement_as_desync`
- `delivery_gates_keep_unknown_unknown_and_map_typed_queue_state`
- `build_supervisor_snapshot_projects_bounded_detail_for_every_live_lane`
- `build_supervisor_snapshot_enriches_roster_from_agent_profiles`

## Scope

This is a deterministic rendering artifact, not a replacement for the repository's E1-E4 physical
terminal approval matrix. It makes no claim about Git, GitHub, database, or worker mutation because
the presentation fixture is read-only.
