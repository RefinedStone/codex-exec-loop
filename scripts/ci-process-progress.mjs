import { spawn } from "node:child_process";

// Keep live diagnostics in the job log even when a contract never returns.
export function runStreamingCommand(command, args, {
  cwd, env = process.env, write = (chunk) => process.stdout.write(chunk),
  writeError = (chunk) => process.stderr.write(chunk), signal,
} = {}) {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) {
      resolve({status:null,signal:"SIGTERM",aborted:true,stdout:""});
      return;
    }
    const child = spawn(command, args, {cwd, env, stdio:["ignore","pipe","pipe"], windowsHide:true});
    let tail = "";
    let aborted = false;
    let terminationSignal;
    let escalation;
    let settled = false;
    const finish = (error, result) => {
      if (settled) return;
      settled = true;
      clearTimeout(escalation);
      signal?.removeEventListener("abort", abort);
      process.removeListener("SIGINT", interrupt);
      process.removeListener("SIGTERM", terminate);
      if (error) reject(error); else resolve(result);
    };
    const stop = (name) => {
      if (settled || aborted) return;
      aborted = true;
      terminationSignal = name;
      child.kill(name);
      escalation = setTimeout(() => child.kill("SIGKILL"), 5000);
      escalation.unref();
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
    child.on("close", (status, childSignal) => finish(null, {
      status, signal:terminationSignal ?? childSignal, aborted, stdout:tail,
    }));
    if (signal?.aborted) abort();
  });
}
