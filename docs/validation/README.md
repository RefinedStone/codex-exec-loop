# Validation Records

This directory stores real terminal-validation results for the matrix in [`../plan/12-platform-validation-matrix.md`](../plan/12-platform-validation-matrix.md).

## Rules

- record only real passes or blockers
- keep one file per exercised terminal/frontend row
- preserve the emitted checklist so rows stay comparable
- use `./scripts/summarize_native_validation.sh` before calling the matrix complete

## Filename Shape

```text
YYYY-MM-DD-<os>-<terminal>-<shell>-<frontend>.txt
```

## Helper Usage

```bash
./scripts/capture_native_validation.sh \
  --frontend inline \
  --terminal "iTerm2 3.5" \
  --result pass \
  --output-dir docs/validation
```

Windows PowerShell:

```powershell
.\scripts\capture_native_validation.ps1 `
  -Frontend inline `
  -Terminal "Windows Terminal 1.22" `
  -Result pass `
  -OutputDir docs\validation
```

Coverage summary:

```bash
./scripts/summarize_native_validation.sh
```

Markdown summary:

```bash
./scripts/summarize_native_validation.sh --format markdown
```
