# Native release smoke (#2589)

Baseline: prerelease 1d538c2fe. Run the smoke test on the target's native OS/architecture, before uploading the verified archive. It checks the archive checksum/manifest and required nonempty license notices, then invokes only the extracted launcher from a separate working directory using a temporary home and allowlisted environment. The config path, write and independent read must succeed. Temporary files are removed; no release publication or real profile access occurs.

Windows x64, 2026-11-28:
- Built a real 1.3.5 debug archive with package_native_release.sh --profile debug.
- Verified and executed the extracted launcher with smoke-native-release.mjs (PASS).
- The stage-packages and smoke unit tests passed (37/37), using Windows System32 tar. Git Bash GNU tar interprets drive-letter archive paths as remote paths.
- The wider npm suite returned 49 pass, 16 fail, 4 skip. The failures were in publish-package.test.js, where the Windows child-process npm fixture could not start (ENOENT). The publish fixture and script are unchanged.
- Release-profile CI matrix execution, npm installation and an upgrade from a previously released binary remain separate verification. This debug smoke is not evidence of those outcomes.


## Upgrade smoke contract (#2589)

Run with two independently verified archives for the same native target:
`node npm/scripts/smoke-native-release.mjs <candidate-assets> <candidate-version> <target> release --previous <previous-assets> <previous-version>`.
The previous archive must be a different release version. The old launcher writes a non-default global setting into a disposable profile and reads it back. The candidate must see it **before** writing, then persist another value. The fresh-install smoke likewise checks an unset setting, a non-default write, and the actual configuration file. This is a local isolated binary-profile compatibility check, not an installer or npm migration test. No real old/new pair has been run by this change; the new regression tests use fixture processes.

## Published legacy input

The config scenario above applies to archives with the current `akra-ade` contract.
It does not establish migration from the actual published `v1.3.5` binary, which
has no `config` command. That limitation of the smoke scenario does not mean the
old executable cannot run.

Use the separate historical-input mode:

```text
node npm/scripts/smoke-native-release.mjs <candidate-assets> <candidate-version> <target> <release|debug> --previous-legacy <legacy-assets> 1.3.5
```

This mode currently accepts only the genuine Windows x64 release archive
`codex-exec-loop-native-1.3.5-x86_64-pc-windows-msvc.tar.gz`, pinned to SHA-256
`496097730bc8b7cb37d6d79759456bc68bf47d5995eaad8e029a1d741ca01eaa`.
The digest matches GitHub asset `452661989` and its downloaded sidecar. It
identifies immutable historical input, not an archive eligible for publication.
The current npm staging/release verifier still requires current filenames,
metadata and all license notices. Historical missing notices are not grandfathered
into new releases.

The historical reader retains snapshot identity, checksum coverage, file/link,
path, byte, compression, output and timeout bounds. It recognizes bounded GNU
long-name records only for this pinned historical format. Current archives still
require the existing ustar-only contract.

The old launcher initializes a disposable workspace with `doctor`, then creates
one synthetic task through `planning-tool run`. It reads the committed task back.
The candidate must read the same task ID, title and description before any
candidate write. It never invokes an execution/model command or edits authority
files directly. An empty candidate store is a failure, not upgrade success.

### Genuine Windows pair, 2026-11-27 UTC / 2026-11-28 KST

Candidate source: `865e0ade186221b24d02a4f54ab58d53f78b693d`, Rust 1.95.0,
`x86_64-pc-windows-msvc`, **debug package**. This is not a release-profile,
installer, npm-install or credential migration result.

| Artifact | SHA-256 |
| --- | --- |
| Published legacy archive | `496097730bc8b7cb37d6d79759456bc68bf47d5995eaad8e029a1d741ca01eaa` |
| Candidate archive | `3dd0a7c2825f0861ce19887702d4b2558d5a63ca74e7aa739802d78802f135aa` |
| Candidate native executable | `f81058c74399941f8f5750e82b7a062626b755d87688e722fb603734f44f34d9` |

The candidate fresh-install archive smoke passed. The historical archive passed
its checksum/manifest checks and the old binary created and read a task. The
candidate then failed to read that authority. A direct public-CLI diagnostic
reported `unsupported authority-store schema version: 5`. Reopening with the old
binary still returned the original task.

A separate disposable Git workspace showed a second boundary: the new repository
incarnation uses a new namespace when the historical namespace is occupied.
`workspace_paths.rs::select_repository_project_namespace` deliberately leaves
ambiguous pre-marker data untouched. It does not prove that the old task migrated.

**N2/R1 acceptance remains blocked.** The smoke assumption is corrected; the
genuine pair exposes the remaining schema-5 migration and pre-incarnation
recovery requirements under #2588. Do not rename the old archive, replace its
metadata, relax current license checks, lower the schema floor, or adopt an
ambiguous namespace to turn this result green.
