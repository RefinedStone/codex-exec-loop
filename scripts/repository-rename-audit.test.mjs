import test from "node:test";
import assert from "node:assert/strict";
import { capture, compare, OLD_REPOSITORY, NEW_REPOSITORY, REPOSITORY_ID } from "./repository-rename-audit.mjs";

const sha = "a".repeat(40);
const refs = [{ ref: "refs/heads/prerelease", sha }, { ref: "refs/tags/v1", sha }];
function fixture(phase, override = {}) {
  const canonical = phase === "before" ? OLD_REPOSITORY : NEW_REPOSITORY;
  const repo = { id: REPOSITORY_ID, full_name: canonical, default_branch: "prerelease",
    private: false, visibility: "public", archived: false };
  const get = endpoint => {
    if (endpoint.endsWith("/rulesets/1")) return { id: 1, name: "protected", target: "branch",
      enforcement: "active", rules: [{type: "deletion"}], conditions: {}, bypass_actors: [] };
    if (endpoint.includes("/branches?")) return [{name: "prerelease", commit: {sha}, protected: true}];
    if (endpoint.includes("/pulls?")) return [{id: 55, number: 1, base: {ref: "prerelease", repo: {id: REPOSITORY_ID}},
      head: {ref: "feature", repo: {id: REPOSITORY_ID}, sha}}];
    if (endpoint.includes("/issues/")) {
      const number = Number(endpoint.split("/").at(-1));
      return {number, id: 10000 + number, state: "open"};
    }
    if (endpoint.includes("/releases?")) return [{id: 8, tag_name: "v1", draft: false, prerelease: false,
      assets: [{id: 9, name: "artifact", size: 1, digest: "sha256:fixture"}]}];
    if (endpoint.includes("/rulesets?")) return [{id: 1}];
    if (endpoint.includes("/actions/runs?")) return {total_count: 0, workflow_runs: []};
    return repo;
  };
  return capture(phase, endpoint => override.api?.(endpoint, get) ?? get(endpoint), override.refs ?? (() => refs));
}

test("rename requires identical repository ID, exact refs, PRs, releases and rulesets", () => {
  const before = fixture("before"), after = fixture("after");
  assert.equal(compare(before, after).verified, true);
  for (const key of ["branches", "pulls", "issueSamples", "releases", "rulesets", "refs"]) {
    const changed = structuredClone(after);
    changed[key] = [];
    assert.throws(() => compare(before, changed), new RegExp(key + " changed|branches|refs|issueSamples"));
  }
  const renamed = structuredClone(after);
  renamed.repository.id++;
  assert.throws(() => compare(before, renamed));
  const busy = structuredClone(before);
  busy.activeRuns[0].count = 1;
  assert.throws(() => compare(busy, after), /not quiescent/);
});

test("capture refuses inaccessible protection, wrong alias and moving branch heads", () => {
  assert.throws(() => fixture("before", {api: (endpoint, get) => endpoint.endsWith("/rulesets/1")
    ? { ...get(endpoint), bypass_actors: undefined } : get(endpoint)}), /ruleset detail is not readable/);
  assert.throws(() => fixture("after", {api: (endpoint, get) => endpoint === "repos/" + OLD_REPOSITORY
    ? {...get(endpoint), id: 99} : get(endpoint)}), /legacy URL resolves/);
  assert.throws(() => fixture("before", {refs: () => []}), /branch changed during capture/);
  assert.throws(() => fixture("after", {refs: repo => repo === OLD_REPOSITORY ? [] : refs}), /legacy Git refs differ/);
});

test("comparison rejects identically incomplete or malformed snapshots, not just differences", () => {
  const complete = fixture("before");
  const mutations = [
    s => { delete s.repository.visibility; },
    s => { delete s.capturedAt; },
    s => { s.activeRuns = []; },
    s => { s.activeRuns.pop(); },
    s => { s.activeRuns[1] = {...s.activeRuns[0]}; },
    s => { s.activeRuns[0].count = -1; },
    s => { s.activeRuns[0].count = 0.5; },
    s => { s.branches = []; s.refs = []; },
    s => { s.branches[0].sha = "bad"; },
    s => { s.refs.push({ref:"refs/heads/unseen",sha}); },
    s => { s.issueSamples.pop(); },
    s => { s.issueSamples[1].id = s.issueSamples[0].id; },
    s => { s.pulls[0].id = 0; },
    s => { s.releases[0].assets[0].size = -1; },
    s => { delete s.rulesets[0].contract; },
  ];
  for (const mutate of mutations) {
    const before = structuredClone(complete);
    mutate(before);
    const after = structuredClone(before);
    after.phase = "after";
    after.repository.name = NEW_REPOSITORY;
    assert.throws(() => compare(before, after));
  }
});
