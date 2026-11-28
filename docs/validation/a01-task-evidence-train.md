# A01: task evidence through `task_result`

Both named fixtures passed. These are deterministic acceptance tests. Neither test proves that a person granted permission in an OS dialog.

## Human review (#3012)

On Windows NT 10.0.26200 x64, Rust 1.95.0 MSVC, session `bash_133` ran at `3b6bbf8c9ca8f21e7ca164f13bd32c5e4d855532`:

```text
cargo test --locked --lib manual_review_binds_the_displayed_artifact -- --nocapture
test application::service::parallel_mode::tests::distributor::evidence_gate::human_review::manual_review_binds_the_displayed_artifact_and_tree_before_enforce_releases_push ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 6346 filtered out; finished in 197.34s
```

Exit code: **0**. Synthetic cancel, fail, and pass responses exercised the review path. The fixture used real sealed files, SQLite, and a local bare Git remote. A source change made the review stale.

## Integrated delivery (#3013)

The Linux CI command was `cargo test --workspace --locked --timings` on Ubuntu 26.04. The four cases ran inside **one Rust test**. The result lines below come from the original jobs, not from a rerun:

| PR head | Job log | Fixture result (UTC) |
|---|---|---|
| `e2d4e2faa27e264a82be9b600bb3789690c458b5` | [46064727333 / 140730187212](https://github.com/RefinedStone/codex-exec-loop/actions/runs/46064727333/job/140730187212) | `...::integrated::a01_task_result_tracks_pre_push_holds_and_their_recovery ... ok` at 2026-11-27 22:37:03 |
| `c8d041a3cb9092d05df59b0e89245e666c338b08` | [46135219032 / 140950234950](https://github.com/RefinedStone/codex-exec-loop/actions/runs/46135219032/job/140950234950) | `...::integrated::a01_task_result_tracks_pre_push_holds_and_their_recovery ... ok` at 2026-11-28 11:05:06 |

The separate Windows command was `cargo test --locked --lib a01_task_result_tracks_pre_push_holds_and_their_recovery -- --nocapture` at `e2d4e2faa`. Session `bash_152` reached fixture execution. Its terminal was unavailable when the task resumed. Its exit code and duration are **unknown**. The Linux logs do not stand in for that Windows exit code.

Each case used the same fixture definition and task ID from accepted criteria through a real running lease, a worker `pass`, the enforce-mode distributor, and the production `task_result` query. Git and SQLite were real. PR metadata came from a GitHub test double.

| Host state | Bare remote before recovery | Final push decision |
|---|---|---|
| No host attempt | Source ref absent; integration ref unchanged | Scoped fixture waiver |
| Real Git check failed | Source ref absent; integration ref unchanged | Scoped fixture waiver |
| Host passed; source then changed | Source ref absent; integration ref unchanged | Scoped fixture waiver |
| Current host pass | No evidence hold | Host passed; no waiver |

In each case the final integration ref matched the commit in `task_result`. A worker report, a host pass, a waiver, the fake PR response, and the real local ref update are separate observations. No external issue was linked or closed. No real PR was merged and no product model turn ran.

**Gate status:** both linked jobs passed the library target (6,473 passed, 0 failed, 26 ignored). They subsequently failed `production_driving_adapters_depend_on_inbound_ports_not_application_services` in the architecture test target because the gateway evidence preview imported an application service type. The fixture pass is not an overall Rust or CI Gate pass.
