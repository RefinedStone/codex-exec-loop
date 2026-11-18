import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { build } from "vite";

const require = createRequire(import.meta.url);
await build({
  configFile: false,
  build: {
    outDir: "out/browser-qa",
    lib: { entry: "src/main/browserAgent/automation.ts", formats: ["cjs"], fileName: () => "automation.cjs" },
    rollupOptions: { external: ["electron", "node:events"] },
  },
});
const env = { ...process.env };
delete env.ELECTRON_RUN_AS_NODE;
const child = spawn(require("electron"), ["test/e2e/browserAutomation.cjs"], { env, stdio: "inherit" });
const timer = setTimeout(() => child.kill(), 90_000);
child.once("error", (error) => { clearTimeout(timer); console.error(error); process.exitCode = 1; });
child.once("exit", (code) => { clearTimeout(timer); process.exitCode = code ?? 1; });
