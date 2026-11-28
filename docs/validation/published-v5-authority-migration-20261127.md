# Published v5 planning authority migration (#2588)

The genuine published Windows v1.3.5 authority now migrates through the existing
SQLite transaction. Its task IDs, all legacy task fields, dependency, queue head,
planning revision and canonical workspace binding survive. The occupied
pre-incarnation Git namespace recovery remains a separate gap.

This succeeds the failing baseline in [PR #3024](https://github.com/RefinedStone/codex-exec-loop/pull/3024).
That historical failure is not rewritten. Machine-readable results and before/after
database hashes are in [the JSON receipt](published-v5-authority-migration-20261127.json).

## Change and boundaries

Schema admission is extracted into `schema_admission.rs`. It explicitly admits the
verified published schema 5 in addition to the existing 7–21 range. Schema 6 is
still unsupported. Empty-store, mode, canonical-root, repository-identity and
future-schema guards are unchanged.

No second migration mechanism is added. `store::ensure_schema` performs the
existing additive changes and schema/identity marker update in its IMMEDIATE
transaction. The regression fixture was exported from the actual old executable
after two synthetic task creations. Only its temporary workspace binding is
replaced in the checked-in SQL. The new test module is under the existing
`tests::authority_store_` native-selector prefix.

## Executed evidence

Host: Windows 10.0.26200 x64, Rust 1.95.0. Date: 2026-11-27 UTC / 2026-11-28 KST.
Parent source: `de1a879deb0b5407b437762faf1530330e8e2f6c`, with this migration patch.
The actual candidate is a **debug package**, not a published release.

| Check | Terminal outcome |
| --- | --- |
| New genuine-v5 regressions and existing v7–v21/unsupported-schema regression | 4 passed, 0 failed, 0 ignored; exit 0 |
| Deliberately replace v5 admission with v6 | 3 new tests failed; exit 101; mutation restored |
| Genuine published archive → actual candidate archive smoke | `verified published legacy task preservation`; exit 0 |
| Genuine two-task database → candidate | Exit 0; schema 21; planning revision remains 4 |
| Independent candidate reopen | Exit 0; same tasks and queue head |
| Schema 4, 6, 22 and malformed version | Each exits 1; entire database SHA-256 unchanged |
| Foreign mode, canonical root and repository identity | Each exits 1; entire database SHA-256 unchanged |
| Trigger aborts the final schema-marker update | Exit 1; entire database SHA-256 unchanged |
| Remove only the injected fault, then retry | Exit 0; both original tasks and every legacy task field preserved |
| Old executable opens the migrated schema-21 database | Exit 1, unsupported schema 21; database SHA-256 unchanged |

The old archive SHA-256 is
`496097730bc8b7cb37d6d79759456bc68bf47d5995eaad8e029a1d741ca01eaa`.
The candidate archive SHA-256 is
`dc17c575773a93e1675c88d72775656c8d048bd09d5c5303e6b7bc1f7e0b26d2`.
The original genuine database backup is retained unchanged in local evidence.

Reproduction commands:

```text
cargo test --locked --all-features --lib --target x86_64-pc-windows-msvc -- authority_store_published_v5 authority_schema_migrates_v7 --nocapture --test-threads=2
node npm/scripts/smoke-native-release.mjs <candidate-assets> 1.3.5 x86_64-pc-windows-msvc debug --previous-legacy <published-assets> 1.3.5
```

The byte-preservation probes operate on disposable copies of the genuine database.
They alter one admission marker or add an abort-only trigger, hash the file, run
the real candidate's `planning-tool run`, and hash it again. No operational database,
credential, model, provider or release setting is used.

## Repository gate and disposable recovery experiment

The already-running repository planner completed with exit 1. It was not restarted.
It passed 107 native contracts containing 991 tests, including all eight tests in
the authority-store selector. Desktop tests passed 1527 tests with eight skipped.
The separate all-targets, all-features Clippy check passed with warnings denied.

The failing native member group passed 28 tests, failed three and ignored one.
The failures were the pinned native fingerprint test, changed pinned fingerprint
test, and host Git/Node executable test. Windows executable ancestor ACL validation
rejected the existing Local AppData and Node installation parent directories.
Those source files are unchanged by this patch. No baseline rerun was performed,
and the full gate is not green. The JSON receipt names each failure exactly.

A second experiment created a task with the genuine old executable in a disposable
Git repository. The candidate isolated the occupied old namespace and returned
`planning task authority is unavailable`, leaving the old database byte-identical.
An exclusive file copy to the occupied destination failed with `EEXIST`; both
files remained unchanged. An explicit copy to an absent destination in a separate
disposable profile then migrated successfully, retaining every legacy task row,
planning revision 5, and the original task ID. It acquired the current repository
identity without rewriting any identity marker. The source remained byte-identical.

This is a quiescent fixture experiment, not a shipped recovery command. A product
flow still needs explicit source confirmation, writer exclusion, race-safe identity
checks, atomic publication, private-file validation, and a policy for an occupied
destination. The disposable profile was removed; sanitized receipts and database
evidence were retained.

## Remaining recovery and support limits

Ordinary Git repositories with an occupied historical namespace still enter a new
incarnation namespace. This change does not automatically claim ambiguous old
data, move the old database, or replace an existing destination. Explicit,
non-destructive recovery with bound source identity and concurrency protection is
still needed under #2588.

Schema 6, versions below 5, future versions, foreign bindings and unmarked
non-empty databases remain rejected. A migrated schema-21 database is not usable
by the old executable; rollback requires the retained pre-upgrade backup. Native
macOS/Linux, live credentials, simultaneous old/new writers and the full installer
or npm upgrade matrix are not certified by this Windows data migration.
