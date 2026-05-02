use crate::application::service::planning::{
    DirectionsMaintenanceDirectionSummary, DirectionsMaintenanceSummary,
    DirectionsSupportingFileStatus,
};

/*
 * Directions maintenance overlay는 planning directions가 요구하는 supporting file 상태를
 * 운영자가 점검하고 복구하는 TUI 흐름이다. application service가 workspace를 읽어
 * DirectionsMaintenanceSummary를 만들고, 이 파일은 그 결과를 화면 단계, 선택 index,
 * 확인 dialog 상태로 바꿔 shell controller와 overlay renderer가 공유할 수 있게 한다.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum DirectionsMaintenanceOverlayStep {
    /*
     * Overview는 service summary를 처음 보여 주는 landing step이다.
     * 여기서 operator는 상태를 훑고 detail doc 생성, 수동 editor, reload 같은 다음 동작을 고른다.
     */
    #[default]
    Overview,
    /*
     * DetailDocSelection은 missing/broken detail doc이 있는 direction만 대상으로 삼는다.
     * 전체 direction 목록을 그대로 보여 주지 않고 action 가능한 항목만 고르는 단계다.
     */
    DetailDocSelection,
    /*
     * DetailDocConfirm은 선택된 direction id/title을 pending 상태로 고정한 뒤
     * 실제 파일 생성을 실행하기 전 operator에게 한 번 더 확인받는 단계다.
     */
    DetailDocConfirm,
    /*
     * ManualEditor는 자동 생성 흐름 대신 directions 파일을 직접 편집하는 escape hatch다.
     * shell_rendering은 이 step일 때 editor overlay가 열려 있음을 별도로 판단한다.
     */
    ManualEditor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum DetailDocConfirmChoice {
    /*
     * 기본 선택을 Yes로 두어 "선택 후 Enter" 흐름을 빠르게 만들지만,
     * 별도의 confirm step을 거치므로 selection 화면에서 곧바로 destructive action이 일어나지는 않는다.
     */
    #[default]
    Yes,
    // No는 confirm dialog를 닫고 selection/overview로 돌아가는 명시적 취소 선택지다.
    No,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingDetailDocCreation {
    /*
     * direction_id는 application service에 detail doc 생성을 요청할 때 사용하는 안정 키다.
     * title은 화면 copy용이므로, 실행은 반드시 id를 기준으로 한다.
     */
    direction_id: String,
    direction_title: String,
}

impl PendingDetailDocCreation {
    pub fn direction_id(&self) -> &str {
        /*
         * controller는 이 값을 planning service 호출 인자로 넘긴다.
         * 내부 String 소유권은 UI state가 유지하고 호출자는 빌린 str만 사용한다.
         */
        self.direction_id.as_str()
    }

    pub fn direction_title(&self) -> &str {
        /*
         * renderer는 confirm copy에 title을 표시한다. id가 실제 실행 키인 반면
         * title은 사람이 선택한 direction을 확인할 수 있게 하는 보조 정보다.
         */
        self.direction_title.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct DirectionsMaintenanceOverlayUiState {
    /*
     * step은 overlay의 현재 화면을 나타내는 상태 머신 축이다.
     * shell controller는 key input을 이 값으로 분기하고 renderer는 같은 값으로 view를 만든다.
     */
    step: DirectionsMaintenanceOverlayStep,
    /*
     * summary는 planning service가 workspace에서 계산한 유지보수 대상 목록이다.
     * None이면 아직 overlay를 열지 않았거나 load 실패 후 초기화된 상태로 본다.
     */
    summary: Option<DirectionsMaintenanceSummary>,
    /*
     * selected_missing_detail_doc_index는 action 가능한 direction 목록 안에서의 위치다.
     * 원본 summary index가 아니라 filtered list index이므로, 목록은 매번 helper에서 재계산한다.
     */
    selected_missing_detail_doc_index: usize,
    /*
     * confirm 단계로 들어갈 때 선택된 direction을 snapshot으로 잡아 둔다.
     * selection 목록이 reload로 바뀌더라도 confirm 화면이 어떤 대상인지 흔들리지 않게 하기 위함이다.
     */
    pending_detail_doc_creation: Option<PendingDetailDocCreation>,
    /*
     * confirm dialog는 Yes/No 두 선택지만 갖는다. 별도 enum으로 두어 key handling과 renderer가
     * bool 의미를 추측하지 않고 같은 선택 상태를 공유한다.
     */
    detail_doc_confirm_choice: DetailDocConfirmChoice,
}

impl DirectionsMaintenanceOverlayUiState {
    pub fn reset(&mut self) {
        /*
         * overlay를 닫거나 shell overlay 전환이 일어날 때 모든 transient state를 지운다.
         * summary까지 비워 다음 open_summary가 service의 최신 workspace 상태를 기준으로 시작하게 한다.
         */
        *self = Self::default();
    }

    pub fn open_summary(&mut self, summary: DirectionsMaintenanceSummary) {
        /*
         * controller가 planning service에서 summary를 성공적으로 읽으면 이 함수로 overlay를 연다.
         * 기존 선택/confirm state는 stale할 수 있으므로 overview와 첫 selection으로 되돌린다.
         */
        self.summary = Some(summary);
        self.step = DirectionsMaintenanceOverlayStep::Overview;
        self.selected_missing_detail_doc_index = 0;
        self.pending_detail_doc_creation = None;
        self.detail_doc_confirm_choice = DetailDocConfirmChoice::Yes;
    }

    pub fn step(&self) -> DirectionsMaintenanceOverlayStep {
        /*
         * shell controller와 renderer가 같은 state machine을 읽어야 input handling과 view가 어긋나지 않는다.
         * Copy enum을 반환해 호출자가 UI state borrow를 오래 잡지 않게 한다.
         */
        self.step
    }

    pub fn summary(&self) -> Option<&DirectionsMaintenanceSummary> {
        /*
         * overlay builder는 summary 내용을 표시하지만 소유하지 않는다.
         * service reload와 reset은 이 state 객체를 통해서만 summary를 교체한다.
         */
        self.summary.as_ref()
    }

    pub fn actionable_detail_doc_directions(&self) -> Vec<&DirectionsMaintenanceDirectionSummary> {
        /*
         * detail doc 생성 flow는 Ready가 아닌 direction만 대상으로 한다.
         * service summary는 전체 direction을 담고 있으므로 TUI 상태에서는 매번 filtered view를 만들어
         * renderer와 selection movement가 같은 "actionable" 목록을 보게 한다.
         */
        self.summary
            .as_ref()
            .map(|summary| {
                summary
                    .directions
                    .iter()
                    .filter(|direction| {
                        direction.detail_doc_status != DirectionsSupportingFileStatus::Ready
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn selected_actionable_detail_doc_direction(
        &self,
    ) -> Option<&DirectionsMaintenanceDirectionSummary> {
        /*
         * selection index는 filtered list 길이가 reload나 status 변화로 줄어도 안전해야 한다.
         * min + saturating_sub로 마지막 항목에 clamp하고, 빈 목록이면 get이 None을 반환하게 둔다.
         */
        let directions = self.actionable_detail_doc_directions();
        directions
            .get(
                self.selected_missing_detail_doc_index
                    .min(directions.len().saturating_sub(1)),
            )
            .copied()
    }

    pub fn open_detail_doc_selection(&mut self) {
        /*
         * overview에서 detail doc repair flow로 진입할 때 항상 첫 action 대상부터 시작한다.
         * 이전 confirm snapshot과 선택값을 지워 사용자가 과거 direction을 실행하지 않게 한다.
         */
        self.step = DirectionsMaintenanceOverlayStep::DetailDocSelection;
        self.selected_missing_detail_doc_index = 0;
        self.pending_detail_doc_creation = None;
        self.detail_doc_confirm_choice = DetailDocConfirmChoice::Yes;
    }

    pub fn return_to_overview(&mut self) {
        /*
         * selection이나 confirm에서 overview로 돌아오면 pending 실행 대상은 폐기한다.
         * summary는 유지해 같은 maintenance snapshot을 계속 볼 수 있게 한다.
         */
        self.step = DirectionsMaintenanceOverlayStep::Overview;
        self.pending_detail_doc_creation = None;
        self.detail_doc_confirm_choice = DetailDocConfirmChoice::Yes;
    }

    pub fn move_missing_detail_doc_selection(&mut self, delta: isize) {
        /*
         * 방향키 이동은 filtered actionable list 안에서만 clamp한다.
         * action 대상이 하나도 없으면 index를 0으로 고정해 이후 reload/open 흐름이 예측 가능하게 만든다.
         */
        let directions = self.actionable_detail_doc_directions();
        if directions.is_empty() {
            self.selected_missing_detail_doc_index = 0;
            return;
        }
        let max_index = directions.len().saturating_sub(1) as isize;
        let next_index =
            (self.selected_missing_detail_doc_index as isize + delta).clamp(0, max_index);
        self.selected_missing_detail_doc_index = next_index as usize;
    }

    pub fn open_detail_doc_confirm(&mut self) {
        /*
         * confirm 단계는 현재 선택된 actionable direction이 있을 때만 열린다.
         * 선택 가능한 항목이 없으면 아무 일도 하지 않아 controller가 별도 오류 상태를 만들 필요가 없다.
         */
        let Some(direction) = self.selected_actionable_detail_doc_direction() else {
            return;
        };
        self.pending_detail_doc_creation = Some(PendingDetailDocCreation {
            direction_id: direction.id.clone(),
            direction_title: direction.title.clone(),
        });
        self.detail_doc_confirm_choice = DetailDocConfirmChoice::Yes;
        self.step = DirectionsMaintenanceOverlayStep::DetailDocConfirm;
    }

    pub fn pending_detail_doc_creation(&self) -> Option<&PendingDetailDocCreation> {
        /*
         * controller는 Enter on Yes에서 이 snapshot을 읽어 service 호출을 만든다.
         * None이면 confirm 화면이 아니거나 선택 대상이 사라진 상태라 실행하지 않는다.
         */
        self.pending_detail_doc_creation.as_ref()
    }

    pub fn detail_doc_confirm_choice(&self) -> DetailDocConfirmChoice {
        /*
         * renderer는 현재 Yes/No 강조를, controller는 Enter 처리 방식을 이 값으로 결정한다.
         * Copy enum이라 borrow 없이 즉시 분기할 수 있다.
         */
        self.detail_doc_confirm_choice
    }

    pub fn move_detail_doc_confirm_choice(&mut self, delta: isize) {
        /*
         * confirm 선택은 두 칸짜리 segmented control처럼 동작한다.
         * 양수 이동은 Yes -> No, 음수 이동은 No -> Yes만 허용하고 끝에서는 그대로 머문다.
         */
        self.detail_doc_confirm_choice = match (self.detail_doc_confirm_choice, delta.is_negative())
        {
            (DetailDocConfirmChoice::Yes, false) => DetailDocConfirmChoice::No,
            (DetailDocConfirmChoice::No, true) => DetailDocConfirmChoice::Yes,
            (choice, _) => choice,
        };
    }

    pub fn open_manual_editor(&mut self) {
        /*
         * manual editor는 summary 기반 자동 repair flow와 다른 화면이다.
         * 기존 summary는 유지해 editor에서 돌아온 뒤 reload/overview 흐름이 context를 잃지 않게 한다.
         */
        self.step = DirectionsMaintenanceOverlayStep::ManualEditor;
    }
}
