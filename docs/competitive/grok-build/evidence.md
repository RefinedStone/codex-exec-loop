# Grok Build Evidence Ledger

## Snapshot Pin

| Field | Value |
| --- | --- |
| Product | Grok Build |
| Source | https://github.com/xai-org/grok-build |
| Commit | `b189869b7755d2b482969acf6c92da3ecfeffd36` |
| Commit subject | Publish harness and TUI open-source |
| Commit date | 2026-07-15 23:47:40 +0100 |
| Local path inspected | `/tmp/grok-build` (shallow clone depth 1) |
| Akra baseline | `12bad93d08232a7626786b5edad91f52c2f5a6fc` |
| Audit date | 2026-07-16 (Asia/Seoul) |

## Inspection Commands

```sh
git clone --depth 1 https://github.com/xai-org/grok-build /tmp/grok-build
cd /tmp/grok-build && git rev-parse HEAD && git log -1 --format='%H %ci %s'
find crates -name 'Cargo.toml' | wc -l
ls crates/codegen/xai-grok-pager/docs/user-guide/
```

## Source Ledger

| ID | Path / artifact | Class | Supports |
| --- | --- | --- | --- |
| E1 | `README.md` | verified | product shape, install, layout, license, ACP/headless mention |
| E2 | `crates/codegen/xai-grok-pager/docs/user-guide/*.md` (01–24) | verified | feature catalog and operator contracts |
| E3 | `crates/codegen/xai-grok-shell/src/leader/mod.rs` | verified | leader multi-client IPC architecture |
| E4 | `crates/codegen/xai-grok-tools/src/tool_taxonomy.rs` | verified | ToolKind taxonomy, read-only classification, meta envelope |
| E5 | `crates/codegen/xai-grok-tools/src/implementations/` | verified | grok_build / codex / opencode tool implementations |
| E6 | `crates/codegen/xai-fast-worktree/src/lib.rs` and crate tree (`api`, `copy`, `btrfs`, `overlay`, `sync`, `worktree/execute`) | verified | CoW / BTRFS / overlay / pool sync worktree engine; see [fast-worktree.md](./fast-worktree.md) |
| E7 | `crates/codegen/xai-hunk-tracker/src/lib.rs` | verified | agent vs external hunk attribution actor |
| E8 | `crates/codegen/xai-ratatui-inline/src/` | verified | inline terminal delivery primitives |
| E9 | `THIRD-PARTY-NOTICES` + tools crate notices | verified | codex/opencode ports, Apache-2.0 first-party |
| E10 | `CONTRIBUTING.md` | verified | external contributions not accepted |
| E11 | `rust-toolchain.toml` | verified | Rust 1.92.0 pin |
| E12 | Akra `docs/reference/current-product.md` | verified | Akra shipped operator contract |
| E13 | Akra `docs/reference/architecture.md` | verified | authority and layer boundaries |
| E14 | Akra `docs/competitive/README.md` | verified | product position pillars and decision rules |

## Evidence Limits

- Shallow clone only; history beyond the publish commit not inspected.
- No binary install, no authenticated API run, no latency/memory benchmarks.
- Some modules use `dead_code` / monorepo-sync residue; code presence ≠ default-enabled product path.
- Online docs at docs.x.ai cited as secondary; local user-guide is primary.

## Claims Explicitly Not Made

- Relative model quality or token cost
- Startup/stream/render latency comparisons
- That every listed crate feature is on by default in public releases
