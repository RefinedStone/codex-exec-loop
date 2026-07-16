# Coding-Agent TUI Architecture Field Report

[Open the interactive report](index.html).

This cross-product report compares the TUI authority, projection, rendering, interaction,
extension, and validation boundaries of GJC/Gajae-Code, OpenCode, OpenAI Codex, Claude Code, Pi,
and jcode. The report is written in Korean and is intended as an architecture decision input for
Akra, not as a feature scorecard.

## Snapshot

| Product | Pinned release | Pinned commit or artifact | Evidence limit |
| --- | --- | --- | --- |
| OpenCode | v1.18.2 | `70b56a0a93d366889cae950379cc9d2537148fa2` | source, docs, and official issues inspected |
| GJC / Gajae-Code | v0.11.0 beta | `8132409c3f10754fea5f3b0108a7bee979c43652` | source and docs inspected; runtime not reproduced |
| OpenAI Codex | rust-v0.144.4 | `8c68d4c87dc54d38861f5114e920c3de2efa5876` | source and tests inspected; TUI performance not benchmarked |
| Claude Code | v2.1.210 | `b7784f2c63ed4585c32bc20b94d3b64cf4fe6df3` and npm artifacts | product source and tests are not public |
| Pi | v0.80.7 | `818d67457cdd6b60bce6b121d16b23141c252dd8` | source and docs inspected; runtime not reproduced |
| jcode | v0.47.0 | `f7f5898cf6614051dd791655b7a87544210c1bd1` | source and docs inspected; performance claims not reproduced |
| Akra baseline | prerelease | `1607f2cf3e43025c2371d5de9b1bbf1437b3fc78` | local source and architecture ratchets inspected |

The user's uncertain product name `py` is interpreted as **Pi**, the coding-agent project now
maintained at `earendil-works/pi`. That interpretation is visible in the report rather than treated
as a hidden assumption.

## Claude Artifact Inspection

The Claude Code implementation is closed-source, so the report does not invent its reducer or
store topology. The official Linux artifact was inspected as a release artifact:

```text
@anthropic-ai/claude-code@2.1.210
  tarball sha256  a218ec0f337d14532df3ab8366f4964dc99b2e74532258e0bcc8a05d0932cfee

@anthropic-ai/claude-code-linux-x64@2.1.210
  tarball sha256  136c0c38fbde7076848f776bdfe167a51f5dce1890c2fde79c73c3bf77b2299d
  binary sha256   e7d2ceb53ed4c2ced1fe7fc1c6331c98dc5f7b4c9b2722d9c5fa3dd5dff6f719
```

The binary contains Bun runtime markers, React reconciler `19.2.0`, Ink-style node names such as
`ink-virtual-text`, Yoga layout nodes, and alternate-screen entry/exit strings. Official fullscreen
documentation separately describes classic/fullscreen renderer behavior. The artifact observations
do not prove that those framework markers are on the current active renderer path; detailed
application-state ownership also remains `unverified`.

## Akra Measurement

The report's Akra metrics include test code and were collected at the pinned baseline:

```bash
find src/adapter/inbound/tui -type f -name '*.rs' | wc -l
find src/adapter/inbound/tui -type f -name '*.rs' -print0 | xargs -0 wc -l
find src/core -type f -name '*.rs' -print0 | xargs -0 wc -l
```

Results: 184 TUI Rust files / 67,740 lines and 18 core Rust files / 6,119 lines. The eight
raw-application-service debt entries are explicit `TUI_RAW_APPLICATION_SERVICE_DEBTS` ratchets in
`tests/architecture_boundaries.rs`, not a subjective count.

## Report Controls

- product tabs switch the architecture authority flow without changing the comparison axes
- renderer search and category filters expose implementation similarities
- the official-asset gallery keeps visual presentation separate from correctness evidence
- the undo lab compares OpenCode's observed path with an Akra authority-ack proposal
- the queue simulator exercises success, stale-revision, and failure responses
- debt filters preserve the distinction between verified source debt and unconfirmed official issues
- the evidence ledger pins the source and artifact assertions used by the synthesis; architecture
  decisions and inferences remain explicitly analytical rather than independently verified

## Validation

Serve this directory over HTTP before browser validation because the report loads local CSS,
JavaScript, and image assets:

```bash
python3 -m http.server 4173 --directory docs/competitive/reports/tui-architecture
```

The report must remain usable at desktop and mobile widths, with keyboard-visible focus,
reduced-motion support, no horizontal page overflow, and functional success/stale/failure undo
simulations.
