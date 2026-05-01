use askama::Template;

use crate::application::service::planning::{
    PlanningAdminManagementView, PlanningAdminOverview, PlanningAdminSessionView,
};

#[derive(Template)]
#[template(path = "admin/dashboard.html")]
pub(super) struct DashboardTemplate {
    pub(super) page_title: String,
    pub(super) current_nav: &'static str,
    pub(super) workspace_dir: String,
    pub(super) csrf_token: String,
    pub(super) notice: Option<String>,
    pub(super) overview: PlanningAdminOverview,
}

#[derive(Template)]
#[template(path = "admin/directions.html")]
pub(super) struct DirectionsTemplate {
    pub(super) page_title: String,
    pub(super) current_nav: &'static str,
    pub(super) workspace_dir: String,
    pub(super) csrf_token: String,
    pub(super) notice: Option<String>,
    pub(super) overview: PlanningAdminOverview,
    pub(super) management: PlanningAdminManagementView,
}

#[derive(Template)]
#[template(path = "admin/tasks.html")]
pub(super) struct TasksTemplate {
    pub(super) page_title: String,
    pub(super) current_nav: &'static str,
    pub(super) workspace_dir: String,
    pub(super) csrf_token: String,
    pub(super) notice: Option<String>,
    pub(super) overview: PlanningAdminOverview,
    pub(super) management: PlanningAdminManagementView,
}

#[derive(Template)]
#[template(path = "admin/controls.html")]
pub(super) struct ControlsTemplate {
    pub(super) page_title: String,
    pub(super) current_nav: &'static str,
    pub(super) workspace_dir: String,
    pub(super) csrf_token: String,
    pub(super) notice: Option<String>,
    pub(super) overview: PlanningAdminOverview,
}

#[derive(Template)]
#[template(path = "admin/editor.html")]
pub(super) struct EditorTemplate {
    pub(super) page_title: String,
    pub(super) current_nav: &'static str,
    pub(super) workspace_dir: String,
    pub(super) csrf_token: String,
    pub(super) notice: Option<String>,
    pub(super) session: PlanningAdminSessionView,
}

#[derive(Template)]
#[template(path = "admin/partials/draft_status.html")]
pub(super) struct DraftStatusTemplate {
    pub(super) notice: Option<String>,
    pub(super) session: PlanningAdminSessionView,
}
