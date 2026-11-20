import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { evaluateAudit, reportedAdvisories, validateExceptions } from "./npm-audit-gate.mjs";

const PICOMATCH = {
  source: 1,
  name: "picomatch",
  title: "picomatch vulnerable to ReDoS",
  url: "https://github.com/advisories/GHSA-fc5q-39xc-vjpf",
  severity: "high",
};

// The shape of `npm audit --json`: the root package carries the advisory, the dependents name the package.
function report(...advisories) {
  const vulnerabilities = { vite: { name: "vite", severity: "high", via: ["picomatch", "tinyglobby"] } };
  for (const advisory of advisories) {
    vulnerabilities[advisory.name] = { name: advisory.name, severity: advisory.severity, via: [advisory] };
  }
  return { vulnerabilities };
}

const EXCEPTION = { advisory: "GHSA-fc5q-39xc-vjpf", package: "picomatch", expires: "2026-12-18", reason: "no patched version" };

test("an advisory is read once, from the package that carries it", () => {
  assert.deepEqual(reportedAdvisories(report(PICOMATCH)).map((advisory) => [advisory.id, advisory.package]), [["GHSA-fc5q-39xc-vjpf", "picomatch"]]);
});

test("a high advisory without an exception blocks", () => {
  const result = evaluateAudit(report(PICOMATCH), [], "2026-11-18");
  assert.deepEqual(result.blocking.map((advisory) => advisory.id), ["GHSA-fc5q-39xc-vjpf"]);
});

test("an exception accepts only its advisory, through its expiry date", () => {
  const other = { ...PICOMATCH, name: "left-pad", url: "https://github.com/advisories/GHSA-aaaa-bbbb-cccc", severity: "critical" };
  const result = evaluateAudit(report(PICOMATCH, other), [EXCEPTION], "2026-12-18");
  assert.deepEqual(result.accepted.map(({ advisory }) => advisory.id), ["GHSA-fc5q-39xc-vjpf"]);
  assert.deepEqual(result.blocking.map((advisory) => advisory.id), ["GHSA-aaaa-bbbb-cccc"]);
});

test("an expired exception blocks again and is reported as expired", () => {
  const result = evaluateAudit(report(PICOMATCH), [EXCEPTION], "2026-12-19");
  assert.equal(result.blocking.length, 1);
  assert.equal(result.expired.length, 1);
  assert.equal(result.accepted.length, 0);
});

test("the same advisory id in another package is not accepted", () => {
  const result = evaluateAudit(report({ ...PICOMATCH, name: "micromatch" }), [EXCEPTION], "2026-11-18");
  assert.equal(result.blocking.length, 1);
});

test("moderate and low advisories do not block, as with --audit-level=high", () => {
  const result = evaluateAudit(report({ ...PICOMATCH, severity: "moderate" }), [], "2026-11-18");
  assert.equal(result.blocking.length, 0);
});

test("an exception that no report uses is listed as unused", () => {
  assert.deepEqual(evaluateAudit({ vulnerabilities: {} }, [EXCEPTION], "2026-11-18").unused, [EXCEPTION]);
});

test("an exception needs an advisory id, a package, a date, and a reason", () => {
  assert.throws(() => validateExceptions([{ ...EXCEPTION, expires: "" }]), /expires/);
  assert.throws(() => validateExceptions([{ ...EXCEPTION, expires: "next month" }]), /date/);
  assert.throws(() => validateExceptions([{ ...EXCEPTION, advisory: "*" }]), /GHSA/);
  assert.throws(() => validateExceptions([{ ...EXCEPTION, reason: " " }]), /reason/);
  assert.throws(() => validateExceptions({}), /array/);
});

test("the committed exception list is valid", () => {
  const committed = JSON.parse(readFileSync(new URL("./npm-audit-exceptions.json", import.meta.url), "utf8"));
  assert.doesNotThrow(() => validateExceptions(committed));
});

for (const order of ["excepted-first", "excepted-last"]) {
  test(`one package exception cannot hide the same advisory in another package (${order})`, () => {
    const other = { ...PICOMATCH, name: "micromatch", severity: "critical" };
    const advisories = order === "excepted-first" ? [PICOMATCH, other] : [other, PICOMATCH];
    const result = evaluateAudit(report(...advisories), [EXCEPTION], "2026-11-18");
    assert.deepEqual(result.accepted.map(({ advisory }) => advisory.package), ["picomatch"]);
    assert.deepEqual(result.blocking.map((advisory) => advisory.package), ["micromatch"]);
    assert.equal(reportedAdvisories(report(...advisories)).length, 2);
  });
}

test("duplicate occurrences of one advisory and package remain one entry", () => {
  const repeated = report(PICOMATCH);
  repeated.vulnerabilities.vite.via.push(PICOMATCH);
  assert.equal(reportedAdvisories(repeated).length, 1);
});
