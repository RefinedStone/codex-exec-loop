/*
학습 주석: inline terminal adapter는 ratatui frame만 다시 그리는 것이 아니라, 대화 history 일부를
host terminal scrollback에도 실제 줄로 밀어 넣습니다. 이 파일은 "이미 scrollback에 기록한 transcript"와
"현재 app state의 transcript"를 비교해 새 suffix만 삽입하고, viewport가 밀릴 때 visible row bookkeeping을 맞춥니다.
*/
use ratatui::Terminal;
use ratatui::backend::Backend;
use ratatui::layout::{Position, Rect, Size};
use ratatui::text::Line;

// 학습 주석: transcript는 무한히 자라지 않고 app 쪽에서 cap됩니다. cap이 찼을 때 앞쪽이 잘린 window를
// "완전히 새 history"로 오인하지 않으려면 같은 상한값으로 shifted overlap을 판정해야 합니다.
use super::super::MAX_CONVERSATION_HISTORY_LINES;
// 학습 주석: HistoryFlushState는 삽입할 줄을 고르고, 실제 terminal mutation은 HistoryInsertionAdapter에 맡깁니다.
// 이렇게 나누면 diff 계산과 terminal escape/scroll-region 전략을 별도로 테스트할 수 있습니다.
use super::super::history_insertion::{
    HistoryInsertionAdapter, HistoryInsertionMode, count_rendered_history_rows,
};

#[derive(Default)]
// 학습 주석: HistoryFlushState는 host scrollback과 app transcript 사이의 작은 synchronization cache입니다.
// inline_terminal_adapter의 state 안에 보관되어 draw tick 사이에서 "무엇을 이미 기록했는지"를 기억합니다.
pub(crate) struct HistoryFlushState {
    // 학습 주석: rendered_lines는 마지막으로 scrollback flush 기준점으로 삼은 transcript snapshot입니다.
    // 새 draw에서 current_lines가 이 snapshot으로 시작하면 뒤쪽 suffix만 terminal에 추가하면 됩니다.
    pub(crate) rendered_lines: Vec<Line<'static>>,
    // 학습 주석: pending_history_lines는 이번 sync에서 삽입할 suffix를 잠시 보관합니다. 테스트가 내부 state를
    // 직접 구성할 수 있도록 field는 pub(crate)이지만, production 흐름에서는 sync가 채우고 마지막에 비웁니다.
    pub(crate) pending_history_lines: Vec<Line<'static>>,
    // 학습 주석: visible_history_rows는 viewport top 위에 실제로 보이는 history row 수입니다. terminal scrollback에
    // 줄을 삽입하면 frame area가 아래로 밀리므로, 이 값을 viewport top으로 clamp해 back buffer invalidation을 판단합니다.
    pub(crate) visible_history_rows: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
// 학습 주석: HistoryFlushResult는 terminal mutation 여부를 caller에게 돌려주는 작은 signal입니다.
// sync_inline_viewport는 이 값을 보고 back buffer를 무효화해 다음 render가 host scrollback 변화와 다시 맞춰지게 합니다.
pub(crate) struct HistoryFlushResult {
    // 학습 주석: inserted_rows는 Line 개수가 아니라 terminal width를 반영한 rendered row 수입니다.
    // 긴 transcript line은 wrap되어 여러 row를 밀 수 있으므로 viewport 보정에는 rendered row가 필요합니다.
    inserted_rows: u16,
}

impl HistoryFlushResult {
    // 학습 주석: caller는 정확한 row 수보다 "scrollback을 건드렸는가"가 중요합니다. inserted는 back buffer
    // invalidation과 viewport 재측정 trigger를 boolean으로 표현합니다.
    pub(crate) fn inserted(self) -> bool {
        self.inserted_rows > 0
    }
}

// 학습 주석: history cap이 찼을 때 작은 우연한 prefix/suffix 일치로 shifted window를 잘못 판단하지 않도록
// 최소 overlap을 요구합니다. 8줄이면 짧은 status block 반복보다 충분히 강한 transcript 연속성 신호입니다.
const MIN_SHIFTED_HISTORY_OVERLAP: usize = 8;

impl HistoryFlushState {
    // 학습 주석: terminal resize나 previous insertion 때문에 visible_history_rows가 frame viewport보다 커지면,
    // host scrollback에 빈 줄을 추가해 viewport를 다시 아래로 밀고 inline frame이 history와 겹치지 않게 합니다.
    pub(crate) fn fit_visible_rows_to_viewport<B: Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
        terminal_size: Size,
        viewport_area: Rect,
    ) -> Result<bool, B::Error> {
        // 학습 주석: viewport top은 ratatui frame이 시작하는 row입니다. visible history가 이보다 적거나 같으면
        // 현재 frame 위쪽 공간 안에 이미 들어가므로 terminal을 추가로 움직일 필요가 없습니다.
        let viewport_top = viewport_area.top();
        if self.visible_history_rows <= viewport_top {
            return Ok(false);
        }

        // 학습 주석: overflow row만큼 host terminal 아래에 line을 append하면 ratatui viewport가 scrollback에서
        // 내려와 다시 frame 영역을 확보합니다. cursor를 마지막 row로 옮긴 뒤 append_lines를 호출해야 합니다.
        let overflow_rows = self.visible_history_rows - viewport_top;
        terminal.backend_mut().set_cursor_position(Position {
            x: 0,
            y: terminal_size.height.saturating_sub(1),
        })?;
        terminal.backend_mut().append_lines(overflow_rows)?;
        self.visible_history_rows = viewport_top;
        Ok(true)
    }

    // 학습 주석: sync는 inline viewport draw cycle의 핵심 flush 단계입니다. app transcript에서 아직 host
    // scrollback에 쓰지 않은 줄을 계산하고, terminal width 기준 rendered row 수를 세어 삽입 전략에 넘깁니다.
    pub(crate) fn sync<B: Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
        current_lines: &[Line<'static>],
        insert_mode: HistoryInsertionMode,
    ) -> Result<HistoryFlushResult, B::Error> {
        self.pending_history_lines = self.pending_lines(current_lines);
        // 학습 주석: Line 수와 실제 terminal row 수는 다릅니다. width를 먼저 읽어 wrap 결과를 계산해야
        // append/scroll 후 viewport bookkeeping이 실제 화면 이동량과 일치합니다.
        let terminal_size = terminal.size()?;
        let width = terminal_size.width;
        let inserted_rows = if self.pending_history_lines.is_empty() {
            0
        } else {
            count_rendered_history_rows(&self.pending_history_lines, width).min(u16::MAX as usize)
                as u16
        };
        // 학습 주석: pending이 없으면 terminal을 건드리지 않습니다. pending이 있으면 선택된 insertion mode가
        // standard scroll-region 또는 newline fallback 중 환경에 맞는 방식으로 실제 줄을 삽입합니다.
        if inserted_rows > 0 {
            HistoryInsertionAdapter::new(insert_mode).insert_with_rendered_rows(
                terminal,
                &self.pending_history_lines,
                inserted_rows,
            )?;
        }
        // 학습 주석: insertion 뒤 frame area top을 다시 읽습니다. host terminal이 줄을 추가하면서 ratatui의
        // viewport origin이 바뀔 수 있기 때문에 이전 viewport top으로 clamp하면 다음 render가 어긋납니다.
        let viewport_top_after_insert = terminal.get_frame().area().top();
        if current_lines.is_empty() {
            // 학습 주석: transcript가 비면 visible history도 없어야 합니다. 이 reset은 새 session attach나
            // history clear 후 stale row count가 남아 다음 frame을 밀어내는 일을 막습니다.
            self.visible_history_rows = 0;
        } else if inserted_rows > 0 {
            // 학습 주석: 전체 transcript가 pending과 같으면 처음 flush하는 상황입니다. 이때는 기존 visible count를
            // 누적하지 않고 이번 삽입량만 viewport 안쪽으로 clamp합니다.
            self.visible_history_rows = if self.pending_history_lines.len() == current_lines.len() {
                inserted_rows.min(viewport_top_after_insert)
            } else {
                // 학습 주석: 일반적인 append에서는 기존 visible rows에 새 rendered rows를 더합니다. saturating_add는
                // 매우 긴 transcript wrap이 들어와도 u16 overflow 없이 viewport clamp까지 도달하게 합니다.
                self.visible_history_rows
                    .saturating_add(inserted_rows)
                    .min(viewport_top_after_insert)
            };
        }
        // 학습 주석: terminal write가 끝난 뒤 current snapshot을 기준점으로 저장하고 pending buffer를 비웁니다.
        // 이후 tick은 이 snapshot과 새 transcript를 비교해 중복 삽입을 피합니다.
        self.remember(current_lines);
        self.pending_history_lines.clear();
        Ok(HistoryFlushResult { inserted_rows })
    }

    // 학습 주석: remember_without_flush는 render mode가 host scrollback에 history를 쓰지 않는 경로에서 사용됩니다.
    // terminal mutation은 생략하지만 기준 snapshot은 갱신해야, 모드를 다시 바꿨을 때 오래된 transcript를 중복 삽입하지 않습니다.
    pub(crate) fn remember_without_flush(&mut self, current_lines: &[Line<'static>]) {
        if current_lines.is_empty() {
            self.visible_history_rows = 0;
        }
        self.pending_history_lines.clear();
        self.remember(current_lines);
    }

    // 학습 주석: remember는 diff 기준선을 교체하는 내부 helper입니다. Line<'static> snapshot을 소유해야
    // 다음 draw cycle에서 runtime app borrow 없이 pending suffix를 계산할 수 있습니다.
    fn remember(&mut self, current_lines: &[Line<'static>]) {
        self.rendered_lines = current_lines.to_vec();
    }

    // 학습 주석: pending_lines는 current transcript 중 아직 scrollback에 쓰지 않은 부분을 고릅니다.
    // 정상 append, session reset, capped history window shift를 구분해 host scrollback 중복과 누락을 모두 피합니다.
    pub(crate) fn pending_lines(&self, current_lines: &[Line<'static>]) -> Vec<Line<'static>> {
        if current_lines.is_empty() {
            return Vec::new();
        }

        // 학습 주석: 가장 흔한 경로는 append-only transcript입니다. 이전 snapshot이 current prefix이면
        // 이미 쓴 prefix를 잘라내고 새 suffix만 flush합니다.
        if current_lines.starts_with(self.rendered_lines.as_slice()) {
            return current_lines[self.rendered_lines.len()..].to_vec();
        }

        // 학습 주석: transcript cap이 찬 뒤 앞쪽 line이 밀려나면 current가 이전 snapshot prefix로 시작하지 않습니다.
        // 대신 이전 snapshot의 suffix와 current prefix가 겹치는 길이를 찾아 그 뒤쪽만 새 history로 봅니다.
        if let Some(overlap_len) = self.shifted_window_overlap_len(current_lines) {
            return current_lines[overlap_len..].to_vec();
        }

        // 학습 주석: prefix도 shifted overlap도 없으면 새 session attach, reset, 또는 완전히 다른 transcript입니다.
        // 이때는 current 전체를 replay해야 host scrollback이 새 app state를 따라갑니다.
        current_lines.to_vec()
    }

    // 학습 주석: shifted_window_overlap_len은 capped transcript 전용 보정입니다. MAX_CONVERSATION_HISTORY_LINES에
    // 도달한 current window에서만 이전 suffix와 현재 prefix의 가장 긴 overlap을 찾습니다.
    fn shifted_window_overlap_len(&self, current_lines: &[Line<'static>]) -> Option<usize> {
        if current_lines.len() != MAX_CONVERSATION_HISTORY_LINES {
            return None;
        }

        // 학습 주석: overlap은 이전 snapshot과 current window 중 더 짧은 길이를 넘을 수 없습니다.
        // 너무 짧은 overlap은 반복되는 blank/status line과 우연히 맞을 위험이 있어 아래 threshold로 거릅니다.
        let max_overlap = self.rendered_lines.len().min(current_lines.len());
        if max_overlap < MIN_SHIFTED_HISTORY_OVERLAP {
            return None;
        }

        // 학습 주석: 긴 overlap부터 찾으면 가장 보수적으로 "이미 쓴 부분"을 인정합니다. 첫 match가 가장 긴
        // 연속 구간이므로 그 뒤의 lines만 pending으로 flush하면 capped window shift에서도 중복이 최소화됩니다.
        (MIN_SHIFTED_HISTORY_OVERLAP..=max_overlap)
            .rev()
            .find(|overlap_len| {
                self.rendered_lines[self.rendered_lines.len() - overlap_len..]
                    == current_lines[..*overlap_len]
            })
    }
}
