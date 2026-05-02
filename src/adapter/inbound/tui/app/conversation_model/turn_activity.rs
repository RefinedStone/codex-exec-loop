// 학습 주석: conversation stream reducer가 tool activity event를 받으면 이 도메인 DTO로 종류와 문구를
// 전달합니다. 이 파일은 그 event를 TUI tail/status와 auto-follow 판단에 필요한 누적 상태로 바꿉니다.
use crate::domain::conversation::{ConversationToolActivity, ConversationToolActivityKind};

// 학습 주석: `TurnActivityState`는 메시지 목록 자체가 아니라 "이번/직전 턴에서 agent가 실제로 무엇을
// 했는가"를 요약하는 side-channel state입니다. tail notice는 현재 값을 보여 주고, auto-follow는 완료된
// 턴의 file change count를 stop rule 입력으로 사용합니다.
#[derive(Debug, Clone, Default)]
pub(crate) struct TurnActivityState {
    // 학습 주석: current_* 필드는 active turn이 streaming되는 동안 계속 갱신됩니다. file change count는
    // patch/edit 도구가 보고한 변경 파일 수를 합산해 live tail과 no-file-change 정책의 원천이 됩니다.
    pub(crate) current_turn_file_change_count: usize,
    // 학습 주석: command count는 command execution activity 하나를 명령 하나로 세는 값입니다. 명령이
    // 몇 개의 출력 줄을 만들었는지가 아니라 agent가 몇 번 실행 boundary를 넘었는지를 보여 줍니다.
    pub(crate) current_turn_command_count: usize,
    // 학습 주석: last summary는 가장 최근 tool activity text입니다. 전체 활동 로그는 메시지 버퍼가 갖고,
    // 이 필드는 status 한 줄에서 "지금 무엇을 하는 중인가"를 빠르게 보여 주는 대표 문구입니다.
    pub(crate) current_turn_last_summary: Option<String>,
    // 학습 주석: planning file path 목록은 턴 종료 시 planning queue/follow-up 쪽에서 변경된 planning
    // 산출물을 추적하기 위한 보조 정보입니다. 중복 제거를 해 두어 같은 파일이 여러 번 보고돼도 한 번만 남습니다.
    pub(crate) current_turn_changed_planning_file_paths: Vec<String>,
    // 학습 주석: last_completed_* 필드는 active turn이 끝날 때 current_*에서 이동한 스냅샷입니다. 턴이
    // 더 이상 running이 아니어도 status panel과 auto-follow decision이 직전 결과를 읽을 수 있게 합니다.
    pub(crate) last_completed_turn_id: Option<String>,
    pub(crate) last_completed_turn_file_change_count: usize,
    pub(crate) last_completed_turn_command_count: usize,
    pub(crate) last_completed_turn_last_summary: Option<String>,
    pub(crate) last_completed_turn_changed_planning_file_paths: Vec<String>,
}

// 학습 주석: 이 impl은 세 가지 흐름을 분리합니다. stream event가 들어올 때 current 값을 누적하고,
// turn 종료 때 last_completed로 이동하며, presentation/auto-follow가 읽을 label과 count를 계산합니다.
impl TurnActivityState {
    // 학습 주석: 새 turn이 시작되면 current bucket만 비웁니다. last_completed는 그대로 남겨 두어 새
    // turn이 아직 activity를 만들기 전에도 직전 턴 요약을 fallback으로 보여 줄 수 있습니다.
    pub(crate) fn start_new_turn(&mut self) {
        self.current_turn_file_change_count = 0;
        self.current_turn_command_count = 0;
        self.current_turn_last_summary = None;
        self.current_turn_changed_planning_file_paths.clear();
    }

    // 학습 주석: stream reducer가 tool activity event를 받을 때마다 호출됩니다. tail status는 마지막
    // activity text를 대표 문구로 보여 주고, 종류별 count는 live activity line의 숫자 필드가 됩니다.
    pub(crate) fn register_tool_activity(&mut self, activity: &ConversationToolActivity) {
        self.current_turn_last_summary = Some(activity.text.clone());
        // 학습 주석: activity kind는 "파일 변경 수를 더할지"와 "명령 실행 횟수를 하나 늘릴지"를 가릅니다.
        // file change activity는 activity 자체가 여러 파일 수를 담을 수 있으므로 `file_change_count`를 더합니다.
        match activity.kind {
            ConversationToolActivityKind::FileChange => {
                self.current_turn_file_change_count += activity.file_change_count;
            }
            // 학습 주석: command execution event는 하나의 shell/tool boundary를 의미합니다. output 양이나
            // exit status가 아니라 "agent가 command를 실행했다"는 사실을 한 번으로 셉니다.
            ConversationToolActivityKind::CommandExecution => {
                self.current_turn_command_count += 1;
            }
        }
    }

    // 학습 주석: 턴 완료 시 current bucket을 last_completed bucket으로 이동합니다. 이렇게 해야
    // active turn flag가 내려간 뒤에도 auto-follow skip reason과 tail notice가 방금 끝난 턴 결과를 참조합니다.
    pub(crate) fn complete_turn(&mut self, turn_id: &str) {
        self.last_completed_turn_id = Some(turn_id.to_string());
        // 학습 주석: `replace`는 count를 last_completed에 넘기면서 current count를 0으로 비웁니다.
        // 복사 후 별도 reset을 하는 것보다 "이동과 초기화가 한 동작"이라는 완료 시점의 의도를 잘 드러냅니다.
        self.last_completed_turn_file_change_count =
            std::mem::replace(&mut self.current_turn_file_change_count, 0);
        self.last_completed_turn_command_count =
            std::mem::replace(&mut self.current_turn_command_count, 0);
        // 학습 주석: summary는 `Option<String>`이라 `take`로 소유권을 옮깁니다. 완료 후 current summary가
        // 남아 있으면 다음 turn이 시작되기 전 `has_current_turn_activity`가 잘못 true가 될 수 있습니다.
        self.last_completed_turn_last_summary = self.current_turn_last_summary.take();
        // 학습 주석: changed planning paths도 완료된 턴의 결과로 옮깁니다. current Vec을 비워 다음 턴이
        // 직전 planning 변경 목록을 상속하지 않게 합니다.
        self.last_completed_turn_changed_planning_file_paths =
            std::mem::take(&mut self.current_turn_changed_planning_file_paths);
    }

    // 학습 주석: turn finish 경로에서 planning 산출물 변경 목록을 current bucket에 등록합니다. tool
    // activity count와 달리 이 값은 streaming event가 아니라 완료 처리에서 확정된 planning file paths입니다.
    pub(crate) fn register_changed_planning_file_paths(&mut self, paths: &[String]) {
        for path in paths {
            // 학습 주석: 같은 planning file이 여러 단계에서 반복 보고될 수 있으므로 Vec 안에서 선형
            // 중복 검사를 합니다. 목록 크기가 작고 순서가 사용자/로그 의미를 가질 수 있어 HashSet으로 바꾸지 않습니다.
            if !self
                .current_turn_changed_planning_file_paths
                .iter()
                .any(|existing| existing == path)
            {
                // 학습 주석: path는 caller slice가 소유하므로 state가 완료 이후에도 보관할 자체 문자열로
                // 복제합니다. 나중에 `complete_turn`이 이 Vec 전체를 last_completed로 이동합니다.
                self.current_turn_changed_planning_file_paths
                    .push(path.clone());
            }
        }
    }

    // 학습 주석: auto-follow의 no-file-change stop rule은 방금 완료된 턴만 봐야 합니다. running turn의
    // 부분 결과를 쓰면 아직 파일 변경이 도착하기 전인데 후속 턴을 멈추는 오판이 생길 수 있습니다.
    pub(crate) fn last_completed_file_change_count(&self) -> usize {
        self.last_completed_turn_file_change_count
    }

    // 학습 주석: current bucket에 의미 있는 신호가 있는지 판단합니다. turn_running이 false여도 flush나
    // finish 순서 때문에 current 값이 잠시 남을 수 있어, presentation은 이 값을 "recent turn"으로 보여 줍니다.
    fn has_current_turn_activity(&self) -> bool {
        self.current_turn_file_change_count > 0
            || self.current_turn_command_count > 0
            || self.current_turn_last_summary.is_some()
    }

    // 학습 주석: tail notice에 붙는 scope label을 고릅니다. running이면 current, running은 아니지만
    // current bucket이 남아 있으면 recent, 둘 다 아니면 last_completed bucket을 보는 "last"로 표현합니다.
    pub(crate) fn activity_scope_label(&self, turn_running: bool) -> &'static str {
        if turn_running {
            "current turn"
        } else if self.has_current_turn_activity() {
            "recent turn"
        } else {
            "last turn"
        }
    }

    // 학습 주석: status panel이 표시할 command count를 고릅니다. active/recent 상태에서는 current
    // bucket을 우선하고, 완전히 idle하면 last_completed bucket을 보여 직전 턴의 결과가 사라지지 않게 합니다.
    pub(crate) fn activity_command_count(&self, turn_running: bool) -> usize {
        if turn_running || self.has_current_turn_activity() {
            self.current_turn_command_count
        } else {
            self.last_completed_turn_command_count
        }
    }

    // 학습 주석: command count와 같은 bucket 선택 규칙을 file change count에도 적용합니다. status copy가
    // 두 숫자를 같은 scope label 아래에 묶어 출력하므로 서로 다른 bucket에서 읽으면 문구가 어긋납니다.
    pub(crate) fn activity_file_change_count(&self, turn_running: bool) -> usize {
        if turn_running || self.has_current_turn_activity() {
            self.current_turn_file_change_count
        } else {
            self.last_completed_turn_file_change_count
        }
    }

    // 학습 주석: activity summary도 count와 같은 bucket을 봅니다. summary가 없을 때 `"none"`을
    // 반환하는 이유는 tail_shared가 이 sentinel과 count를 함께 보고 activity line 표시 여부를 결정하기 때문입니다.
    pub(crate) fn activity_summary(&self, turn_running: bool) -> &str {
        if turn_running || self.has_current_turn_activity() {
            self.current_turn_last_summary.as_deref().unwrap_or("none")
        } else {
            // 학습 주석: last_completed summary는 Option<String>으로 저장하지만, renderer에는 borrowed
            // `&str`만 넘깁니다. `as_deref`로 소유 문자열을 빌린 문자열 옵션으로 낮춰 allocation을 피합니다.
            self.last_completed_turn_last_summary
                .as_deref()
                .unwrap_or("none")
        }
    }
}
