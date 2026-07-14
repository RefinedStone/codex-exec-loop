(() => {
  const root = document.querySelector("[data-admin-graphic]");
  if (!root) return;

  const pollIntervalMs = Number(root.dataset.pollIntervalMs || "10000");
  const dashboardUrl = "/api/admin/akra/dashboard";
  const eventsUrl = "/api/admin/akra/events";

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
    header.append(createText("h3", "", title), createText("small", "", subtitle));
    return header;
  };
  const metricLine = (label, value, note) => {
    const line = document.createElement("div");
    line.className = "metric-line";
    line.append(createText("small", "", label), createText("strong", "", value));
    if (note) line.appendChild(createText("small", "", note));
    return line;
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
  let detailTrigger = null;
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

  const openDetailDrawer = (
    source,
    { focusDrawer = true, rememberTrigger = true, trigger = source } = {}
  ) => {
    if (!source || !detailDrawer || !detailDrawerBody) return;
    if (rememberTrigger) detailTrigger = trigger?.isConnected ? trigger : null;
    const type = source.dataset.detailType;
    const rows = detailRowsByType[type] || [];
    detailDrawerTitle.textContent = source.dataset.detailTitle || "상세";
    detailDrawerSubtitle.textContent = source.dataset.detailSubtitle || "선택한 항목";
    detailDrawerBody.replaceChildren(
      ...rows.map(([label, key, style]) =>
        renderDetailRow(label, source.dataset[key], style, source.dataset.detailSeverity)
      )
    );
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
    detailDrawer.setAttribute("aria-hidden", "true");
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
    source.dataset.detailSubtitle = "read-only admin snapshot refresh";
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
    const summary = document.createElement("div");
    summary.className = "campaign-summary";
    for (const [label, value] of [
      ["활성 레인", campaign.activeLaneCount],
      ["정보 신호", campaign.signalCount]
    ]) {
      const item = document.createElement("span");
      item.append(createText("small", "", label), createText("strong", "", formatValue(value, "0")));
      summary.appendChild(item);
    }

    const lanes = asArray(campaign.laneCards);
    const laneBody = lanes.length > 0
      ? (() => {
          const list = document.createElement("div");
          list.className = "campaign-lanes";
          list.append(...lanes.map(createCampaignLane));
          return list;
        })()
      : createText("p", "", dashboard.agents?.emptyState || "표시할 시도 레인이 없습니다.");
    updatePanel("#campaign", [
      panelTitle("임무 현황", campaign.summary || ""),
      summary,
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
      panelTitle("워크트리 풀", `${formatValue(pool.summary?.running, "0")} / ${formatValue(pool.configuredSize, "0")}`),
      ...asArray(pool.slots).map(createSlotButton)
    );
  };

  const actorDetailDataset = (actor) => ({
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
      diagnostics: asArray(scene.diagnostics)
    });
    if (root.dataset.sceneSignature === nextSignature) return;
    for (const node of board.querySelectorAll(".desk[data-actor-id]")) node.remove();
    const anchor = board.querySelector(".distributor-desk") || board.querySelector(".event-board");
    for (const actor of asArray(scene.actors)) board.insertBefore(createActorButton(actor), anchor);
    root.querySelector("[data-scene-actor-list]")?.replaceChildren(
      ...asArray(scene.actors).map(createActorListButton)
    );
    renderSceneDiagnostics(scene.diagnostics);
    root.dataset.sceneSignature = nextSignature;
    window.dispatchEvent(new CustomEvent("akra:scene-rendered", { detail: { scene } }));
  };

  const syncStageHud = (dashboard) => {
    const hud = root.querySelector(".stage-hud");
    if (!hud) return;
    const strong = hud.querySelector("strong");
    const small = hud.querySelector("[data-stage-summary]");
    if (strong) strong.textContent = `${formatValue(dashboard.campaign?.activeLaneCount, "0")} lanes`;
    if (small) {
      small.textContent = `${optionalText(dashboard.distributor?.barrierState)} · ${formatValue(dashboard.eventFeed?.totalEventCount, "0")} signals`;
    }
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
    syncStageHud(dashboard);
    syncDistributorDesk(dashboard.distributor);
    syncEventBoard(dashboard);
  };

  const renderSelectedTask = (dashboard) => {
    const task = dashboard.selectedTask;
    const children = [panelTitle("작업 상세", "selected session")];
    if (!task) {
      children.push(createText("p", "", dashboard.agents?.emptyState || "선택된 작업이 없습니다."));
      updatePanel("#tasks", children);
      return;
    }

    children.push(
      metricLine(task.taskId, task.taskTitle),
      metricLine("담당 요원", `${optionalText(task.agentId)} / ${optionalText(task.slotId)}`),
      metricLine("브랜치", task.branchName)
    );
    const progressLine = document.createElement("div");
    progressLine.className = "metric-line";
    const track = document.createElement("span");
    track.className = "progress-track";
    const fill = document.createElement("span");
    fill.className = "progress-fill";
    const hasProgress = Number.isFinite(task.progressPercent);
    fill.style.width = hasProgress ? `${Number(task.progressPercent)}%` : "0";
    track.appendChild(fill);
    progressLine.append(
      createText("small", "", hasProgress ? `진행률 ${task.progressPercent}%` : "진행률 미집계"),
      track
    );
    children.push(progressLine, metricLine("검증 결과", task.validationSummary));

    const timeline = document.createElement("div");
    timeline.className = "timeline";
    const list = document.createElement("ul");
    list.className = "trail-list";
    for (const trail of asArray(task.trail)) list.appendChild(createText("li", "", trail));
    timeline.append(createText("small", "", "트레일"), list);
    children.push(timeline);
    updatePanel("#tasks", children);
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
    events: asArray(dashboard.events)
  });

  const renderDashboardPanels = (dashboard) => {
    renderCampaign(dashboard);
    renderBoard(dashboard);
    renderSelectedTask(dashboard);
    renderPipeline(dashboard.distributor);
    initializeDetailControls();
    syncSelectedDetail();
    window.dispatchEvent(new CustomEvent("akra:dashboard-rendered", { detail: { dashboard } }));
  };

  const updateDashboard = (dashboard) => {
    setText(
      "[data-planning-revision]",
      dashboard.planningRevision == null ? "미집계" : `rev ${dashboard.planningRevision}`,
    );
    setText("[data-summary-active-agents]", `${dashboard.kpis.activeAgents} / ${dashboard.kpis.totalAgents}`);
    setText("[data-summary-idle-slots]", formatValue(dashboard.kpis.poolIdle, "0"));
    setText("[data-summary-queue-depth]", formatValue(dashboard.kpis.queueDepth, "0"));
    setText("[data-summary-generated-time]", optionalText(dashboard.generatedTimeLabel));
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
    setOperationalState(".command-summary, .command-alert", dashboard.workspace.readiness);
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

  initializeDetailControls();

  root.addEventListener("click", (event) => {
    if (event.target.closest("[data-detail-close]")) {
      closeDetailDrawer();
      return;
    }

    const refreshButton = event.target.closest("[data-refresh-dashboard]");
    if (refreshButton) {
      setManualRefreshState(true);
      Promise.allSettled([pollDashboard(), pollEvents()])
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

  const pollStatus = document.createElement("small");
  pollStatus.className = "poll-status";
  pollStatus.setAttribute("role", "status");
  pollStatus.setAttribute("aria-live", "polite");
  pollStatus.setAttribute("aria-atomic", "true");
  root.querySelector(".stage-hud")?.appendChild(pollStatus);

  const pollState = { snapshot: "live", events: "live", snapshotError: "", eventsError: "" };
  const renderPollStatus = () => {
    const snapshotLabel = pollState.snapshot === "error"
      ? `stale snapshot${pollState.snapshotError ? `: ${pollState.snapshotError}` : ""}`
      : "live snapshot";
    const eventsLabel = pollState.events === "error"
      ? `stale events${pollState.eventsError ? `: ${pollState.eventsError}` : ""}`
      : "live events";
    const nextText = `${snapshotLabel} · ${eventsLabel}`;
    if (pollStatus.textContent !== nextText) pollStatus.textContent = nextText;
    const stale = pollState.snapshot === "error" || pollState.events === "error";
    pollStatus.classList.toggle("is-stale", stale);
    pollStatus.classList.toggle("is-error", pollState.snapshot === "error");
  };
  renderPollStatus();

  let dashboardRequest = null;
  let eventsRequest = null;

  const pollDashboard = () => {
    if (dashboardRequest) return dashboardRequest;
    dashboardRequest = (async () => {
      try {
        const response = await fetch(dashboardUrl, { headers: { "Accept": "application/json" } });
        if (!response.ok) throw new Error(`dashboard ${response.status}`);
        const dashboard = await response.json();
        updateDashboard(dashboard);
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

  const pollEvents = () => {
    if (eventsRequest) return eventsRequest;
    eventsRequest = (async () => {
      const latest = Number(root.dataset.latestEventSequence || "0");
      const url = latest > 0 ? `${eventsUrl}?afterSequence=${latest}&limit=50` : `${eventsUrl}?limit=50`;
      try {
        const response = await fetch(url, { headers: { "Accept": "application/json" } });
        if (!response.ok) throw new Error(`events ${response.status}`);
        const payload = await response.json();
        if (Array.isArray(payload.events)) {
          if (payload.feed?.incremental) {
            prependEventRows(payload.events);
          } else {
            replaceEventRows(payload.events);
          }
        }
        const newest = payload.feed?.newestSequence;
        if (Number.isFinite(newest)) root.dataset.latestEventSequence = String(newest);
        if (Number.isFinite(payload.feed?.totalEventCount) && !payload.feed?.incremental) {
          root.dataset.eventTotalCount = String(payload.feed.totalEventCount);
        }
        setEventStatus();
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

  window.setInterval(() => {
    pollDashboard();
    pollEvents();
  }, Math.max(pollIntervalMs, 5000));
})();
