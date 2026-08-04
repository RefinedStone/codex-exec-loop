#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { pathToFileURL } from "node:url";

import { classifyChangedPaths } from "./ci-scope.mjs";

function gitLines(arguments_) {
  const output = execFileSync("git", arguments_, { encoding: "utf8" });
  return output.split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
}

export function collectChangedPaths(baseRef = "origin/prerelease") {
  const mergeBase = gitLines(["merge-base", baseRef, "HEAD"])[0];
  if (!mergeBase) {
    throw new Error(`could not resolve a merge base with ${baseRef}; fetch it before planning`);
  }
  return [...new Set([
    ...gitLines(["diff", "--name-only", "--diff-filter=ACMRD", mergeBase, "HEAD"]),
    ...gitLines(["diff", "--name-only", "--diff-filter=ACMRD"]),
    ...gitLines(["diff", "--cached", "--name-only", "--diff-filter=ACMRD"]),
    ...gitLines(["ls-files", "--others", "--exclude-standard"]),
  ])].sort();
}

export function commandsForPlan(plan, platform = process.platform) {
  const commands = ["git diff --check"];
  if (plan.docs_only) {
    return commands;
  }
  if (platform === "win32") {
    const mode = plan.full || (plan.rust && plan.node)
      ? "Full"
      : plan.rust
        ? "Rust"
        : plan.node
          ? "Admin"
          : "Portable";
    commands.push(`powershell -File scripts/check_native_pr.ps1 -Mode ${mode}`);
    return commands;
  }
  if (plan.full || (plan.rust && plan.node)) {
    commands.push("bash scripts/check_native_pr.sh");
    return commands;
  }
  if (plan.rust) {
    commands.push(
      "cargo fmt --all -- --check",
      "cargo test --locked",
      "cargo clippy --locked --all-targets --all-features -- -D warnings",
    );
  }
  if (plan.node) {
    commands.push("bash scripts/check_node_surfaces.sh");
  }
  if (plan.smoke) {
    commands.push("cargo check --locked --all-features --lib --bins");
  }
  return commands;
}

function parseBaseRef(arguments_) {
  const baseIndex = arguments_.indexOf("--base");
  if (baseIndex === -1) {
    return "origin/prerelease";
  }
  const value = arguments_[baseIndex + 1];
  if (!value) {
    throw new Error("--base requires a git ref");
  }
  return value;
}

function main() {
  const baseRef = parseBaseRef(process.argv.slice(2));
  const paths = collectChangedPaths(baseRef);
  const plan = classifyChangedPaths(paths);
  console.log(`Change plan: ${plan.scope} (${paths.length} path(s) against ${baseRef})`);
  for (const path of paths) {
    console.log(`  ${path}`);
  }
  console.log("\nRequired local checks:");
  for (const command of commandsForPlan(plan)) {
    console.log(`  ${command}`);
  }
  console.log("\nDelivery unit: one PR for this user-visible outcome.");
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main();
}
