// 학습 주석: popup view DTO는 이미 ratatui `Line`으로 변환된 presentation copy를 담습니다. builder가 domain
// 상태를 여기까지 낮춰 두면 renderer는 string 조합이나 service 상태 해석 없이 화면 배치만 맡습니다.
use super::super::super::Line;
// 학습 주석: session overlay만 list cursor/selection metadata가 필요합니다. 공통 `OverlayListView`를 재사용해
// session browser와 다른 overlay list 계열이 같은 scrolling/selection contract를 따르게 합니다.
use super::super::OverlayListView;

// 학습 주석: startup overlay view는 shell boot diagnostics를 popup renderer에 넘기는 read model입니다.
// startup_banner builder가 환경 점검 결과를 이 구획들로 나누고, renderer는 header/summary/check/warning/key
// 순서로 배치해 사용자가 실행 가능 여부와 다음 키 입력을 한 화면에서 확인하게 합니다.
pub(crate) struct StartupOverlayView {
    // 학습 주석: header_lines는 product title과 startup context입니다. 다른 startup 세부 정보와 분리해
    // popup chrome의 최상단 identity가 diagnostics 결과에 따라 흔들리지 않게 합니다.
    pub(crate) header_lines: Vec<Line<'static>>,
    // 학습 주석: summary_lines는 workspace, attachment mode, app-server 상태처럼 startup 판단의 요약입니다.
    // check_lines보다 먼저 보여 사용자가 전체 상태를 빠르게 스캔하게 합니다.
    pub(crate) summary_lines: Vec<Line<'static>>,
    // 학습 주석: check_lines는 개별 prerequisite 검사 결과입니다. 각 line은 이미 성공/주의/실패 copy로 만들어져
    // renderer가 severity 계산을 반복하지 않습니다.
    pub(crate) check_lines: Vec<Line<'static>>,
    // 학습 주석: warning_lines는 startup은 계속 가능하지만 사용자가 알아야 하는 제한을 모읍니다. summary와
    // check를 읽은 뒤 remediation hint처럼 따라붙는 영역입니다.
    pub(crate) warning_lines: Vec<Line<'static>>,
    // 학습 주석: key_lines는 Enter/Esc 같은 현재 startup overlay의 affordance입니다. 상태 copy와 분리해
    // renderer가 footer 성격의 command hint로 일정하게 처리합니다.
    pub(crate) key_lines: Vec<Line<'static>>,
}

// 학습 주석: session overlay view는 session catalog popup의 read model입니다. session_shell_controller가
// catalog state와 selection을 갱신하고, builder는 그 결과를 list/detail/warning/key 영역으로 나눕니다.
pub(crate) struct SessionOverlayView {
    // 학습 주석: header_lines는 "session browser" 성격과 catalog loading status를 고정 위치에 둡니다.
    // list 내용이 비거나 실패해도 overlay 목적은 유지됩니다.
    pub(crate) header_lines: Vec<Line<'static>>,
    // 학습 주석: list_view는 session rows와 selected index/scroll metadata를 함께 담습니다. renderer가
    // cursor 위치를 계산하려면 line copy뿐 아니라 selection state도 필요합니다.
    pub(crate) list_view: OverlayListView,
    // 학습 주석: detail_lines는 선택된 session의 thread id, cwd, summary 같은 오른쪽/하단 설명 영역입니다.
    // list row에는 넣기 어려운 긴 정보를 여기로 빼서 scanning과 inspection을 분리합니다.
    pub(crate) detail_lines: Vec<Line<'static>>,
    // 학습 주석: warning_lines는 catalog load 실패나 attach 제한처럼 선택과 별개로 표시해야 하는 메시지입니다.
    // detail_lines와 분리되어 selected row가 바뀌어도 경고 의미가 흐려지지 않습니다.
    pub(crate) warning_lines: Vec<Line<'static>>,
    // 학습 주석: key_lines는 session browser의 open/new/cancel/navigation hint입니다. controller key handling과
    // 일치해야 하므로 view 계약 안에 별도 footer 영역으로 둡니다.
    pub(crate) key_lines: Vec<Line<'static>>,
}

// 학습 주석: supersession overlay view는 parallel/supersession orchestration 상태를 여러 패널로 분해합니다.
// 각 필드는 scheduler capability, worker pool, roster, distributor처럼 서로 다른 운영 관심사를 담습니다.
pub(crate) struct SupersessionOverlayView {
    // 학습 주석: header_lines는 supersession overlay의 제목과 현재 mode를 담아 다른 popup들과 구분합니다.
    pub(crate) header_lines: Vec<Line<'static>>,
    // 학습 주석: summary_lines는 orchestration 전체 상태를 짧게 압축합니다. 아래 capability/pool/roster
    // 패널을 읽기 전에 현재 운전 상태를 먼저 제공합니다.
    pub(crate) summary_lines: Vec<Line<'static>>,
    // 학습 주석: capability_lines는 현재 runtime이 parallel controls를 지원하는지, 수동 handoff인지 같은
    // 제어 가능성을 설명합니다. 실제 button/key availability의 근거가 되는 영역입니다.
    pub(crate) capability_lines: Vec<Line<'static>>,
    // 학습 주석: pool_lines는 agent/worker pool의 크기와 사용 상태를 보여 줍니다. roster가 개별 항목이라면
    // pool은 capacity와 saturation을 보는 요약 패널입니다.
    pub(crate) pool_lines: Vec<Line<'static>>,
    // 학습 주석: roster_lines는 현재 참여 중인 worker/session들의 목록입니다. 사용자가 supersession 상태를
    // 추적할 때 "누가 무엇을 하고 있는지"를 읽는 주 영역입니다.
    pub(crate) roster_lines: Vec<Line<'static>>,
    // 학습 주석: detail_lines는 선택 또는 현재 focus와 관련된 추가 설명입니다. roster의 compact row와
    // 별도로 긴 path/status/reason copy를 담을 수 있습니다.
    pub(crate) detail_lines: Vec<Line<'static>>,
    // 학습 주석: distributor_lines는 task distribution/assignment 쪽 상태를 보여 줍니다. worker pool이 있어도
    // 실제 분배 정책과 backlog 상태는 별도 관심사라 독립 필드로 둡니다.
    pub(crate) distributor_lines: Vec<Line<'static>>,
    // 학습 주석: key_lines는 supersession overlay에서 가능한 제어 명령을 담습니다. capability_lines가
    // "가능/불가 이유"라면 key_lines는 "지금 누를 키"를 말합니다.
    pub(crate) key_lines: Vec<Line<'static>>,
}

// 학습 주석: queue overlay view는 planning runtime queue를 shell popup에 투영하는 DTO입니다. builder가
// ConversationState/PlanningRuntimeSnapshot을 읽어 queue, proposal, note 영역으로 분리합니다.
pub(crate) struct QueueOverlayView {
    // 학습 주석: header_lines는 planning queue overlay의 title과 runtime context입니다. Loading/Failed
    // conversation에서도 header가 유지되어 overlay 의미가 사라지지 않습니다.
    pub(crate) header_lines: Vec<Line<'static>>,
    // 학습 주석: summary_lines는 accepted queue revision, idle policy, invalid snapshot 같은 전체 상태입니다.
    // 실제 task row를 보기 전에 runtime이 믿을 수 있는지 판단하는 첫 단서입니다.
    pub(crate) summary_lines: Vec<Line<'static>>,
    // 학습 주석: queue_lines는 ready/skipped task rows입니다. domain queue 구조를 renderer가 직접 알 필요 없도록
    // 이미 display order와 copy가 정리된 Line 목록으로 넘깁니다.
    pub(crate) queue_lines: Vec<Line<'static>>,
    // 학습 주석: proposal_lines는 아직 accepted queue에 들어가지 않은 proposed next work를 보여 줍니다.
    // queue_lines와 분리해 확정된 작업과 후보 작업이 섞여 보이지 않게 합니다.
    pub(crate) proposal_lines: Vec<Line<'static>>,
    // 학습 주석: note_lines는 empty queue, invalid runtime, auto-follow 제한 같은 보조 설명입니다. queue/proposal
    // 자체가 비어 있어도 사용자가 왜 비어 있는지 알 수 있게 합니다.
    pub(crate) note_lines: Vec<Line<'static>>,
    // 학습 주석: key_lines는 queue overlay에서 가능한 navigation/close 명령입니다. queue 상태와 독립된
    // footer contract라서 마지막 필드로 일정하게 둡니다.
    pub(crate) key_lines: Vec<Line<'static>>,
}

// 학습 주석: task intake overlay view는 `:task` prompt를 planning task proposal/commit으로 이어 주는 modal의
// read model입니다. shell_controller가 state를 바꾸고 popup/task_intake builder가 이 DTO를 채웁니다.
pub(crate) struct TaskIntakeOverlayView {
    // 학습 주석: header_lines는 사용자가 지금 "새 ready task"를 drafting 중이라는 고정 맥락입니다.
    pub(crate) header_lines: Vec<Line<'static>>,
    // 학습 주석: prompt_lines는 사용자가 입력한 raw prompt echo입니다. service가 만든 preview와 분리해
    // 사용자가 어떤 입력에서 이 proposal이 나왔는지 되돌아볼 수 있게 합니다.
    pub(crate) prompt_lines: Vec<Line<'static>>,
    // 학습 주석: preview_lines는 runtime service가 만든 task draft summary입니다. Y commit 전에 title,
    // direction, priority, task_id를 확인하는 의사결정 영역입니다.
    pub(crate) preview_lines: Vec<Line<'static>>,
    // 학습 주석: status_lines는 editing/preview ready/error/accepted 결과를 담습니다. prompt와 preview copy는
    // 내용이고, status는 방금 수행한 action의 결과입니다.
    pub(crate) status_lines: Vec<Line<'static>>,
    // 학습 주석: key_lines는 Prompt 단계와 Preview 단계의 단축키가 다르기 때문에 state-dependent footer로 둡니다.
    pub(crate) key_lines: Vec<Line<'static>>,
}

// 학습 주석: planning init overlay view는 planning workspace를 처음 만들거나 기존 workspace를 선택하는
// setup modal의 DTO입니다. selection builder와 existing-workspace builder가 같은 renderer contract를 씁니다.
pub(crate) struct PlanningInitOverlayView {
    // 학습 주석: header_lines는 setup 단계의 title과 현재 mode입니다. 새 workspace 생성과 기존 workspace
    // 검토가 같은 popup frame을 쓰므로 header가 mode 전환의 기준점입니다.
    pub(crate) header_lines: Vec<Line<'static>>,
    // 학습 주석: summary_lines는 workspace path, existing state, queue/failure summary 같은 상단 요약입니다.
    // option 선택 전에 현재 planning setup의 출발점을 설명합니다.
    pub(crate) summary_lines: Vec<Line<'static>>,
    // 학습 주석: option_lines는 selectable choices 또는 review rows입니다. controller의 selection index와
    // renderer의 highlighted row가 이 영역을 기준으로 맞물립니다.
    pub(crate) option_lines: Vec<Line<'static>>,
    // 학습 주석: status_lines는 validation result, generation status, repair guidance처럼 선택 결과를 설명합니다.
    // option list의 내용과 별개로 현재 setup 진행 상태를 전달합니다.
    pub(crate) status_lines: Vec<Line<'static>>,
    // 학습 주석: key_lines는 setup mode별 confirm/edit/cancel hint입니다. setup flow가 여러 단계여도
    // renderer는 footer lines만 그리면 됩니다.
    pub(crate) key_lines: Vec<Line<'static>>,
}

// 학습 주석: planning draft editor overlay view는 manual planning draft editor의 renderer contract입니다.
// 파일 목록, editor buffer, cursor/scroll, validation status를 한 DTO로 묶어 popup과 inline inspection이 공유합니다.
pub(crate) struct PlanningDraftEditorOverlayView {
    // 학습 주석: header_lines는 draft editor title, draft/session label, dirty state 같은 상단 context입니다.
    // editor body와 분리해 어떤 draft를 편집 중인지 항상 보이게 합니다.
    pub(crate) header_lines: Vec<Line<'static>>,
    // 학습 주석: file_lines는 staged/detail/generated 파일 목록과 현재 selection copy입니다. editor_lines가
    // 파일 내용이라면 file_lines는 "어떤 파일을 보고 있는가"를 말합니다.
    pub(crate) file_lines: Vec<Line<'static>>,
    // 학습 주석: editor_title은 active document label입니다. `Line`이 아니라 String인 이유는 renderer가
    // bordered editor pane title처럼 별도 widget title로 사용할 수 있기 때문입니다.
    pub(crate) editor_title: String,
    // 학습 주석: editor_lines는 현재 buffer를 syntax-neutral text lines로 낮춘 값입니다. cursor와 scroll은
    // 아래 별도 필드가 담당하므로 line content만 순수하게 담습니다.
    pub(crate) editor_lines: Vec<Line<'static>>,
    // 학습 주석: editor_scroll은 renderer가 editor pane을 어디서부터 보여 줄지 결정하는 vertical offset입니다.
    // projection 단계에서 계산해 popup/inline renderer의 scroll 동작을 동일하게 맞춥니다.
    pub(crate) editor_scroll: u16,
    // 학습 주석: editor_cursor_offset은 visible cursor 위치입니다. None이면 read-only/status-only state처럼
    // renderer가 cursor를 배치하지 않아야 하는 상황을 표현합니다.
    pub(crate) editor_cursor_offset: Option<(u16, u16)>,
    // 학습 주석: status_lines는 validation errors, save state, close confirmation risk 같은 editor 결과 copy입니다.
    // 파일 내용과 분리해 command feedback이 editor body를 밀어내지 않게 합니다.
    pub(crate) status_lines: Vec<Line<'static>>,
    // 학습 주석: key_lines는 editing/review/close-confirm 단계별 command footer입니다. controller key map과
    // 직접 연결되는 계약이라 renderer는 그대로 그리는 쪽에 머뭅니다.
    pub(crate) key_lines: Vec<Line<'static>>,
}
