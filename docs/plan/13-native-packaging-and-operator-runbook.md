# Native Packaging And Operator Runbook

This runbook describes the current bundle and operator handoff for the Rust client.

## Package Commands

Build the current host target:

```bash
cd /path/to/codex-exec-loop
./scripts/package_native_release.sh
```

Build a specific target:

```bash
cd /path/to/codex-exec-loop
./scripts/package_native_release.sh --target aarch64-apple-darwin
```

Notes:

- default build profile is `release`
- `--profile debug` is available for local validation bundles
- prefer native Windows packaging for Windows bundles instead of unvalidated cross-linking
- `SOURCE_DATE_EPOCH` fixes bundle/archive timestamps; when omitted, packaging uses the checked-out
  `HEAD` commit timestamp
- every target, including Windows, keeps the existing `.tar.gz` release format

## Output Layout

```text
dist/native/
  codex-exec-loop-native-<version>-<target>/
  codex-exec-loop-native-<version>-<target>.tar.gz
  codex-exec-loop-native-<version>-<target>.tar.gz.sha256
```

Bundle contents:

- native binary
- runtime app-server skill assets under `assets/app-server/skills/`
- `scripts/gh-akra.sh`
- `akra` launcher on macOS/Linux or `akra.cmd` on Windows
- `README.md`
- `OPERATOR.md`
- `THIRD_PARTY_NOTICES/Galmuri-OFL.txt` for the Korean admin font embedded in the binary
- `VERSION.txt`
- `examples/`, including `examples/README.md` and guarded English/Korean prompt templates
- `SHA256SUMS.txt`

The example prompts are never executed automatically. Start `akra` in the target repository,
review the matching language template, then paste it into the TUI. Repository instructions remain
authoritative, and the templates prohibit remote writes when no delivery workflow is defined.

The npm distribution uses a Codex-style split:

- main package `@refinedstone/akra`
- platform packages published as npm optional dependencies
- a tiny JavaScript launcher that resolves the installed native binary and executes it

The TUI itself still runs inside the Rust binary. JavaScript only acts as the npm entrypoint and platform selector.

## Integrity Verification

Verify both archive and unpacked bundle:

```bash
cd /path/to/codex-exec-loop
./scripts/verify_native_release.sh \
  --archive dist/native/codex-exec-loop-native-<version>-<target>.tar.gz \
  --bundle-dir dist/native/codex-exec-loop-native-<version>-<target> \
  --version <version> \
  --target <target> \
  --profile release
```

Verification requires Node.js 18 or newer; archive policy verification also requires `tar`.

The packaging flow emits:

- `SHA256SUMS.txt` for files inside the unpacked bundle
- `<archive>.tar.gz.sha256` for the tarball itself

Archives use sorted `ustar` members, normalized file modes and ownership, the fixed release epoch,
and a timestamp-free gzip header. Rebuilding the same target from the same source and binary must
therefore produce byte-identical archive and checksum files.

## Operator Prerequisites

- Official Codex CLI installed and on `PATH`
- Codex login already completed
- access to the target workspace
- normal access to `~/.codex/history.jsonl` and `~/.codex/sessions/`
- on Linux, either an official complete-package install with its adjacent `codex-resources/bwrap`
  helper or a compatible system `bwrap`; the simple one-binary platform tar is unsupported when no
  system helper is present because sandboxed commands fail even though version/startup probes pass
- on Unix, a private `$CODEX_HOME` (default `~/.codex`) whose existing auth and rollout files are not
  group/world accessible; Akra does not repair upstream file modes

Rust is not required on the operator machine after the bundle is built.

## Launch

If the unpacked bundle directory is on `PATH`, launch from any workspace with:

macOS or Linux:

```bash
cd /path/to/workspace
akra
```

Windows PowerShell:

```powershell
Set-Location C:\path\to\workspace
akra
```

You can still run the native binary directly if you prefer:

macOS or Linux:

```bash
cd /path/to/workspace
/path/to/codex-exec-loop-native
```

Windows PowerShell:

```powershell
Set-Location C:\path\to\workspace
C:\path\to\codex-exec-loop-native.exe
```

Useful env vars:

- `CODEX_EXEC_LOOP_GITHUB_PR=owner/repo#123`
- `CODEX_EXEC_LOOP_GITHUB_POLL_INTERVAL_SECS=60`
- `AKRA_GITHUB_LOGIN=<login>` or repo-local `git config akra.githubLogin <login>` is required for
  GitHub writes. `bash scripts/gh-akra.sh auth write-status` verifies the API token identity and
  repository target against that login
- `AKRA_GITHUB_TOKEN=<token>` for non-`gh` GitHub API fallback
- `AKRA_GITHUB_LEGACY_CREDENTIAL_SCAN` is unsupported and any presence fails closed. Discovery is
  limited to explicit token variables or trusted `gh auth token`; repository credential helpers and
  direct credential-file reads are not used by the GitHub API helper

## Smoke Checklist

1. On Linux, run `codex sandbox -C "$PWD" -P :read-only /usr/bin/true` and require exit zero before
   launching Akra; `codex --version` alone does not prove sandbox readiness.
2. Start the binary from a real workspace.
3. Confirm startup diagnostics pass.
4. Open recent sessions or start a new draft.
5. Send one prompt and confirm streaming output appears.
6. Open `:planning` once if planning is part of the workflow.
7. Open `:queue` once and confirm the compact queue summary appears.
8. If GitHub polling is expected, set `CODEX_EXEC_LOOP_GITHUB_PR` and confirm the shell shows an active GitHub state.

## Validation Handoff

After a platform-facing change, record the validation result instead of rewriting the matrix by hand:

```bash
bash scripts/capture_native_validation.sh \
  --frontend inline \
  --check-profile terminal-baseline \
  --terminal "iTerm2 3.5" \
  --result pass \
  --output-dir docs/validation
```

If the change touches Phase 1 operator-surface behavior such as status language, resume context, queue or automation explanation, or `akra` / `:` lifecycle commands, use:

```bash
bash scripts/capture_native_validation.sh \
  --frontend inline \
  --check-profile phase1-operator-surface \
  --terminal "iTerm2 3.5" \
  --result pass \
  --output-dir docs/validation
```

Coverage summary (informational; warns when required rows are incomplete):

```bash
bash scripts/summarize_native_validation.sh
```

Coverage gate:

```bash
bash scripts/summarize_native_validation.sh --fail-on-incomplete
```

Markdown summary:

```bash
bash scripts/summarize_native_validation.sh --format markdown
```

## Release Notes

- package from a clean checkout when possible
- attach the exact commit SHA used for the build
- use the checked-in Rust `1.95.0` toolchain for local, CI, and release builds
- keep the emitted `.sha256` file with the archive
- fix platform-specific terminal defects in focused follow-up branches instead of widening the packaging change

## GitHub Release Assets

The repository can publish native bundles directly to GitHub Release assets from a tag push.

- workflow: `.github/workflows/release-native-assets.yml`
- release tag convention: stable `vMAJOR.MINOR.PATCH` only
- the workflow parses `[package].version` with `tomllib`, requires the tag to resolve to
  `GITHUB_SHA`, and requires that commit to be contained in explicitly fetched `origin/prerelease`;
  it strips the leading `v` and requires the remaining version to match `Cargo.toml`;
  prerelease and build-metadata versions fail closed until an explicit npm dist-tag policy is added
- published assets:
  - Linux `x86_64-unknown-linux-gnu`
  - Windows `x86_64-pc-windows-msvc`
  - macOS `aarch64-apple-darwin`
- each asset upload includes the archive and matching `.sha256` file
- workflow actions are pinned to reviewed commit SHAs, and each publish job independently verifies
  the downloaded archive checksum, exact asset set, bounded expansion, safe member paths, and
  regular-file member types before npm staging or GitHub release upload
- every npm and GitHub Release mutation resolves lightweight or annotated `v*` tags through the
  GitHub API and requires the final commit to remain exactly `GITHUB_SHA` before and after the
  mutation. A missing, moved, cyclic, over-deep, or non-commit tag fails closed; the repository
  ruleset remains the server-side control that prevents the small check-to-mutation race entirely
- asset file names still use the package version declared in `Cargo.toml`
- create a protected GitHub Environment named `npm-release`, store `NPM_TOKEN` there rather than as
  an unprotected repository secret, require a reviewer, and restrict deployments to the intended
  stable tag policy. Use a read/write granular token limited to `@refinedstone/akra`; enable 2FA
  bypass only when the npm package publish policy requires it
- configure a repository ruleset that restricts creation, update, and deletion of `v*` tags to
  release maintainers. The checked-in ancestry validation cannot enforce repository/environment
  settings and cannot protect runs using a workflow from an older commit; those server-side rules
  are part of the fail-closed release contract
- the same tag publishes:
  - `@refinedstone/akra@<tag-version>`
  - optional dependency aliases:
    - `@refinedstone/akra-linux-x64`
    - `@refinedstone/akra-darwin-arm64`
    - `@refinedstone/akra-win32-x64`
  - published platform versions:
    - `@refinedstone/akra@<tag-version>-linux-x64`
    - `@refinedstone/akra@<tag-version>-darwin-arm64`
    - `@refinedstone/akra@<tag-version>-win32-x64`
- npm platform packages and the main package must finish before the matching GitHub Release is
  created or updated; a missing or invalid environment-scoped `NPM_TOKEN` fails before npm
  publication

npm publish notes:

- publish platform packages before the main `@refinedstone/akra` package
- publish every immutable version first with the non-semver `release-staging` dist-tag; never pass
  `latest` directly to `npm publish`
- after each publish or verified retry, read the registry's complete version set and converge
  `latest` to its highest stable version; concurrent older/newer tag workflows may finish in either
  order without moving `latest` backward
- converge `platform` independently to the highest published platform version; exact npm aliases in
  the main package remain authoritative for platform installation
- publish every npm package with `--provenance`; the npm job receives only read access to repository
  contents plus `id-token: write` for the GitHub Actions OIDC attestation
- expose `NODE_AUTH_TOKEN` only to authentication and publish steps, never to checkout, package
  staging, artifact download, or native build steps
- pin npm authentication, version lookup, and publication to `https://registry.npmjs.org`; do not
  honor runner `NPM_CONFIG_REGISTRY` or user `.npmrc` registry drift for an official release
- npm versions are immutable, so a corrected republish needs a new tag version
- create one tarball with `npm pack --json`, publish that exact tarball, and skip an existing npm
  version only when its registry `dist.integrity` matches the local pack integrity
- disable package lifecycle scripts during release packing and publication so the publish credential
  is not exposed to package-controlled hooks
- when a GitHub Release already exists, require exact tag/title/stable metadata, reject every
  unexpected or duplicate asset name, download expected same-named assets and compare them
  byte-for-byte, upload only missing assets, and recheck the exact final asset set
- if a run fails after publishing some npm packages, rerun the same tag workflow; existing versions
  are verified, a newer `latest` is preserved, and the GitHub Release remains gated on completion
  of the whole npm job
- npm intentionally completes before the public GitHub Release. A later release-metadata or asset
  failure can therefore leave npm published alone; repair the conflicting GitHub Release and rerun
  the same tag. The immutable npm integrity check makes that recovery idempotent
- the npm platform package ships the native binary plus runtime app-server skill assets and `scripts/` under `vendor/<target>/akra/`; admin visual assets are embedded in the binary and ignored build output such as `node_modules/`, `.vite/`, and `dist/` must not be published

The Rust compiler is pinned, and archive metadata is deterministic, but GitHub-hosted runner images,
system linkers, and platform SDKs are not immutable. A long-delayed rebuild of the same tag can still
produce different native binary bytes after a runner image update. The release publisher will
intentionally reject those bytes instead of overwriting an existing asset; recover and reuse the
original artifact or issue a new release version.

Typical release flow:

```bash
git tag v0.1.0
git push origin v0.1.0
```

After the workflow finishes, download the archives from the GitHub `Releases` page for that tag.
