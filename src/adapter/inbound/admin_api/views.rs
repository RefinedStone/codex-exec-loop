// Admin views are the Askama boundary: handlers load application read models, then these structs define the exact
// template context available to `templates/admin/*.html`.
use askama::Template;

use crate::application::service::planning::{
    PlanningAdminManagementView, PlanningAdminOverview, PlanningAdminSessionView,
};

// Dashboard is read-only workspace status; it needs the overview bundle but not editable management/session state.
#[derive(Template)]
#[template(path = "admin/dashboard.html")]
pub(super) struct DashboardTemplate {
    // Shared layout identity and navigation fields keep every admin page on the same base shell.
    pub(super) page_title: String,
    pub(super) current_nav: &'static str,
    // Every page repeats workspace and CSRF context because web forms do not inherit TUI process state.
    pub(super) workspace_dir: String,
    pub(super) csrf_token: String,
    // Notice is the redirect flash channel; None lets templates omit the whole alert region.
    pub(super) notice: Option<String>,
    // Overview carries runtime, queue, doctor, and direction summaries for the landing page.
    pub(super) overview: PlanningAdminOverview,
}

// Directions combines workspace health with the editable direction/task management bundle.
#[derive(Template)]
#[template(path = "admin/directions.html")]
pub(super) struct DirectionsTemplate {
    pub(super) page_title: String,
    pub(super) current_nav: &'static str,
    pub(super) workspace_dir: String,
    pub(super) csrf_token: String,
    pub(super) notice: Option<String>,
    // Keep overview visible beside editing tables so operators see validation/runtime impact while changing directions.
    pub(super) overview: PlanningAdminOverview,
    // Management is intentionally page-wide; direction forms still need task/direction cross references from one read model.
    pub(super) management: PlanningAdminManagementView,
}

// Tasks uses the same management bundle, but templates focus on accepted/proposed/skipped task authority.
#[derive(Template)]
#[template(path = "admin/tasks.html")]
pub(super) struct TasksTemplate {
    pub(super) page_title: String,
    pub(super) current_nav: &'static str,
    pub(super) workspace_dir: String,
    pub(super) csrf_token: String,
    pub(super) notice: Option<String>,
    // Queue/runtime summary remains on task pages because task edits directly affect follow-up execution.
    pub(super) overview: PlanningAdminOverview,
    // Task edit forms need direction choices, so a task-only DTO would force template-side lookups.
    pub(super) management: PlanningAdminManagementView,
}

// Controls exposes workspace-level actions such as reset/reload/doctor without the heavier editing bundles.
#[derive(Template)]
#[template(path = "admin/controls.html")]
pub(super) struct ControlsTemplate {
    pub(super) page_title: String,
    pub(super) current_nav: &'static str,
    pub(super) workspace_dir: String,
    pub(super) csrf_token: String,
    pub(super) notice: Option<String>,
    // Overview is enough for button availability, destructive-action context, and current runtime explanation.
    pub(super) overview: PlanningAdminOverview,
}

// Editor is session-scoped: draft files, validation, queue preview, and active file state travel as one read model.
#[derive(Template)]
#[template(path = "admin/editor.html")]
pub(super) struct EditorTemplate {
    pub(super) page_title: String,
    pub(super) current_nav: &'static str,
    pub(super) workspace_dir: String,
    pub(super) csrf_token: String,
    pub(super) notice: Option<String>,
    // Session view prevents templates from calling back into services for active file, validation, or queue preview data.
    pub(super) session: PlanningAdminSessionView,
}

// Draft status partial is the HTMX-sized subset of the editor context for validation/status refreshes.
#[derive(Template)]
#[template(path = "admin/partials/draft_status.html")]
pub(super) struct DraftStatusTemplate {
    // Partial responses still use the same flash vocabulary as full editor reloads.
    pub(super) notice: Option<String>,
    // Reusing the full session view keeps partial and full-page status copy from drifting.
    pub(super) session: PlanningAdminSessionView,
}
