# Native release smoke (#2589)

Baseline: prerelease 1d538c2fe. Run the smoke test on the target's native OS/architecture, before uploading the verified archive. It checks the archive checksum/manifest and required nonempty license notices, then invokes only the extracted launcher from a separate working directory using a temporary home and allowlisted environment. The config path, write and independent read must succeed. Temporary files are removed; no release publication or real profile access occurs.

Windows x64, 2026-11-28:
- Built a real 1.3.5 debug archive with package_native_release.sh --profile debug.
- Verified and executed the extracted launcher with smoke-native-release.mjs (PASS).
- The stage-packages and smoke unit tests passed (37/37), using Windows System32 tar. Git Bash GNU tar interprets drive-letter archive paths as remote paths.
- The wider npm suite returned 49 pass, 16 fail, 4 skip. The failures were in publish-package.test.js, where the Windows child-process npm fixture could not start (ENOENT). The publish fixture and script are unchanged.
- Release-profile CI matrix execution, npm installation and an upgrade from a previously released binary remain separate verification. This debug smoke is not evidence of those outcomes.
