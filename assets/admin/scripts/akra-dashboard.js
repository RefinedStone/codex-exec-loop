(() => {
  const root = document.querySelector("[data-admin-graphic]");
  if (!root) return;

  const pollIntervalMs = Number(root.dataset.pollIntervalMs || "10000");
  const dashboardUrl = "/api/admin/akra/dashboard";
  const eventsUrl = "/api/admin/akra/events";
  const streamUrl = "/api/admin/akra/stream";
  const controlUrl = "/api/admin/akra/control";
  const debugHarnessUrl = "/api/admin/akra/debug-harness";
  const validationBaseUrl = "/api/admin/akra/validations";
  const validationEvidenceUrl = "/api/admin/akra/pr-validation/evidence";
  const csrfToken = document.querySelector('meta[name="csrf-token"]')?.content || "";

  const setText = (selector, value) => {
    for (const node of root.querySelectorAll(selector)) {
      if (node.textContent !== value) node.textContent = value;
    }
  };
  const setOperationalState = (selector, readiness) => {
    const state = ["ready", "degraded", "blocked"].includes(readiness) ? readiness : "degraded";
    for (const node of root.querySelectorAll(selector)) {
      node.classList.remove("state-ready", "state-degraded", "state-blocked");
      node.classList.add(`state-${state}`);
    }
  };

  const formatValue = (value, fallback = "-") => value === null || value === undefined ? fallback : String(value);
  const asArray = (value) => Array.isArray(value) ? value : [];
  const optionalText = (value, fallback = "-") => {
    const text = formatValue(value, fallback);
    return text.trim() === "" ? fallback : text;
  };
  const formatEventFeedStatus = (visible, total) => {
    const visibleCount = Math.max(Number(visible) || 0, 0);
    const totalCount = Math.max(Number(total) || visibleCount, visibleCount);
    if (totalCount > visibleCount) return `LIVE · 최근 ${visibleCount}개 · 총 ${totalCount}개`;
    return `LIVE · 총 ${totalCount}개`;
  };
  const agentAvatarClass = (classLabel) => {
    const normalized = optionalText(classLabel, "Runner").replace(/[^A-Za-z0-9_-]/g, "");
    return `avatar-${normalized || "Runner"}`;
  };
  const createText = (tagName, className, value) => {
    const node = document.createElement(tagName);
    if (className) node.className = className;
    node.textContent = formatValue(value);
    return node;
  };
  const setDataset = (node, values) => {
    for (const [key, value] of Object.entries(values)) {
      node.dataset[key] = value === null || value === undefined ? "" : String(value);
    }
  };
  const panelTitle = (title, subtitle) => {
    const header = document.createElement("div");
    header.className = "panel-title";
    header.appendChild(createText("h3", "", title));
    if (String(subtitle || "").trim() !== "") {
      header.appendChild(createText("small", "", subtitle));
    }
    return header;
  };
  const updatePanel = (selector, children) => {
    const panel = root.querySelector(selector);
    if (panel) panel.replaceChildren(...children);
  };
  const eventList = root.querySelector("[data-event-list]");
  const eventFeedStatus = root.querySelector("[data-event-feed-status]");
  const eventLimit = 50;
  const detailDrawer = root.querySelector("[data-detail-drawer]");
  const detailDrawerTitle = root.querySelector("[data-detail-drawer-title]");
  const detailDrawerSubtitle = root.querySelector("[data-detail-drawer-subtitle]");
  const detailDrawerBody = root.querySelector("[data-detail-drawer-body]");
  const evidenceDetailTrigger = root.querySelector("[data-evidence-detail-trigger]");
  let detailTrigger = null;
  let validationDetailRequestSequence = 0;
  let evidenceDetailRequestSequence = 0;
  let validationCommandRequest = null;
  let validationCommandFeedback = null;
  let currentValidationEvidence = null;
  const detailRowsByType = {
    slot: [
      ["상태", "detailState", "chip"],
      ["슬롯", "detailSlot"],
      ["작업 ID", "detailTask"],
      ["브랜치", "detailBranch"],
      ["Worktree", "detailWorktree"],
      ["Owner", "detailOwner"],
      ["Owner Agent", "detailOwnerAgent"],
      ["Owner Session", "detailOwnerSession"],
      ["Lease Generation", "detailLeaseGeneration"],
      ["Note", "detailNote"]
    ],
    agent: [
      ["상태", "detailState", "chip"],
      ["슬롯", "detailSlot"],
      ["작업", "detailTask"],
      ["브랜치", "detailBranch"],
      ["런타임", "detailRuntime"],
      ["진행률", "detailProgress"],
      ["최근 신호", "detailSummary"]
    ],
    distributor: [
      ["상태", "detailState", "chip"],
      ["Head", "detailHead"],
      ["Queue Depth", "detailDepth"],
      ["Barrier", "detailBarrier"],
      ["Integration", "detailWorktree"],
      ["운영 메모", "detailNote"]
    ],
    eventFeed: [
      ["상태", "detailState", "chip"],
      ["Visible", "detailVisible"],
      ["Total", "detailTotal"],
      ["Newest", "detailNewest"]
    ],
    director: [
      ["상태", "detailState", "chip"],
      ["운영 메모", "detailNote"]
    ],
    event: [
      ["상태", "detailState", "chip"],
      ["시간", "detailTime"],
      ["Projection", "detailProjection"],
      ["요약", "detailSummary"]
    ],
    queueItem: [
      ["상태", "detailState", "chip"],
      ["작업", "detailTask"],
      ["요원", "detailAgent"],
      ["브랜치", "detailBranch"],
      ["Commit", "detailCommit"],
      ["운영 메모", "detailNote"]
    ],
    campaignLane: [
      ["상태", "detailState", "chip"],
      ["요원", "detailAgent"],
      ["슬롯", "detailSlot"],
      ["작업", "detailTask"],
      ["진행률", "detailProgress"],
      ["최근 신호", "detailSummary"]
    ],
    refresh: [
      ["상태", "detailState", "chip"],
      ["Snapshot", "detailSnapshot"],
      ["Events", "detailEvents"],
      ["주의", "detailNote"]
    ]
  };

  const setEventStatus = () => {
    if (!eventFeedStatus) return;
    const visible = root.querySelectorAll("[data-event-sequence]").length;
    const total = Number(root.dataset.eventTotalCount || visible);
    eventFeedStatus.textContent = formatEventFeedStatus(visible, total);
  };

  const severityClass = (severity) => `severity-${severity || "normal"}`;

  const detailSourceSelector = "[data-detail-type]";
  const linkedSourceSelector = "[data-slot-id], [data-agent-id], [data-task-id], [data-projection-kind], [data-projection-key]";

  const detailSourceKey = (source) => {
    if (!source) return "";
    const type = source.dataset.detailType || "detail";
    const stableId = source.dataset.eventSequence
      || source.dataset.validationRecordKey
      || source.dataset.slotId
      || source.dataset.agentId
      || source.dataset.taskId
      || source.dataset.projectionKey
      || source.dataset.detailTitle
      || source.dataset.detailSubtitle
      || "";
    return `${type}:${stableId}`;
  };

  const initializeDetailControl = (source) => {
    if (!source) return;
    source.setAttribute("aria-controls", "akra-detail-drawer");
    if (!source.hasAttribute("aria-expanded")) source.setAttribute("aria-expanded", "false");
  };

  const initializeDetailControls = () => {
    for (const source of root.querySelectorAll(detailSourceSelector)) {
      initializeDetailControl(source);
    }
  };

  const setSelectedDetail = (source) => {
    const nextKey = detailSourceKey(source);
    for (const node of root.querySelectorAll(detailSourceSelector)) {
      const selected = Boolean(source) && detailSourceKey(node) === nextKey;
      node.classList.toggle("is-selected", selected);
      node.setAttribute("aria-expanded", selected && detailDrawer && !detailDrawer.hidden ? "true" : "false");
    }
    if (nextKey) {
      root.dataset.selectedDetailKey = nextKey;
    } else {
      delete root.dataset.selectedDetailKey;
    }
  };

  const clearRelated = () => {
    for (const node of root.querySelectorAll(".is-linked")) {
      node.classList.remove("is-linked");
      node.removeAttribute("aria-current");
    }
    root.dataset.relatedSelectionCount = "0";
  };

  const slotFromSessionKey = (key) => {
    const [slot] = String(key || "").split("@");
    return slot && slot.startsWith("slot-") ? slot : "";
  };

  const selectionTokens = (source) => {
    const tokens = { slot: "", agent: "", task: "" };
    if (!source) return tokens;
    tokens.slot = source.dataset.slotId || source.dataset.detailSlot || "";
    tokens.agent = source.dataset.agentId || source.dataset.detailAgent || "";
    tokens.task = source.dataset.taskId || "";

    const projectionKind = source.dataset.projectionKind || "";
    const projectionKey = source.dataset.projectionKey || "";
    if (projectionKind === "slot_lease") {
      tokens.slot = projectionKey;
    } else if (projectionKind === "session_detail" || projectionKind === "distributor_queue") {
      tokens.slot = slotFromSessionKey(projectionKey);
    } else if (projectionKind.includes("agent")) {
      tokens.agent = projectionKey;
    } else if (projectionKind.includes("task")) {
      tokens.task = projectionKey;
    }
    return tokens;
  };

  const projectionSlotToken = (source) => {
    const projectionKind = source?.dataset?.projectionKind || "";
    const projectionKey = source?.dataset?.projectionKey || "";
    if (projectionKind === "slot_lease") return projectionKey;
    if (projectionKind === "session_detail" || projectionKind === "distributor_queue") {
      return slotFromSessionKey(projectionKey);
    }
    return "";
  };

  const markRelated = (source) => {
    clearRelated();
    const tokens = selectionTokens(source);
    let linkedCount = 0;
    for (const node of root.querySelectorAll(linkedSourceSelector)) {
      const projectionSlot = projectionSlotToken(node);
      const linked = Boolean(
        (tokens.slot && node.dataset.slotId === tokens.slot)
        || (tokens.agent && node.dataset.agentId === tokens.agent)
        || (tokens.task && node.dataset.taskId === tokens.task)
        || (tokens.slot && projectionSlot === tokens.slot)
      );
      node.classList.toggle("is-linked", linked);
      if (linked) {
        node.setAttribute("aria-current", "true");
        linkedCount += 1;
      } else {
        node.removeAttribute("aria-current");
      }
    }
    root.dataset.relatedSelectionCount = String(linkedCount);
  };

  const syncSelectedDetail = () => {
    const selectedKey = root.dataset.selectedDetailKey || "";
    if (!selectedKey) return;
    const source = [...root.querySelectorAll(detailSourceSelector)]
      .find((node) => detailSourceKey(node) === selectedKey);
    if (source) {
      if (detailDrawer?.classList.contains("is-open")) {
        if (!detailTrigger?.isConnected) detailTrigger = source;
        openDetailDrawer(source, { focusDrawer: false, rememberTrigger: false });
      } else {
        setSelectedDetail(source);
        markRelated(source);
      }
    } else if (selectedKey.startsWith("event:")) {
      setSelectedDetail(null);
      clearRelated();
    } else {
      closeDetailDrawer();
    }
  };

  const renderDetailRow = (label, value, style, severity) => {
    const row = document.createElement("div");
    row.className = "drawer-row";
    const key = document.createElement("span");
    key.className = "drawer-key";
    key.textContent = label;
    const body = document.createElement("span");
    body.className = "drawer-value";
    if (style === "chip") {
      const chip = document.createElement("span");
      chip.className = `detail-chip ${severityClass(severity)}`;
      chip.textContent = value || "-";
      body.appendChild(chip);
    } else {
      body.textContent = value || "-";
    }
    row.append(key, body);
    return row;
  };

  let openValidationDetailDrawer = null;

  const openDetailDrawer = (
    source,
    { focusDrawer = true, rememberTrigger = true, trigger = source } = {}
  ) => {
    if (!source || !detailDrawer || !detailDrawerBody) return;
    if (source.dataset.detailType === "validation" && openValidationDetailDrawer) {
      openValidationDetailDrawer(source, { focusDrawer, rememberTrigger, trigger });
      return;
    }
    if (rememberTrigger) detailTrigger = trigger?.isConnected ? trigger : null;
    evidenceDetailTrigger?.setAttribute("aria-expanded", "false");
    const type = source.dataset.detailType;
    const rows = detailRowsByType[type] || [];
    detailDrawerTitle.textContent = source.dataset.detailTitle || "상세";
    detailDrawerSubtitle.textContent = source.dataset.detailSubtitle || "선택한 항목";
    detailDrawerBody.replaceChildren(
      ...rows.map(([label, key, style]) =>
        renderDetailRow(label, source.dataset[key], style, source.dataset.detailSeverity)
      )
    );
    detailDrawer.dataset.detailMode = type || "generic";
    detailDrawer.hidden = false;
    detailDrawer.setAttribute("aria-hidden", "false");
    window.requestAnimationFrame(() => {
      detailDrawer.classList.add("is-open");
      if (focusDrawer) detailDrawer.focus({ preventScroll: true });
    });
    setSelectedDetail(source);
    markRelated(source);
  };

  const closeDetailDrawer = () => {
    if (!detailDrawer) return;
    const trigger = detailTrigger;
    detailTrigger = null;
    detailDrawer.classList.remove("is-open");
    validationDetailRequestSequence += 1;
    evidenceDetailRequestSequence += 1;
    delete detailDrawer.dataset.detailMode;
    detailDrawer.setAttribute("aria-hidden", "true");
    evidenceDetailTrigger?.setAttribute("aria-expanded", "false");
    setSelectedDetail(null);
    clearRelated();
    window.setTimeout(() => {
      if (!detailDrawer.classList.contains("is-open")) detailDrawer.hidden = true;
    }, 180);
    if (trigger?.isConnected && trigger.getClientRects().length > 0) {
      trigger.focus({ preventScroll: true });
    }
  };

  const openRefreshDetail = (snapshotState, eventsState, trigger) => {
    const source = document.createElement("span");
    source.dataset.detailType = "refresh";
    source.dataset.detailTitle = "Refresh";
    source.dataset.detailSubtitle = "authoritative admin snapshot refresh";
    source.dataset.detailState = snapshotState === "ok" && eventsState === "ok" ? "완료" : "부분 실패";
    source.dataset.detailSeverity = snapshotState === "ok" && eventsState === "ok" ? "normal" : "warning";
    source.dataset.detailSnapshot = snapshotState;
    source.dataset.detailEvents = eventsState;
    source.dataset.detailNote = "pool reconcile, distributor tick, queue mutation은 호출하지 않습니다.";
    openDetailDrawer(source, { trigger });
  };

  const createEventRow = (event) => {
    const row = document.createElement("button");
    row.type = "button";
    row.className = `event-row ${severityClass(event.severity)}`;
    row.dataset.eventSequence = String(event.sequence || "");
    row.dataset.projectionKind = event.projectionKind || "";
    row.dataset.projectionKey = event.projectionKey || "";
    row.dataset.detailType = "event";
    row.dataset.detailTitle = `이벤트 #${event.sequence || "-"}`;
    row.dataset.detailSubtitle = `${event.eventKind || "runtime_event"} / rev ${event.observedPlanningRevision ?? "-"}`;
    row.dataset.detailState = event.severity || "normal";
    row.dataset.detailSeverity = event.severity || "normal";
    row.dataset.detailTime = event.recordedAt || "-";
    row.dataset.detailProjection = `${event.projectionKind || ""}:${event.projectionKey || ""}`;
    row.dataset.detailSummary = event.summary || "";
    row.title = `${event.recordedAt || ""} / ${event.summary || ""}`;

    const icon = document.createElement("span");
    icon.className = "event-icon";
    icon.textContent = event.icon || "E";
    row.appendChild(icon);

    const body = document.createElement("div");
    const kind = document.createElement("strong");
    kind.textContent = event.eventKind || "runtime_event";
    const meta = document.createElement("small");
    meta.textContent = `${event.recordedAt || "-"} · ${event.projectionKey || "-"}`;
    const summary = document.createElement("small");
    summary.textContent = event.summary || "";
    body.append(kind, meta, summary);
    row.appendChild(body);
    initializeDetailControl(row);
    return row;
  };

  const removeEmptyEventCopy = () => {
    root.querySelector("[data-event-empty]")?.remove();
  };

  const trimEventRows = () => {
    const rows = [...root.querySelectorAll("[data-event-sequence]")];
    for (const row of rows.slice(eventLimit)) row.remove();
  };

  const replaceEventRows = (events) => {
    if (!eventList) return;
    for (const row of eventList.querySelectorAll("[data-event-sequence]")) row.remove();
    removeEmptyEventCopy();
    if (events.length === 0) {
      const empty = createText("p", "", "표시할 이벤트가 없습니다.");
      empty.dataset.eventEmpty = "";
      eventList.appendChild(empty);
    } else {
      eventList.append(...events.map(createEventRow));
    }
    trimEventRows();
    setEventStatus();
    syncSelectedDetail();
  };

  const prependEventRows = (events) => {
    if (!eventList || events.length === 0) return;
    removeEmptyEventCopy();
    const existing = new Set([...eventList.querySelectorAll("[data-event-sequence]")].map((row) => row.dataset.eventSequence));
    for (const event of [...events].reverse()) {
      const sequence = String(event.sequence || "");
      if (!sequence || existing.has(sequence)) continue;
      const row = createEventRow(event);
      row.classList.add("is-new");
      eventList.insertBefore(row, eventList.querySelector("[data-event-sequence]"));
      existing.add(sequence);
    }
    trimEventRows();
    setEventStatus();
    syncSelectedDetail();
  };

  const createCampaignLane = (lane) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = `campaign-lane ${severityClass(lane.severity)}`;
    setDataset(button, {
      detailType: "campaignLane",
      detailTitle: `시도 레인 · ${optionalText(lane.agentId)}`,
      detailSubtitle: `${optionalText(lane.classLabel)} / ${optionalText(lane.slotId)}`,
      detailState: lane.state,
      detailSeverity: lane.severity,
      detailAgent: lane.agentId,
      detailSlot: lane.slotId,
      detailTask: lane.taskTitle,
      detailProgress: lane.progressLabel,
      detailSummary: lane.summary,
      agentId: lane.agentId,
      slotId: lane.slotId
    });
    const body = document.createElement("div");
    body.append(
      createText("strong", "", `${optionalText(lane.agentId)} · ${optionalText(lane.classLabel)}`),
      createText("small", "", lane.taskTitle),
      createText("small", "", lane.summary)
    );
    button.append(body, createText("span", "score-chip", lane.state));
    initializeDetailControl(button);
    return button;
  };

  const renderCampaign = (dashboard) => {
    const campaign = dashboard.campaign || {};
    const lanes = asArray(campaign.laneCards);
    const laneBody = lanes.length > 0
      ? (() => {
          const list = document.createElement("div");
          list.className = "campaign-lanes";
          list.append(...lanes.map(createCampaignLane));
          return list;
        })()
      : createText("p", "panel-empty", "현재 수행 중인 레인이 없습니다.");
    updatePanel("#campaign", [
      panelTitle("활성 레인", `${formatValue(campaign.activeLaneCount, "0")} active`),
      laneBody
    ]);
  };

  const createSlotButton = (slot) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = `slot-card ${severityClass(slot.severity)}`;
    const slotDisplayLabel = optionalText(slot.displaySlotLabel || slot.slotId, "슬롯");
    const slotStateLabel = optionalText(slot.label || slot.bubbleLabel, "상태");
    const slotTaskId = optionalText(slot.taskId, "-");
    const slotBranchName = optionalText(slot.branchName, "-");
    const slotWorktreeLabel = optionalText(slot.worktreeLabel, "-");
    const slotOwnerLabel = optionalText(slot.ownerLabel, "-");
    const slotNote = optionalText(slot.note, "-");
    setDataset(button, {
      slotId: slot.slotId,
      taskId: slot.taskId || "",
      ownerAgentId: slot.ownerAgentId || "",
      ownerSessionKey: slot.ownerSessionKey || "",
      leaseGeneration: slot.leaseGeneration || "",
      detailType: "slot",
      detailTitle: `워크트리 풀 · ${slotDisplayLabel}`,
      detailSubtitle: slotStateLabel,
      detailState: slotStateLabel,
      detailSeverity: slot.severity,
      detailSlot: slotDisplayLabel,
      detailTask: slotTaskId,
      detailBranch: slotBranchName,
      detailWorktree: slotWorktreeLabel,
      detailOwner: slotOwnerLabel,
      detailOwnerAgent: optionalText(slot.ownerAgentId, "-"),
      detailOwnerSession: optionalText(slot.ownerSessionKey, "-"),
      detailLeaseGeneration: optionalText(slot.leaseGeneration, "-"),
      detailNote: slotNote
    });
    button.title = `${slotDisplayLabel} · ${slotStateLabel} · task ${slotTaskId} · branch ${slotBranchName} · worktree ${slotWorktreeLabel} · owner ${slotOwnerLabel} · note ${slotNote}`;
    const stationIcon = document.createElement("span");
    stationIcon.className = "station-icon";
    stationIcon.setAttribute("aria-hidden", "true");
    const body = document.createElement("div");
    body.append(
      createText("strong", "", slotDisplayLabel),
      createText("small", "", slotTaskId),
      createText("small", "slot-state", slotStateLabel)
    );
    button.append(stationIcon, body);
    initializeDetailControl(button);
    return button;
  };

  const renderPool = (pool) => {
    const panel = root.querySelector("#pool");
    if (!panel || !pool) return;
    panel.replaceChildren(
      panelTitle("워크트리 풀", ""),
      ...asArray(pool.slots).map(createSlotButton)
    );
  };

  const actorDetailDataset = (actor) => ({
    characterId: actor.actorId,
    presenceKind: "active",
    actorId: actor.actorId,
    agentId: actor.agentId,
    slotId: actor.slotId,
    taskId: actor.taskId,
    leaseGeneration: actor.leaseGeneration || "",
    sceneSeatIndex: actor.seatIndex,
    archetypeKey: actor.archetypeKey,
    visualState: actor.visualState,
    staticPose: actor.staticPose,
    detailType: "agent",
    detailTitle: `${optionalText(actor.displayName)} · ${optionalText(actor.roleLabel)}`,
    detailSubtitle: optionalText(actor.lifecycleState),
    detailState: optionalText(actor.statusLabel),
    detailSeverity: actor.severity,
    detailSlot: actor.slotId,
    detailTask: `${optionalText(actor.taskId)} · ${optionalText(actor.taskTitle)}`,
    detailBranch: actor.branchName,
    detailRuntime: actor.durationLabel,
    detailProgress: actor.progressLabel,
    detailSummary: actor.latestSummary
  });

  const standbyPresenceDataset = (character) => ({
    standbyCharacter: "true",
    characterId: character.characterId,
    presenceKind: character.presenceKind || "configured_standby",
    agentId: character.agentId,
    sceneStandbyIndex: character.locationIndex,
    archetypeKey: character.archetypeKey,
    visualState: character.visualState,
    staticPose: character.staticPose,
    detailSeverity: character.severity
  });

  const createStandbyPresence = (character) => {
    const presence = document.createElement("span");
    presence.className = "scene-standby-presence";
    presence.hidden = true;
    presence.setAttribute("aria-hidden", "true");
    setDataset(presence, standbyPresenceDataset(character));
    return presence;
  };

  const createActorButton = (actor) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = `scene-object desk agent-${Number(actor.seatIndex) || 1} ${severityClass(actor.severity)}`;
    setDataset(button, actorDetailDataset(actor));
    button.title = `${optionalText(actor.displayName)} · ${optionalText(actor.statusLabel)} · ${optionalText(actor.taskTitle)} · ${optionalText(actor.branchName)}`;
    const sprite = document.createElement("span");
    sprite.className = `agent-sprite ${agentAvatarClass(actor.archetypeKey)}`;
    sprite.setAttribute("aria-hidden", "true");
    sprite.appendChild(createText("span", "sprite-label", actor.displayName));
    const label = document.createElement("span");
    label.className = "object-label";
    label.append(
      createText("strong", "", actor.displayName),
      createText("span", "", actor.taskTitle),
      createText("small", "", `${optionalText(actor.statusLabel)} · ${optionalText(actor.durationLabel)}`)
    );
    button.append(createText("span", "speech", actor.bubbleLabel), sprite, label);
    initializeDetailControl(button);
    return button;
  };

  const createActorListButton = (actor) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = `scene-actor-row ${severityClass(actor.severity)}`;
    setDataset(button, actorDetailDataset(actor));
    button.append(
      createText("strong", "", actor.displayName),
      createText("span", "", `${optionalText(actor.statusLabel)} · ${optionalText(actor.slotId)}`),
      createText("small", "", actor.taskTitle)
    );
    initializeDetailControl(button);
    return button;
  };

  const renderSceneDiagnostics = (diagnostics) => {
    const panel = root.querySelector("[data-scene-diagnostics]");
    if (!panel) return;
    const rows = asArray(diagnostics).map((diagnostic) => {
      const row = document.createElement("p");
      row.className = severityClass(diagnostic.severity);
      row.dataset.diagnosticCode = optionalText(diagnostic.code, "unknown");
      row.append(
        createText("strong", "", `표시 보류 · ${optionalText(diagnostic.code, "unknown")}`),
        createText("span", "", diagnostic.message)
      );
      return row;
    });
    panel.replaceChildren(...rows);
    panel.hidden = rows.length === 0;
  };

  const renderActors = (scene) => {
    const board = root.querySelector(".office-board");
    if (!board || !scene) return;
    const nextSignature = JSON.stringify({
      stations: asArray(scene.stations),
      actors: asArray(scene.actors),
      standbyProfileCount: scene.standbyProfileCount,
      standbyCharacters: asArray(scene.standbyCharacters),
      diagnostics: asArray(scene.diagnostics),
      validation: scene.validation || null
    });
    if (root.dataset.sceneSignature === nextSignature) return;
    for (const node of board.querySelectorAll(".desk[data-actor-id], [data-standby-character]")) node.remove();
    const anchor = board.querySelector(".distributor-desk") || board.querySelector(".event-board");
    for (const actor of asArray(scene.actors)) board.insertBefore(createActorButton(actor), anchor);
    for (const character of asArray(scene.standbyCharacters)) {
      board.insertBefore(createStandbyPresence(character), anchor);
    }
    const standbyCharacters = asArray(scene.standbyCharacters);
    const standbyProfileCount = Number(scene.standbyProfileCount ?? standbyCharacters.length);
    const restArea = board.querySelector("[data-standby-rest-area]");
    if (restArea) {
      restArea.dataset.standbyCount = String(standbyCharacters.length);
      restArea.dataset.standbyTotalCount = String(standbyProfileCount);
      restArea.setAttribute(
        "aria-label",
        `대기 프로필 총 ${standbyProfileCount}명 중 ${standbyCharacters.length}명 표시 · 작업 미할당`
      );
      const speech = restArea.querySelector(".speech");
      if (speech) speech.textContent = standbyProfileCount > 0 ? "업무 배정 대기" : "대기 프로필 없음";
      const label = restArea.querySelector(".object-label strong");
      if (label) label.textContent = `대기 프로필 ${standbyCharacters.length}/${standbyProfileCount}`;
    }
    root.querySelector("[data-scene-actor-list]")?.replaceChildren(
      ...asArray(scene.actors).map(createActorListButton)
    );
    renderSceneDiagnostics(scene.diagnostics);
    root.dataset.sceneSignature = nextSignature;
  };

  const syncDistributorDesk = (distributor) => {
    const desk = root.querySelector(".distributor-desk");
    if (!desk || !distributor) return;
    setDataset(desk, {
      detailType: "distributor",
      detailTitle: "분배관 · Distributor",
      detailSubtitle: distributor.roleLabel,
      detailState: distributor.barrierState,
      detailSeverity: distributor.blockedReason ? "danger" : "normal",
      detailHead: distributor.headSummary,
      detailDepth: distributor.queueDepth,
      detailBarrier: distributor.barrierState,
      detailWorktree: distributor.integrationWorktreeReadiness,
      detailNote: distributor.note
    });
    desk.title = `${optionalText(distributor.roleLabel)} / ${optionalText(distributor.integrationWorktreeReadiness)}`;
    const speech = desk.querySelector(".speech");
    if (speech) speech.textContent = optionalText(distributor.bubbleLabel, "배포 파이프라인");
    const label = desk.querySelector(".object-label");
    if (label) {
      label.replaceChildren(
        createText("strong", "", "분배관"),
        createText("span", "", distributor.headSummary),
        createText("small", "", `queue ${formatValue(distributor.queueDepth, "0")}`)
      );
    }
  };

  const syncEventBoard = (dashboard) => {
    const board = root.querySelector(".event-board");
    if (!board) return;
    const feed = dashboard.eventFeed || {};
    setDataset(board, {
      detailType: "eventFeed",
      detailTitle: "실시간 이벤트",
      detailSubtitle: "Runtime event feed",
      detailState: "LIVE",
      detailSeverity: "normal",
      detailVisible: feed.visibleEventCount,
      detailTotal: feed.totalEventCount,
      detailNewest: feed.newestSequence || 0
    });
    board.title = `Runtime event feed / total ${formatValue(feed.totalEventCount, "0")}`;
    const label = board.querySelector(".object-label");
    if (label) {
      label.replaceChildren(
        createText("strong", "", "실시간 이벤트"),
        createText("span", "", `${formatValue(asArray(dashboard.events).length, "0")} visible`),
        createText("small", "", dashboard.workspace?.branch || "-")
      );
    }
  };

  const renderBoard = (dashboard) => {
    renderPool(dashboard.pool);
    renderActors(dashboard.scene);
    syncDistributorDesk(dashboard.distributor);
    syncEventBoard(dashboard);
  };

  const createPipelineStep = (step) => {
    const node = document.createElement("div");
    node.className = `step ${step.state || "waiting"}`;
    node.append(createText("strong", "", step.label), document.createElement("br"), createText("small", "", step.state));
    return node;
  };

  const createQueueItem = (item) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "queue-row";
    setDataset(button, {
      agentId: item.sourceAgent,
      detailType: "queueItem",
      detailTitle: `큐 작업 · ${optionalText(item.sourceAgent)}`,
      detailSubtitle: `${optionalText(item.queueState)} / ${optionalText(item.commitShortSha)}`,
      detailState: item.queueState,
      detailSeverity: "info",
      detailTask: item.taskTitle,
      detailAgent: item.sourceAgent,
      detailBranch: item.branchName,
      detailCommit: item.commitShortSha,
      detailNote: item.integrationNote
    });
    const body = document.createElement("div");
    body.append(
      createText("strong", "", item.taskTitle),
      createText("small", "", `${optionalText(item.branchName)} · ${optionalText(item.commitShortSha)}`)
    );
    button.append(
      createText("span", "agent-token", item.sourceAgent),
      body,
      createText("span", "queue-state", item.queueState)
    );
    initializeDetailControl(button);
    return button;
  };

  const renderPipeline = (distributor) => {
    if (!distributor) return;
    const pipeline = document.createElement("div");
    pipeline.className = "pipeline";
    pipeline.append(...asArray(distributor.pipeline).map(createPipelineStep));
    const children = [panelTitle("배포 파이프라인", distributor.barrierState), pipeline];
    const queueItems = asArray(distributor.queueItems);
    if (queueItems.length === 0) {
      children.push(createText("p", "", distributor.headSummary));
    } else {
      children.push(...queueItems.map(createQueueItem));
    }
    updatePanel("#pipeline", children);
  };

  const validationStageClass = (state) => {
    if (["verified", "complete", "succeeded", "integrated"].includes(state)) return "is-complete";
    if (["blocked", "failed", "actionable_failure", "policy_blocked", "missing"].includes(state)) return "is-danger";
    if (["remediation_queued", "remediation_running", "stale", "paused"].includes(state)) return "is-warning";
    return "is-active";
  };

  const createValidationStage = (state, label) => {
    const stage = createText("span", `validation-stage ${validationStageClass(state)}`, label);
    stage.dataset.validationStage = state || "unknown";
    return stage;
  };

  const createValidationCell = (label, stage, meta) => {
    const cell = document.createElement("span");
    cell.className = "validation-cell";
    cell.dataset.label = label;
    cell.append(stage, createText("small", "", meta));
    return cell;
  };

  const createValidationRecord = (record) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = `validation-record ${severityClass(record.severity)}`;
    setDataset(button, {
      validationRecordKey: record.recordKey,
      validationRevision: record.observationRevision,
      detailType: "validation",
      detailTitle: `PR #${record.pullRequestNumber} · ${optionalText(record.akraId)}`,
      detailSubtitle: `${optionalText(record.phaseLabel)} · ${optionalText(record.targetShortSha)}`,
      detailState: record.phaseLabel,
      detailSeverity: record.severity
    });
    button.setAttribute(
      "aria-label",
      `PR ${record.pullRequestNumber}, ${optionalText(record.phaseLabel)}, 검증 상세 보기`
    );

    const identity = document.createElement("span");
    identity.className = "validation-cell";
    identity.dataset.label = "PR / Akra ID";
    identity.append(
      createText("strong", "", `#${record.pullRequestNumber} · ${optionalText(record.akraId)}`),
      createText("small", "", `${optionalText(record.repository)} · ${optionalText(record.targetShortSha)}`)
    );

    const integratedState = record.integrated ? "integrated" : "pending";
    const actionsState = record.verified ? "succeeded" : record.phase;
    const findingsState = Number(record.findingCount) > Number(record.remediationCount)
      ? "actionable_failure"
      : "succeeded";
    const remediationState = ["remediation_queued", "remediation_running"].includes(record.phase)
      ? record.phase
      : "complete";
    const verifiedState = record.verified ? "verified" : record.phase;
    const latestRequiredAttempt = Number(record.latestRequiredAttempt) || 0;

    button.append(
      identity,
      createValidationCell(
        "Integrated",
        createValidationStage(integratedState, record.integrated ? "증거 확인" : "대기"),
        optionalText(record.evidenceShortSha, "evidence 없음")
      ),
      createValidationCell(
        "Actions",
        createValidationStage(actionsState, `${Number(record.requiredChecksSucceeded) || 0}/${Number(record.requiredChecksTotal) || 0} required`),
        `latest attempt ${latestRequiredAttempt}`
      ),
      createValidationCell(
        "Findings",
        createValidationStage(findingsState, `${Number(record.findingCount) || 0} finding`),
        record.stale ? `stale ${Number(record.staleSeconds) || 0}s` : "관측 최신"
      ),
      createValidationCell(
        "Remediation",
        createValidationStage(remediationState, `${Number(record.remediationCount) || 0} correlated`),
        `${Number(record.correlationCount) || 0} worker link`
      ),
      createValidationCell(
        "Verified",
        createValidationStage(verifiedState, optionalText(record.phaseLabel)),
        record.paused ? "paused" : `rev ${Number(record.observationRevision) || 0}`
      )
    );
    initializeDetailControl(button);
    return button;
  };

  const evidenceStatusView = (status) => ({
    ready: { mark: "OK", label: "READY · 사용 가능" },
    hold: { mark: "HOLD", label: "HOLD · 관찰 필요" },
    stale: { mark: "OLD", label: "STALE · 갱신 필요" },
    unavailable: { mark: "N/A", label: "UNAVAILABLE · 수집 불가" },
    invalid: { mark: "ERR", label: "INVALID · 증거 거부" }
  }[status] || { mark: "N/A", label: "UNAVAILABLE · 상태 미상" });

  const metricLabelText = (label) => ({
    projected: "PROJECTED",
    mixed_actual_and_projected: "MIXED",
    actual: "ACTUAL",
    unavailable: "UNAVAILABLE"
  }[label] || "UNAVAILABLE");

  const evidenceNumber = (value) => {
    if (value === null || value === undefined || value === "") return null;
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : null;
  };

  const formatEvidenceSeconds = (value) => {
    const parsed = evidenceNumber(value);
    return parsed === null
      ? "미수집"
      : `${new Intl.NumberFormat("ko-KR", { maximumFractionDigits: 1 }).format(parsed)}초`;
  };

  const formatEvidenceTimestamp = (value) => {
    if (!value) return "미수집";
    const parsed = new Date(value);
    if (Number.isNaN(parsed.getTime())) return "미수집";
    return new Intl.DateTimeFormat("ko-KR", {
      dateStyle: "medium",
      timeStyle: "medium"
    }).format(parsed);
  };

  const evidenceSourceLabel = (source) => ({
    debug_fixture_historical_projection: "Historical rollout projection",
    debug_fixture_historical_with_actual_row: "Historical distribution + actual row",
    debug_fixture_independent_actual_run: "Independent actual Fast Gate run",
    debug_fixture_actual_not_collected: "Actual Fast Gate not collected",
    debug_fixture_ci_gate: "GitHub Actions CI Gate",
    debug_fixture_post_merge_gate: "GitHub Actions Post-Merge Gate",
    debug_fixture_rollout_window: "Merged PR sample window",
    debug_fixture_partial_pagination: "Incomplete provider page window",
    debug_fixture_previous_window: "Previous valid sample window"
  }[source] || optionalText(source, "출처 미수집").replaceAll("_", " "));

  const renderEvidenceMetric = (key, metric, generatedAt) => {
    const card = root.querySelector(`[data-evidence-metric="${key}"]`);
    if (!card) return;
    const snapshot = metric || {};
    const label = optionalText(snapshot.label, "unavailable");
    const labelNode = card.querySelector("[data-metric-label]");
    if (labelNode) {
      labelNode.textContent = metricLabelText(label);
      labelNode.className = `evidence-metric-label is-${label}`;
    }
    const sampleCount = evidenceNumber(snapshot.sampleCount);
    const hasSamples = sampleCount !== null && sampleCount >= 0;
    const p50 = card.querySelector("[data-metric-p50]");
    const p95 = card.querySelector("[data-metric-p95]");
    const samples = card.querySelector("[data-metric-samples]");
    const source = card.querySelector("[data-metric-source]");
    const generated = card.querySelector("[data-metric-generated]");
    if (p50) p50.textContent = formatEvidenceSeconds(snapshot.p50Seconds);
    if (p95) p95.textContent = formatEvidenceSeconds(snapshot.p95Seconds);
    if (samples) samples.textContent = hasSamples ? `표본 ${sampleCount}` : "표본 미수집";
    if (source) {
      source.textContent = evidenceSourceLabel(snapshot.source);
      source.title = optionalText(snapshot.source, "출처 미수집");
    }
    if (generated) generated.textContent = formatEvidenceTimestamp(generatedAt);
    card.classList.toggle("is-unavailable", label === "unavailable");
  };

  const normalizeSchedulerMode = (value) => {
    if (value === "remediation") return "remediate";
    if (value === "shadow") return "observe";
    return optionalText(value, "off");
  };

  const renderEvidenceAttention = (evidence, rollout) => {
    const attention = root.querySelector("[data-evidence-attention]");
    if (!attention) return;
    const status = optionalText(evidence?.status, "unavailable");
    const currentMode = normalizeSchedulerMode(rollout?.stage);
    const recommendedMode = optionalText(evidence?.recommendedSchedulerMode, "observe");
    const mismatch = status !== "ready" || currentMode !== recommendedMode;
    attention.hidden = !mismatch;
    if (!mismatch) return;
    const blocker = asArray(evidence?.blockers)[0];
    const statusCopy = {
      hold: "관찰 조건이 아직 충족되지 않았습니다.",
      stale: "evidence freshness window가 지났습니다.",
      unavailable: "최신 evidence를 읽을 수 없습니다.",
      invalid: "schema 또는 SHA 검증에서 evidence가 거부되었습니다."
    }[status];
    const modeCopy = currentMode !== recommendedMode
      ? `현재 ${currentMode} · 권장 ${recommendedMode}`
      : "현재 mode는 유지됩니다.";
    setText("[data-evidence-attention-title]", status === "ready" ? "운영 mode 불일치" : "자동 전환 보류");
    setText("[data-evidence-attention-body]", blocker || statusCopy || "evidence 상태를 확인하세요.");
    setText(
      "[data-evidence-attention-action]",
      `${modeCopy} · 자동 전환하지 않습니다. evidence 갱신 후 운영자가 다시 확인하세요.`
    );
  };

  const renderValidationEvidence = (evidence, rollout) => {
    currentValidationEvidence = evidence || null;
    const status = optionalText(evidence?.status, "unavailable");
    const statusView = evidenceStatusView(status);
    const shell = root.querySelector("[data-validation-evidence]");
    const statusNode = root.querySelector("[data-evidence-status]");
    if (shell) shell.dataset.validationEvidenceState = status;
    if (statusNode) {
      statusNode.dataset.state = status;
      statusNode.className = `evidence-status is-${status}`;
    }
    setText("[data-evidence-status-mark]", statusView.mark);
    setText("[data-evidence-status-label]", statusView.label);
    setText(
      "[data-evidence-generated-at]",
      `Evidence 생성 ${formatEvidenceTimestamp(evidence?.generatedAt)}`
    );
    renderEvidenceAttention(evidence, rollout);
    renderEvidenceMetric("historicalFastGate", evidence?.historicalFastGate, evidence?.generatedAt);
    renderEvidenceMetric("actualFastGate", evidence?.actualFastGate, evidence?.generatedAt);
    renderEvidenceMetric("ciGate", evidence?.ciGate, evidence?.generatedAt);
    renderEvidenceMetric("postMergeGate", evidence?.postMergeGate, evidence?.generatedAt);
  };

  const renderValidationRail = (validation) => {
    if (!validation) return;
    setText("[data-validation-mode]", optionalText(validation.schedulerMode, "observe"));
    const rollout = validation.rollout || {};
    const rolloutStage = optionalText(rollout.stage, "shadow");
    const rolloutPanel = root.querySelector("[data-validation-rollout]");
    if (rolloutPanel) {
      rolloutPanel.dataset.rolloutStage = rolloutStage;
      rolloutPanel.classList.toggle("is-off", rolloutStage === "off");
      rolloutPanel.classList.toggle("is-shadow", rolloutStage === "shadow");
      rolloutPanel.classList.toggle("is-remediation", rolloutStage === "remediation");
    }
    setText("[data-validation-rollout-status]", optionalText(rollout.statusLabel, "SHADOW · Queue 차단"));
    setText("[data-validation-rollout-detail]", optionalText(rollout.detail, "GitHub evidence를 관찰합니다."));
    setText(
      "[data-validation-rollout-admission]",
      rollout.queueAdmissionEnabled ? "QUEUE ADMISSION ON" : "QUEUE ADMISSION BLOCKED"
    );
    setText(
      "[data-validation-rollout-ruleset]",
      rollout.rulesetChangeRequiresApproval === false ? "RULESET MANAGED" : "RULESET · APPROVAL REQUIRED"
    );
    renderValidationEvidence(validation.rolloutEvidence || null, rollout);
    setText("[data-validation-count]", String(asArray(validation.records).length));
    const list = root.querySelector("[data-validation-list]");
    if (!list) return;

    const head = document.createElement("div");
    head.className = "validation-rail-head";
    head.setAttribute("aria-hidden", "true");
    for (const label of ["PR / Akra ID", "Integrated", "Actions", "Findings", "Remediation", "Verified"]) {
      head.appendChild(createText("span", "", label));
    }
    const records = asArray(validation.records);
    if (records.length === 0) {
      const empty = createText("p", "validation-empty", "관찰 중인 PR 검증 레코드가 없습니다.");
      empty.dataset.validationEmpty = "true";
      list.replaceChildren(head, empty);
      return;
    }
    list.replaceChildren(head, ...records.map(createValidationRecord));
  };

  const validationDetailSection = (title, children) => {
    const section = document.createElement("section");
    section.className = "validation-detail-section";
    section.append(createText("h4", "", title), ...children);
    return section;
  };

  const validationDetailCard = (title, value, meta = "", kind = "") => {
    const card = document.createElement("div");
    card.className = `validation-detail-card${kind ? ` is-${kind}` : ""}`;
    if (kind) card.dataset.detailKind = kind;
    card.append(createText("strong", "", title), createText("span", "", optionalText(value)));
    if (String(meta || "").trim() !== "") card.append(createText("small", "", meta));
    return card;
  };

  const validationDetailGrid = (cards) => {
    const grid = document.createElement("div");
    grid.className = "validation-detail-grid";
    grid.append(...cards);
    return grid;
  };

  const statusLabel = (value) => optionalText(value).replaceAll("_", " ");

  const workflowSelectionCopy = (basis) => ({
    newest_run: "가장 늦게 생성된 run을 선택했습니다. attempt 번호는 서로 다른 run 사이에서 비교하지 않습니다.",
    latest_attempt: "같은 run 안에서 가장 높은 attempt를 선택했습니다.",
    deterministic_tie_break: "생성 시각이 같은 관측을 갱신 시각과 안정적인 provider 순서로 결정했습니다.",
    legacy_unknown: "legacy 기록이라 선택 근거를 복원할 수 없습니다. 현재 선택 상태만 보존합니다."
  }[basis] || "선택 근거가 제공되지 않았습니다.");

  const workflowNotSelectedCopy = (workflow, selected) => {
    if (!selected) return "선택된 run과 비교할 수 없습니다.";
    const selectedCreated = Date.parse(selected.createdAt || "");
    const candidateCreated = Date.parse(workflow.createdAt || "");
    if (Number.isFinite(selectedCreated) && Number.isFinite(candidateCreated) && selectedCreated > candidateCreated) {
      return `attempt ${workflow.runAttempt}이 더 높더라도 이전 run입니다. 새 run의 생성 시각이 더 늦어 선택되지 않았습니다.`;
    }
    if (selected.createdAt === workflow.createdAt && Number(selected.runAttempt) > Number(workflow.runAttempt)) {
      return `같은 run의 이전 attempt ${workflow.runAttempt}입니다. attempt ${selected.runAttempt}이 선택되었습니다.`;
    }
    return "동일 시각 후보보다 갱신 시각 또는 안정적인 tie-break 순서가 후순위라 선택되지 않았습니다.";
  };

  const renderWorkflowRow = (workflow, selected) => {
    const row = document.createElement("article");
    row.className = `validation-workflow${workflow.selected ? " is-selected" : ""}`;
    row.dataset.workflowSelected = workflow.selected ? "true" : "false";
    row.dataset.detailKind = "workflow";
    const head = document.createElement("div");
    head.className = "validation-workflow-head";
    head.append(
      createText("strong", "", `${optionalText(workflow.name)} · attempt ${workflow.runAttempt ?? "-"}`),
      createText(
        "span",
        "validation-workflow-state",
        workflow.selected ? `SELECTED · ${statusLabel(workflow.status)}` : `HISTORY · ${statusLabel(workflow.status)}`
      )
    );
    const times = document.createElement("div");
    times.className = "validation-workflow-times";
    for (const [label, value] of [
      ["Created", workflow.createdAt],
      ["Started", workflow.startedAt],
      ["Updated", workflow.updatedAt]
    ]) {
      const item = document.createElement("span");
      item.append(createText("b", "", label), document.createTextNode(formatEvidenceTimestamp(value)));
      times.appendChild(item);
    }
    const reason = createText(
      "p",
      "validation-workflow-reason",
      workflow.selected
        ? workflowSelectionCopy(workflow.selectionBasis)
        : workflowNotSelectedCopy(workflow, selected)
    );
    row.append(head, times, reason);
    return row;
  };

  const validationEvidenceDetail = (evidence) => {
    const summary = evidence || {};
    const sampleWindow = summary.sampleWindow || {};
    const grid = validationDetailGrid([
      validationDetailCard(
        "Projected Fast Gate",
        `${formatEvidenceSeconds(summary.historicalFastGate?.p50Seconds)} p50 · ${formatEvidenceSeconds(summary.historicalFastGate?.p95Seconds)} p95`,
        `${metricLabelText(summary.historicalFastGate?.label)} · 표본 ${summary.historicalFastGate?.sampleCount ?? "미수집"} · ${evidenceSourceLabel(summary.historicalFastGate?.source)}`,
        "metric"
      ),
      validationDetailCard(
        "Actual Fast Gate",
        `${formatEvidenceSeconds(summary.actualFastGate?.p50Seconds)} p50 · ${formatEvidenceSeconds(summary.actualFastGate?.p95Seconds)} p95`,
        `${metricLabelText(summary.actualFastGate?.label)} · 표본 ${summary.actualFastGate?.sampleCount ?? "미수집"} · ${evidenceSourceLabel(summary.actualFastGate?.source)}`,
        "metric"
      ),
      validationDetailCard(
        "Sample window",
        sampleWindow.pullRequestCount == null ? "미수집" : `${sampleWindow.pullRequestCount} PR · ${sampleWindow.windowHours ?? "-"}h`,
        `${formatEvidenceTimestamp(sampleWindow.earliestMergedAt)} → ${formatEvidenceTimestamp(sampleWindow.latestMergedAt)} · ${evidenceSourceLabel(sampleWindow.source)}`,
        "metric"
      ),
      validationDetailCard(
        "Evidence state",
        evidenceStatusView(optionalText(summary.status, "unavailable")).label,
        `생성 ${formatEvidenceTimestamp(summary.generatedAt)} · SHA ${optionalText(summary.evidenceShortSha, "미수집")}`,
        "metric"
      )
    ]);
    return validationDetailSection("Rollout evidence · 출처와 표본", [grid]);
  };

  const canonicalGithubUrl = (value, kind) => {
    try {
      const url = new URL(value);
      if (url.protocol !== "https:" || url.hostname !== "github.com" || url.port || url.username || url.password || url.search || url.hash) {
        return null;
      }
      const pattern = kind === "run"
        ? /^\/[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+\/actions\/runs\/\d+(?:\/job\/\d+)?\/?$/
        : /^\/[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+\/pull\/\d+\/?$/;
      return pattern.test(url.pathname) ? url.href : null;
    } catch (_error) {
      return null;
    }
  };

  const renderValidationDetail = (record) => {
    const detail = document.createElement("div");
    detail.className = "validation-detail";
    detail.dataset.validationDetailRecordKey = record.recordKey;
    detail.dataset.validationDetailRevision = String(record.observationRevision ?? "");

    const prLink = document.createElement("a");
    const canonicalPrUrl = canonicalGithubUrl(record.canonicalPrUrl, "pull");
    prLink.href = canonicalPrUrl || "#";
    prLink.target = canonicalPrUrl ? "_blank" : "";
    prLink.rel = canonicalPrUrl ? "noreferrer" : "";
    prLink.textContent = canonicalPrUrl
      ? `PR #${record.pullRequestNumber} 열기`
      : `PR #${record.pullRequestNumber} 링크 검증 불가`;
    if (!canonicalPrUrl) {
      prLink.removeAttribute("href");
      prLink.setAttribute("aria-disabled", "true");
    }
    const identity = validationDetailGrid([
      validationDetailCard("Akra ID", record.akraId, record.repository),
      validationDetailCard("상태", record.phaseLabel, `${statusLabel(record.severity)} · rev ${record.observationRevision}`),
      validationDetailCard("Target / Evidence", record.targetShortSha, optionalText(record.evidenceShortSha, "evidence 없음")),
      validationDetailCard("Integration", record.integrationMethod, optionalText(record.integratedAt, "integration 미확인"))
    ]);
    detail.append(validationDetailSection("검증 증거", [identity, prLink]));

    const checks = asArray(record.checks);
    const checksGrid = validationDetailGrid(checks.length > 0
      ? checks.map((check) => validationDetailCard(
        `${check.required ? "필수" : "선택"} · ${check.context}`,
        statusLabel(check.status),
        `attempt ${check.latestAttempt ?? "-"} · ${optionalText(check.appSlug, "provider 미상")}`,
        "check"
      ))
      : [validationDetailCard("Checks", "관측 없음", "", "check")]);
    detail.append(validationDetailSection("Required / Optional Checks", [checksGrid]));

    const workflows = asArray(record.workflows);
    const workflowList = document.createElement("div");
    workflowList.className = "validation-workflow-list";
    const selectedWorkflow = workflows.find((workflow) => workflow.selected) || null;
    workflowList.append(...(workflows.length > 0
      ? workflows.map((workflow) => renderWorkflowRow(workflow, selectedWorkflow))
      : [validationDetailCard("Workflow attempts", "관측 없음", "", "workflow")]));
    detail.append(validationDetailSection("Workflow run 선택", [workflowList]));
    detail.append(validationEvidenceDetail(currentValidationEvidence));

    const providers = asArray(record.providers);
    const providerCards = providers.map((provider) => validationDetailCard(
      provider.key,
      `${statusLabel(provider.lifecycle)} · ${statusLabel(provider.status)}`,
      provider.pageComplete ? "pagination complete" : "pagination incomplete"
    ));
    providerCards.push(validationDetailCard(
      "Poll / Backoff",
      `attempt ${record.schedule?.pollAttempt ?? 0} · errors ${record.schedule?.consecutiveErrorCount ?? 0}`,
      `next ${optionalText(record.schedule?.nextPollAt, "없음")} · rate ${record.schedule?.rateLimitRemaining ?? "-"}`
    ));
    detail.append(validationDetailSection("Provider / Retry", [validationDetailGrid(providerCards)]));

    const timeline = asArray(record.timeline);
    const timelineList = document.createElement("div");
    timelineList.className = "validation-detail-grid";
    timelineList.append(...(timeline.length > 0
      ? timeline.map((entry) => validationDetailCard(
        entry.label,
        statusLabel(entry.state),
        `${optionalText(entry.occurredAt, "시간 미상")} · attempt ${entry.attempt ?? "-"}`
      ))
      : [validationDetailCard("Timeline", "관측 없음")]));
    detail.append(validationDetailSection("검증 Timeline", [timelineList]));

    const correlations = asArray(record.correlations);
    const correlationGrid = validationDetailGrid(correlations.length > 0
      ? correlations.map((correlation) => validationDetailCard(
        correlation.remediationAkraId,
        correlation.finding,
        correlation.slotId
          ? `${correlation.slotId} · ${optionalText(correlation.workerState, correlation.taskState)}`
          : `queue · ${optionalText(correlation.taskState, "대기")}`
      ))
      : [validationDetailCard("Worker correlation", "실제 remediation lease 없음", "passive CI에는 요원을 만들지 않습니다.")]);
    detail.append(validationDetailSection("Finding / Remediation Correlation", [correlationGrid]));

    const commandList = document.createElement("div");
    commandList.className = "validation-command-list";
    for (const command of asArray(record.commands)) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "validation-command";
      button.dataset.validationCommand = command.action;
      button.dataset.validationRecordKey = record.recordKey;
      button.dataset.validationRevision = String(record.observationRevision);
      button.textContent = command.label;
      button.disabled = !command.enabled;
      if (command.disabledReason) {
        button.title = command.disabledReason;
        button.setAttribute("aria-describedby", `validation-command-status-${record.observationRevision}`);
      }
      commandList.appendChild(button);
    }
    const commandFeedback = validationCommandFeedback?.recordKey === record.recordKey
      ? validationCommandFeedback
      : null;
    const commandStatus = createText(
      "p",
      "validation-command-status",
      commandFeedback?.text || (
        record.paused
          ? "검증 polling이 운영자에 의해 일시정지되었습니다."
          : `scheduler ${optionalText(record.schedulerMode, root.querySelector("[data-validation-mode]")?.textContent || "observe")} · optimistic rev ${record.observationRevision}`
      )
    );
    commandStatus.id = `validation-command-status-${record.observationRevision}`;
    commandStatus.dataset.validationCommandStatus = "true";
    if (commandFeedback?.state) commandStatus.dataset.validationCommandOutcome = commandFeedback.state;
    commandStatus.setAttribute("role", "status");
    commandStatus.setAttribute("aria-live", "polite");
    detail.append(validationDetailSection("운영 명령", [commandList, commandStatus]));
    return detail;
  };

  const renderEvidenceHistoryPage = (page) => {
    const latest = page?.latest || {};
    const summary = latest.summary || currentValidationEvidence || {};
    const detail = document.createElement("div");
    detail.className = "validation-detail evidence-detail";
    detail.dataset.evidenceDetail = "true";
    const statusView = evidenceStatusView(optionalText(summary.status, "unavailable"));
    const overview = document.createElement("div");
    overview.className = "evidence-detail-summary";
    overview.append(
      createText("strong", "", statusView.label),
      createText(
        "small",
        "",
        `${optionalText(summary.repository, "repository 미수집")} · ${optionalText(summary.baseBranch, "base 미수집")} · evidence ${optionalText(summary.evidenceShortSha, "미수집")}`
      ),
      createText(
        "small",
        "",
        `생성 ${formatEvidenceTimestamp(summary.generatedAt)} · 관측 ${formatEvidenceTimestamp(latest.observedAt)} · 권장 mode ${optionalText(summary.recommendedSchedulerMode, "미정")}`
      )
    );
    detail.append(validationDetailSection("최신 evidence", [overview]));
    detail.append(validationEvidenceDetail(summary));

    const blockers = asArray(summary.blockers);
    const blockerGrid = validationDetailGrid(blockers.length > 0
      ? blockers.map((blocker, index) => validationDetailCard(
        `Blocker ${index + 1}`,
        blocker,
        "자동 mode 전환에는 사용하지 않습니다.",
        "check"
      ))
      : [validationDetailCard("Blocker", "없음", "현재 snapshot 기준", "check")]);
    detail.append(validationDetailSection("판정과 안전 조치", [blockerGrid]));

    const historyList = document.createElement("div");
    historyList.className = "evidence-history-list";
    historyList.append(...asArray(page?.history).map((snapshot) => {
      const row = document.createElement("article");
      row.className = "evidence-history-row";
      row.dataset.evidenceHistoryRow = "true";
      row.append(
        createText("strong", "", evidenceStatusView(optionalText(snapshot.summary?.status, "unavailable")).label),
        createText("small", "", `생성 ${formatEvidenceTimestamp(snapshot.summary?.generatedAt)} · 관측 ${formatEvidenceTimestamp(snapshot.observedAt)}`),
        createText("code", "", optionalText(snapshot.artifactShortSha, "sha 미수집"))
      );
      return row;
    }));
    if (!historyList.childElementCount) {
      historyList.append(validationDetailCard("Evidence history", "관측 없음"));
    }
    if (page?.nextCursor) {
      historyList.append(createText("small", "validation-workflow-reason", "표시 범위 밖의 이전 snapshot이 있습니다. API cursor로 계속 조회할 수 있습니다."));
    }
    const lastValid = page?.lastValid;
    if (lastValid && latest.artifactShortSha !== lastValid.artifactShortSha) {
      historyList.prepend(createText(
        "p",
        "validation-workflow-reason",
        `최신 snapshot은 사용할 수 없습니다. 마지막 유효 evidence: ${formatEvidenceTimestamp(lastValid.summary?.generatedAt)} · ${optionalText(lastValid.artifactShortSha, "sha 미수집")}`
      ));
    }
    detail.append(validationDetailSection("Bounded evidence history", [historyList]));

    const canaryUrl = canonicalGithubUrl(latest.productionSuccessCanary?.runUrl, "run");
    if (canaryUrl) {
      const canaryLink = document.createElement("a");
      canaryLink.href = canaryUrl;
      canaryLink.target = "_blank";
      canaryLink.rel = "noreferrer";
      canaryLink.textContent = `Production canary PR #${latest.productionSuccessCanary.pullRequestNumber} 실행 열기`;
      detail.append(validationDetailSection("검증된 외부 링크", [canaryLink]));
    }
    return detail;
  };

  const openEvidenceDetailDrawer = () => {
    if (!evidenceDetailTrigger || !detailDrawer || !detailDrawerBody) return;
    detailTrigger = evidenceDetailTrigger;
    const requestSequence = ++evidenceDetailRequestSequence;
    validationDetailRequestSequence += 1;
    detailDrawer.dataset.detailMode = "evidence";
    detailDrawerTitle.textContent = "Rollout evidence";
    detailDrawerSubtitle.textContent = "bounded history · application projection";
    const loading = createText("p", "validation-empty", "검증 evidence 이력을 불러오는 중…");
    loading.setAttribute("role", "status");
    detailDrawerBody.replaceChildren(loading);
    detailDrawer.hidden = false;
    detailDrawer.setAttribute("aria-hidden", "false");
    evidenceDetailTrigger.setAttribute("aria-expanded", "true");
    window.requestAnimationFrame(() => {
      detailDrawer.classList.add("is-open");
      detailDrawer.focus({ preventScroll: true });
    });
    fetch(`${validationEvidenceUrl}?limit=10`, { headers: { "Accept": "application/json" } })
      .then(async (response) => {
        if (!response.ok) throw new Error(`evidence history ${response.status}`);
        return response.json();
      })
      .then((page) => {
        if (requestSequence !== evidenceDetailRequestSequence) return;
        detailDrawerBody.replaceChildren(renderEvidenceHistoryPage(page));
      })
      .catch((error) => {
        if (requestSequence !== evidenceDetailRequestSequence) return;
        const failure = createText(
          "p",
          "validation-empty severity-danger",
          `Evidence 이력을 불러오지 못했습니다 · ${error.message}. 대시보드의 마지막 summary는 유지됩니다.`
        );
        failure.setAttribute("role", "alert");
        detailDrawerBody.replaceChildren(failure);
      });
  };

  openValidationDetailDrawer = (
    source,
    { focusDrawer = true, rememberTrigger = true, trigger = source } = {}
  ) => {
    if (!source || !detailDrawer || !detailDrawerBody) return;
    if (rememberTrigger) detailTrigger = trigger?.isConnected ? trigger : null;
    evidenceDetailTrigger?.setAttribute("aria-expanded", "false");
    const recordKey = source.dataset.validationRecordKey;
    const requestSequence = ++validationDetailRequestSequence;
    detailDrawer.dataset.detailMode = "validation";
    detailDrawer.dataset.validationRecordKey = recordKey || "";
    detailDrawerTitle.textContent = source.dataset.detailTitle || "PR 검증 상세";
    detailDrawerSubtitle.textContent = source.dataset.detailSubtitle || "authoritative validation projection";
    const loading = createText("p", "validation-empty", "검증 상세를 불러오는 중…");
    loading.setAttribute("role", "status");
    detailDrawerBody.replaceChildren(loading);
    detailDrawer.hidden = false;
    detailDrawer.setAttribute("aria-hidden", "false");
    window.requestAnimationFrame(() => {
      detailDrawer.classList.add("is-open");
      if (focusDrawer) detailDrawer.focus({ preventScroll: true });
    });
    setSelectedDetail(source);
    markRelated(source);

    fetch(`${validationBaseUrl}/${encodeURIComponent(recordKey)}`, {
      headers: { "Accept": "application/json" }
    })
      .then(async (response) => {
        if (!response.ok) throw new Error(`validation detail ${response.status}`);
        return response.json();
      })
      .then((record) => {
        if (requestSequence !== validationDetailRequestSequence) return;
        detailDrawerTitle.textContent = `PR #${record.pullRequestNumber} · ${optionalText(record.akraId)}`;
        detailDrawerSubtitle.textContent = `${optionalText(record.phaseLabel)} · ${optionalText(record.targetShortSha)} · rev ${record.observationRevision}`;
        detailDrawerBody.replaceChildren(renderValidationDetail(record));
      })
      .catch((error) => {
        if (requestSequence !== validationDetailRequestSequence) return;
        detailDrawerBody.replaceChildren(createText("p", "validation-empty severity-danger", `상세 조회 실패 · ${error.message}`));
      });
  };

  const validationCommandId = () => {
    if (globalThis.crypto?.randomUUID) return `admin-${globalThis.crypto.randomUUID()}`;
    return `admin-${Date.now()}-${Math.random().toString(16).slice(2)}`;
  };

  const runValidationCommand = (button) => {
    if (!button || button.disabled || validationCommandRequest) return validationCommandRequest;
    const recordKey = button.dataset.validationRecordKey;
    const expectedRevision = Number(button.dataset.validationRevision);
    const action = button.dataset.validationCommand;
    const commandId = button.dataset.validationCommandId || validationCommandId();
    button.dataset.validationCommandId = commandId;
    validationCommandFeedback = null;
    const status = detailDrawerBody?.querySelector("[data-validation-command-status]");
    const commandButtons = [
      ...(detailDrawerBody?.querySelectorAll("[data-validation-command]") || [])
    ];
    const priorDisabledStates = new Map(
      commandButtons.map((command) => [command, command.disabled])
    );
    for (const command of commandButtons) {
      command.disabled = true;
    }
    if (status) status.textContent = `${button.textContent} 명령 전송 중…`;

    validationCommandRequest = fetch(
      `${validationBaseUrl}/${encodeURIComponent(recordKey)}/commands`,
      {
        method: "POST",
        headers: {
          "Accept": "application/json",
          "Content-Type": "application/json",
          "X-CSRF-Token": csrfToken
        },
        body: JSON.stringify({ commandId, action, expectedRevision })
      }
    )
      .then(async (response) => {
        const result = await response.json().catch(() => null);
        if (!result) throw new Error(`validation command ${response.status}`);
        const outcome = response.ok ? "적용" : `거절 · ${statusLabel(result.rejection)}`;
        const feedbackText = `${outcome} · ${optionalText(result.message)}${result.duplicate ? " · duplicate replay" : ""}`;
        validationCommandFeedback = {
          recordKey,
          state: response.ok ? "applied" : `rejected:${optionalText(result.rejection, "unknown")}`,
          text: feedbackText
        };
        if (status) status.textContent = feedbackText;
        delete button.dataset.validationCommandId;
        return Promise.allSettled([
          pollDashboard({ fresh: true }),
          pollEvents({ fresh: true })
        ]).then(() => result);
      })
      .then((result) => {
        const currentSource = [...root.querySelectorAll("[data-validation-record-key][data-detail-type='validation']")]
          .find((candidate) => candidate.dataset.validationRecordKey === recordKey);
        if (currentSource && detailDrawer?.classList.contains("is-open")) {
          openValidationDetailDrawer(currentSource, { focusDrawer: false, rememberTrigger: false });
        } else if (status) {
          status.textContent = optionalText(result.message);
        }
        return result;
      })
      .catch((error) => {
        const feedbackText = `명령 실패 · ${error.message} · 같은 command ID로 다시 시도할 수 있습니다.`;
        validationCommandFeedback = { recordKey, state: "transport_error", text: feedbackText };
        if (status) status.textContent = feedbackText;
        for (const command of commandButtons) {
          command.disabled = priorDisabledStates.get(command) ?? true;
        }
      })
      .finally(() => {
        validationCommandRequest = null;
      });
    return validationCommandRequest;
  };

  const dashboardSignature = (dashboard) => JSON.stringify({
    agents: dashboard.agents || null,
    scene: dashboard.scene || null,
    pool: dashboard.pool || null,
    distributor: dashboard.distributor || null,
    campaign: dashboard.campaign || null,
    selectedTask: dashboard.selectedTask || null,
    kpis: dashboard.kpis || null,
    workspace: dashboard.workspace || null,
    eventFeed: dashboard.eventFeed || null,
    debugHarness: dashboard.debugHarness || null,
    validation: dashboard.validation || null,
    events: asArray(dashboard.events)
  });

  const renderDashboardPanels = (dashboard) => {
    renderCampaign(dashboard);
    renderBoard(dashboard);
    renderPipeline(dashboard.distributor);
    renderValidationRail(dashboard.validation);
    initializeDetailControls();
    syncSelectedDetail();
    window.AkraAdminGame?.applyDashboard?.(dashboard);
    window.dispatchEvent(new CustomEvent("akra:dashboard-rendered", { detail: { dashboard } }));
  };

  const updateDashboard = (dashboard) => {
    renderDebugHarness(dashboard.debugHarness);
    setText(
      "[data-planning-revision]",
      dashboard.planningRevision == null ? "미집계" : `rev ${dashboard.planningRevision}`,
    );
    setText("[data-summary-active-agents]", `${dashboard.kpis.activeAgents} / ${dashboard.kpis.totalAgents}`);
    setText("[data-summary-idle-slots]", formatValue(dashboard.kpis.poolIdle, "0"));
    setText("[data-summary-queue-depth]", formatValue(dashboard.kpis.queueDepth, "0"));
    setText("[data-summary-generated-time]", optionalText(dashboard.generatedTimeLabel));
    setText("[data-validation-kpi='verifying']", formatValue(dashboard.kpis.validationVerifying, "0"));
    setText("[data-validation-kpi='failed']", formatValue(dashboard.kpis.validationFailed, "0"));
    setText("[data-validation-kpi='queued']", formatValue(dashboard.kpis.validationRemediationQueued, "0"));
    setText("[data-validation-kpi='stale']", formatValue(dashboard.kpis.validationStale, "0"));
    setText("[data-command-readiness]", optionalText(dashboard.workspace.readiness, "미집계"));
    setText("[data-command-branch]", optionalText(dashboard.workspace.branch, "not-a-git-worktree"));
    setText(
      "[data-operational-notice]",
      optionalText(dashboard.workspace.readinessNotice, "운영 알림 미집계")
    );
    setText("[data-operational-action]", optionalText(dashboard.workspace.blockedAction, "운영 조치 미집계"));
    const operationalDetail = dashboard.workspace.topNotice || "";
    setText("[data-operational-detail]", operationalDetail);
    for (const group of root.querySelectorAll("[data-operational-detail-group]")) {
      group.hidden = operationalDetail === "";
      if (group.hidden) group.open = false;
    }
    const readiness = optionalText(dashboard.workspace.readiness, "degraded");
    setOperationalState(".command-summary, .attention-strip", readiness);
    const attentionStrip = root.querySelector("[data-attention-strip]");
    if (attentionStrip) {
      attentionStrip.hidden = readiness === "ready" || root.dataset.debugHarnessEnabled === "true";
    }
    const previousSignature = root.dataset.dashboardSignature || "";
    const nextSignature = dashboardSignature(dashboard);
    if (previousSignature !== nextSignature) {
      renderDashboardPanels(dashboard);
      root.dataset.dashboardSignature = nextSignature;
    }
    if (Number.isFinite(dashboard.eventFeed?.totalEventCount)) {
      root.dataset.eventTotalCount = String(dashboard.eventFeed.totalEventCount);
    }
    setEventStatus();
  };

  const selectableSources = () => [...root.querySelectorAll(detailSourceSelector)]
    .filter((source) => !source.disabled && !source.closest("[hidden]") && source.getClientRects().length > 0);

  const navigateDetailSelection = (source, direction) => {
    const sources = selectableSources();
    const index = sources.indexOf(source);
    if (index < 0 || sources.length === 0) return;
    const next = sources[(index + direction + sources.length) % sources.length];
    next.focus({ preventScroll: true });
    next.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "smooth" });
    openDetailDrawer(next, { focusDrawer: false });
  };

  const setManualRefreshState = (busy) => {
    root.setAttribute("aria-busy", busy ? "true" : "false");
    for (const button of root.querySelectorAll("[data-refresh-dashboard]")) {
      if (!button.dataset.refreshIdleText) {
        button.dataset.refreshIdleText = button.textContent || "R";
        button.dataset.refreshIdleLabel = button.getAttribute("aria-label") || "Refresh dashboard snapshot";
      }
      button.disabled = busy;
      button.textContent = busy ? "…" : button.dataset.refreshIdleText;
      button.setAttribute(
        "aria-label",
        busy ? "Refreshing dashboard snapshot" : button.dataset.refreshIdleLabel
      );
    }
  };

  const loopControlStatus = root.querySelector("[data-loop-control-status]");
  let loopControlRequest = null;
  const debugHarnessPanel = root.querySelector("[data-debug-harness]");
  const debugScenario = root.querySelector("[data-debug-scenario]");
  const debugStatus = root.querySelector("[data-debug-status]");
  let debugHarnessRequest = null;
  let currentDebugHarness = null;

  const renderDebugHarness = (harness, busy = false) => {
    const enabled = Boolean(harness?.enabled && debugHarnessPanel);
    root.dataset.debugHarnessEnabled = String(enabled);
    if (!debugHarnessPanel) return;
    debugHarnessPanel.hidden = !enabled;
    if (!enabled) return;
    currentDebugHarness = harness;
    root.dataset.debugHarnessRevision = String(harness.revision || "");
    debugHarnessPanel.dataset.debugPlaying = String(Boolean(harness.playing));
    debugHarnessPanel.dataset.debugStage = optionalText(harness.stageKey, "ready");
    setText("[data-debug-scenario-label]", optionalText(harness.scenarioLabel));
    setText("[data-debug-stage-label]", optionalText(harness.stageLabel));
    setText("[data-debug-stage-index]", String((Number(harness.stageIndex) || 0) + 1));
    setText("[data-debug-stage-count]", String(Math.max(1, Number(harness.stageCount) || 1)));
    setText("[data-debug-stage-summary]", optionalText(harness.stageSummary));
    const progress = Math.max(0, Math.min(100, Number(harness.progressPercent) || 0));
    const progressBar = root.querySelector("[data-debug-progress]");
    if (progressBar) progressBar.style.setProperty("--debug-progress", `${progress}%`);
    if (debugScenario && debugScenario.value !== harness.scenarioKey) {
      debugScenario.value = harness.scenarioKey;
    }
    for (const control of root.querySelectorAll("[data-debug-command], [data-debug-scenario]")) {
      control.disabled = busy;
    }
    if (debugStatus) {
      debugStatus.textContent = busy
        ? "Fake 명령 적용 중"
        : `${harness.playing ? "자동 재생" : "일시정지"} · ${harness.stageLabel} · fake rev ${harness.revision}`;
    }
  };

  const renderLoopControl = (control, busy = false) => {
    if (root.dataset.debugHarnessEnabled === "true") {
      for (const button of root.querySelectorAll("[data-loop-command]")) {
        button.disabled = true;
        button.setAttribute("aria-pressed", "false");
      }
      if (loopControlStatus) {
        loopControlStatus.textContent =
          "Application Fake 활성 · 실제 parallel 명령은 보호를 위해 비활성화";
      }
      return;
    }
    const modeEnabled = Boolean(control?.modeEnabled);
    const controlBusy = busy || Boolean(control?.controlEffectInFlight);
    const latestCommand = control?.latestCommand || null;
    for (const button of root.querySelectorAll("[data-loop-command]")) {
      const action = button.dataset.loopCommand;
      button.disabled = controlBusy
        || (action === "disable" && !modeEnabled)
        || (action === "dispatch" && controlBusy);
      button.setAttribute("aria-pressed", String(action === "enable" && modeEnabled));
    }
    if (loopControlStatus) {
      const epoch = control?.currentEpochId == null ? "epoch 없음" : `epoch ${control.currentEpochId}`;
      const withheld = optionalText(control?.lastDispatchWithheldReason, "");
      loopControlStatus.textContent = controlBusy
        ? "제어 명령 처리 중…"
        : `${modeEnabled ? "자동 루프 가동" : "자동 루프 정지"} · ${epoch}${withheld ? ` · 보류: ${withheld}` : ""}`;
    }
    if (loopControlStatus && !controlBusy && latestCommand) {
      const commandStateLabels = {
        accepted: "접수됨",
        running: "처리 중",
        completed: "완료",
        blocked: "보류"
      };
      const commandState = commandStateLabels[latestCommand.state] || latestCommand.state;
      loopControlStatus.textContent =
        `${latestCommand.action} · ${commandState} · ${optionalText(latestCommand.message)}`;
    }
    root.dataset.loopMode = modeEnabled ? "enabled" : "disabled";
    root.dataset.latestCommandId = latestCommand?.commandId || "";
    root.dataset.latestCommandState = latestCommand?.state || "";
  };

  const fetchLoopControl = () =>
    fetch(controlUrl, { headers: { "Accept": "application/json" } })
      .then((response) => {
        if (!response.ok) throw new Error(`control ${response.status}`);
        return response.json();
      })
      .then((control) => {
        renderLoopControl(control);
        return control;
      });

  const runLoopCommand = (action) => {
    if (loopControlRequest) return loopControlRequest;
    renderLoopControl(null, true);
    loopControlRequest = fetch(controlUrl, {
      method: "POST",
      headers: {
        "Accept": "application/json",
        "Content-Type": "application/json",
        "X-CSRF-Token": csrfToken,
      },
      body: JSON.stringify({ action }),
    })
      .then((response) => {
        if (!response.ok) throw new Error(`control ${response.status}`);
        return response.json();
      })
      .then((control) => {
        renderLoopControl(control);
        if (loopControlStatus) loopControlStatus.textContent = control.message;
        return Promise.allSettled([
          pollDashboard({ fresh: true }),
          pollEvents({ fresh: true })
        ]).then(() => control);
      })
      .catch((error) => {
        renderLoopControl(null);
        if (loopControlStatus) loopControlStatus.textContent = `제어 실패 · ${error.message}`;
      })
      .finally(() => {
        loopControlRequest = null;
      });
    return loopControlRequest;
  };

  const runDebugHarnessCommand = (action, scenario = null) => {
    if (!debugHarnessPanel || debugHarnessRequest) return debugHarnessRequest;
    if (currentDebugHarness) renderDebugHarness(currentDebugHarness, true);
    debugHarnessRequest = fetch(debugHarnessUrl, {
      method: "POST",
      headers: {
        "Accept": "application/json",
        "Content-Type": "application/json",
        "X-CSRF-Token": csrfToken,
      },
      body: JSON.stringify({ action, scenario }),
    })
      .then((response) => {
        if (!response.ok) throw new Error(`debug harness ${response.status}`);
        return response.json();
      })
      .then((harness) => {
        renderDebugHarness(harness);
        return Promise.allSettled([
          pollDashboard({ fresh: true }),
          pollEvents({ reset: true, fresh: true })
        ]);
      })
      .catch((error) => {
        if (debugStatus) debugStatus.textContent = `Fake 명령 실패 · ${error.message}`;
      })
      .finally(() => {
        debugHarnessRequest = null;
        for (const control of root.querySelectorAll("[data-debug-command], [data-debug-scenario]")) {
          control.disabled = false;
        }
      });
    return debugHarnessRequest;
  };

  initializeDetailControls();

  root.addEventListener("click", (event) => {
    if (event.target.closest("[data-detail-close]")) {
      closeDetailDrawer();
      return;
    }

    if (event.target.closest("[data-evidence-detail-trigger]")) {
      openEvidenceDetailDrawer();
      return;
    }

    const loopControl = event.target.closest("[data-loop-command]");
    if (loopControl) {
      runLoopCommand(loopControl.dataset.loopCommand);
      return;
    }

    const debugControl = event.target.closest("[data-debug-command]");
    if (debugControl) {
      runDebugHarnessCommand(debugControl.dataset.debugCommand);
      return;
    }

    const validationCommand = event.target.closest("[data-validation-command]");
    if (validationCommand) {
      runValidationCommand(validationCommand);
      return;
    }

    const refreshButton = event.target.closest("[data-refresh-dashboard]");
    if (refreshButton) {
      setManualRefreshState(true);
      Promise.allSettled([
        pollDashboard({ fresh: true }),
        pollEvents({ fresh: true })
      ])
        .then(([snapshot, events]) => {
          const snapshotOk = snapshot.status === "fulfilled" && snapshot.value === true;
          const eventsOk = events.status === "fulfilled" && events.value === true;
          openRefreshDetail(
            snapshotOk ? "ok" : "error",
            eventsOk ? "ok" : "error",
            refreshButton
          );
        })
        .finally(() => setManualRefreshState(false));
      return;
    }

    const focus = event.target.closest("[data-focus-target]");
    if (focus) {
      const panel = document.getElementById(focus.dataset.focusTarget);
      if (panel) {
        panel.classList.add("is-focused");
        panel.scrollIntoView({ block: "nearest", behavior: "smooth" });
        window.setTimeout(() => panel.classList.remove("is-focused"), 1400);
      }
    }

    const detailSource = event.target.closest("[data-detail-type]");
    if (detailSource) openDetailDrawer(detailSource);
  });

  root.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && detailDrawer?.classList.contains("is-open")) {
      event.preventDefault();
      closeDetailDrawer();
      return;
    }

    const detailSource = event.target.closest(detailSourceSelector);
    if (!detailSource) return;
    if (event.key === "ArrowRight" || event.key === "ArrowDown") {
      event.preventDefault();
      navigateDetailSelection(detailSource, 1);
    } else if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
      event.preventDefault();
      navigateDetailSelection(detailSource, -1);
    }
  });

  debugScenario?.addEventListener("change", () => {
    runDebugHarnessCommand("scenario", debugScenario.value);
  });

  window.addEventListener("akra:scene-selection-requested", (event) => {
    const detail = event.detail || {};
    if (detail.kind === "actor" && detail.actorId) {
      const source = [...root.querySelectorAll("[data-actor-id][data-detail-type]")]
        .find((node) => node.dataset.actorId === detail.actorId);
      if (source) openDetailDrawer(source);
      return;
    }
    if (detail.kind !== "poi") return;
    const selectors = {
      director: ".office-board .boss-seat",
      review: "#campaign [data-detail-type]",
      pipeline: ".office-board .distributor-desk",
      events: ".office-board .event-board",
      standby: ".office-board .rest-area",
      validation: "#validation-rail [data-validation-record-key]"
    };
    const selector = selectors[detail.detailTarget];
    const source = selector ? root.querySelector(selector) : null;
    if (source) openDetailDrawer(source);
  });

  const pollStatus = root.querySelector("[data-realtime-status]") || document.createElement("small");
  pollStatus.classList.add("poll-status");
  pollStatus.setAttribute("role", "status");
  pollStatus.setAttribute("aria-live", "polite");
  pollStatus.setAttribute("aria-atomic", "true");
  if (!pollStatus.isConnected) root.querySelector(".command-summary")?.appendChild(pollStatus);

  const pollState = {
    snapshot: "live",
    events: "live",
    stream: "connecting",
    snapshotError: "",
    eventsError: ""
  };
  const renderPollStatus = () => {
    const snapshotLabel = pollState.snapshot === "error"
      ? `stale snapshot${pollState.snapshotError ? `: ${pollState.snapshotError}` : ""}`
      : "live snapshot";
    const eventsLabel = pollState.events === "error"
      ? `stale events${pollState.eventsError ? `: ${pollState.eventsError}` : ""}`
      : "live events";
    const streamLabel = pollState.stream === "live"
      ? "realtime stream"
      : pollState.stream === "unsupported"
        ? "polling fallback"
        : "stream reconnecting";
    const nextText = `${streamLabel} · ${snapshotLabel} · ${eventsLabel}`;
    if (pollStatus.textContent !== nextText) pollStatus.textContent = nextText;
    const stale = pollState.snapshot === "error" || pollState.events === "error";
    pollStatus.classList.toggle("is-stale", stale);
    pollStatus.classList.toggle("is-error", pollState.snapshot === "error");
  };
  renderPollStatus();
  fetchLoopControl().catch((error) => {
    if (loopControlStatus) loopControlStatus.textContent = `제어 상태 확인 실패 · ${error.message}`;
  });

  let dashboardRequest = null;
  let eventsRequest = null;
  let lastDashboardPollAt = 0;

  const pollDashboard = ({ fresh = false } = {}) => {
    if (dashboardRequest) {
      return fresh ? dashboardRequest.then(() => pollDashboard()) : dashboardRequest;
    }
    dashboardRequest = (async () => {
      try {
        const response = await fetch(dashboardUrl, { headers: { "Accept": "application/json" } });
        if (!response.ok) throw new Error(`dashboard ${response.status}`);
        const dashboard = await response.json();
        updateDashboard(dashboard);
        lastDashboardPollAt = Date.now();
        pollState.snapshot = "live";
        pollState.snapshotError = "";
        renderPollStatus();
        return true;
      } catch (error) {
        pollState.snapshot = "error";
        pollState.snapshotError = error.message;
        renderPollStatus();
        return false;
      }
    })().finally(() => {
      dashboardRequest = null;
    });
    return dashboardRequest;
  };

  const applyEventsPayload = (payload) => {
    if (Array.isArray(payload.events)) {
      if (payload.feed?.incremental) {
        prependEventRows(payload.events);
      } else {
        replaceEventRows(payload.events);
      }
    }
    const newest = payload.feed?.newestSequence;
    if (Number.isFinite(newest)) root.dataset.latestEventSequence = String(newest);
    if (Number.isFinite(payload.feed?.eventCursor) && !Number.isFinite(newest)) {
      root.dataset.latestEventSequence = String(payload.feed.eventCursor);
    }
    if (Number.isFinite(payload.feed?.totalEventCount) && !payload.feed?.incremental) {
      root.dataset.eventTotalCount = String(payload.feed.totalEventCount);
    }
    setEventStatus();
  };

  const pollEvents = ({ reset = false, fresh = false } = {}) => {
    if (eventsRequest) {
      return fresh
        ? eventsRequest.then(() => pollEvents({ reset }))
        : eventsRequest;
    }
    eventsRequest = (async () => {
      const latest = Number(root.dataset.latestEventSequence || "0");
      const url = !reset && latest > 0
        ? `${eventsUrl}?afterSequence=${latest}&limit=50`
        : `${eventsUrl}?limit=50`;
      try {
        const response = await fetch(url, { headers: { "Accept": "application/json" } });
        if (!response.ok) throw new Error(`events ${response.status}`);
        const payload = await response.json();
        applyEventsPayload(payload);
        pollState.events = "live";
        pollState.eventsError = "";
        renderPollStatus();
        return true;
      } catch (error) {
        pollState.events = "error";
        pollState.eventsError = error.message;
        renderPollStatus();
        return false;
      }
    })().finally(() => {
      eventsRequest = null;
    });
    return eventsRequest;
  };

  let realtimeSource = null;
  let lastRealtimeFrameAt = 0;

  const setRealtimeState = (state) => {
    pollState.stream = state;
    root.dataset.realtimeState = state;
    renderPollStatus();
  };

  const applyRealtimeFrame = (frame) => {
    if (!frame || frame.schemaVersion !== 1) return;
    lastRealtimeFrameAt = Date.now();
    setRealtimeState("live");
    applyEventsPayload(frame);
    if (frame.control) renderLoopControl(frame.control);
    if (frame.debugHarness) renderDebugHarness(frame.debugHarness);
    if (frame.cursorResetRequired) {
      pollEvents({ reset: true });
    }
    if (frame.refreshDashboard) {
      pollDashboard();
    }
  };

  const connectRealtimeStream = () => {
    if (!("EventSource" in window)) {
      setRealtimeState("unsupported");
      return;
    }
    const latest = Number(root.dataset.latestEventSequence || "0");
    realtimeSource = new EventSource(`${streamUrl}?afterSequence=${Math.max(latest, 0)}`);
    realtimeSource.addEventListener("open", () => setRealtimeState("live"));
    realtimeSource.addEventListener("update", (event) => {
      try {
        applyRealtimeFrame(JSON.parse(event.data));
      } catch (error) {
        pollState.events = "error";
        pollState.eventsError = `stream payload: ${error.message}`;
        renderPollStatus();
      }
    });
    realtimeSource.addEventListener("error", () => setRealtimeState("reconnecting"));
    window.addEventListener("beforeunload", () => realtimeSource?.close(), { once: true });
  };

  window.setInterval(() => {
    const streamHealthy =
      pollState.stream === "live" && Date.now() - lastRealtimeFrameAt < 15_000;
    if (!streamHealthy) {
      pollDashboard();
      pollEvents();
      fetchLoopControl().catch(() => {});
      return;
    }
    if (Date.now() - lastDashboardPollAt >= Math.max(pollIntervalMs * 6, 30_000)) {
      pollDashboard();
    }
  }, Math.max(pollIntervalMs, 5000));
  pollDashboard();
  pollEvents();
  connectRealtimeStream();
})();
