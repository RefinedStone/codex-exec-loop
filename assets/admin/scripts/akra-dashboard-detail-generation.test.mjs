import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { Script, createContext } from "node:vm";

const source = readFileSync(new URL("./akra-dashboard.js", import.meta.url), "utf8");
const extract = (start, end) => {
  const a = source.indexOf(start);
  const b = source.indexOf(end, a + start.length);
  assert.ok(a >= 0 && b > a, "missing dashboard code boundary: " + start);
  return source.slice(a, b);
};
const actualHandlers = [
  extract("  const openDetailDrawer = (", "  const closeDetailDrawer = () => {"),
  extract("  const closeDetailDrawer = () => {", "  const openRefreshDetail = ("),
  extract("  const openEvidenceDetailDrawer = ({ focusDrawer = true } = {}) => {", "  openValidationDetailDrawer = ("),
  extract("  openValidationDetailDrawer = (", "  const validationCommandId = () => {"),
].join("\n");

function harness() {
  const requests = [];
  const classes = new Set(["is-open"]);
  const detailDrawer = {
    dataset: {},
    hidden: false,
    classList: {
      add(value) { classes.add(value); },
      remove(value) { classes.delete(value); },
      contains(value) { return classes.has(value); },
    },
    setAttribute() {},
    focus() {},
  };
  const detailDrawerBody = {
    children: [],
    replaceChildren(...children) { this.children = children; },
  };
  const detailDrawerTitle = { textContent: "" };
  const detailDrawerSubtitle = { textContent: "" };
  const evidenceDetailTrigger = { setAttribute() {} };
  const ctx = createContext({
    detailDrawer,
    detailDrawerBody,
    detailDrawerTitle,
    detailDrawerSubtitle,
    evidenceDetailTrigger,
    detailTrigger: null,
    detailRowsByType: { slot: [["Identity", "detailTitle"]] },
    detailDrawerGeneration: 0,
    validationDetailRequestSequence: 0,
    evidenceDetailRequestSequence: 0,
    openValidationDetailDrawer: null,
    validationBaseUrl: "/api/validation",
    validationEvidenceUrl: "/api/evidence",
    setSelectedDetail() {},
    markRelated() {},
    clearRelated() {},
    renderDetailRow(_label, value) { return { type: "entity-row", value }; },
    renderValidationDetail(record) { return { type: "validation-result", record }; },
    renderEvidenceHistoryPage(page) { return { type: "evidence-result", page }; },
    createText(_tag, _cls, text) { return { type: "loading", text, setAttribute() {} }; },
    optionalText(value) { return value || ""; },
    window: {
      requestAnimationFrame(callback) { callback(); },
      setTimeout(callback) { callback(); },
    },
    fetch(url) {
      let resolve;
      let reject;
      const promise = new Promise((yes, no) => {
        resolve = yes;
        reject = no;
      });
      requests.push({ url, resolve, reject });
      return promise;
    },
  });
  new Script(
    actualHandlers + "\nglobalThis.detailTestApi = { openDetailDrawer, openEvidenceDetailDrawer, openValidationDetailDrawer };",
  ).runInContext(ctx);
  const entity = { isConnected: true, dataset: { detailType: "slot", detailTitle: "Current slot" } };
  const pr = (key) => ({
    isConnected: true,
    dataset: {
      detailType: "validation",
      validationRecordKey: key,
      detailTitle: "PR " + key,
    },
  });
  const settle = () => new Promise((resolve) => setImmediate(resolve));
  return { ctx, requests, detailDrawer, detailDrawerBody, detailDrawerTitle, entity, pr, settle };
}

test("late PR details cannot overwrite a generic drawer selection", async () => {
  const ui = harness();
  ui.ctx.detailTestApi.openValidationDetailDrawer(ui.pr("old"));
  assert.equal(ui.requests.length, 1);
  ui.ctx.detailTestApi.openDetailDrawer(ui.entity);
  assert.equal(ui.detailDrawer.dataset.detailMode, "slot");

  ui.requests[0].resolve({
    ok: true,
    async json() { return { pullRequestNumber: 7, phaseLabel: "stale" }; },
  });
  await ui.settle();
  assert.equal(ui.detailDrawer.dataset.detailMode, "slot");
  assert.equal(ui.detailDrawerTitle.textContent, "Current slot");
  assert.equal(ui.detailDrawerBody.children[0].type, "entity-row");
});

test("late evidence response cannot replace a newer PR drawer", async () => {
  const ui = harness();
  ui.ctx.detailTestApi.openEvidenceDetailDrawer();
  ui.ctx.detailTestApi.openValidationDetailDrawer(ui.pr("new"));
  assert.equal(ui.requests.length, 2);

  ui.requests[0].resolve({ ok: true, async json() { return { items: ["stale"] }; } });
  await ui.settle();
  assert.equal(ui.detailDrawer.dataset.detailMode, "validation");
  assert.equal(ui.detailDrawerBody.children[0].type, "loading");

  ui.requests[1].resolve({
    ok: true,
    async json() { return { pullRequestNumber: 9, akraId: "akra-9", phaseLabel: "ready" }; },
  });
  await ui.settle();
  assert.equal(ui.detailDrawerBody.children[0].type, "validation-result");
  assert.equal(ui.detailDrawerBody.children[0].record.pullRequestNumber, 9);
});

test("late error and duplicate PR key do not replace the latest selection", async () => {
  const ui = harness();
  ui.ctx.detailTestApi.openValidationDetailDrawer(ui.pr("old"));
  ui.ctx.detailTestApi.openValidationDetailDrawer(ui.pr("new"));

  ui.requests[0].reject(new Error("old fetch failed"));
  await ui.settle();
  assert.equal(ui.detailDrawer.dataset.validationRecordKey, "new");
  assert.equal(ui.detailDrawerBody.children[0].type, "loading");

  ui.requests[1].resolve({
    ok: true,
    async json() { return { pullRequestNumber: 11, phaseLabel: "new" }; },
  });
  await ui.settle();
  assert.equal(ui.detailDrawerBody.children[0].record.pullRequestNumber, 11);
});
