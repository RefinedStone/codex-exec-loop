// 로컬 복원본 창의 화면이다 (#2427). 받은 글은 `textContent` 로만 넣는다: Markdown 이나 HTML 로 그리지 않는다.
// 상태가 바뀔 때마다 DOM 을 통째로 바꾼다. 잠긴 뒤에는 복원한 글이 DOM 과 이 모듈의 변수에 남지 않는다.
import {
  type VaultApprovedMapping,
  type VaultField,
  type VaultReconnectReview,
  type VaultRestoreAction,
  type VaultRestoreViewState,
  vaultRestoreView,
} from "../../shared/vaultRestore";
import type { AkraVaultRestoreBridge } from "../../shared/bridge";
import "../design/tokens.css";
import "./vaultRestore.css";

const root = document.getElementById("root");
if (!root) {
  throw new Error("root element is missing");
}
// 이 HTML 은 전용 preload 만 읽는다. 일반 화면의 AkraBridge 를 이 창에 적용하지 않는다.
import { archivePreview } from "./backupPreview";

const bridge = (window as unknown as { readonly akra: AkraVaultRestoreBridge }).akra.vaultRestore;
const FIELD_LABEL: Record<VaultField, string> = { title: "제목", body: "본문" };
const ACTION_LABEL: Record<VaultRestoreAction, string> = { unlock: "확인하고 열기", lock: "잠그기", review: "재연결 검토" };

function element<K extends keyof HTMLElementTagNameMap>(tag: K, text?: string, testId?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (text !== undefined) node.textContent = text;
  if (testId) node.dataset.testid = testId;
  return node;
}

function button(label: string, testId: string, onClick: () => void): HTMLButtonElement {
  const node = element("button", label, testId);
  node.type = "button";
  node.addEventListener("click", onClick);
  return node;
}

function section(title: string, text: string, testId: string): HTMLElement[] {
  return [element("h2", title), element("pre", text, testId)];
}

/** 표식마다 지금 공유본의 자리를 고르는 양식이다. 기본은 "연결하지 않음" 이다: 사용자가 고른 것만 연결한다. */
function reviewForm(review: VaultReconnectReview, generation: number): HTMLElement[] {
  const choices = new Map<string, VaultApprovedMapping | null>();
  const fieldsets = review.markers.map((marker, index) => {
    const fieldset = element("fieldset");
    fieldset.dataset.testid = "vault-reconnect-marker";
    fieldset.append(element("legend", marker.marker));
    fieldset.append(element("p", `기존 공유본: ${FIELD_LABEL[marker.previous.field]} ${marker.previous.position}번째 바이트`));
    if (marker.candidates.length === 0) fieldset.append(element("p", "지금 공유본에 이 표식이 없습니다."));
    const option = (label: string, mapping: VaultApprovedMapping | null, checked: boolean) => {
      const row = element("label");
      const radio = element("input");
      radio.type = "radio";
      radio.name = `marker-${index}`;
      radio.checked = checked;
      radio.addEventListener("change", () => choices.set(marker.marker, mapping));
      row.append(radio, ` ${label}`);
      fieldset.append(row);
    };
    option("연결하지 않음 (표식으로 남김)", null, true);
    for (const candidate of marker.candidates) {
      const mapping = { marker: marker.marker, field: candidate.field, position: candidate.position };
      option(`지금 공유본: ${FIELD_LABEL[candidate.field]} ${candidate.position}번째 바이트 — ${candidate.context}`, mapping, false);
    }
    return fieldset;
  });
  const confirm = button("고른 표식 재연결", "vault-reconnect-confirm", () => {
    confirm.disabled = true;
    bridge.reconnect(generation, [...choices.values()].filter((mapping) => mapping !== null));
  });
  return [
    ...section("지금 공유본 · 제목", review.publishedTitle, "vault-review-title"),
    ...section("지금 공유본 · 본문", review.publishedBody, "vault-review-body"),
    ...fieldsets,
    confirm,
  ];
}

function render(state: VaultRestoreViewState, viewGeneration = -1, backupAvailable = false): void {
  const view = vaultRestoreView(state);
  const actions = element("div");
  actions.className = "actions";
  const run: Record<VaultRestoreAction, () => void> = { unlock: () => bridge.unlock(viewGeneration), lock: bridge.lock, review: () => bridge.review(viewGeneration) };
  actions.append(...view.actions.map((action) => button(ACTION_LABEL[action], `vault-restore-${action}`, run[action])));
  const heading = element("h1", view.heading, "vault-restore-heading");
  const nodes: HTMLElement[] = [heading, element("p", view.detail, "vault-restore-detail")];
  if (state.phase === "restored") {
    nodes.push(...section("제목", state.title, "vault-restored-title"), ...section("본문", state.body, "vault-restored-body"));
    const until = element("p", `${new Date(state.expiresAt).toLocaleTimeString()} 에 다시 잠깁니다.`);
    until.className = "muted";
    nodes.push(until);
  }
  if (state.phase === "review") nodes.push(...reviewForm(state.review, viewGeneration));
  if ((state.phase === "locked" || state.phase === "held") && backupAvailable) nodes.push(archivePreview(bridge, viewGeneration));
  nodes.push(actions);
  root!.dataset.phase = state.phase;
  root!.replaceChildren(...nodes);
}

bridge.onState(render);
render({ phase: "locked", cause: null });
bridge.ready();
