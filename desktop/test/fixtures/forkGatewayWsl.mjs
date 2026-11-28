// Runs in WSL. Only the disposable Linux AF_UNIX endpoint is reachable from the relay.
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { once } from "node:events";
import { chmodSync, copyFileSync, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { createConnection } from "node:net";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

const [mode, root, binary] = process.argv.slice(2);
assert.match(root, /^\/dev\/shm\/akra-fork-[a-f0-9-]{36}$/);
switch (mode) {
  case "relay": {
    const socket = createConnection(join(root, "akra/run/gateway.sock"));
    socket.on("error", error => { console.error(error.message); process.exitCode = 1; process.stdin.destroy(); });
    socket.on("close", () => process.stdin.destroy());
    process.stdin.pipe(socket).pipe(process.stdout);
    break;
  }
  case "release-delete":
  case "release-reply":
    writeFileSync(join(root, mode), "", { flag: "wx" });
    break;
  case "requests":
    process.stdout.write(readFileSync(join(root, "requests.jsonl")));
    break;
  case "serve": {
    mkdirSync(root, { mode: 0o700 });
    let child;
    try {
      for (const dir of ["home", "codex", "bin", "repo", "tmp"]) mkdirSync(join(root, dir), { mode: 0o700 });
      const env = {
        PATH: `${root}/bin:/usr/bin:/bin`, HOME: join(root, "home"), TMPDIR: join(root, "tmp"),
        XDG_CONFIG_HOME: join(root, "home/config"), XDG_DATA_HOME: join(root, "home/data"),
        XDG_CACHE_HOME: join(root, "home/cache"), CODEX_HOME: join(root, "codex"),
        AKRA_HOME: join(root, "akra"),
        GIT_MASTER: "1", GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: "/dev/null",
      };
      const repo = join(root, "repo");
      const git = (...args) => execFileSync("/usr/bin/git", args, { cwd: repo, env, stdio: "pipe" });
      git("init", "-b", "main");
      git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.test", "commit", "--allow-empty", "-m", "base");
      git("worktree", "add", "-b", "first", join(root, "first"));
      git("worktree", "add", "-b", "second", join(root, "second"));
      const recorded = readFileSync(new URL("../fake-gateway/fixtures/thread-start.jsonl", import.meta.url), "utf8").split(/\r?\n/);
      const template = JSON.parse(JSON.parse(recorded[1]).text).result;
      const thread = (suffix, name, worktree, parent = null) => {
        const id = `01a1abfb-8f0a-7581-a8e4-c2953347f47${suffix}`;
        const cwd = join(root, worktree);
        return { ...template.thread, id, sessionId: id, cwd, name, forkedFromId: parent, turns: [],
          environments: [{ environmentId: "local", cwd, runtimeWorkspaceRoots: [cwd] }],
          gitInfo: { branch: worktree, sha: null, originUrl: null } };
      };
      const source = thread("1", "Fork fixture source", "first");
      const deleted = thread("2", "Deleted fork fixture child", "first", source.id);
      const wrong = thread("3", "Wrong worktree fixture child", "second", source.id);
      writeFileSync(join(root, "threads.json"), JSON.stringify({ template, source, deleted, wrong }));
      const fixture = fileURLToPath(new URL("./forkGatewayUpstream.mjs", import.meta.url));
      const executable = join(root, "bin/codex");
      writeFileSync(executable, `#!/usr/bin/node\nvoid import("./peer.mjs");\n`);
      copyFileSync(fixture, join(root, "bin/peer.mjs"));
      mkdirSync(join(root, "bin/node_modules"));
      symlinkSync(fileURLToPath(new URL("../../node_modules/ws", import.meta.url)), join(root, "bin/node_modules/ws"));
      chmodSync(executable, 0o700);
      child = spawn(binary, [], { cwd: repo, env, stdio: ["ignore", "pipe", "inherit"] });
      createInterface({ input: child.stdout }).on("line", line => console.log(line));
      const exited = once(child, "exit");
      process.stdin.resume();
      const stop = () => child.kill("SIGTERM");
      process.stdin.once("end", stop);
      process.once("SIGTERM", stop);
      const [code, signal] = await exited;
      process.stdin.pause();
      assert.equal(code, 0, `Gateway exit ${code}/${signal}`);
    } finally {
      if (child?.exitCode === null && child.signalCode === null) child.kill("SIGTERM");
      rmSync(root, { recursive: true, force: true });
    }
    break;
  }
  default:
    assert.fail(`Unknown fixture mode: ${mode}`);
}
