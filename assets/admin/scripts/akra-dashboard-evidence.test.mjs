import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { Script, createContext } from "node:vm";

const dashboard = readFileSync(
  new URL("./akra-dashboard.js", import.meta.url),
  "utf8",
);

function extract(start, end) {
  const from = dashboard.indexOf(start);
  assert.ok(from >= 0, "missing declaration: " + start);
  const to = dashboard.indexOf(end, from + start.length);
  assert.ok(to > from, "missing declaration end: " + end);
  return dashboard.slice(from, to);
}

const selectionCode = [
  extract("  const setSelectedDetail = (source) => {", "  const clearRelated = () => {"),
  extract("  const clearRelated = () => {", "  const slotFromSessionKey = (key) => {"),
  extract("  const syncSelectedDetail = () => {", "  const renderDetailRow = (label, value, style, severity) => {"),
].join("\n");
const evidenceCode = extract(
  "  const openEvidenceDetailDrawer = ({ focusDrawer = true } = {}) => {",
  "  openValidationDetailDrawer = (",
);

function fakeSurface() {
  const selectionChanges = [];
  const entity = {
    isConnected: true,
    classList: {
      toggle(name, enabled) { selectionChanges.push([name, enabled]); },
      remove(name) { selectionChanges.push([name, false]); },
    },
    setAttribute() {},
    removeAttribute() {},
  };
  const root = {
    dataset: { selectedDetailKey: "slot:old", relatedSelectionCount: "1" },
    querySelectorAll(selector) {
      return selector === "[data-detail-type]" || selector === ".is-linked"
        ? [entity]
        : [];
    },
  };
  const drawerClasses = new Set(["is-open"]);
  const detailDrawer = {
    dataset: { detailMode: "slot" },
    hidden: false,
    classList: {
      contains(name) { return drawerClasses.has(name); },
      add(name) { drawerClasses.add(name); },
      remove(name) { drawerClasses.delete(name); },
    },
    setAttribute() {},
    focus() {},
  };
  const replacements = [];
  const context = createContext({
    root,
    detailDrawer,
    detailDrawerBody: { replaceChildren(...nodes) { replacements.push(nodes); } },
    detailDrawerTitle: { textContent: "" },
    detailDrawerSubtitle: { textContent: "" },
    detailSourceSelector: "[data-detail-type]",
    detailSourceKey(node) { return node === entity ? "slot:old" : ""; },
    detailTrigger: entity,
    evidenceDetailTrigger: { setAttribute() {} },
    validationDetailRequestSequence: 0,
    evidenceDetailRequestSequence: 0,
    detailDrawerGeneration: 0,
    validationEvidenceUrl: "/admin/api/validation/evidence",
    createText() { return { setAttribute() {} }; },
    window: { requestAnimationFrame(callback) { callback(); } },
    fetch() { return new Promise(() => {}); },
    renderEvidenceHistoryPage() { throw new Error("fetch was not released"); },
    openDetailDrawer() { throw new Error("stale entity must not reopen"); },
    closeDetailDrawer() { throw new Error("evidence mode must not close"); },
    markRelated() { throw new Error("evidence mode must not relink prior entity"); },
  });
  return { context, root, detailDrawer, entity, selectionChanges, replacements };
}

test("opening evidence clears entity identity and refresh cannot revive prior entity", () => {
  const fixture = fakeSurface();
  new Script(
    selectionCode + "\n" + evidenceCode + "\nopenEvidenceDetailDrawer();\nsyncSelectedDetail();",
    { filename: "admin-evidence-drawer-selection.js" },
  ).runInContext(fixture.context);
  assert.equal(fixture.root.dataset.selectedDetailKey, undefined);
  assert.equal(fixture.root.dataset.relatedSelectionCount, "0");
  assert.equal(fixture.detailDrawer.dataset.detailMode, "evidence");
  assert.equal(fixture.replacements.length, 1);
  assert.ok(
    fixture.selectionChanges.some(([name, enabled]) => name === "is-selected" && !enabled),
  );
});

test("ordinary entity detail still synchronizes when evidence mode is not active", () => {
  const fixture = fakeSurface();
  let reopened = 0;
  fixture.context.openDetailDrawer = (_source, options) => {
    reopened += 1;
    assert.equal(options.focusDrawer, false);
  };
  new Script(selectionCode + "\nsyncSelectedDetail();").runInContext(fixture.context);
  assert.equal(reopened, 1);
});
