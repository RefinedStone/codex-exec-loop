// 학습 주석: session collector는 도메인 검증을 다시 수행하지 않습니다. editor UI state 안에 보관된
// `PlanningValidationReport`를 빌려 와 surface/status 단계가 같은 검증 스냅샷을 보게 합니다.
use crate::domain::planning::PlanningValidationReport;

// 학습 주석: planning draft editor UI state는 입력 처리와 버퍼 mutation을 담당하는 상태 저장소입니다.
// 이 파일은 그 상태 저장소에서 렌더링에 필요한 읽기 전용 조각만 꺼내 presentation snapshot으로 묶습니다.
use super::super::super::super::super::planning_draft_editor_ui::{
    PlanningDraftEditorBufferState, PlanningDraftEditorUiState,
};

// 학습 주석: `PlanningDraftEditorSessionView`는 열린 수동 planning draft editor 세션의 렌더링 입력 묶음입니다.
// surface builder는 app state 대신 이 snapshot을 받아 projection/runtime/status copy 단계로 나누어 전달합니다.
pub(super) struct PlanningDraftEditorSessionView<'a> {
    // 학습 주석: draft 이름은 사용자가 어떤 planning draft를 보고 있는지 식별하는 값입니다. header/status
    // 문구에서 세션 정체성을 유지해야 하므로 UI state의 문자열을 그대로 빌립니다.
    pub(super) draft_name: &'a str,
    // 학습 주석: draft directory는 header copy의 원천입니다. 파일 목록의 active path와 달리 실제
    // draft 작업 디렉터리 위치를 보여 주는 session-level metadata입니다.
    pub(super) draft_directory: &'a str,
    // 학습 주석: buffers는 좌측 파일 목록과 우측 editor projection이 함께 쓰는 전체 편집 대상입니다.
    // selected_index와 selected_buffer는 반드시 이 slice 안의 같은 항목을 가리킨다는 불변식을 공유합니다.
    pub(super) buffers: &'a [PlanningDraftEditorBufferState],
    // 학습 주석: selected_index는 zero-based cursor 위치입니다. surface 단계에서 사용자 표시용으로만
    // 1을 더하고, projection 단계에는 원래 index를 넘겨 리스트 강조와 editor 본문을 맞춥니다.
    pub(super) selected_index: usize,
    // 학습 주석: selected_buffer는 현재 editor panel이 실제로 그릴 버퍼입니다. active/staged path,
    // cursor, scroll, body가 모두 여기서 나오므로 selected_index와 분리해 명시적으로 들고 갑니다.
    pub(super) selected_buffer: &'a PlanningDraftEditorBufferState,
    // 학습 주석: validation_report는 저장 가능 여부와 첫 검증 이슈 문구의 기준입니다. session snapshot에
    // 포함해야 runtime 해석과 status copy가 같은 validation 상태를 공유합니다.
    pub(super) validation_report: &'a PlanningValidationReport,
    // 학습 주석: dirty_labels는 UI state가 buffer dirty flag를 사람이 읽을 파일 라벨로 낮춘 결과입니다.
    // runtime은 존재 여부를 보고, status copy는 내용을 요약하므로 이 경계에서 한 번만 계산합니다.
    pub(super) dirty_labels: Vec<String>,
}

// 학습 주석: `collect_planning_draft_editor_session_view`는 app-level surface가 UI state 내부 accessor를
// 여러 번 직접 호출하지 않게 하는 수집 지점입니다. 필요한 조각 중 하나라도 없으면 아직 editor session을
// 그릴 수 없다는 뜻이므로 `None`으로 전체 popup 생성을 중단합니다.
pub(super) fn collect_planning_draft_editor_session_view(
    // 학습 주석: `PlanningDraftEditorUiState`는 session이 열리기 전에는 대부분의 accessor가 None을
    // 반환합니다. 이 함수는 그 부분 상태를 presentation 계층에서 처리할 단일 `Option` 계약으로 접습니다.
    ui_state: &PlanningDraftEditorUiState,
) -> Option<PlanningDraftEditorSessionView<'_>> {
    Some(PlanningDraftEditorSessionView {
        // 학습 주석: `?`는 값이 없으면 즉시 None을 반환합니다. draft 이름이 없다는 것은 열린
        // session 자체가 없다는 신호라 이후 필드도 의미 있게 조립할 수 없습니다.
        draft_name: ui_state.draft_name()?,
        // 학습 주석: directory도 session metadata입니다. 이름만 있고 directory가 없으면 header가
        // 잘못된 세션을 보여 줄 수 있으므로 같은 Option 체인에서 필수값으로 취급합니다.
        draft_directory: ui_state.draft_directory()?,
        // 학습 주석: buffers가 없으면 파일 목록과 editor 본문을 둘 다 만들 수 없습니다. surface가
        // 빈 list를 임의로 그리지 않고 popup 자체를 생략하도록 None 전파를 유지합니다.
        buffers: ui_state.buffers()?,
        // 학습 주석: selected index는 buffers와 같은 snapshot에서 온 값이어야 합니다. 상태 접근을
        // 이 함수 안에 모아 두면 surface의 projection 호출이 서로 다른 시점의 값을 섞지 않습니다.
        selected_index: ui_state.selected_file_index()?,
        // 학습 주석: selected buffer를 별도로 꺼내 둬서 downstream builder가 index로 다시 lookup하지
        // 않게 합니다. 이렇게 하면 projection/status가 모두 같은 선택 버퍼를 읽는다는 점이 코드에 드러납니다.
        selected_buffer: ui_state.selected_buffer()?,
        // 학습 주석: validation report가 없으면 저장 가능 여부와 검증 안내를 만들 수 없습니다. 열린
        // session의 필수 구성요소로 취급해 잘못된 "valid" 기본값을 만들어 내지 않습니다.
        validation_report: ui_state.validation_report()?,
        // 학습 주석: dirty labels는 Option이 아니라 현재 buffers에서 계산되는 파생값입니다. session
        // 필수값들이 준비된 뒤 마지막에 계산해 snapshot 안에서 dirty 상태와 buffer 목록이 함께 읽히게 합니다.
        dirty_labels: ui_state.dirty_file_labels(),
    })
}

// 학습 주석: 이 테스트 모듈은 editor renderer를 거치지 않고 session collector의 계약만 확인합니다.
// 즉 "세션이 없으면 None", "세션이 있으면 필요한 모든 렌더링 입력이 같은 snapshot에 담김"을 고정합니다.
#[cfg(test)]
mod tests {
    // 학습 주석: 내부 helper를 직접 테스트해 surface builder 실패 원인이 session 수집인지 renderer인지
    // 분리해서 볼 수 있게 합니다.
    use super::collect_planning_draft_editor_session_view;
    // 학습 주석: `PlanningDraftEditorUiState`는 실제 입력 처리 상태 타입입니다. fixture가 production
    // accessor를 그대로 통과하므로 collector 테스트가 실제 runtime 상태 모양을 반영합니다.
    use crate::adapter::inbound::tui::app::planning_draft_editor_ui::PlanningDraftEditorUiState;
    // 학습 주석: service DTO를 사용해 open_session이 받는 실제 session payload를 만듭니다. 테스트용
    // 가짜 구조체를 두지 않아 application service와 TUI state 사이의 필드 연결을 함께 확인합니다.
    use crate::application::service::planning::{
        PlanningDraftEditorFile, PlanningDraftEditorSession,
    };
    // 학습 주석: 기본 validation report를 넣어 "검증 이슈가 없는 정상 세션"의 수집 결과를 확인합니다.
    use crate::domain::planning::PlanningValidationReport;

    // 학습 주석: default UI state는 아직 draft editor session을 열지 않은 상태입니다. 이때 collector가
    // 빈 snapshot을 만들면 surface가 실체 없는 popup을 렌더링할 수 있으므로 None 계약을 고정합니다.
    #[test]
    fn collect_returns_none_when_editor_session_is_missing() {
        // 학습 주석: `default`는 session metadata, buffers, validation report가 모두 없는 baseline입니다.
        let ui_state = PlanningDraftEditorUiState::default();

        assert!(collect_planning_draft_editor_session_view(&ui_state).is_none());
    }

    // 학습 주석: 열린 세션에서는 collector가 header metadata, 전체 buffers, 선택 index/buffer,
    // validation report, dirty label까지 surface가 필요한 값을 빠짐없이 싣는지 확인합니다.
    #[test]
    fn collect_returns_current_session_snapshot() {
        // 학습 주석: open_session은 application service가 넘긴 draft payload를 editor buffer state로
        // 변환합니다. collector는 그 변환 이후의 UI state를 읽습니다.
        let mut ui_state = PlanningDraftEditorUiState::default();
        ui_state.open_session(sample_session());

        // 학습 주석: 열린 세션에서는 모든 필수 accessor가 Some이어야 합니다. None이면 session view
        // assembly가 너무 엄격하거나 open_session이 필수 필드를 채우지 못한 것입니다.
        let session = collect_planning_draft_editor_session_view(&ui_state)
            .expect("session view should exist");

        assert_eq!(session.draft_name, "draft-123");
        assert_eq!(session.draft_directory, "/tmp/planning-draft");
        assert_eq!(session.buffers.len(), 2);
        assert_eq!(session.selected_index, 0);
        assert_eq!(
            session.selected_buffer.active_path(),
            "planning/result-output.md"
        );
        assert!(session.validation_report.is_valid());
        assert!(session.dirty_labels.is_empty());
    }

    // 학습 주석: dirty labels는 runtime next-action과 status dirty summary의 공통 입력입니다. 편집 후
    // collector가 변경된 파일 라벨을 포함해야 저장/닫기 안내가 실제 버퍼 상태와 맞습니다.
    #[test]
    fn collect_includes_dirty_labels_after_edits() {
        // 학습 주석: 세션을 연 뒤 현재 선택 버퍼에 문자를 넣어 dirty flag가 켜지는 실제 UI state
        // 경로를 사용합니다. dirty label을 직접 조작하지 않아 collector와 buffer mutation 연결을 함께 봅니다.
        let mut ui_state = PlanningDraftEditorUiState::default();
        ui_state.open_session(sample_session());
        ui_state.insert_character('#');

        // 학습 주석: 수집된 dirty label은 selected buffer의 file_label에서 나온 사용자 표시명입니다.
        // downstream status copy는 이 문자열을 그대로 요약하므로 여기서 파일명 형태를 고정합니다.
        let session = collect_planning_draft_editor_session_view(&ui_state)
            .expect("session view should exist");

        assert_eq!(session.dirty_labels, vec!["result-output.md".to_string()]);
    }

    // 학습 주석: sample session은 application service가 TUI로 넘기는 planning draft editor payload를
    // 최소 크기로 재현합니다. 두 파일을 넣어 buffers 전체와 selected buffer가 분리되어 수집되는지 봅니다.
    fn sample_session() -> PlanningDraftEditorSession {
        PlanningDraftEditorSession {
            // 학습 주석: draft_name/directory는 header와 status에서 세션 단위를 설명하는 metadata입니다.
            draft_name: "draft-123".to_string(),
            draft_directory: "/tmp/planning-draft".to_string(),
            // 학습 주석: editable_files는 좌측 파일 목록과 우측 editor 본문의 원천입니다. 같은 active path를
            // 두 번 둔 것은 collector가 경로 유일성 검사 대신 state가 준 buffers를 그대로 전달함을 보여 줍니다.
            editable_files: vec![
                PlanningDraftEditorFile {
                    // 학습 주석: 첫 파일은 기본 선택 버퍼가 되며, 테스트의 selected_buffer.active_path
                    // assertion과 dirty label assertion의 기준이 됩니다.
                    active_path: "planning/result-output.md".to_string(),
                    staged_path: ".draft/planning/result-output.md".to_string(),
                    body: "title = \"Directions\"\n".to_string(),
                },
                PlanningDraftEditorFile {
                    // 학습 주석: 두 번째 파일은 전체 buffers 길이와 목록 projection 입력을 확인하기 위한
                    // 추가 항목입니다. collector는 현재 선택되지 않은 버퍼도 slice에 포함해야 합니다.
                    active_path: "planning/result-output.md".to_string(),
                    staged_path: ".draft/planning/result-output.md".to_string(),
                    body: "{}\n".to_string(),
                },
            ],
            // 학습 주석: 기본 report는 valid 상태입니다. collector가 report reference를 잃거나 새로
            // 만들지 않고 session의 값을 그대로 넘기는지 `is_valid` assertion으로 확인합니다.
            validation_report: PlanningValidationReport::default(),
        }
    }
}
