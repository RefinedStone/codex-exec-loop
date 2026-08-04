# Long-session scroll troubleshooting retrospective

Open [`index.html`](./index.html) for the Korean developer achievement report covering the
2026-08-05 fullscreen transcript performance correction.

The report is intentionally self-contained and uses no network assets. Its measurements link to
the checked-in [`long-session-scroll-performance-2026-08-05`](../../artifacts/long-session-scroll-performance-2026-08-05/README.md)
artifacts and merged implementation in [PR #2077](https://github.com/RefinedStone/codex-exec-loop/pull/2077).

The report distinguishes renderer-isolated `TestBackend` numbers from future real-PTY
input-to-present validation, and records prioritized hardening work rather than presenting the
optimization as the end of performance work.
