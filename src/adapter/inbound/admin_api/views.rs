/*
 * views.rs는 Askama template boundary다.
 * handler는 application service에서 read model을 미리 로드하고, 이 struct들은 admin template 파일이 접근할 수 있는
 * 변수 목록을 compile-time contract로 고정한다. template은 service나 filesystem을 직접 호출하지 못하므로,
 * 화면에 필요한 planning state는 여기 필드로 드러난 값 안에서만 렌더링된다.
 */
use askama::Template;

use crate::application::service::planning::{
    PlanningAdminManagementView, PlanningAdminOverview, PlanningAdminSessionView,
};

// dashboard는 read-only workspace status 화면이라 overview bundle만 받고 editable management/session state는 받지 않는다.
#[derive(Template)]
#[template(path = "admin/dashboard.html")]
pub(super) struct DashboardTemplate {
    // shared layout identity와 nav marker를 모든 admin page가 반복해 같은 base shell 위에서 현재 위치를 표시한다.
    pub(super) page_title: String,
    pub(super) current_nav: &'static str,
    // browser form은 TUI process state를 상속하지 않으므로 workspace와 CSRF context를 각 page context에 명시한다.
    pub(super) workspace_dir: String,
    pub(super) csrf_token: String,
    // notice는 redirect flash channel이다. None이면 template이 alert region 전체를 생략할 수 있다.
    pub(super) notice: Option<String>,
    // overview는 landing page가 필요한 runtime, queue, doctor, direction summary를 하나의 projection으로 운반한다.
    pub(super) overview: PlanningAdminOverview,
}

// directions page는 workspace health와 editable direction/task management bundle을 함께 보여준다.
#[derive(Template)]
#[template(path = "admin/directions.html")]
pub(super) struct DirectionsTemplate {
    pub(super) page_title: String,
    pub(super) current_nav: &'static str,
    pub(super) workspace_dir: String,
    pub(super) csrf_token: String,
    pub(super) notice: Option<String>,
    // direction edit 중에도 validation/runtime 영향이 보이도록 editing table 옆에 overview를 유지한다.
    pub(super) overview: PlanningAdminOverview,
    // management는 page-wide read model이다. direction form도 task/direction cross reference를 같은 projection에서 가져와야 한다.
    pub(super) management: PlanningAdminManagementView,
}

// tasks page는 같은 management bundle을 쓰되 accepted/proposed/skipped task authority를 중심으로 렌더링한다.
#[derive(Template)]
#[template(path = "admin/tasks.html")]
pub(super) struct TasksTemplate {
    pub(super) page_title: String,
    pub(super) current_nav: &'static str,
    pub(super) workspace_dir: String,
    pub(super) csrf_token: String,
    pub(super) notice: Option<String>,
    // task edit은 follow-up execution에 직접 영향을 주므로 queue/runtime summary를 task page에도 남긴다.
    pub(super) overview: PlanningAdminOverview,
    // task edit form은 direction choice가 필요하므로 task-only DTO를 쓰면 template-side lookup이 생긴다.
    pub(super) management: PlanningAdminManagementView,
}

// controls page는 reset/reload/doctor 같은 workspace-level action을 노출하고 무거운 editing bundle은 받지 않는다.
#[derive(Template)]
#[template(path = "admin/controls.html")]
pub(super) struct ControlsTemplate {
    pub(super) page_title: String,
    pub(super) current_nav: &'static str,
    pub(super) workspace_dir: String,
    pub(super) csrf_token: String,
    pub(super) notice: Option<String>,
    // button availability, destructive-action context, current runtime 설명에는 overview projection만 있으면 충분하다.
    pub(super) overview: PlanningAdminOverview,
}

// editor는 session-scoped 화면이다. draft file, validation, queue preview, active file state가 하나의 read model로 이동한다.
#[derive(Template)]
#[template(path = "admin/editor.html")]
pub(super) struct EditorTemplate {
    pub(super) page_title: String,
    pub(super) current_nav: &'static str,
    pub(super) workspace_dir: String,
    pub(super) csrf_token: String,
    pub(super) notice: Option<String>,
    // session view를 통째로 넘겨 template이 active file, validation, queue preview를 얻기 위해 service를 다시 부르지 않게 한다.
    pub(super) session: PlanningAdminSessionView,
}

// draft status partial은 validation/status refresh에 필요한 editor context의 HTMX-sized subset이다.
#[derive(Template)]
#[template(path = "admin/partials/draft_status.html")]
pub(super) struct DraftStatusTemplate {
    // partial response도 full editor reload와 같은 flash vocabulary를 써서 notice 표현이 갈라지지 않게 한다.
    pub(super) notice: Option<String>,
    // full session view를 재사용하면 partial과 full-page의 status copy가 서로 다른 projection으로 drift하지 않는다.
    pub(super) session: PlanningAdminSessionView,
}
