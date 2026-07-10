# Gemini Repository Guidance

`AGENTS.md` is the authoritative instruction entrypoint. Read it first, then open only the linked
task-specific guidance. When this file and `AGENTS.md` differ, follow `AGENTS.md`.

- Write review comments, implementation explanations, and rebuttals in Korean.
- Make every change in a dedicated worktree based on `origin/prerelease`.
- For each meaningful slice, follow the repository's commit, push, PR-to-`prerelease`, rebase-merge,
  and worktree-cleanup workflow. Verify the active GitHub identity before remote writes.
- Use Rust edition 2024 as declared in `Cargo.toml` and the pinned Rust 1.95.0 toolchain used by CI.
  Do not assume syntax or features newer than the toolchain that passes the repository checks.
- Run the focused tests for the changed boundary and the validation commands required by
  `AGENTS.md` before reporting completion.

Rust let chains used by this repository are stable under its edition/toolchain baseline; do not
misclassify supported `if let ... && ...` syntax as an unstable feature.
