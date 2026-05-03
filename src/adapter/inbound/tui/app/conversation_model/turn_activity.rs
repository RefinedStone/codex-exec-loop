use crate::domain::conversation::{ConversationToolActivity, ConversationToolActivityKind};

/*
 * active turn과 가장 최근 completed turn을 위한 side-channel activity summary다.
 * full message stream은 transcript가 보관하고, 이 state는 stream event reduce 이후 footer/tail rendering과
 * auto-follow stop rule이 필요로 하는 작은 counter 및 latest activity label만 유지한다.
 */
#[derive(Debug, Clone, Default)]
pub(crate) struct TurnActivityState {
    // turn 완료 전 관측된 tool-file-change event가 쌓이는 streaming bucket이다.
    pub(crate) current_turn_file_change_count: usize,
    // command output line 수가 아니라 command execution boundary 수를 센다.
    pub(crate) current_turn_command_count: usize,
    // compact live-status line에 보여 줄 latest activity 문장이다. 전체 history는 transcript message에 남는다.
    pub(crate) current_turn_last_summary: Option<String>,
    // turn completion에서 확정된 planning artifact다. post-turn planning evaluation을 위해 중복 제거해서 보관한다.
    pub(crate) current_turn_changed_planning_file_paths: Vec<String>,
    // finish_turn 때 current bucket에서 옮긴 snapshot이다. idle footer copy와 auto-follow decision이 읽는다.
    pub(crate) last_completed_turn_id: Option<String>,
    pub(crate) last_completed_turn_file_change_count: usize,
    pub(crate) last_completed_turn_command_count: usize,
    pub(crate) last_completed_turn_last_summary: Option<String>,
    pub(crate) last_completed_turn_changed_planning_file_paths: Vec<String>,
}

// streaming accumulation, completion rollover, presentation bucket selection을 담당하는 state machine이다.
impl TurnActivityState {
    // turn 시작은 live activity만 지운다. last_completed는 새 activity가 오기 전까지 footer/decision용으로 남긴다.
    pub(crate) fn start_new_turn(&mut self) {
        self.current_turn_file_change_count = 0;
        self.current_turn_command_count = 0;
        self.current_turn_last_summary = None;
        self.current_turn_changed_planning_file_paths.clear();
    }

    // conversation stream reducer가 낸 tool-activity event 하나를 current turn bucket에 반영한다.
    pub(crate) fn register_tool_activity(&mut self, activity: &ConversationToolActivity) {
        self.current_turn_last_summary = Some(activity.text.clone());
        match activity.kind {
            // file-change event는 여러 파일을 보고할 수 있으므로 payload count를 누적한다.
            ConversationToolActivityKind::FileChange => {
                self.current_turn_file_change_count += activity.file_change_count;
            }
            // command event는 output 크기나 exit status와 무관하게 실행 경계 하나로 센다.
            ConversationToolActivityKind::CommandExecution => {
                self.current_turn_command_count += 1;
            }
        }
    }

    // active-turn flag가 내려가기 전에 live activity를 completed bucket으로 옮긴다.
    pub(crate) fn complete_turn(&mut self, turn_id: &str) {
        self.last_completed_turn_id = Some(turn_id.to_string());
        // replace/take를 써 model 관점의 rollover를 원자적으로 만든다. completed는 값을 받고 current는 reset된다.
        self.last_completed_turn_file_change_count =
            std::mem::replace(&mut self.current_turn_file_change_count, 0);
        self.last_completed_turn_command_count =
            std::mem::replace(&mut self.current_turn_command_count, 0);
        self.last_completed_turn_last_summary = self.current_turn_last_summary.take();
        self.last_completed_turn_changed_planning_file_paths =
            std::mem::take(&mut self.current_turn_changed_planning_file_paths);
    }

    // streaming tool event가 아니라 finish_turn에서 결정된 planning artifact를 등록한다.
    pub(crate) fn register_changed_planning_file_paths(&mut self, paths: &[String]) {
        for path in paths {
            // list는 작고 diagnostic에서 순서가 의미 있을 수 있어 set 대신 linear de-duplication을 쓴다.
            if !self
                .current_turn_changed_planning_file_paths
                .iter()
                .any(|existing| existing == path)
            {
                self.current_turn_changed_planning_file_paths
                    .push(path.clone());
            }
        }
    }

    // auto-follow no-file-change rule은 partial streaming state가 아니라 completed bucket만 읽는다.
    pub(crate) fn last_completed_file_change_count(&self) -> usize {
        self.last_completed_turn_file_change_count
    }

    // finish/flush ordering 중에는 current activity가 running flag보다 잠깐 더 오래 남을 수 있다.
    fn has_current_turn_activity(&self) -> bool {
        self.current_turn_file_change_count > 0
            || self.current_turn_command_count > 0
            || self.current_turn_last_summary.is_some()
    }

    // presentation이 activity count와 summary를 읽을 bucket의 label을 고른다.
    pub(crate) fn activity_scope_label(&self, turn_running: bool) -> &'static str {
        if turn_running {
            "current turn"
        } else if self.has_current_turn_activity() {
            "recent turn"
        } else {
            "last turn"
        }
    }

    // scope label과 같은 bucket에서 command count를 고른다.
    pub(crate) fn activity_command_count(&self, turn_running: bool) -> usize {
        if turn_running || self.has_current_turn_activity() {
            self.current_turn_command_count
        } else {
            self.last_completed_turn_command_count
        }
    }

    // footer copy가 scope를 섞지 않도록 command count와 같은 bucket에서 file-change count를 고른다.
    pub(crate) fn activity_file_change_count(&self, turn_running: bool) -> usize {
        if turn_running || self.has_current_turn_activity() {
            self.current_turn_file_change_count
        } else {
            self.last_completed_turn_file_change_count
        }
    }

    // 같은 bucket에서 latest summary를 고른다. "none"은 tail_shared가 소비하는 sentinel이다.
    pub(crate) fn activity_summary(&self, turn_running: bool) -> &str {
        if turn_running || self.has_current_turn_activity() {
            self.current_turn_last_summary.as_deref().unwrap_or("none")
        } else {
            self.last_completed_turn_last_summary
                .as_deref()
                .unwrap_or("none")
        }
    }
}
