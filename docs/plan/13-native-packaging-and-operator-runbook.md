# Native Packaging and Release

[한국어](../ko/reference/release.md)

This is the current operator, bundle, and publication contract for the Rust client.

## Build a Bundle

```bash
./scripts/package_native_release.sh
./scripts/package_native_release.sh --target aarch64-apple-darwin
./scripts/package_native_release.sh --profile debug
```

The default profile is `release`. Prefer native Windows packaging over unvalidated cross-linking.
`SOURCE_DATE_EPOCH` fixes timestamps; otherwise packaging uses the checked-out `HEAD` timestamp.
Every target uses the existing `.tar.gz` release format.

```text
dist/native/
  codex-exec-loop-native-<version>-<target>/
  codex-exec-loop-native-<version>-<target>.tar.gz
  codex-exec-loop-native-<version>-<target>.tar.gz.sha256
```

The bundle contains the native binary, launcher, runtime app-server skills, `scripts/gh-akra.sh`,
`README.md`, this document as `OPERATOR.md`, font notice, `VERSION.txt`, guarded example prompts,
and `SHA256SUMS.txt`. Example prompts are never executed automatically.

The npm distribution uses `@refinedstone/akra` plus platform-specific optional dependencies. The
JavaScript launcher only selects and executes the Rust binary.

## Verify Integrity

```bash
./scripts/verify_native_release.sh \
  --archive dist/native/codex-exec-loop-native-<version>-<target>.tar.gz \
  --bundle-dir dist/native/codex-exec-loop-native-<version>-<target> \
  --version <version> \
  --target <target> \
  --profile release
```

Verification requires Node.js 18+; archive-policy verification also requires `tar`. The bundle
manifest hashes unpacked files, and the adjacent `.sha256` hashes the archive. Sorted `ustar`
members, normalized metadata, a fixed epoch, and a timestamp-free gzip header make identical
source/binary inputs reproducible.

## Operator Prerequisites and Launch

- official Codex CLI on a trusted `PATH`, with `codex login` complete
- target workspace access and normal Codex session/history access
- Linux sandbox readiness: official adjacent `codex-resources/bwrap` or compatible system `bwrap`
- a private `$CODEX_HOME` on Unix; Akra does not repair upstream auth/rollout file modes

Rust is not required after the bundle is built.

```bash
cd /path/to/workspace
akra
```

On Windows PowerShell, change to the workspace and run `akra`; the bundle launcher is `akra.cmd`.
The native binary may also be run directly.

Useful configuration:

- `CODEX_EXEC_LOOP_GITHUB_PR=owner/repo#123`
- `CODEX_EXEC_LOOP_GITHUB_POLL_INTERVAL_SECS=60`
- `AKRA_GITHUB_LOGIN=<login>` or repo-local `akra.githubLogin`
- `AKRA_GITHUB_TOKEN=<token>` only when a trusted `gh auth token` is unavailable

Before GitHub writes, run `bash scripts/gh-akra.sh auth write-status`. The removed legacy credential
scan is unsupported; the GitHub API helper does not read repository credential helpers or credential
files directly.

## Smoke Check

1. On Linux, require `codex sandbox -C "$PWD" -P :read-only /usr/bin/true` to pass.
2. Start Akra from a real workspace and confirm startup diagnostics.
3. Open a prior session or a new draft and stream one prompt.
4. Open `:planning` and `:queue` when planning is in scope.
5. If GitHub polling is configured, confirm the shell shows the expected review state.
6. Exit and confirm terminal restoration.

Platform-facing changes record evidence with the helpers described in
[Platform Validation Matrix](12-platform-validation-matrix.md).

## Publication Contract

The tag workflow is `.github/workflows/release-native-assets.yml`.

- Only stable `vMAJOR.MINOR.PATCH` tags are accepted.
- The resolved tag commit must equal `GITHUB_SHA`, be contained in fetched `origin/prerelease`, and
  match the version in `Cargo.toml`.
- Published targets are Linux x64, Windows x64, and macOS Apple Silicon.
- Every upload includes the archive and its checksum; jobs verify exact assets, bounded safe
  extraction, regular member types, and archive identity before mutation.
- A protected `npm-release` environment supplies `NPM_TOKEN`, requires the intended reviewer/tag
  policy, and grants only the minimum registry scope. Repository rules must restrict `v*` tag
  creation, movement, and deletion.
- Platform packages publish before the main package. Immutable versions first use the
  `release-staging` dist-tag; the workflow then converges `latest` and `platform` forward without
  allowing an older concurrent run to move them backward.
- Publication uses one verified `npm pack --json` tarball per package, `--provenance`, pinned npm
  registry routing, and no lifecycle scripts. Token exposure is limited to auth/publish steps.
- Rerunning the same tag verifies already-published versions and same-named release assets
  byte-for-byte before completing missing mutations.
- GitHub Release creation waits until all npm packages succeed. A failure after partial npm
  publication leaves no public GitHub Release; rerun the unchanged tag after fixing the external
  cause.
- Release publication is not byte-atomic across npm and GitHub. The workflow is instead
  identity-pinned, immutable, retryable, and convergent.

Package from a clean checkout with the pinned Rust toolchain, preserve the exact source SHA and
archive checksum, and download published assets once for final verification.
