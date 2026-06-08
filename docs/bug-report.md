# Bug Report

This document records a directory-by-directory audit of usage pitfalls, bugs, and logic gaps. Each section is completed after inspecting one directory only.

## Progress

| Directory | Status | Completed at | Notes |
| --- | --- | --- | --- |
| `npm/` | Completed | 2026-06-08 | npm package runtime shim, platform mapping, package staging, and npm tests inspected. |

## `npm/`

### Scope

- Inspected files: `npm/package.json`, `npm/bin/akra.js`, `npm/lib/platform.js`, `npm/lib/runtime.js`, `npm/scripts/stage-npm-packages.mjs`, `npm/test/*.test.js`.
- Validation run: `npm test --prefix npm`.
- The audit did not inspect other top-level directories in this pass.

### Findings

#### NPM-001: signal exits from the native binary can be reported as success

- Severity: High
- Evidence: `npm/bin/akra.js:49` registers handlers for `SIGINT`, `SIGTERM`, and `SIGHUP`; `npm/bin/akra.js:53` then handles a child signal exit by calling `process.kill(process.pid, signal)`.
- Why this is a bug: because the parent process already has a listener for the same signal, the self-signal is caught by the forwarding handler instead of restoring Node's default signal termination behavior. A native binary that exits from `SIGTERM` can leave the wrapper alive until the event loop drains, and the wrapper can then exit with code `0`.
- Reproduction used during audit:

```bash
node --input-type=module - <<'NODE'
import { spawn } from "node:child_process";
const child = spawn(process.execPath, ["-e", "process.kill(process.pid, 'SIGTERM')"], { stdio: "ignore" });
const forwardSignal = (signal) => {
  if (child.killed) return;
  try { child.kill(signal); } catch {}
};
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.on(signal, () => forwardSignal(signal));
}
child.on("exit", (code, signal) => {
  console.log(JSON.stringify({ code, signal, killed: child.killed }));
  if (signal) {
    process.kill(process.pid, signal);
    setTimeout(() => console.log("still-alive"), 100);
    return;
  }
  process.exit(code ?? 1);
});
setTimeout(() => console.log("timeout"), 500);
NODE
echo exit=$?
```

- Observed output:

```text
{"code":null,"signal":"SIGTERM","killed":false}
still-alive
timeout
exit=0
```

- User impact: shell scripts, CI, service managers, and package-manager wrappers can treat an interrupted or terminated `akra` run as successful. This is most visible for cancellation, terminal shutdown, or supervisor-initiated termination.
- Suggested fix: before re-sending the signal to the parent, remove the installed signal listeners or translate the signal into the conventional exit code `128 + signalNumber`. Add a subprocess test that launches `npm/bin/akra.js` against a fixture binary that terminates via `SIGTERM`.

#### NPM-002: binary resolution failures surface as raw Node exceptions

- Severity: Medium
- Evidence: `npm/bin/akra.js:15` calls `resolveBinaryPath` at module top level. `npm/lib/runtime.js:47`, `npm/lib/runtime.js:69`, and `npm/lib/runtime.js:84` throw user-facing resolution errors, but the CLI wrapper does not catch them.
- Why this is a usage gap: unsupported platforms, missing optional dependencies, or damaged platform packages will print a Node stack trace instead of a compact `akra` diagnostic. The underlying message is useful, but the JS stack frames are implementation detail.
- User impact: install failures look like a JavaScript crash even though the actionable path is reinstalling the package or using a supported target.
- Suggested fix: wrap binary resolution in a small `main()` function, catch `Error`, print `akra: <message>`, and exit `1`. Keep stack traces behind a debug environment variable if needed.

#### NPM-003: supported platform policy is implicit and easy to misread

- Severity: Low
- Evidence: `npm/lib/platform.js:1` lists only three publish targets: Linux x64, macOS arm64, and Windows x64. `npm/lib/runtime.js:47` rejects unsupported platform/architecture pairs.
- Why this is a logic gap: the npm package metadata and README path do not make the platform support boundary as explicit as the runtime does. Common environments such as macOS x64 and Linux arm64 will fail only at execution time after installation.
- User impact: users can install the package successfully and only discover at first run that their platform is not supported.
- Suggested fix: document the supported platform matrix prominently in `npm/README.md` and the root README install section. If unsupported installs should fail earlier, consider platform-specific optional dependency guidance or a postinstall check that prints a concise warning without blocking supported package-manager behavior.

### Test Gaps

- `npm/test/runtime.test.js` covers binary path selection and unsupported platform mapping, but does not execute `npm/bin/akra.js` as a subprocess.
- No test asserts parent exit status when the native binary exits by signal.
- No test asserts the CLI text emitted for missing optional dependency, missing binary, or unsupported platform failures.
