use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::application::service::planning::{
    PlanningAdminDraftFileUpdate, PlanningAdminDraftKind, PlanningAdminOverview,
    PlanningAdminSessionView,
};

/*
 * forms.rs is the narrow transport DTO layer for planning admin. The application service already
 * owns validation, domain enums, and filesystem mutation policy; this module only preserves the
 * shapes that Axum extracts from URL queries, classic browser forms, and JSON bodies. Keeping those
 * shapes here stops pages.rs and api.rs from sharing ad-hoc HashMap parsing rules.
 */
#[derive(Debug, Clone, Deserialize)]
pub(super) struct EditorQuery {
    /*
     * The draft name lives in the route path, while kind and optional direction_id select which
     * service branch can interpret that name. notice is UI-only redirect state and never becomes
     * part of the draft session request.
     */
    pub(super) kind: PlanningAdminDraftKind,
    #[serde(default)]
    pub(super) direction_id: Option<String>,
    #[serde(default)]
    pub(super) notice: Option<String>,
}

/*
 * Browser form DTOs intentionally include csrf_token in the body. pages.rs verifies that token
 * against the cookie before building typed application requests, so service-layer mutation methods
 * never need to know whether a call came from HTML, HTMX, or JSON.
 */
#[derive(Debug, Clone, Deserialize)]
pub(super) struct CreateDraftForm {
    pub(super) csrf_token: String,
    // direction_id is meaningful only for direction-detail drafts; the service validates that pair.
    pub(super) kind: PlanningAdminDraftKind,
    #[serde(default)]
    pub(super) direction_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct DraftMutationForm {
    pub(super) csrf_token: String,
    pub(super) kind: PlanningAdminDraftKind,
    #[serde(default)]
    pub(super) direction_id: Option<String>,
    /*
     * The editor renders editable files with dynamic field names such as file_result_output. Axum
     * cannot deserialize that into a fixed struct, so the adapter flattens unknown fields and
     * pages::extract_file_updates admits only the supported PlanningAdminFileKey variants.
     */
    #[serde(flatten)]
    pub(super) values: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ResetForm {
    pub(super) csrf_token: String,
    // Kept as transport text so page and API handlers both flow through parse_reset_target.
    pub(super) target: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct DirectionMutationForm {
    pub(super) csrf_token: String,
    /*
     * HTML form controls send every admin direction field as text. Empty id means create, non-empty
     * id means update, and the remaining strings are normalized by PlanningAdminDirectionMutation
     * handling so browser quirks do not leak into domain authority documents.
     */
    #[serde(default)]
    pub(super) id: String,
    pub(super) title: String,
    #[serde(default)]
    pub(super) summary: String,
    #[serde(default)]
    pub(super) success_criteria_text: String,
    #[serde(default)]
    pub(super) scope_hints_text: String,
    #[serde(default)]
    pub(super) detail_doc_path: String,
    #[serde(default)]
    pub(super) state: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct IdDeleteForm {
    pub(super) csrf_token: String,
    // Shared by direction and task delete routes; the route decides which service request owns it.
    pub(super) id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct TaskMutationForm {
    pub(super) csrf_token: String,
    /*
     * Task editing is intentionally stringly at the form boundary. Priority numbers, status
     * labels, dependency lists, and blocker lists are parsed in the application mutation service
     * where direction cross-references and queue semantics are available.
     */
    #[serde(default)]
    pub(super) id: String,
    #[serde(default)]
    pub(super) direction_id: String,
    pub(super) title: String,
    #[serde(default)]
    pub(super) description: String,
    #[serde(default)]
    pub(super) status: String,
    #[serde(default)]
    pub(super) base_priority: String,
    #[serde(default)]
    pub(super) dynamic_priority_delta: String,
    #[serde(default)]
    pub(super) priority_reason: String,
    #[serde(default)]
    pub(super) depends_on_text: String,
    #[serde(default)]
    pub(super) blocked_by_text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct FileSyncForm {
    // Export/apply controls have no operator payload; CSRF is the whole browser form contract.
    pub(super) csrf_token: String,
}

/*
 * JSON requests omit csrf_token from the body because api.rs verifies the same cookie-bound token
 * through the x-csrf-token header. The body can therefore mirror the service inputs more closely
 * than the classic HTML form structs above.
 */
#[derive(Debug, Clone, Deserialize)]
pub(super) struct CreateDraftRequest {
    pub(super) kind: PlanningAdminDraftKind,
    #[serde(default)]
    pub(super) direction_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct SaveDraftRequest {
    pub(super) kind: PlanningAdminDraftKind,
    #[serde(default)]
    pub(super) direction_id: Option<String>,
    // JSON clients can send already-typed file updates, avoiding HTML's dynamic file_* field map.
    #[serde(default)]
    pub(super) files: Vec<PlanningAdminDraftFileUpdate>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ResetRequest {
    // API reset uses the same accepted labels as ResetForm via parse_reset_target.
    pub(super) target: String,
}

/*
 * API responses expose read models with just enough adapter metadata for the browser client. The
 * csrf_token returned from summary lets a single-page admin client bootstrap later JSON mutations,
 * while planning state stays in application-owned projection types.
 */
#[derive(Debug, Clone, Serialize)]
pub(super) struct OverviewApiResponse {
    pub(super) csrf_token: String,
    pub(super) overview: PlanningAdminOverview,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct DraftPromoteApiResponse {
    /*
     * Promotion returns both a compact success summary and the refreshed session. The booleans and
     * counts power lightweight client feedback; the session lets the editor redraw validation and
     * file state without issuing a second load request.
     */
    pub(super) promoted_file_count: usize,
    pub(super) is_valid: bool,
    pub(super) session: PlanningAdminSessionView,
}
