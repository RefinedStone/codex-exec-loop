use super::pages::{extract_file_updates, nav_for_kind};
use super::parse_reset_target;
use crate::application::service::planning::{
    PlanningAdminDraftKind, PlanningAdminFileKey, PlanningResetTarget,
};
use std::collections::HashMap;

/*
 * admin_api tests는 service 내부가 아니라 inbound HTML/form boundary를 보호한다.
 * pages.rs가 form field를 어떤 application request로 인정하는지, template이 destructive POST 앞에서 어떤 browser guard를
 * 제공하는지 같은 adapter contract를 고정한다. template 파일은 compile-time fixture로 포함해 마크업 변경이 Rust test와
 * 함께 review되게 한다.
 */
const BASE_TEMPLATE: &str = include_str!("../../../../templates/admin/base.html");
const CONTROLS_TEMPLATE: &str = include_str!("../../../../templates/admin/controls.html");
const DIRECTIONS_TEMPLATE: &str = include_str!("../../../../templates/admin/directions.html");
const EDITOR_TEMPLATE: &str = include_str!("../../../../templates/admin/editor.html");
const TASKS_TEMPLATE: &str = include_str!("../../../../templates/admin/tasks.html");
const DASHBOARD_TEMPLATE: &str = include_str!("../../../../templates/admin/dashboard.html");
const AKRA_DASHBOARD_TEMPLATE: &str =
    include_str!("../../../../templates/admin/akra_dashboard.html");
const AKRA_METRICS_TEMPLATE: &str = include_str!("../../../../templates/admin/akra_metrics.html");
const ADMIN_GRAPHIC_VISUAL_SCRIPT: &str =
    include_str!("../../../../scripts/check_admin_graphic_visual.sh");
const GAMEBALJEONGUK_SPRITE_PACK_README: &str =
    include_str!("../../../../templates/admin/resources/gamebaljeonguk_sprite_pack/README.txt");
const GAMEBALJEONGUK_SPRITE_METADATA: &str = include_str!(
    "../../../../templates/admin/resources/gamebaljeonguk_sprite_pack/gamebaljeonguk_sprite_metadata.json"
);
const AKRA_DIORAMA_JS: &str = include_str!("../../../../assets/admin/game/akra-diorama.js");
const AKRA_DIORAMA_TS: &str = include_str!("../../../../assets/admin/game/src/akra-diorama.ts");
const ADMIN_GAME_PACKAGE_JSON: &str = include_str!("../../../../assets/admin/game/package.json");
const ADMIN_GAME_VITE_CONFIG: &str = include_str!("../../../../assets/admin/game/vite.config.ts");
const ADMIN_GAME_PROMOTE_BUILD: &str =
    include_str!("../../../../assets/admin/game/scripts/promote-build.mjs");
const ADMIN_API: &str = include_str!("api.rs");
const AKRA_DASHBOARD_RS: &str = include_str!("akra_dashboard.rs");
const ADMIN_MOD: &str = include_str!("mod.rs");
const ADMIN_PAGES: &str = include_str!("pages.rs");
const ADMIN_STATIC_ASSETS: &str = include_str!("static_assets.rs");

/*
 * 제거된 raw-authority field는 stale browser tab이나 오래된 bookmark/form replay에서 여전히 들어올 수 있다.
 * extract_file_updates는 그런 이름을 application-level file mutation으로 승격하지 않아야 한다.
 * 이 테스트는 inbound adapter의 allow-list가 old transport vocabulary를 조용히 drop하는지 검증한다.
 */
#[test]
fn page_mutation_ignores_removed_raw_authority_file_updates() {
    // 현재 지원되는 field를 함께 넣어 parser가 전체 실패가 아니라 selective filtering을 수행한다는 점을 증명한다.
    let updates = extract_file_updates(HashMap::from([
        ("file_task_authority".to_string(), "{}".to_string()),
        ("file_directions".to_string(), "version = 1".to_string()),
        (
            "file_queue_idle_prompt".to_string(),
            "# Queue prompt".to_string(),
        ),
    ]));

    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].key, PlanningAdminFileKey::QueueIdlePrompt);
}

/*
 * draft-kind navigation은 adapter policy다.
 * service enum이 어떤 admin tab 아래에서 editor를 열지 결정하는 것은 HTML navigation surface의 책임이다.
 * raw task authority draft kind가 visible navigation에서 제거된 상태도 여기서 고정한다.
 */
#[test]
fn nav_no_longer_has_raw_task_authority_draft_kind() {
    assert_eq!(
        nav_for_kind(PlanningAdminDraftKind::FullPlanning),
        "dashboard"
    );
    assert_eq!(
        nav_for_kind(PlanningAdminDraftKind::QueueIdlePrompt),
        "directions"
    );
}

#[test]
fn reset_form_and_json_spelling_maps_to_shared_application_target() {
    /*
     * HTML forms and JSON callers share parse_reset_target in admin_api::mod.
     * Keep the accepted labels mapped directly to PlanningResetTarget so admin
     * never grows a surface-specific destructive reset vocabulary.
     */
    for (raw, expected) in [
        ("queue", PlanningResetTarget::Queue),
        ("directions", PlanningResetTarget::Directions),
        ("all", PlanningResetTarget::All),
    ] {
        assert_eq!(parse_reset_target(raw).unwrap(), expected);
    }
    assert!(parse_reset_target("tasks").is_err());
}

#[test]
fn admin_html_and_json_reset_routes_share_parser_and_facade() {
    /*
     * Reset is exposed as both a browser POST and a JSON POST. They may render
     * different responses, but they must share the same text-to-target parser
     * and facade mutation so queue/directions/all cannot drift by transport.
     */
    for route in [
        ".route(\"/admin/controls/reset\", post(pages::reset_page))",
        ".route(\"/api/planning/reset\", post(api::reset_api))",
    ] {
        assert!(
            ADMIN_MOD.contains(route),
            "route table should keep paired reset route {route}"
        );
    }

    assert!(ADMIN_PAGES.contains("let target = parse_reset_target(&form.target)?;"));
    assert!(ADMIN_PAGES.contains(".reset_workspace(target)"));
    assert!(ADMIN_API.contains(".reset_workspace(parse_reset_target(&request.target)?)"));
}

#[test]
fn admin_html_and_json_draft_routes_share_mutation_facade_methods() {
    /*
     * Draft save/validate/promote has HTML and JSON variants. This source-level
     * guard keeps both transports on PlanningAdminDraftMutationRequest and the
     * same facade methods while still allowing different response rendering.
     */
    for route in [
        ".route(\n            \"/admin/drafts/{draft_name}/save\",\n            post(pages::save_draft_page),\n        )",
        ".route(\n            \"/admin/drafts/{draft_name}/validate\",\n            post(pages::validate_draft_page),\n        )",
        ".route(\n            \"/admin/drafts/{draft_name}/promote\",\n            post(pages::promote_draft_page),\n        )",
        ".route(\n            \"/api/planning/drafts/{draft_name}\",\n            get(api::load_draft_api).put(api::save_draft_api),\n        )",
        ".route(\n            \"/api/planning/drafts/{draft_name}/validate\",\n            post(api::validate_draft_api),\n        )",
        ".route(\n            \"/api/planning/drafts/{draft_name}/promote\",\n            post(api::promote_draft_api),\n        )",
    ] {
        assert!(
            ADMIN_MOD.contains(route),
            "route table should keep paired draft route {route}"
        );
    }

    for (label, source) in [("HTML", ADMIN_PAGES), ("JSON", ADMIN_API)] {
        assert!(
            source.contains("PlanningAdminDraftMutationRequest"),
            "{label} draft path should use the shared draft mutation request"
        );
        assert!(
            source.contains(".save_draft("),
            "{label} draft path should call the shared save facade method"
        );
        assert!(
            source.contains(".promote_draft("),
            "{label} draft path should call the shared promote facade method"
        );
    }
    assert!(ADMIN_PAGES.contains("page_mutation_request(draft_name, form)"));
    assert!(ADMIN_API.contains("PlanningAdminDraftMutationRequest {"));
}

#[test]
fn admin_html_and_json_direction_task_routes_share_facade_methods() {
    /*
     * Direction and task CRUD are the easiest places to accidentally add a
     * browser-only or API-only rule. Pair the route table and facade calls so
     * both transports keep the same application mutation owner.
     */
    for route in [
        ".route(\n            \"/admin/directions/upsert\",\n            post(pages::upsert_direction_page),\n        )",
        ".route(\n            \"/admin/directions/delete\",\n            post(pages::delete_direction_page),\n        )",
        ".route(\"/admin/tasks/upsert\", post(pages::upsert_task_page))",
        ".route(\"/admin/tasks/delete\", post(pages::delete_task_page))",
        ".route(\"/api/planning/directions\", post(api::upsert_direction_api))",
        ".route(\n            \"/api/planning/directions/delete\",\n            post(api::delete_direction_api),\n        )",
        ".route(\"/api/planning/tasks\", post(api::upsert_task_api))",
        ".route(\"/api/planning/tasks/delete\", post(api::delete_task_api))",
    ] {
        assert!(
            ADMIN_MOD.contains(route),
            "route table should keep paired admin CRUD route {route}"
        );
    }

    for method in [
        ".upsert_direction(",
        ".delete_direction(",
        ".upsert_task(",
        ".delete_task(",
    ] {
        assert!(
            ADMIN_PAGES.contains(method),
            "HTML admin path should call shared facade method {method}"
        );
        assert!(
            ADMIN_API.contains(method),
            "JSON admin path should call shared facade method {method}"
        );
    }
}

/*
 * admin 개편의 첫 화면 계약은 route handler가 아니라 template shell에 있다.
 * sidebar landmark와 dashboard quick routes가 사라지면 로컬 운영자가 편집/제어 surface로 바로 이동하지 못하므로
 * fixture test로 최소 구조를 고정한다.
 */
#[test]
fn admin_shell_exposes_sidebar_navigation_and_dashboard_routes() {
    assert!(BASE_TEMPLATE.contains("class=\"admin-layout\""));
    assert!(BASE_TEMPLATE.contains("aria-label=\"Admin navigation\""));
    assert!(BASE_TEMPLATE.contains("class=\"workspace-chip\""));
    assert!(!BASE_TEMPLATE.contains("legacy"));
    assert!(BASE_TEMPLATE.contains("href=\"/admin/akra#pool\""));
    assert!(BASE_TEMPLATE.contains("href=\"/admin/akra#pipeline\""));
    assert!(BASE_TEMPLATE.contains("href=\"/admin/akra/metrics#system\""));
    assert!(BASE_TEMPLATE.contains(r#"current_nav == "akra_dashboard" || current_nav == "akra_metrics" || current_nav == "tasks""#));
    assert!(BASE_TEMPLATE.contains(
        r#"href="/admin/tasks" class="{% if current_nav == "tasks" %}active{% endif %}""#
    ));
    assert!(BASE_TEMPLATE.contains("akraHashTabRoutes"));
    assert!(BASE_TEMPLATE.contains("window.location.pathname !== \"/admin/akra\""));
    assert!(BASE_TEMPLATE.contains("tasks: \"/admin/tasks\""));
    assert!(BASE_TEMPLATE.contains("window.addEventListener(\"hashchange\", redirectAkraHashTab)"));
    assert!(BASE_TEMPLATE.contains("AKRA v0.9.0-beta"));
    assert!(ADMIN_MOD.contains("AKRA_ADMIN_GRAPHIC_ENABLED"));
    assert!(ADMIN_MOD.contains("AKRA_ADMIN_API_BASE_URL"));
    assert!(ADMIN_MOD.contains("AKRA_ADMIN_GRAPHIC_POLL_MS"));

    for route in [
        "href=\"/admin/tasks\"",
        "href=\"/admin/directions\"",
        "href=\"/admin/controls\"",
    ] {
        assert!(
            DASHBOARD_TEMPLATE.contains(route),
            "dashboard should expose quick route {route}"
        );
    }

    assert!(DASHBOARD_TEMPLATE.contains("Open Full Planning Draft"));
    assert!(ADMIN_MOD.contains(".route(\"/admin\", get(pages::dashboard_page))"));
    assert!(ADMIN_MOD.contains(".route(\"/\", get(pages::dashboard_page))"));
    assert!(BASE_TEMPLATE.contains("current_nav == \"dashboard\""));
}

#[test]
fn tasks_page_uses_game_bureau_tab_without_losing_admin_forms() {
    for token in [
        "class=\"akra-task-console\" aria-label=\"게임발전국 작업 관리\"",
        "class=\"task-command-deck\"",
        "class=\"task-tab-strip\" aria-label=\"작업 관리 탭\"",
        "href=\"/admin/tasks\" class=\"active\" aria-current=\"page\"",
        "class=\"task-command-grid\"",
        "class=\"task-panel task-create-panel\"",
        "class=\"task-panel task-proposal-panel\"",
        "class=\"task-panel task-board-panel\"",
        "class=\"task-ticket-list\" id=\"task-list\"",
        "class=\"task-ticket status-{{ task.status }}\"",
        "data-list-filter=\"task-list\"",
        "data-filter-empty=\"task-list\"",
        "overview.runtime.proposed_tasks",
        "management.tasks.len()",
        "management.directions.len()",
        ".task-ticket.status-ready summary",
        "rgba(53, 208, 127, 0.82)",
        "rgba(53, 208, 127, 0.22)",
        "color: #f7fff9",
        ".task-ticket.status-done",
        "rgba(152, 171, 196, 0.09)",
        "color: #d4dde8",
    ] {
        assert!(
            TASKS_TEMPLATE.contains(token),
            "tasks game tab should expose {token}"
        );
    }

    for token in [
        "action=\"/admin/tasks/upsert\"",
        "action=\"/admin/tasks/delete\"",
        "action=\"/admin/files/export\"",
        "action=\"/admin/files/apply\"",
        "name=\"csrf_token\"",
        "name=\"id\"",
        "name=\"title\"",
        "name=\"direction_id\"",
        "name=\"status\"",
        "name=\"base_priority\"",
        "name=\"dynamic_priority_delta\"",
        "name=\"priority_reason\"",
        "name=\"description\"",
        "name=\"depends_on_text\"",
        "name=\"blocked_by_text\"",
    ] {
        assert!(
            TASKS_TEMPLATE.contains(token),
            "tasks game tab should keep admin form contract {token}"
        );
    }

    assert_eq!(
        TASKS_TEMPLATE
            .matches("action=\"/admin/tasks/upsert\"")
            .count(),
        2
    );
    assert_eq!(
        TASKS_TEMPLATE
            .matches("action=\"/admin/tasks/delete\"")
            .count(),
        1
    );
    assert!(!TASKS_TEMPLATE.contains("Task catalog view"));
    assert!(ADMIN_PAGES.contains("page_title: \"작업 관리\".to_string()"));
}

#[test]
fn akra_graphic_dashboard_keeps_admin_and_snapshot_surfaces() {
    for copy in [
        "게임발전국",
        "AKRA Admin Control Center",
        "워크트리 풀",
        "배포 파이프라인",
        "실시간 이벤트",
        "시도 보드",
        "최근 시도 로그",
        "정보 카드",
        "data-admin-graphic",
        "data-poll-interval-ms",
        "gamebaljeonguk_atlas_64x96.png",
        "background-image: var(--agent-sprite-sheet)",
        "background-size: 384px 504px",
        "avatar-Artificer",
        "agentAvatarClass",
        "background-image: var(--object-sprite-sheet)",
        "background-size: 627px 627px",
        "role-distributor",
        "role-events",
        "data-focus-target=\"pipeline\"",
        "data-event-drawer",
        "data-detail-drawer",
        "id=\"akra-detail-drawer\"",
        "data-detail-type=\"campaignLane\"",
        "data-detail-type=\"campaignAttempt\"",
        "data-detail-type=\"campaignIntel\"",
        "data-projection-kind",
        "data-agent-id=\"{{ item.source_agent }}\"",
        "data-refresh-dashboard",
        "openDetailDrawer",
        "navigateDetailSelection",
        "selectionTokens",
        "projectionSlotToken",
        "aria-controls",
        "aria-pressed",
        "relatedSelectionCount",
        "openRefreshDetail",
        "data-event-feed-status",
        "akra-office-background.png",
        "akra-object-sprites.png",
        "MISSION FLOW",
        "stage-refresh-btn",
        "detailSourceKey(node) === nextKey",
        "is-bursting",
        "akra:mission-pulse",
        "pulseStage",
        "has-changed",
        "prependEventRows",
        "stale snapshot",
        "pollEvents",
        "/admin/assets/game/akra-diorama.js",
        "data-automation-epoch",
        "akra:dashboard-rendered",
        "renderDashboardPanels",
        "dashboardSignature",
        "renderCampaign",
        "renderBoard",
        "renderPipeline",
        "renderSelectedTask",
        "agents: dashboard.agents || null",
        "pool: dashboard.pool || null",
        "distributor: dashboard.distributor || null",
        "campaign: dashboard.campaign || null",
        "selectedTask: dashboard.selectedTask || null",
        "kpis: dashboard.kpis || null",
        "workspace: dashboard.workspace || null",
        "eventFeed: dashboard.eventFeed || null",
        "events: asArray(dashboard.events)",
        "skeleton-line",
        "campaign-grid",
        "score-chip",
    ] {
        assert!(
            AKRA_DASHBOARD_TEMPLATE.contains(copy),
            "graphic dashboard should expose {copy}"
        );
    }

    for anchor in [
        "id=\"pool\"",
        "id=\"agents\"",
        "id=\"pipeline\"",
        "id=\"campaign\"",
        "id=\"attempts\"",
        "id=\"intel\"",
    ] {
        assert!(
            AKRA_DASHBOARD_TEMPLATE.contains(anchor),
            "graphic dashboard should expose sidebar target {anchor}"
        );
    }

    for route in [
        ".route(\"/admin/akra\", get(pages::akra_dashboard_page))",
        ".route(\"/admin/akra/metrics\", get(pages::akra_metrics_page))",
        "\"/api/admin/akra/dashboard\"",
        "\"/api/admin/akra/pool\"",
        "\"/api/admin/akra/agents\"",
        "\"/api/admin/akra/distributor\"",
        "\"/api/admin/akra/events\"",
        "\"/admin/assets/graphics/{asset_name}\"",
        "\"/admin/assets/game/{asset_name}\"",
    ] {
        assert!(
            ADMIN_MOD.contains(route),
            "admin route table should keep {route}"
        );
    }

    for token in [
        "mountDiorama",
        "rebuildAgentUnits",
        "PIXI.Application",
        "gamebaljeonguk_atlas_128x192.png",
        "src/akra-diorama.ts",
        "chooseRoamPoint",
        "updateRoamMotion",
        "applyWalkFrame",
        "buildAgentFrameSets",
    ] {
        assert!(
            AKRA_DIORAMA_JS.contains(token),
            "admin game diorama asset should expose {token}"
        );
    }
}

#[test]
fn akra_graphic_dashboard_game_bundle_is_vite_typescript_input() {
    for token in [
        "\"build\": \"vite build --config vite.config.ts && node scripts/promote-build.mjs\"",
        "\"check\": \"tsc --noEmit --project tsconfig.json\"",
        "\"typescript\":",
        "\"vite\":",
    ] {
        assert!(
            ADMIN_GAME_PACKAGE_JSON.contains(token),
            "admin game package should keep {token}"
        );
    }

    for token in [
        "entry: \"src/akra-diorama.ts\"",
        "formats: [\"iife\"]",
        "fileName: () => \"akra-diorama.js\"",
        "name: \"AkraAdminDioramaBundle\"",
        "outDir: \"dist\"",
    ] {
        assert!(
            ADMIN_GAME_VITE_CONFIG.contains(token),
            "admin game Vite config should keep {token}"
        );
    }

    for token in [
        "type StatusSeverity",
        "interface DioramaHandle",
        "declare const PIXI",
        "const mountDiorama = (): DioramaHandle | null",
        "window.AkraAdminGame",
        "PIXI.Assets.load",
        "app.ticker.add",
        "type Facing = \"down\" | \"side\" | \"up\"",
        "interface AgentFrameSet",
        "interface AgentSpeechBubble",
        "const chooseRoamPoint",
        "const updateRoamMotion",
        "const applyWalkFrame",
        "const speechTextStyleFor",
        "window.getComputedStyle(speechNode)",
        "fontFamily: speechStyle?.fontFamily",
        "const makeSpeechBubble",
        "const applySpeechBubbleFrame",
        "const AGENT_FRAME_WIDTH = 128",
        "const AGENT_FRAME_HEIGHT = 192",
        "const AGENT_SPRITE_SCALE = 0.4675",
        "const AGENT_SPEECH_BUBBLES_DEFAULT_ENABLED = true",
        "setSpeechBubblesEnabled",
        "gamebaljeonguk_atlas_128x192.png",
    ] {
        assert!(
            AKRA_DIORAMA_TS.contains(token),
            "admin game TypeScript source should keep {token}"
        );
    }

    for token in ["dist/akra-diorama.js", "akra-diorama.js", "copyFileSync"] {
        assert!(
            ADMIN_GAME_PROMOTE_BUILD.contains(token),
            "admin game promote script should keep {token}"
        );
    }
}

#[test]
fn akra_graphic_dashboard_visual_contract_has_regression_guardrails() {
    for token in [
        "grid-template-columns: repeat(8",
        "class=\"office-board\" id=\"agents\"",
        "class=\"pool-overlay\" id=\"pool\"",
        "class=\"scene-object object-sprite server-rack\"",
        "background-image: var(--object-sprite-sheet)",
        "background-size: 627px 627px",
        "background-image: var(--agent-sprite-sheet)",
        "background-size: 384px 504px",
        "background-position: -288px 0",
        "--office-board-height: 720px",
        "grid-template-columns: minmax(0, 1fr)",
        "overflow: auto",
        "text-overflow: ellipsis",
        "@media (max-width: 860px)",
        "generated_time_label",
        "automation_epoch",
        "readiness_notice",
        "blocked_action",
        "queue_depth_basis",
        "mock_metric_note",
        "CampaignView",
        "map_campaign",
        "stage {progress}/100",
        "--office-bg-image",
        "--object-sprite-sheet",
        "--agent-sprite-sheet",
        "var(--office-bg-image)",
        "data-detail-type=\"slot\"",
        "data-detail-type=\"distributor\"",
        "data-detail-type=\"queueItem\"",
        "class=\"scene-object desk agent-{{ loop.index }} severity-{{ slot.severity }}\"",
        "data-task-id=\"{{ slot.task_id.as_deref().unwrap_or(\"\") }}\"",
        "avatar-{{ slot.avatar_class_label }}",
        "const createSlotAgentButton",
        "renderAgents(dashboard.pool)",
        "button.append(createText(\"span\", \"speech\", slot.bubbleLabel), sprite, label);",
        "data-detail-title=\"워크트리 풀 · {{ slot.display_slot_label }}\"",
        "data-detail-subtitle=\"{{ slot.label }}\"",
        "data-detail-slot=\"{{ slot.display_slot_label }}\"",
        "data-detail-task=\"{{ slot.task_id.as_deref().unwrap_or(\"-\") }}\"",
        "data-detail-branch=\"{{ slot.branch_name }}\"",
        "data-detail-worktree=\"{{ slot.worktree_label }}\"",
        "data-detail-owner=\"{{ slot.owner_label }}\"",
        "title=\"{{ slot.display_slot_label }} · {{ slot.label }} · task",
        "const slotDisplayLabel = optionalText(slot.displaySlotLabel || slot.slotId, \"슬롯\")",
        "const slotTaskId = optionalText(slot.taskId, \"-\")",
        "detailTitle: `워크트리 풀 · ${slotDisplayLabel}`",
        "detailSlot: slotDisplayLabel",
        "detailTask: slotTaskId",
        "createText(\"strong\", \"\", slotDisplayLabel)",
        "createText(\"small\", \"\", slotStateLabel)",
        "class=\"admin-detail-drawer\"",
        "pool reconcile, distributor tick, queue mutation은 호출하지 않습니다.",
        "var(--office-bg-image) center / cover no-repeat",
        "akraStageScan",
        "makePacket",
        "statusPalette",
        "chooseRoamPoint",
        "updateRoamMotion",
        "applyWalkFrame",
    ] {
        assert!(
            AKRA_DASHBOARD_TEMPLATE.contains(token)
                || BASE_TEMPLATE.contains(token)
                || AKRA_DASHBOARD_RS.contains(token)
                || AKRA_DIORAMA_JS.contains(token),
            "graphic visual contract should keep {token}"
        );
    }

    for removed in [
        "class=\"akra-topbar\"",
        "class=\"ops-status\"",
        "class=\"right-stack\"",
        "id=\"metrics\"",
        "id=\"system\"",
        "akra_admin",
        "Last Updated",
        "길드 성과",
        "운영 지표",
        "read-only 운영 관제",
        "게임화 정책",
        "도메인 매핑",
        "blocked-copy",
        "renderOpsStatus",
        "syncTopNotice",
        "renderMetrics",
        "renderSystem",
        "error-notice",
    ] {
        assert!(
            !AKRA_DASHBOARD_TEMPLATE.contains(removed),
            "graphic dashboard should not restore removed top header token {removed}"
        );
    }

    for removed in [
        "data-detail-title=\"풀 슬롯 · {{ slot.slot_id }}\"",
        "data-detail-subtitle=\"{{ slot.label }} / {{ slot.note }}\"",
        "title=\"{{ slot.branch_name }} / {{ slot.worktree_label }} / {{ slot.note }}\"",
        "<strong>{{ slot.slot_id }}</strong>",
        "<small>{{ slot.owner_agent_id.as_deref().unwrap_or(\"-\") }}</small>",
        "detailTitle: `풀 슬롯 · ${optionalText(slot.slotId)}`",
        "detailSubtitle: `${optionalText(slot.label)} / ${optionalText(slot.note)}`",
        "button.title = `${optionalText(slot.branchName)} / ${optionalText(slot.worktreeLabel)} / ${optionalText(slot.note)}`",
        "createText(\"strong\", \"\", slot.slotId)",
        "createText(\"small\", \"\", slot.ownerAgentId || \"-\")",
        "const slotStatusLabel = optionalText(slot.bubbleLabel || slot.label, \"풀\")",
    ] {
        assert!(
            !AKRA_DASHBOARD_TEMPLATE.contains(removed),
            "pool slot hover should not expose raw operator token {removed}"
        );
    }

    for token in [
        "aria-label=\"AKRA detached metrics\"",
        "id=\"metrics\"",
        "id=\"system\"",
        "길드 성과",
        "운영 지표",
        "풀 활용률",
        "지표 출처",
        "dashboard.metrics.badges",
        "dashboard.metrics.pool_utilization_percent",
    ] {
        assert!(
            AKRA_METRICS_TEMPLATE.contains(token),
            "detached metrics page should expose {token}"
        );
    }

    for token in [
        "templates/admin/resources/main-sprite.png",
        "gamebaljeonguk_atlas_64x96.png",
        "ADMIN_GRAPHIC_CAPTURE",
        "ADMIN_GAME_BUILD",
        "npm --prefix assets/admin/game run check",
        "npm --prefix assets/admin/game run build",
        "akra-admin",
        "/admin/akra",
        "/admin/akra/metrics",
        "/admin/tasks",
        "admin-tasks.html",
        "/admin/assets/graphics/akra-office-background.png",
        "/admin/assets/graphics/akra-object-sprites.png",
        "/admin/assets/graphics/gamebaljeonguk_atlas_64x96.png",
        "/admin/assets/graphics/gamebaljeonguk_atlas_128x192.png",
        "/admin/assets/game/akra-diorama.js",
        "/api/admin/akra/dashboard",
        "/api/admin/akra/events?limit=50",
        "/api/admin/akra/events?afterSequence=0&limit=50",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "${HOME}/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        "id=\"campaign\"",
        "id=\"attempts\"",
        "id=\"intel\"",
        "aria-label=\"게임발전국 작업 관리\"",
        "class=\"task-command-grid\"",
        "data-list-filter=\"task-list\"",
        "\"campaign\"",
        "\"laneCards\"",
        "\"intelCards\"",
        "served office background asset does not match workspace asset",
        "served object sprite asset does not match workspace asset",
        "served gamebaljeonguk agent atlas does not match workspace asset",
        "served large gamebaljeonguk agent atlas does not match workspace asset",
        "--screenshot=",
        "admin graphic visual contract ok",
    ] {
        assert!(
            ADMIN_GRAPHIC_VISUAL_SCRIPT.contains(token),
            "visual regression script should keep {token}"
        );
    }

    for token in [
        "include_bytes!(\"../../../../assets/admin/graphics/akra-office-background.png\")",
        "include_bytes!(\"../../../../assets/admin/graphics/akra-object-sprites.png\")",
        "include_bytes!(\"../../../../assets/admin/graphics/gamebaljeonguk_atlas_64x96.png\")",
        "include_bytes!(\"../../../../assets/admin/graphics/gamebaljeonguk_atlas_128x192.png\")",
        "include_bytes!(\"../../../../assets/admin/game/akra-diorama.js\")",
        "image/png",
        "text/javascript; charset=utf-8",
        "public, max-age=86400",
    ] {
        assert!(
            ADMIN_STATIC_ASSETS.contains(token),
            "admin graphic asset route should keep {token}"
        );
    }
}

#[test]
fn akra_dashboard_reads_planning_queue_through_admin_facade_projection() {
    assert!(
        AKRA_DASHBOARD_RS.contains("load_runtime_application_projection"),
        "dashboard should ask the admin facade for the shared planning projection"
    );
    assert!(
        AKRA_DASHBOARD_RS.contains("inspect_dashboard_snapshot_from_projection"),
        "dashboard should pass planning projection facts into parallel control-plane readiness"
    );
    assert!(
        !AKRA_DASHBOARD_RS.contains("PlanningApplicationProjection::from_runtime_projection"),
        "dashboard adapter should not rebuild planning projection from runtime internals"
    );
    assert!(
        !AKRA_DASHBOARD_RS.contains("PlanningServices"),
        "dashboard adapter should not depend on the broad planning service bundle"
    );
    assert!(
        !AKRA_DASHBOARD_RS.contains(".queue_projection()"),
        "dashboard adapter should not read queue projection internals directly"
    );
}

#[test]
fn akra_parallel_admin_surface_is_read_only_snapshot_projection() {
    /*
     * Admin Akra routes inspect parallel mode through the application
     * control-plane composition; they do not provide a second manual
     * tick/mutation surface beside CLI/TUI.
     */
    assert!(
        AKRA_DASHBOARD_RS.contains("inspect_dashboard_snapshot_from_projection"),
        "admin dashboard should render through the parallel control-plane composition"
    );
    assert!(
        AKRA_DASHBOARD_RS.contains("build_runtime_events_snapshot"),
        "admin event feed should render through the control-plane read surface"
    );
    for forbidden in [
        "run_orchestrator_tick",
        "process_distributor_queue",
        "ParallelModeService",
        "ParallelModeControlPlaneCommand",
        "ParallelModeControlPlaneEvent",
    ] {
        assert!(
            !AKRA_DASHBOARD_RS.contains(forbidden),
            "admin dashboard should not issue parallel control-plane commands: {forbidden}"
        );
        assert!(
            !ADMIN_API.contains(forbidden),
            "admin API routes should not issue parallel control-plane commands: {forbidden}"
        );
    }
}

#[test]
fn akra_graphic_dashboard_gamebaljeonguk_sprite_pack_is_reviewable() {
    for token in [
        "gamebaljeonguk_original_transparent.png",
        "gamebaljeonguk_atlas_128x192.png",
        "gamebaljeonguk_atlas_64x96.png",
        "$gamebaljeonguk_planner.png",
        "$gamebaljeonguk_coffee_addict.png",
        "Cell size: 64x96",
    ] {
        assert!(
            GAMEBALJEONGUK_SPRITE_PACK_README.contains(token),
            "gamebaljeonguk sprite pack readme should keep {token}"
        );
    }

    for token in [
        "\"file\": \"gamebaljeonguk_atlas_64x96.png\"",
        "\"cell_width\": 64",
        "\"cell_height\": 96",
        "\"$gamebaljeonguk_planner.png\"",
        "\"$gamebaljeonguk_coffee_addict.png\"",
        "\"planner_down_01\"",
        "\"coffee_addict_down_01\"",
    ] {
        assert!(
            GAMEBALJEONGUK_SPRITE_METADATA.contains(token),
            "gamebaljeonguk sprite metadata should keep {token}"
        );
    }
}

/*
 * browser confirmation은 destructive admin POST가 page를 떠나기 전 마지막 inbound guard다.
 * 서버의 CSRF 검증은 caller intent를 확인하지만, operator가 클릭 실수를 했는지는 template만 막을 수 있다.
 * 그래서 이 테스트는 global submit hook과 per-button data-confirm marker를 함께 확인한다.
 */
#[test]
fn risky_admin_mutations_require_browser_confirmation() {
    // capture-phase registration은 nested form/button 구조가 confirmation hook을 우회하지 못하게 한다.
    assert!(BASE_TEMPLATE.contains("document.addEventListener(\"submit\""));
    assert!(BASE_TEMPLATE.contains("}, true);"));

    // 첫 pass는 특정 template이 risky-action marker를 모두 잃었을 때 page 이름이 보이는 실패 메시지를 제공한다.
    for (template_name, template) in [
        ("controls", CONTROLS_TEMPLATE),
        ("directions", DIRECTIONS_TEMPLATE),
        ("editor", EDITOR_TEMPLATE),
        ("tasks", TASKS_TEMPLATE),
    ] {
        assert!(
            template.contains("data-confirm="),
            "{template_name} should mark risky submit buttons"
        );
    }

    // exact count는 mutating button 추가/삭제가 confirmation contract 변경으로 review되도록 강제한다.
    assert_eq!(CONTROLS_TEMPLATE.matches("data-confirm=").count(), 4);
    assert_eq!(DIRECTIONS_TEMPLATE.matches("data-confirm=").count(), 2);
    assert_eq!(EDITOR_TEMPLATE.matches("data-confirm=").count(), 1);
    assert_eq!(TASKS_TEMPLATE.matches("data-confirm=").count(), 2);
}

#[test]
fn controls_page_exposes_parallel_agent_persona_selector() {
    assert!(CONTROLS_TEMPLATE.contains("Parallel Agent Persona"));
    assert!(CONTROLS_TEMPLATE.contains("name=\"persona\""));
    assert!(CONTROLS_TEMPLATE.contains("No persona prompt will be injected."));
    assert!(ADMIN_MOD.contains("\"/admin/controls/parallel-persona\""));
    assert!(ADMIN_MOD.contains("update_parallel_persona_page"));
}
