#!/usr/bin/env node
// G1 (#2591): read-only capture and comparison. Never renames, publishes or modifies a remote.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import assert from "node:assert/strict";

export const REPOSITORY_ID = 1201646585;
export const OLD_REPOSITORY = "RefinedStone/codex-exec-loop";
export const NEW_REPOSITORY = "RefinedStone/akra-ade";


const sorted = (items, key) => [...items].sort((a, b) => String(key(a)).localeCompare(String(key(b)), "en"));
function canonical(value) {
  if (Array.isArray(value)) return value.map(canonical);
  if (value !== null && typeof value === "object") return Object.fromEntries(
    Object.keys(value).sort().map(key => [key, canonical(value[key])]));
  return value;
}
const digest = value => createHash("sha256").update(JSON.stringify(canonical(value))).digest("hex");

function readApi(endpoint, paginate = false) {
  // Never emit gh's raw output or error body (which may include private data).
  let output;
  try {
    output = execFileSync("gh", ["api", "--hostname", "github.com", "--method", "GET",
      ...(paginate ? ["--paginate", "--slurp"] : []), endpoint],
    { encoding: "utf8", timeout: 60_000, maxBuffer: 32 * 1024 * 1024, stdio: ["ignore", "pipe", "pipe"] });
  } catch {
    throw new Error("read-only GitHub API capture failed: " + endpoint);
  }
  const data = JSON.parse(output);
  if (paginate) {
    assert.ok(Array.isArray(data) && data.every(Array.isArray), "incomplete pagination shape");
    return data.flat();
  }
  return data;
}

function remoteRefs(repository) {
  const output = execFileSync("git", ["ls-remote", "--heads", "--tags", "https://github.com/" + repository + ".git"], {
    env: { ...process.env, GIT_TERMINAL_PROMPT: "0", GCM_INTERACTIVE: "never" },
    encoding: "utf8", timeout: 45_000, maxBuffer: 8 * 1024 * 1024, stdio: ["ignore", "pipe", "pipe"],
  });
  return output.trim().split(/\r?\n/).filter(Boolean).map(line => {
    const [sha, ref] = line.split("\t");
    assert.match(sha, /^[a-f0-9]{40,64}$/);
    assert.ok(ref.startsWith("refs/heads/") || ref.startsWith("refs/tags/"));
    return { ref, sha };
  }).sort((a, b) => a.ref.localeCompare(b.ref, "en"));
}

export function capture(phase, api = readApi, readRefs = remoteRefs) {
  assert.ok(phase === "before" || phase === "after", "capture phase must be before or after");
  const repository = phase === "before" ? OLD_REPOSITORY : NEW_REPOSITORY;
  const prefix = "repos/" + repository;
  const repo = api(prefix);
  assert.equal(repo.id, REPOSITORY_ID, "repository identity mismatch");
  assert.equal(repo.full_name, repository, "canonical repository name mismatch");
  const branches = sorted(api(prefix + "/branches?per_page=100", true).map(branch => ({
    name: branch.name, sha: branch.commit.sha, protected: branch.protected,
  })), x => x.name);
  assert.ok(branches.some(branch => branch.name === "prerelease"), "prerelease is missing");
  assert.ok(branches.some(branch => branch.name === repo.default_branch), "default branch is missing");
  const pulls = sorted(api(prefix + "/pulls?state=open&per_page=100", true).map(pr => ({
    number: pr.number, id: pr.id, base: pr.base.ref, baseRepoId: pr.base.repo?.id,
    head: pr.head.ref, headRepoId: pr.head.repo?.id ?? null, headSha: pr.head.sha,
  })), x => x.number);
  const issueSamples = [2414, 2555, 2584, 2591].map(number => {
    const issue = api(prefix + "/issues/" + number);
    assert.equal(issue.number, number);
    assert.ok(Number.isSafeInteger(issue.id) && !issue.pull_request, "issue identity is not readable");
    return { number, id: issue.id, state: issue.state };
  });
  const releases = sorted(api(prefix + "/releases?per_page=100", true).map(release => ({
    id: release.id, tag: release.tag_name, draft: release.draft, prerelease: release.prerelease,
    assets: sorted(release.assets.map(asset => ({ id: asset.id, name: asset.name, size: asset.size,
      digest: asset.digest ?? null })), asset => asset.id),
  })), x => x.id);
  const rulesets = sorted(api(prefix + "/rulesets?per_page=100", true).map(rule => {
    const full = api(prefix + "/rulesets/" + rule.id);
    assert.equal(full.id, rule.id);
    assert.ok(Array.isArray(full.rules) && Array.isArray(full.bypass_actors) && full.conditions,
      "ruleset detail is not readable; do not certify protection");
    return { id: full.id, name: full.name, target: full.target, enforcement: full.enforcement,
      contract: digest({ conditions: full.conditions, rules: full.rules, bypassActors: full.bypass_actors }) };
  }), x => x.id);
  // The alias must still resolve to the *same* repository after rename.
  const legacy = phase === "after" ? api("repos/" + OLD_REPOSITORY) : repo;
  assert.equal(legacy.id, REPOSITORY_ID, "legacy URL resolves to a different repository");
  const refs = readRefs(repository);
  const legacyRefs = phase === "after" ? readRefs(OLD_REPOSITORY) : refs;
  assert.deepEqual(legacyRefs, refs, "legacy Git refs differ from canonical refs");
  for (const branch of branches) assert.ok(refs.some(ref => ref.ref === "refs/heads/" + branch.name && ref.sha === branch.sha),
    "branch changed during capture; take a new snapshot in a quiet window");
  const activeRuns = ["queued", "in_progress", "waiting", "requested", "pending"].flatMap(status => {
    const data = api(prefix + "/actions/runs?status=" + status + "&per_page=100");
    assert.ok(Number.isInteger(data.total_count) && Array.isArray(data.workflow_runs));
    return [{ status, count: data.total_count }];
  });
  return { schema: 1, capturedAt: new Date().toISOString(), phase, repository: {
    id: repo.id, name: repo.full_name, defaultBranch: repo.default_branch,
    private: repo.private, visibility: repo.visibility, archived: repo.archived,
  }, branches, pulls, issueSamples, releases, rulesets, refs, activeRuns };
}

export function compare(before, after) {
  assert.equal(before.schema, 1);
  assert.equal(after.schema, 1);
  assert.equal(before.phase, "before");
  assert.equal(after.phase, "after");
  assert.equal(before.repository.name, OLD_REPOSITORY);
  assert.equal(after.repository.name, NEW_REPOSITORY);
  assert.equal(before.repository.id, REPOSITORY_ID);
  assert.equal(after.repository.id, REPOSITORY_ID);
  const { name: oldName, ...oldRepo } = before.repository;
  const { name: newName, ...newRepo } = after.repository;
  assert.deepEqual(oldRepo, newRepo, "repository settings changed during rename");
  for (const key of ["branches", "pulls", "issueSamples", "releases", "rulesets", "refs"]) {
    assert.deepEqual(before[key], after[key], key + " changed; reconcile before claiming a successful rename");
  }
  assert.ok(before.activeRuns.every(run => run.count === 0), "pre-rename capture was not quiescent");
  return { verified: true, repositoryId: REPOSITORY_ID, branches: after.branches.length,
    openPullRequests: after.pulls.length, releases: after.releases.length, rulesets: after.rulesets.length };
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  try {
    const [action, first, second, ...extra] = process.argv.slice(2);
    assert.ok(first && second && extra.length === 0, "Usage: repository-rename-audit.mjs capture <before|after> <new.json> | compare <before.json> <after.json>");
    if (action === "capture") {
      const snapshot = capture(first);
      writeFileSync(second, JSON.stringify(snapshot, null, 2) + "\n", { flag: "wx", mode: 0o600 });
      console.log(JSON.stringify({ phase: snapshot.phase, repositoryId: snapshot.repository.id,
        branches: snapshot.branches.length, openPullRequests: snapshot.pulls.length,
        releases: snapshot.releases.length, rulesets: snapshot.rulesets.length,
        activeRuns: snapshot.activeRuns }));
    } else if (action === "compare") {
      console.log(JSON.stringify(compare(JSON.parse(readFileSync(first, "utf8")), JSON.parse(readFileSync(second, "utf8")))));
    } else throw new Error("unknown audit command");
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
