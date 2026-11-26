# Desktop Git fixture timeouts on Windows (#2869)

Date: 2026-11-26. Machine: Windows 11, 24 cores, Git for Windows, no other work running. No model turns.

## Failing tests and conditions

`npx vitest run` in `desktop/` with the default worker count (cores - 1 = 23) failed 6 tests with
`Test timed out in 5000ms.`:

- `gitBranch.test.ts`: "lists the commits after the default branch and marks the pushed ones",
  "asks gh for the PR of the remote branch and compares against the PR base"
- `gitHunks.test.ts`: "preserves other hunks and pre-staged changes when stage runs"
- `gitPublish.test.ts`: "pushes the current branch to the branch of the same name and sets the upstream"
- `gitStatus.test.ts`: "stages, unstages and discards one file and leaves the other files alone",
  "discards a staged new file and a staged rename"

The same four files alone (4 workers) passed, with the slowest test at 4.4 s. Earlier reports
(PR #2643, #2726, #2780 and others) saw 3 to 13 failures per run, different tests each time, and
`EPERM` when a timed-out test's cleanup removed a folder that a git process still held.

## Cause

- One of these tests starts about 50 git processes. "stages, unstages and discards one file" alone:
  49 processes (46 from the product's `runGit`), 3.5 s of git time.
- On this machine one `git` process takes about 30 ms when started alone and about 150 ms when 23
  are started at the same time (measured with `child_process.spawn("git", ["--version"])`).
- With 23 workers, the Git tests ran next to the rest of the suite, so 50 processes took more than
  5 s. Global `core.fsmonitor` and LFS filters were not the cause: the product passes
  `core.fsmonitor=false` and empty LFS filters, and the fixtures have no `filter` attributes.

## Fix

- `desktop/vitest.config.mts` puts the five real-git files in their own project `real-git`. It
  runs after the other tests (`sequence.groupOrder: 1`) with 2 workers, so its git processes do not
  compete with 23 workers. The project gets a 15 s limit as a margin for loaded machines and CI;
  the other tests keep 5 s.
- The fixtures' folder cleanup retries (`maxRetries: 5`), so a held handle does not fail a later
  test.

## Result

| Run | Failed | Slowest real-git test | Duration |
| --- | --- | --- | --- |
| before (one run) | 6 | over 5 s | 54.0 s |
| after 1 | 0 | 4.0 s | 46.3 s |
| after 2 | 0 | 3.1 s | 46.2 s |
| after 3 | 0 | 4.5 s | 46.4 s |
| after 4 | 0 | 4.8 s | 50.1 s |
| after 5 | 0 | 3.8 s | 47.1 s |

## Limits

- Measured on one Windows machine. CI (Linux) has no record of this failure.
- The product still starts about 50 git processes for one file action (repeated `rev-parse`,
  `status`, `ls-files` checks). Reducing them is a separate product change.
