import { spawn } from "node:child_process";
import { win32 } from "node:path";

function windowsTaskkillPath() {
  const systemRoot = process.env.SystemRoot ?? process.env.WINDIR;
  if (!systemRoot || !win32.isAbsolute(systemRoot)) {
    throw new Error("Windows process cleanup requires an absolute SystemRoot.");
  }
  return win32.join(systemRoot, "System32", "taskkill.exe");
}

// Keep live diagnostics in the job log even when a contract never returns.
export function runStreamingCommand(command, args, {
  cwd, env = process.env, write = (chunk) => process.stdout.write(chunk),
  writeError = (chunk) => process.stderr.write(chunk), signal,
  terminationGraceMs = 5000, drainTimeoutMs = 1000, spawnProcess = spawn,
} = {}) {
  return new Promise((resolve, reject) => {
    for (const duration of [terminationGraceMs, drainTimeoutMs]) {
      if (!Number.isSafeInteger(duration) || duration < 1 || duration > 60000) {
        throw new Error("Process cleanup waits must be finite positive milliseconds, at most 60000.");
      }
    }
    if (signal?.aborted) {
      resolve({status:null, signal:"SIGTERM", aborted:true, stdout:""});
      return;
    }
    // A POSIX group belongs only to this invocation. Do not signal the CI shell's group.
    const child = spawnProcess(command, args, {
      cwd, env, stdio:["ignore", "pipe", "pipe"], windowsHide:true,
      detached:process.platform !== "win32",
    });
    let tail = "";
    let aborted = false;
    let terminationSignal;
    let escalation;
    let cancellationDeadline;
    let helperDeadline;
    let treeTerminator;
    let treeCleanupDone = false;
    let closeResult;
    let settled = false;
    const finish = (error, result) => {
      if (settled) return;
      settled = true;
      clearTimeout(escalation);
      clearTimeout(cancellationDeadline);
      clearTimeout(helperDeadline);
      signal?.removeEventListener("abort", abort);
      process.removeListener("SIGINT", interrupt);
      process.removeListener("SIGTERM", terminate);
      if (treeTerminator && treeTerminator.exitCode === null && treeTerminator.signalCode === null) {
        treeTerminator.kill("SIGKILL");
      }
      if (error || aborted) {
        // An escaped/inherited writer cannot keep the cancelled supervisor alive forever.
        child.stdout.destroy();
        child.stderr.destroy();
      }
      if (error) reject(error); else resolve(result);
    };
    const cleanupFailed = (detail) => {
      if (settled) return;
      if (child.exitCode === null && child.signalCode === null) {
        try { child.kill("SIGKILL"); } catch { /* The failure is reported below. */ }
      }
      finish(new Error("Process tree cleanup failed: " + detail));
    };
    const maybeFinish = () => {
      if (!closeResult || (aborted && !treeCleanupDone)) return;
      finish(null, {
        ...closeResult, signal:terminationSignal ?? closeResult.signal, aborted, stdout:tail,
      });
    };
    const signalOwnedGroup = (name) => {
      if (!child.pid) throw new Error("The owned process group has no leader PID.");
      try { process.kill(-child.pid, name); }
      catch (error) { if (error.code !== "ESRCH") throw error; }
    };
    const terminateWindowsTree = () => {
      // taskkill must enumerate descendants while the directly owned leader is still live.
      // After exit its numeric PID is not enough evidence to target a process tree.
      if (child.exitCode !== null || child.signalCode !== null) {
        cleanupFailed("cannot prove Windows descendant cleanup after the direct child exited");
        return;
      }
      let executable;
      try { executable = windowsTaskkillPath(); }
      catch (error) { cleanupFailed(error.message); return; }
      try {
        treeTerminator = spawn(executable, ["/PID", String(child.pid), "/T", "/F"], {
          env:process.env, windowsHide:true, stdio:["ignore", "ignore", "pipe"],
        });
      } catch (error) { cleanupFailed(error.message); return; }
      let diagnostics = "";
      treeTerminator.stderr.setEncoding("utf8");
      treeTerminator.stderr.on("data", (chunk) => { diagnostics = (diagnostics + chunk).slice(-4096); });
      treeTerminator.on("error", (error) => cleanupFailed(error.message));
      treeTerminator.on("close", (status, helperSignal) => {
        clearTimeout(helperDeadline);
        if (settled) return;
        if (status !== 0) {
          cleanupFailed("taskkill exited " + (helperSignal ?? status) + ": " + diagnostics.trim());
          return;
        }
        treeCleanupDone = true;
        maybeFinish();
      });
      helperDeadline = setTimeout(() => cleanupFailed("taskkill did not finish before its deadline"), terminationGraceMs);
    };
    const stop = (name) => {
      if (settled || aborted) return;
      aborted = true;
      terminationSignal = name;
      // Keep this timer referenced: a pending Promise alone must not permit a zero-code exit.
      cancellationDeadline = setTimeout(() => {
        cleanupFailed("child output pipes did not close before the cancellation deadline");
      }, terminationGraceMs + drainTimeoutMs);
      if (process.platform === "win32") {
        terminateWindowsTree();
        return;
      }
      try {
        signalOwnedGroup(name);
        if (child.exitCode !== null || child.signalCode !== null) signalOwnedGroup("SIGKILL");
        treeCleanupDone = true;
        escalation = setTimeout(() => {
          try { signalOwnedGroup("SIGKILL"); }
          catch (error) { cleanupFailed(error.message); }
        }, terminationGraceMs);
        maybeFinish();
      } catch (error) { cleanupFailed(error.message); }
    };
    const abort = () => stop("SIGTERM");
    const interrupt = () => stop("SIGINT");
    const terminate = () => stop("SIGTERM");
    signal?.addEventListener("abort", abort, {once:true});
    process.on("SIGINT", interrupt);
    process.on("SIGTERM", terminate);
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      // Only the final libtest summary is needed after streaming. Bound memory use.
      tail = (tail + chunk).slice(-1024 * 1024);
      write(chunk);
    });
    child.stderr.on("data", writeError);
    child.on("error", (error) => finish(error));
    child.on("exit", () => {
      if (!aborted || process.platform === "win32" || settled) return;
      // The leader can exit before a descendant that ignores SIGTERM and does not hold a pipe.
      try { signalOwnedGroup("SIGKILL"); }
      catch (error) { cleanupFailed(error.message); }
    });
    child.on("close", (status, childSignal) => {
      closeResult = {status, signal:childSignal};
      maybeFinish();
    });
    if (signal?.aborted) abort();
  });
}
