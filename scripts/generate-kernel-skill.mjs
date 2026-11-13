#!/usr/bin/env node
import { readFileSync } from "node:fs";

const spec = JSON.parse(readFileSync(new URL("../schema/akra-kernel-spec.json", import.meta.url), "utf8"));
const lines = [
  "---",
  "name: akra-kernel",
  "description: Inspect and control the repository kernel through the Akra JSON CLI.",
  "---",
  "",
  "# Akra kernel",
  "",
  "Generated from schema/akra-kernel-spec.json. Do not edit by hand.",
  "",
  "Run `akra describe --json` to inspect input schemas, reasons, and next actions.",
  "Run commands from the target repository.",
  "Each invocation prints one JSON line. Exit codes: 0 accepted, 2 rejected, 1 invalid input or unavailable.",
  "Use `--idempotency-key` for mutations that may be retried across separate invocations.",
  "An omitted key is generated for this invocation only. Preserve an explicit key when retrying.",
  "When a command ends with an error, the output carries `idempotency_key` and, for a kernel error, `error.code`.",
  "Send the same command again with that key: the kernel does not run it twice. Find the code in `errors` of `describe`.",
  "Destructive commands require `--confirm true`.",
  "A missing Gateway permits store commands only. Runtime commands require a running Gateway.",
  "A command from this CLI starts the kernel only for that command. The session list is not kept between invocations:",
  "`load_sessions` helps only while another client (Akra Desktop) keeps the kernel up.",
  "Follow structured `next` actions after rejection. Admission does not mean completion.",
  "Use `akra kernel operation <op_id> --wait --json` to observe completion.",
  "Use `akra kernel history --after <sequence> --wait --json` to wait for records.",
  "",
  "## Commands",
  "",
  "| Invocation | Effect | Summary |",
  "| --- | --- | --- |",
];
for (const command of spec.commands) {
  lines.push(`| \`akra kernel ${command.scope} ${command.name} --json\` | ${command.effect} | ${command.summary} |`);
}
lines.push("", "Pass input fields as `--field value`. Use the input schema from `describe` for required fields and types.", "", "## Queries", "");
for (const query of spec.queries) {
  lines.push(`- \`akra kernel query ${query.name} --json\`: ${query.summary}`);
}
const generated = `${lines.join("\n")}\n`;
if (process.argv.includes("--check")) {
  const shipped = readFileSync(new URL("../assets/app-server/hub-skills/akra-kernel/SKILL.md", import.meta.url), "utf8");
  if (shipped !== generated) {
    console.error("The kernel skill differs from the spec. Regenerate with scripts/generate-kernel-skill.mjs.");
    process.exitCode = 1;
  }
} else {
  process.stdout.write(generated);
}
