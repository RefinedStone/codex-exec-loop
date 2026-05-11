(function() {
	//#region src/akra-diorama.ts
	var AGENT_FRAME_WIDTH = 128;
	var AGENT_FRAME_HEIGHT = 192;
	var AGENT_SPRITE_SCALE = .4675;
	var AGENT_SHADOW_WIDTH = 26.35;
	var AGENT_SHADOW_HEIGHT = 6.8;
	var AGENT_RING_WIDTH = 21.25;
	var AGENT_RING_HEIGHT = 5.95;
	(() => {
		const isStatusSeverity = (value) => value === "normal" || value === "success" || value === "warning" || value === "danger" || value === "info" || value === "muted";
		const isPixiTexture = (texture) => texture !== null;
		const mountDiorama = () => {
			const container = document.getElementById("pixi-diorama");
			if (!container || typeof PIXI === "undefined") return null;
			const boardEl = container.closest(".office-board");
			if (!boardEl || container.dataset.akraDioramaMounted === "true") return null;
			container.dataset.akraDioramaMounted = "true";
			const initialWidth = boardEl.offsetWidth || 900;
			const initialHeight = boardEl.offsetHeight || 540;
			const app = new PIXI.Application({
				width: initialWidth,
				height: initialHeight,
				backgroundAlpha: 0,
				antialias: false,
				resolution: window.devicePixelRatio || 1,
				autoDensity: true
			});
			container.appendChild(app.view);
			PIXI.BaseTexture.defaultOptions.scaleMode = PIXI.SCALE_MODES.NEAREST;
			const basePath = "/admin/assets/graphics/";
			const assets = {
				floor: basePath + "sprite_floor_tile.png",
				desk: basePath + "sprite_desk_workstation.png",
				server: basePath + "sprite_server_rack.png",
				whiteboard: basePath + "sprite_whiteboard.png",
				sofa: basePath + "sprite_sofa.png",
				plant: basePath + "sprite_potted_plant.png",
				agentAtlas: basePath + "gamebaljeonguk_atlas_128x192.png"
			};
			const root = boardEl.closest("[data-admin-graphic]");
			const pathLayer = new PIXI.Graphics();
			const packetLayer = new PIXI.Container();
			const agentLayer = new PIXI.Container();
			agentLayer.sortableChildren = true;
			app.stage.addChild(pathLayer, packetLayer, agentLayer);
			const statusPalette = {
				normal: 3526783,
				success: 3526783,
				warning: 16107595,
				danger: 16739179,
				info: 6138367,
				muted: 10005444
			};
			let textures = {};
			let agentFrameSets = [];
			let agentUnits = [];
			let roamSnapshots = /* @__PURE__ */ new Map();
			let stageBurst = 0;
			let elapsed = 0;
			let resizeObserver = null;
			const boardSize = () => ({
				width: boardEl.offsetWidth || initialWidth,
				height: boardEl.offsetHeight || initialHeight
			});
			const fallbackPoints = () => {
				const { width, height } = boardSize();
				return [
					{
						x: width * .35,
						y: height * .5
					},
					{
						x: width * .5,
						y: height * .65
					},
					{
						x: width * .28,
						y: height * .72
					},
					{
						x: width * .6,
						y: height * .52
					},
					{
						x: width * .43,
						y: height * .82
					}
				];
			};
			let targets = {
				distributor: {
					x: initialWidth * .8,
					y: initialHeight * .52
				},
				events: {
					x: initialWidth * .82,
					y: initialHeight * .76
				}
			};
			const parseSeverity = (node) => {
				if (isStatusSeverity(node.dataset.detailSeverity)) return node.dataset.detailSeverity;
				if (node.classList.contains("severity-danger")) return "danger";
				if (node.classList.contains("severity-warning")) return "warning";
				if (node.classList.contains("severity-info")) return "info";
				return "normal";
			};
			const colorFor = (severity) => statusPalette[severity] || statusPalette.normal;
			const clamp = (value, min, max) => Math.min(Math.max(value, min), max);
			const lerp = (a, b, t) => a + (b - a) * t;
			const distanceBetween = (a, b) => Math.hypot(a.x - b.x, a.y - b.y);
			const seededRatio = (index, routeIndex, salt) => {
				const raw = Math.sin((index + 1) * 12.9898 + (routeIndex + 1) * 78.233 + salt * 37.719) * 43758.5453;
				return raw - Math.floor(raw);
			};
			const roamBounds = () => {
				const { width, height } = boardSize();
				const horizontalInset = Math.min(width * .5, Math.max(56, width * .08));
				const topInset = Math.min(height * .5, Math.max(168, height * .24));
				const bottomInset = Math.max(58, height * .08);
				return {
					left: horizontalInset,
					right: Math.max(horizontalInset, width - horizontalInset),
					top: topInset,
					bottom: Math.max(topInset, height - bottomInset)
				};
			};
			const clampRoamPoint = (point) => {
				const bounds = roamBounds();
				return {
					x: clamp(point.x, bounds.left, bounds.right),
					y: clamp(point.y, bounds.top, bounds.bottom)
				};
			};
			const chooseRoamPoint = (index, routeIndex) => {
				const bounds = roamBounds();
				return {
					x: lerp(bounds.left, bounds.right, seededRatio(index, routeIndex, 1)),
					y: lerp(bounds.top, bounds.bottom, seededRatio(index, routeIndex, 2))
				};
			};
			const makeAtlasFrame = (texture, col, row = 0) => {
				const baseTexture = texture?.baseTexture;
				if (!baseTexture || typeof PIXI.Rectangle === "undefined") return null;
				return new PIXI.Texture(baseTexture, new PIXI.Rectangle(col * AGENT_FRAME_WIDTH, row * AGENT_FRAME_HEIGHT, AGENT_FRAME_WIDTH, AGENT_FRAME_HEIGHT));
			};
			const makeFrameRow = (texture, row, startCol) => Array.from({ length: 4 }, (_, index) => makeAtlasFrame(texture, startCol + index, row)).filter(isPixiTexture);
			const buildAgentFrameSets = (texture) => {
				return [
					{
						down: makeFrameRow(texture, 0, 0),
						side: makeFrameRow(texture, 1, 0),
						up: makeFrameRow(texture, 2, 0)
					},
					{
						down: makeFrameRow(texture, 0, 4),
						side: makeFrameRow(texture, 1, 4),
						up: makeFrameRow(texture, 2, 4)
					},
					{
						down: makeFrameRow(texture, 3, 0),
						side: makeFrameRow(texture, 4, 0),
						up: makeFrameRow(texture, 4, 0)
					},
					{
						down: makeFrameRow(texture, 3, 4),
						side: makeFrameRow(texture, 4, 4),
						up: makeFrameRow(texture, 4, 4)
					}
				].filter((set) => set.down.length > 0 && set.side.length > 0 && set.up.length > 0);
			};
			const resolvePoint = (node, fallback, xBias = .5, yBias = .76) => {
				if (!node) return fallback;
				const boardRect = boardEl.getBoundingClientRect();
				const rect = node.getBoundingClientRect();
				if (rect.width <= 0 && rect.height <= 0) return fallback;
				return {
					x: rect.left - boardRect.left + rect.width * xBias,
					y: rect.top - boardRect.top + rect.height * yBias
				};
			};
			const makePacket = (color) => {
				const packet = new PIXI.Graphics();
				packet.beginFill(color, .92);
				packet.lineStyle(1, 16777215, .42);
				packet.drawPolygon([
					0,
					-6,
					6,
					0,
					0,
					6,
					-6,
					0
				]);
				packet.endFill();
				packet.alpha = .84;
				packetLayer.addChild(packet);
				return packet;
			};
			const makeAgentUnit = (node, index) => {
				const agentId = node.dataset.agentId || `agent-${index}`;
				const severity = parseSeverity(node);
				const color = colorFor(severity);
				const group = new PIXI.Container();
				const shadow = new PIXI.Graphics();
				shadow.beginFill(0, .26);
				shadow.drawEllipse(0, 0, AGENT_SHADOW_WIDTH, AGENT_SHADOW_HEIGHT);
				shadow.endFill();
				const ring = new PIXI.Graphics();
				const frameSet = agentFrameSets.length ? agentFrameSets[index % agentFrameSets.length] : null;
				const texture = frameSet?.down[0] || null;
				const sprite = texture ? new PIXI.Sprite(texture) : null;
				if (sprite) {
					sprite.anchor.set(.5, 1);
					sprite.scale.set(AGENT_SPRITE_SCALE);
					group.addChild(shadow, ring, sprite);
				} else group.addChild(shadow, ring);
				group.alpha = severity === "muted" ? .58 : .95;
				agentLayer.addChild(group);
				const packet = makePacket(color);
				node.addEventListener("pointerenter", () => {
					group.scale.set(1.08);
					packet.scale.set(1.2);
				});
				node.addEventListener("pointerleave", () => {
					group.scale.set(1);
					packet.scale.set(1);
				});
				node.addEventListener("click", () => {
					stageBurst = Math.min(stageBurst + .55, 1.4);
				});
				const points = fallbackPoints();
				const fallbackPoint = resolvePoint(node, points[index % points.length], .5, .78);
				const snapshot = roamSnapshots.get(agentId);
				const routeIndex = snapshot?.routeIndex ?? index * 5;
				const point = clampRoamPoint(snapshot?.point || fallbackPoint);
				let destination = clampRoamPoint(snapshot?.destination || chooseRoamPoint(index, routeIndex));
				if (distanceBetween(point, destination) < 54) destination = chooseRoamPoint(index, routeIndex + 1);
				return {
					agentId,
					node,
					index,
					color,
					group,
					ring,
					sprite,
					packet,
					frameSet,
					point,
					destination,
					phase: index * .23,
					speed: .16 + index * .025,
					walkSpeed: 34 + index * 4,
					routeIndex,
					waitUntil: snapshot?.waitUntil ?? 0,
					facing: "down",
					facingSign: 1,
					isWalking: false,
					targetKind: index % 2 === 0 ? "distributor" : "events"
				};
			};
			const syncLayout = () => {
				const { width, height } = boardSize();
				if (width > 0 && height > 0) app.renderer.resize(width, height);
				targets = {
					distributor: resolvePoint(root?.querySelector(".distributor-desk"), {
						x: width * .8,
						y: height * .52
					}, .5, .6),
					events: resolvePoint(root?.querySelector(".event-board"), {
						x: width * .82,
						y: height * .76
					}, .5, .58)
				};
				const points = fallbackPoints();
				for (const unit of agentUnits) {
					const fallbackPoint = resolvePoint(unit.node, points[unit.index % points.length], .5, .78);
					unit.point = clampRoamPoint(unit.point || fallbackPoint);
					unit.destination = clampRoamPoint(unit.destination || chooseRoamPoint(unit.index, 0));
					unit.group.x = unit.point.x;
					unit.group.y = unit.point.y;
				}
			};
			const rememberRoamSnapshots = () => {
				roamSnapshots = new Map(agentUnits.map((unit) => [unit.agentId, {
					point: { ...unit.point },
					destination: { ...unit.destination },
					routeIndex: unit.routeIndex,
					waitUntil: unit.waitUntil
				}]));
			};
			const rebuildAgentUnits = () => {
				rememberRoamSnapshots();
				for (const child of packetLayer.removeChildren()) child.destroy({ children: true });
				for (const child of agentLayer.removeChildren()) child.destroy({ children: true });
				agentUnits = root ? [...root.querySelectorAll(".desk[data-agent-id]")].map(makeAgentUnit) : [];
				syncLayout();
			};
			const setNextRoamTarget = (unit) => {
				unit.routeIndex += 1;
				unit.destination = chooseRoamPoint(unit.index, unit.routeIndex);
				if (distanceBetween(unit.point, unit.destination) < 72) {
					unit.routeIndex += 1;
					unit.destination = chooseRoamPoint(unit.index, unit.routeIndex);
				}
				unit.waitUntil = 0;
			};
			const updateFacing = (unit, dx, dy) => {
				if (Math.abs(dx) > Math.abs(dy) * .72) {
					unit.facing = "side";
					unit.facingSign = dx < 0 ? 1 : -1;
					return;
				}
				unit.facing = dy < 0 ? "up" : "down";
			};
			const updateRoamMotion = (unit, delta) => {
				const dt = Math.min(delta / 60, .08);
				const dx = unit.destination.x - unit.point.x;
				const dy = unit.destination.y - unit.point.y;
				const distance = Math.hypot(dx, dy);
				if (distance <= 3) {
					unit.isWalking = false;
					if (unit.waitUntil === 0) unit.waitUntil = elapsed + .2 + seededRatio(unit.index, unit.routeIndex, 3) * .8;
					if (elapsed >= unit.waitUntil) setNextRoamTarget(unit);
					return;
				}
				unit.isWalking = true;
				updateFacing(unit, dx, dy);
				const step = Math.min(distance, unit.walkSpeed * (1 + stageBurst * .1) * dt);
				unit.point = clampRoamPoint({
					x: unit.point.x + dx / distance * step,
					y: unit.point.y + dy / distance * step
				});
			};
			const applyWalkFrame = (unit) => {
				if (!unit.sprite || !unit.frameSet) return;
				const frames = unit.frameSet[unit.facing];
				if (frames.length === 0) return;
				const frameIndex = unit.isWalking ? Math.floor((elapsed * 7.5 + unit.phase * 6) % frames.length) : 0;
				unit.sprite.texture = frames[frameIndex] || frames[0];
				unit.sprite.scale.set(unit.facing === "side" ? unit.facingSign * AGENT_SPRITE_SCALE : AGENT_SPRITE_SCALE, AGENT_SPRITE_SCALE);
			};
			const renderTick = (delta) => {
				elapsed += delta / 60;
				stageBurst = Math.max(0, stageBurst - delta * .018);
				pathLayer.clear();
				for (const unit of agentUnits) {
					updateRoamMotion(unit, delta);
					applyWalkFrame(unit);
					const target = targets[unit.targetKind] || targets.distributor;
					const start = {
						x: unit.point.x,
						y: unit.point.y - 24
					};
					const end = {
						x: target.x,
						y: target.y - 18
					};
					const alpha = .22 + stageBurst * .16;
					pathLayer.lineStyle(2, unit.color, alpha);
					pathLayer.moveTo(start.x, start.y);
					pathLayer.lineTo(end.x, end.y);
					const travel = (elapsed * unit.speed + unit.phase) % 1;
					const arc = Math.sin(travel * Math.PI) * (24 + stageBurst * 10);
					unit.packet.x = lerp(start.x, end.x, travel);
					unit.packet.y = lerp(start.y, end.y, travel) - arc;
					unit.packet.rotation += delta * .08;
					unit.packet.alpha = .42 + Math.sin(travel * Math.PI) * .42 + stageBurst * .12;
					unit.packet.scale.set(.92 + stageBurst * .18);
					const bob = Math.sin(elapsed * 3.4 + unit.phase * 8) * 2.2;
					unit.group.x = unit.point.x;
					unit.group.y = unit.point.y + bob - stageBurst * 1.5;
					unit.group.zIndex = unit.point.y;
					unit.ring.clear();
					unit.ring.lineStyle(2, unit.color, .58 + stageBurst * .2);
					const ringPulse = 1 + Math.sin(elapsed * 4.2 + unit.phase * 4) * .12 + stageBurst * .12;
					unit.ring.drawEllipse(0, 0, AGENT_RING_WIDTH * ringPulse, AGENT_RING_HEIGHT * ringPulse);
				}
			};
			const loadTextures = async () => {
				textures = {};
				for (const [key, url] of Object.entries(assets)) try {
					textures[key] = await PIXI.Assets.load(url);
				} catch (error) {
					console.warn(`Failed to load sprite: ${key}`, error);
				}
				agentFrameSets = buildAgentFrameSets(textures.agentAtlas);
			};
			const onMissionPulse = (event) => {
				const detail = event.detail;
				const count = Number(detail?.count || 1);
				stageBurst = Math.min(stageBurst + .45 + count * .06, 1.6);
			};
			const onResize = () => syncLayout();
			const destroy = () => {
				window.removeEventListener("akra:mission-pulse", onMissionPulse);
				window.removeEventListener("akra:dashboard-rendered", rebuildAgentUnits);
				window.removeEventListener("resize", onResize);
				resizeObserver?.disconnect();
				app.destroy(true, {
					children: true,
					texture: false,
					baseTexture: false
				});
				delete container.dataset.akraDioramaMounted;
			};
			loadTextures().then(() => {
				rebuildAgentUnits();
				app.ticker.add(renderTick);
				window.addEventListener("akra:mission-pulse", onMissionPulse);
				window.addEventListener("akra:dashboard-rendered", rebuildAgentUnits);
				window.addEventListener("resize", onResize);
				if (typeof ResizeObserver !== "undefined") {
					resizeObserver = new ResizeObserver(syncLayout);
					resizeObserver.observe(boardEl);
				}
			});
			return {
				app,
				destroy,
				rebuildAgentUnits,
				syncLayout
			};
		};
		window.AkraAdminGame = {
			...window.AkraAdminGame || {},
			mountDiorama
		};
		if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", mountDiorama, { once: true });
		else mountDiorama();
	})();
	//#endregion
})();
