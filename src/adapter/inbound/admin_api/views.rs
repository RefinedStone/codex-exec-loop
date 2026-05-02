// 학습 주석: admin_api views는 Askama가 Rust struct를 HTML template context로 컴파일하는 경계입니다.
// handler는 application service view를 가져와 이 struct에 넣고, Askama는 templates/admin/*.html에서 필드를 읽습니다.
use askama::Template;

// 학습 주석: admin HTML은 domain entity를 직접 노출하지 않고 application service가 만든 read model을 받습니다.
// 이 inbound adapter는 overview/management/session view를 페이지별 template context에 재배치하는 역할만 합니다.
use crate::application::service::planning::{
    PlanningAdminManagementView, PlanningAdminOverview, PlanningAdminSessionView,
};

// 학습 주석: dashboard page는 planning admin의 첫 화면입니다. workspace-wide overview만 필요하므로
// management/session 편집 모델을 싣지 않고, runtime/doctor/queue 요약을 빠르게 렌더링하는 context입니다.
#[derive(Template)]
// 학습 주석: Askama가 이 struct의 필드 이름을 `templates/admin/dashboard.html`에서 직접 참조하도록 연결합니다.
#[template(path = "admin/dashboard.html")]
pub(super) struct DashboardTemplate {
    // 학습 주석: page_title은 layout title과 browser title에 쓰이는 페이지 identity입니다. handler가 route별
    // 제목을 넣어 같은 base layout에서도 현재 화면 맥락이 보이게 합니다.
    pub(super) page_title: String,
    // 학습 주석: current_nav는 template sidebar/header에서 active tab을 표시하는 key입니다. &'static str인
    // 이유는 route별 고정 값이라 request lifetime이나 allocation이 필요 없기 때문입니다.
    pub(super) current_nav: &'static str,
    // 학습 주석: workspace_dir은 admin operation의 대상 planning workspace입니다. web page는 TUI와 달리
    // URL/form만 보고는 workspace context가 약해지므로 모든 page context에 명시적으로 싣습니다.
    pub(super) workspace_dir: String,
    // 학습 주석: csrf_token은 mutating form submit을 보호하는 request-local token입니다. dashboard에도
    // control form이나 partial action이 붙을 수 있어 공통 context로 둡니다.
    pub(super) csrf_token: String,
    // 학습 주석: notice는 redirect/action 뒤 사용자에게 보여 줄 flash message입니다. None이면 template은
    // notice block을 생략해 빈 알림 영역을 만들지 않습니다.
    pub(super) notice: Option<String>,
    // 학습 주석: overview는 planning admin facade가 만든 workspace health/readiness snapshot입니다. dashboard는
    // 이 값을 중심으로 runtime, queue, doctor, directions 요약을 보여 줍니다.
    pub(super) overview: PlanningAdminOverview,
}

// 학습 주석: directions page는 planning directions를 읽고 관리하는 화면입니다. overview로 workspace 상태를
// 유지하면서, management view로 실제 direction 목록과 수정 action의 대상 데이터를 제공합니다.
#[derive(Template)]
// 학습 주석: 이 template은 `admin/directions.html`의 context입니다. Askama derive가 compile-time에 필드 접근을 검증합니다.
#[template(path = "admin/directions.html")]
pub(super) struct DirectionsTemplate {
    // 학습 주석: page_title은 directions 관리 화면의 문서 title입니다. base layout의 공통 title slot에 들어갑니다.
    pub(super) page_title: String,
    // 학습 주석: current_nav는 directions tab을 active 처리하는 routing key입니다.
    pub(super) current_nav: &'static str,
    // 학습 주석: workspace_dir은 direction 파일과 planning state가 속한 root를 사용자에게 확인시키는 page context입니다.
    pub(super) workspace_dir: String,
    // 학습 주석: csrf_token은 direction add/edit/delete form이 같은 protection token을 공유하게 합니다.
    pub(super) csrf_token: String,
    // 학습 주석: notice는 direction mutation 이후 결과를 page reload에서 전달하는 flash channel입니다.
    pub(super) notice: Option<String>,
    // 학습 주석: overview는 directions 화면에서도 runtime/validation 상태를 함께 보여 주기 위한 상단 요약입니다.
    pub(super) overview: PlanningAdminOverview,
    // 학습 주석: management는 editable directions/tasks 목록을 포함합니다. directions page는 이 중 direction
    // management slice를 주로 사용하지만, service가 같은 management bundle을 만들어 page 간 데이터 계약을 맞춥니다.
    pub(super) management: PlanningAdminManagementView,
}

// 학습 주석: tasks page는 planning task authority를 관리하는 화면입니다. accepted/proposed/skipped task를
// service projection에서 가져와 HTML form/action과 함께 렌더링합니다.
#[derive(Template)]
// 학습 주석: `admin/tasks.html`은 task management table과 queue-related actions를 이 context에서 읽습니다.
#[template(path = "admin/tasks.html")]
pub(super) struct TasksTemplate {
    // 학습 주석: page_title은 task management 화면의 상단/문서 제목입니다.
    pub(super) page_title: String,
    // 학습 주석: current_nav는 task tab active state를 결정하는 고정 key입니다.
    pub(super) current_nav: &'static str,
    // 학습 주석: workspace_dir은 task mutation이 적용될 workspace를 template과 사용자에게 명확히 전달합니다.
    pub(super) workspace_dir: String,
    // 학습 주석: csrf_token은 task promote/edit/delete 같은 POST action form에 들어갑니다.
    pub(super) csrf_token: String,
    // 학습 주석: notice는 task mutation 결과를 route reload 후 사용자에게 보여 주는 optional message입니다.
    pub(super) notice: Option<String>,
    // 학습 주석: overview는 task page 상단에서 queue health와 runtime state를 함께 보여 주는 공통 요약입니다.
    pub(super) overview: PlanningAdminOverview,
    // 학습 주석: management는 task table rows와 direction options를 함께 담습니다. task edit form은 direction
    // selection이 필요하므로 task-only DTO보다 management bundle이 page 요구에 맞습니다.
    pub(super) management: PlanningAdminManagementView,
}

// 학습 주석: controls page는 planning runtime/admin operations의 버튼성 control surface입니다. queue reset,
// doctor/reload 같은 workspace-level action을 overview 상태와 함께 보여 줍니다.
#[derive(Template)]
// 학습 주석: `admin/controls.html`은 별도 management table 없이 overview와 form controls만 읽습니다.
#[template(path = "admin/controls.html")]
pub(super) struct ControlsTemplate {
    // 학습 주석: page_title은 controls screen의 문서/페이지 제목입니다.
    pub(super) page_title: String,
    // 학습 주석: current_nav는 controls navigation item을 active로 표시합니다.
    pub(super) current_nav: &'static str,
    // 학습 주석: workspace_dir은 destructive/control actions가 어떤 workspace에 적용되는지 확인시키는 안전 정보입니다.
    pub(super) workspace_dir: String,
    // 학습 주석: csrf_token은 controls page의 POST forms에 들어가는 mutation guard입니다.
    pub(super) csrf_token: String,
    // 학습 주석: notice는 control action 실행 결과나 실패 사유를 reload된 page에 전달합니다.
    pub(super) notice: Option<String>,
    // 학습 주석: overview는 controls의 enable/disable copy와 현 runtime state 설명의 근거입니다.
    pub(super) overview: PlanningAdminOverview,
}

// 학습 주석: editor page는 planning draft session을 편집하는 HTML surface입니다. draft files, validation,
// queue preview를 `PlanningAdminSessionView` 하나로 받아 form fields와 status panels에 배치합니다.
#[derive(Template)]
// 학습 주석: `admin/editor.html`은 draft authoring 전체 page이며, session view 없이는 렌더링할 수 없습니다.
#[template(path = "admin/editor.html")]
pub(super) struct EditorTemplate {
    // 학습 주석: page_title은 editor page의 title slot입니다. draft kind나 session 상태와 별개로 route identity를 둡니다.
    pub(super) page_title: String,
    // 학습 주석: current_nav는 editor/draft navigation item active state를 결정합니다.
    pub(super) current_nav: &'static str,
    // 학습 주석: workspace_dir은 draft session이 어느 planning workspace에 속하는지 editor header와 forms에 전달합니다.
    pub(super) workspace_dir: String,
    // 학습 주석: csrf_token은 draft save/promote/file mutation form이 공유하는 protection token입니다.
    pub(super) csrf_token: String,
    // 학습 주석: notice는 draft save/promote 결과를 full editor page reload에서 보여 주는 flash message입니다.
    pub(super) notice: Option<String>,
    // 학습 주석: session은 draft editor의 핵심 read model입니다. active file, editable content, validation,
    // queue preview를 모두 포함해 template이 service/domain을 다시 호출하지 않게 합니다.
    pub(super) session: PlanningAdminSessionView,
}

// 학습 주석: draft status partial은 editor page 전체가 아니라 validation/status fragment만 다시 그릴 때 쓰는
// lightweight context입니다. HTMX/partial response 경로가 full page context 없이 session status만 갱신합니다.
#[derive(Template)]
// 학습 주석: 이 template은 `admin/partials/draft_status.html`에 연결되어 editor 내부 status region만 렌더링합니다.
#[template(path = "admin/partials/draft_status.html")]
pub(super) struct DraftStatusTemplate {
    // 학습 주석: notice는 partial response에서도 save/validate action의 결과 message를 같은 방식으로 보여 줍니다.
    pub(super) notice: Option<String>,
    // 학습 주석: session은 status partial이 validation, dirty files, queue preview를 읽는 source입니다. full page
    // template과 같은 service view를 사용해 partial/full rendering 사이 copy drift를 막습니다.
    pub(super) session: PlanningAdminSessionView,
}
