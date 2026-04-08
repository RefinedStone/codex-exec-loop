# Validation Records

This directory is the canonical checked-in location for real native terminal validation results.

Use it for actual macOS and Windows runs from the matrix in [`../plan/12-platform-validation-matrix.md`](../plan/12-platform-validation-matrix.md).

Rules:

- record only real validation passes or blockers; do not add speculative or placeholder rows
- keep one file per terminal/frontend row that was exercised
- keep the emitted checklist intact so later readers can compare rows quickly
- use `./scripts/summarize_native_validation.sh` to see which required rows are still missing before calling the matrix complete
- if a row finds a real Windows-specific issue, open the focused `F2` follow-up from the latest `origin/prerelease` instead of broadening the validation commit
- when the bash helper runs inside WSL, it should record the Windows host OS plus the distro name and prefer `TERMINAL_EMULATOR` for IDE terminals that do not expose `WT_SESSION`

Recommended filename shape:

```text
YYYY-MM-DD-<os>-<terminal>-<shell>-<frontend>.txt
```

Recommended helper usage from the repository root:

```bash
./scripts/capture_native_validation.sh \
  --frontend inline \
  --terminal "iTerm2 3.5" \
  --result pass \
  --output-dir native/docs/validation
```

Windows PowerShell:

```powershell
.\scripts\capture_native_validation.ps1 `
  -Frontend inline `
  -Terminal "Windows Terminal 1.22" `
  -Result pass `
  -OutputDir native\docs\validation
```

Coverage summary from the repository root:

```bash
./scripts/summarize_native_validation.sh
```

Markdown summary for a PR or issue comment:

```bash
./scripts/summarize_native_validation.sh --format markdown
```
