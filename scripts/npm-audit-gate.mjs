#!/usr/bin/env node
// `npm audit --audit-level=high` with dated exceptions.
//
// npm has no way to accept one advisory. So a new advisory without a patched version fails every
// pull request until the upstream package releases a fix. This gate runs the same audit and accepts
// only the advisories that `scripts/npm-audit-exceptions.json` lists. Each exception has an expiry
// date; after that date the gate fails again, so that an exception is looked at again and not forgotten.
//
// Usage: node scripts/npm-audit-gate.mjs <package directory>
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const BLOCKING_SEVERITIES = new Set(["high", "critical"]);

function advisoryId(url) {
  return String(url ?? "").split("/").filter(Boolean).at(-1) ?? "";
}

/** The advisories of an audit report, one entry per advisory id. */
export function reportedAdvisories(report) {
  const advisories = new Map();
  for (const vulnerability of Object.values(report.vulnerabilities ?? {})) {
    for (const cause of vulnerability.via ?? []) {
      // A string names another vulnerable package; that package has its own entry with the advisory.
      if (typeof cause !== "object" || cause === null) continue;
      const id = advisoryId(cause.url);
      advisories.set(id, { id, package: cause.name, severity: cause.severity, title: cause.title, url: cause.url });
    }
  }
  return [...advisories.values()];
}

/**
 * Decides the gate. `today` is an ISO date (`YYYY-MM-DD`). An exception applies through its expiry date.
 * Returns the advisories that block, the exceptions that applied, the expired ones, and the unused ones.
 */
export function evaluateAudit(report, exceptions, today) {
  const blocking = [];
  const accepted = [];
  const expired = [];
  const used = new Set();
  for (const advisory of reportedAdvisories(report)) {
    if (!BLOCKING_SEVERITIES.has(advisory.severity)) continue;
    const exception = exceptions.find((entry) => entry.advisory === advisory.id && entry.package === advisory.package);
    if (!exception) {
      blocking.push(advisory);
      continue;
    }
    used.add(exception);
    if (today > exception.expires) {
      expired.push({ advisory, exception });
      blocking.push(advisory);
    } else {
      accepted.push({ advisory, exception });
    }
  }
  return { blocking, accepted, expired, unused: exceptions.filter((entry) => !used.has(entry)) };
}

/** Rejects an exception list that could hide more than it says. */
export function validateExceptions(exceptions) {
  if (!Array.isArray(exceptions)) throw new Error("the exception list must be an array");
  for (const entry of exceptions) {
    for (const field of ["advisory", "package", "expires", "reason"]) {
      if (typeof entry?.[field] !== "string" || entry[field].trim() === "") {
        throw new Error(`an exception needs a non-empty "${field}"`);
      }
    }
    if (!/^GHSA(-[0-9a-z]{4}){3}$/.test(entry.advisory)) throw new Error(`"${entry.advisory}" is not a GHSA advisory id`);
    if (!/^\d{4}-\d{2}-\d{2}$/.test(entry.expires) || Number.isNaN(Date.parse(entry.expires))) {
      throw new Error(`the expiry of ${entry.advisory} must be a date (YYYY-MM-DD)`);
    }
  }
  return exceptions;
}

function main() {
  const prefix = process.argv[2];
  if (!prefix) {
    console.error("usage: node scripts/npm-audit-gate.mjs <package directory>");
    return 2;
  }
  const exceptionsPath = join(dirname(fileURLToPath(import.meta.url)), "npm-audit-exceptions.json");
  const exceptions = validateExceptions(JSON.parse(readFileSync(exceptionsPath, "utf8")));
  // `npm` is a script on Windows, so it needs a shell there. The arguments are fixed text and a repository path.
  const audit = spawnSync("npm", ["--prefix", prefix, "audit", "--json"], {
    encoding: "utf8",
    shell: process.platform === "win32",
    maxBuffer: 64 * 1024 * 1024,
  });
  let report;
  try {
    report = JSON.parse(audit.stdout);
  } catch {
    console.error(`npm-audit-gate: npm audit did not print a report for ${prefix} (exit ${audit.status})`);
    console.error(audit.stderr || audit.stdout || String(audit.error ?? ""));
    return 1;
  }
  if (report.error || !report.vulnerabilities) {
    console.error(`npm-audit-gate: npm audit failed for ${prefix}: ${JSON.stringify(report.error ?? report)}`);
    return 1;
  }
  const today = new Date().toISOString().slice(0, 10);
  const result = evaluateAudit(report, exceptions, today);
  for (const { advisory, exception } of result.accepted) {
    console.log(`npm-audit-gate: accepted ${advisory.id} (${advisory.package}, ${advisory.severity}) until ${exception.expires}: ${exception.reason}`);
  }
  for (const { advisory, exception } of result.expired) {
    console.error(`npm-audit-gate: the exception for ${advisory.id} (${advisory.package}) expired on ${exception.expires}. Update the package, or look at the advisory again and set a new date in scripts/npm-audit-exceptions.json.`);
  }
  for (const advisory of result.blocking) {
    console.error(`npm-audit-gate: ${advisory.severity} ${advisory.id} in ${advisory.package}: ${advisory.title} ${advisory.url}`);
  }
  for (const exception of result.unused) {
    // Each package directory has its own report, so an exception can be unused here and used in another directory.
    console.log(`npm-audit-gate: ${prefix} does not report ${exception.advisory} (${exception.package}).`);
  }
  if (result.blocking.length > 0) {
    console.error(`npm-audit-gate: ${prefix} has ${result.blocking.length} blocking advisory(ies). Run \`npm --prefix ${prefix} audit\` for the dependency paths.`);
    return 1;
  }
  console.log(`npm-audit-gate: ${prefix} has no blocking advisory (high or critical).`);
  return 0;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  process.exitCode = main();
}
