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
| Focused board with CJK task composer | `120x32` TestBackend | pass |
| Readiness blocker with CJK task submission | `120x32` Orca PTY | pass |

The current focused board stays inside Akra's alternate-screen transaction and reserves the task
composer after supervisor loading finishes. Loading still hides and locks the composer while
preserving its draft. A loaded readiness blocker remains visible above an active composer.

## Evidence

- [Rendered wide/narrow comparison (PNG)](parallel-operations-capture.png)
- [Rendered wide/narrow comparison (SVG source)](parallel-operations-capture.svg)
- Golden text captures:
  - `parallel_operations_board_80.snap`
  - `parallel_operations_board_120.snap`
  - `parallel_operations_board_160.snap`

The PNG is rendered from the deterministic Ratatui `TestBackend` golden text. Semantic color accents
are reconstructed for readability; the golden `.snap` files remain the exact text authority for
the board geometries they capture. Those earlier board-only captures predate the active focused
composer contract and do not define prompt ownership.

## Automated Proof

- `parallel_operations_board_scales_across_80_120_and_160_columns`
- `focused_parallel_operations_survives_resize_and_close_without_chrome_in_scrollback`
- `parallel_projection_refresh_preserves_supersession_overlay_focus_and_selection`
- `ready_parallel_control_tower_keeps_task_composer_active`
- `loading_parallel_control_tower_preserves_the_hidden_task_draft`
- `focused_parallel_operations_reserves_the_task_composer_tail`
- `focused_parallel_operations_renders_the_cjk_task_composer`
- `operations_lane_surfaces_projection_disagreement_as_desync`
- `delivery_gates_keep_unknown_unknown_and_map_typed_queue_state`
- `build_supervisor_snapshot_projects_bounded_detail_for_every_live_lane`
- `build_supervisor_snapshot_enriches_roster_from_agent_profiles`

## Scope

The deterministic rendering artifacts are not a replacement for the repository's E1-E4 physical
terminal approval matrix. The focused-composer correction was also exercised through the native
Akra entry point in an Orca PTY: `:parallel` retained a real readiness blocker, `verify 작업`
rendered without layout drift, and `Enter` cleared the composer and produced accepted dispatch
`#1`. This evidence makes no claim about GitHub or database mutation.
