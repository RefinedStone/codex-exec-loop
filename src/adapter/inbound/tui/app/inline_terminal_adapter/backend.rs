use std::ops::Range;

use ratatui::backend::{Backend, ClearType};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};

/*
 * InlineTerminalAdapter는 frame을 host terminal scrollback 안에 끼워 넣어 그린다.
 * ratatui::Backend에는 "resize 중 append_lines를 잠시 무시해 달라"는 훅이 없으므로,
 * adapter가 필요한 제어점을 이 작은 확장 trait로 추가한다. 실제 구현은 아래 wrapper가 맡고,
 * 상위 inline_terminal_adapter.rs는 Backend + 이 resize 제어 계약만 알고 동작한다.
 */
pub(crate) trait InlineResizeBackend: Backend {
    fn set_resize_append_lines_suppressed(&mut self, suppressed: bool);
}

/*
 * InlineTerminalBackend는 실제 터미널 backend 앞에 놓이는 얇은 보정막이다.
 * draw는 terminal 크기 밖 cell을 버리고, append_lines는 inline viewport resize 중에
 * host scrollback을 밀지 않도록 억제할 수 있으며, cursor 위치는 append_lines 이후에도
 * 상위 rendering 코드가 같은 좌표계를 이어 쓰도록 캐시한다.
 */
pub(crate) struct InlineTerminalBackend<B> {
    /*
     * 실제 출력은 CrosstermBackend, TestBackend, Vt100Backend 같은 inner가 수행한다.
     * 이 wrapper는 I/O 자체를 바꾸기보다 inline terminal에 필요한 전후 보정만 추가한다.
     */
    inner: B,
    /*
     * autoresize_inline_viewport와 draw_inline_frame은 크기 조정 중 ratatui가 호출하는
     * append_lines가 host scrollback에 새 줄을 밀어 넣지 않도록 이 flag를 켠다.
     */
    suppress_resize_append_lines: bool,
    /*
     * ratatui backend의 cursor query는 실제 터미널에 물어볼 수 있어 비용과 부작용이 있다.
     * 한 번 읽거나 설정한 좌표를 기억해 inline history 삽입 뒤에도 shell 위치 계산을 안정화한다.
     */
    tracked_cursor_position: Option<Position>,
}

impl<B> InlineTerminalBackend<B> {
    pub(crate) fn new(inner: B) -> Self {
        /*
         * 기본 상태에서는 일반 backend처럼 모든 append_lines를 통과시킨다.
         * cursor는 첫 query나 set_cursor_position에서 채워져 이후 append_lines 보정에 쓰인다.
         */
        Self {
            inner,
            suppress_resize_append_lines: false,
            tracked_cursor_position: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn inner(&self) -> &B {
        /*
         * 테스트는 wrapper가 inner backend에 몇 번 질의했는지, 어떤 buffer를 남겼는지
         * 확인해야 하므로 read-only 접근자를 cfg(test)로만 연다.
         */
        &self.inner
    }

    #[cfg(test)]
    pub(crate) fn inner_mut(&mut self) -> &mut B {
        /*
         * fixture backend의 cursor나 화면 상태를 직접 준비하기 위한 테스트 전용 통로다.
         * production 경로에서는 wrapper의 보정 규칙을 우회하지 못하게 닫아 둔다.
         */
        &mut self.inner
    }
}

impl<B: Backend> InlineResizeBackend for InlineTerminalBackend<B> {
    fn set_resize_append_lines_suppressed(&mut self, suppressed: bool) {
        /*
         * 상위 adapter는 resize/replay 구간의 앞뒤에서 이 값을 토글한다.
         * flag만 바꾸고 flush하지 않아야 같은 draw transaction 안에서 host scrollback 변화만 막을 수 있다.
         */
        self.suppress_resize_append_lines = suppressed;
    }
}

impl<B: Backend> Backend for InlineTerminalBackend<B> {
    type Error = B::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        /*
         * inline viewport는 resize와 replay가 섞일 때 ratatui buffer보다 실제 backend가 작을 수 있다.
         * 크기 밖 cell을 inner에 넘기면 테스트 backend와 vt100 backend가 서로 다르게 반응할 수 있어,
         * wrapper에서 현재 size 안의 cell만 통과시켜 draw의 좌표계를 단일화한다.
         */
        let size = self.inner.size()?;
        self.inner
            .draw(content.filter(move |(x, y, _)| *x < size.width && *y < size.height))
    }

    fn append_lines(&mut self, n: u16) -> Result<(), Self::Error> {
        /*
         * append_lines는 host terminal 기준으로 스크롤백에 줄을 추가하는 강한 동작이다.
         * inline viewport를 다시 그리는 동안에는 화면 높이 보정용 호출이 실제 scrollback 삽입으로
         * 번지면 안 되므로 flag가 켜진 구간에서는 성공한 no-op으로 처리한다.
         */
        if self.suppress_resize_append_lines {
            return Ok(());
        }
        self.inner.append_lines(n)?;
        self.track_cursor_after_append_lines(n);
        Ok(())
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        /*
         * cursor visibility는 inline 보정 상태와 독립적인 terminal side effect다.
         * wrapper가 의미를 추가하지 않으므로 inner backend에 그대로 위임한다.
         */
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        // hide_cursor와 같은 이유로 visibility 복원도 inner의 책임을 그대로 따른다.
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        /*
         * 첫 cursor query 뒤에는 wrapper가 append_lines와 set_cursor_position을 추적한다.
         * 이렇게 해야 history flush가 여러 번 이어져도 실제 terminal query를 반복하지 않고
         * inline shell positioning이 같은 좌표를 기준으로 계산된다.
         */
        if let Some(position) = self.tracked_cursor_position {
            return Ok(position);
        }
        let position = self.inner.get_cursor_position()?;
        self.tracked_cursor_position = Some(position);
        Ok(position)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        /*
         * cursor를 실제 backend에 이동시킨 뒤 같은 값을 cache에도 반영한다.
         * 이후 append_lines 보정은 이 cached position을 화면 아래쪽으로 밀어 shell cursor를 따라간다.
         */
        let position = position.into();
        self.inner.set_cursor_position(position)?;
        self.tracked_cursor_position = Some(position);
        Ok(())
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        /*
         * clear 계열은 ratatui가 terminal surface를 비우는 명령이다.
         * inline wrapper는 scrollback 삽입과 cursor tracking만 보정하므로 clear 의미는 바꾸지 않는다.
         */
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        // region clear도 draw clipping 대상이 아니라 backend 고유 구현에 그대로 맡긴다.
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> Result<Size, Self::Error> {
        /*
         * size는 draw clipping과 상위 layout 계산의 공통 기준이다.
         * wrapper가 별도 viewport 크기를 들고 있지 않으므로 inner backend의 현재 크기가 곧 진실이다.
         */
        self.inner.size()
    }

    fn window_size(&mut self) -> Result<ratatui::backend::WindowSize, Self::Error> {
        /*
         * pixel/row 단위 window 정보는 terminal backend만 알 수 있다.
         * inline adapter는 cell 좌표 보정만 하므로 window_size를 변형하지 않는다.
         */
        self.inner.window_size()
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        /*
         * flush 시점은 ratatui Terminal이 draw transaction을 끝내는 경계다.
         * wrapper 내부 flag나 cursor cache를 여기서 초기화하면 같은 frame의 replay 계산이 깨지므로 위임만 한다.
         */
        self.inner.flush()
    }

    fn scroll_region_up(&mut self, region: Range<u16>, line_count: u16) -> Result<(), Self::Error> {
        /*
         * scroll_region_*은 backend가 지원하는 viewport 내부 스크롤 명령이다.
         * host scrollback에 새 줄을 추가하는 append_lines와 달리 region 내부 동작이라 그대로 보낸다.
         */
        self.inner.scroll_region_up(region, line_count)
    }

    fn scroll_region_down(
        &mut self,
        region: Range<u16>,
        line_count: u16,
    ) -> Result<(), Self::Error> {
        // scroll_region_up과 대칭인 동작이며 wrapper가 추가 상태를 갱신하지 않는다.
        self.inner.scroll_region_down(region, line_count)
    }
}

impl<B: Backend> InlineTerminalBackend<B> {
    fn track_cursor_after_append_lines(&mut self, line_count: u16) {
        /*
         * append_lines는 terminal 하단에 줄을 추가하며 cursor를 다음 줄로 밀 수 있다.
         * 실제 backend마다 cursor query 결과가 달라질 수 있으므로, wrapper가 알고 있는 cursor를
         * 같은 규칙으로 갱신해 이후 get_cursor_position이 일관된 값을 반환하게 한다.
         */
        if line_count == 0 {
            return;
        }
        let Some(mut position) = self.tracked_cursor_position else {
            /*
             * 아직 cursor를 관찰한 적이 없으면 추정하지 않는다.
             * 첫 query에서 inner backend의 실제 위치를 받아오는 편이 더 안전하다.
             */
            return;
        };
        if let Ok(size) = self.inner.size() {
            /*
             * 높이를 모르면 cursor clamping도 할 수 없으므로 size 조회 성공 시에만 갱신한다.
             * height 0은 backend fixture나 비정상 resize에서 나올 수 있는 방어 케이스다.
             */
            if size.height == 0 {
                return;
            }
            position.x = 0;
            position.y = position
                .y
                .saturating_add(line_count)
                .min(size.height.saturating_sub(1));
            self.tracked_cursor_position = Some(position);
        }
    }
}

impl<B: std::fmt::Display> std::fmt::Display for InlineTerminalBackend<B> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        /*
         * snapshot 테스트와 debug 출력은 inner backend의 display 구현을 기대한다.
         * wrapper 상태를 함께 출력하면 기존 terminal 화면 비교가 흔들리므로 그대로 위임한다.
         */
        self.inner.fmt(formatter)
    }
}
