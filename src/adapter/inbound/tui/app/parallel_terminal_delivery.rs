use ratatui::layout::Rect;
use ratatui::text::Line;

use super::parallel_supervisor_events::{
    ParallelEventStreamSnapshot, ParallelStreamEventId, ProjectedParallelEvent,
    rendered_parallel_event_line_rows,
};
use super::{InlineHistoryRenderMode, TuiLanguage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ParallelDeliveryCursor {
    stream_generation: u64,
    delivered_through: Option<u64>,
}

impl ParallelDeliveryCursor {
    fn initial(stream_generation: u64) -> Self {
        Self {
            stream_generation,
            delivered_through: None,
        }
    }

    fn through(event_id: ParallelStreamEventId) -> Self {
        Self {
            stream_generation: event_id.stream_generation(),
            delivered_through: Some(event_id.ordinal()),
        }
    }

    fn next_ordinal(self) -> u64 {
        self.delivered_through.map_or(0, |ordinal| {
            ordinal
                .checked_add(1)
                .expect("parallel delivery cursor exhausted")
        })
    }

    fn contains(self, event_id: ParallelStreamEventId) -> bool {
        self.stream_generation == event_id.stream_generation()
            && self
                .delivered_through
                .is_some_and(|ordinal| event_id.ordinal() <= ordinal)
    }

    #[cfg(test)]
    fn delivered_through(self) -> Option<u64> {
        self.delivered_through
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParallelRetentionGap {
    stream_generation: u64,
    missing_from: u64,
    missing_through: u64,
}

impl ParallelRetentionGap {
    fn line(&self, language: TuiLanguage) -> Line<'static> {
        Line::from(
            language.parallel_delivery_retention_gap(self.missing_from, self.missing_through),
        )
    }

    #[cfg(test)]
    fn range(&self) -> std::ops::RangeInclusive<u64> {
        self.missing_from..=self.missing_through
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParallelHostBatch {
    expected_cursor: ParallelDeliveryCursor,
    commit_through: ParallelStreamEventId,
    retention_gap: Option<ParallelRetentionGap>,
    events: Vec<ProjectedParallelEvent>,
}

impl ParallelHostBatch {
    pub(super) fn expected_cursor(&self) -> ParallelDeliveryCursor {
        self.expected_cursor
    }

    pub(super) fn proposed_cursor(&self) -> ParallelDeliveryCursor {
        ParallelDeliveryCursor::through(self.commit_through)
    }

    pub(super) fn lines(&self, language: TuiLanguage) -> Vec<Line<'static>> {
        self.retention_gap
            .as_ref()
            .map(|gap| gap.line(language))
            .into_iter()
            .chain(self.events.iter().map(|event| event.line().clone()))
            .collect()
    }

    #[cfg(test)]
    fn event_ids(&self) -> Vec<ParallelStreamEventId> {
        self.events.iter().map(ProjectedParallelEvent::id).collect()
    }

    #[cfg(test)]
    fn retention_gap(&self) -> Option<&ParallelRetentionGap> {
        self.retention_gap.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ParallelDeliveryGeometry {
    terminal_surface_generation: u64,
    event_area: Rect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParallelLiveLayout {
    Pending,
    Planned {
        title_visible: bool,
        scroll_offset: u16,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParallelLiveStreamModel {
    stream_generation: u64,
    after_cursor: ParallelDeliveryCursor,
    events: Vec<ProjectedParallelEvent>,
    status_lines: Vec<Line<'static>>,
    layout: ParallelLiveLayout,
}

pub(super) struct ParallelLiveRenderParts {
    pub(super) title_visible: bool,
    pub(super) scroll_offset: u16,
    pub(super) lines: Vec<Line<'static>>,
}

impl ParallelLiveStreamModel {
    pub(super) fn pending_viewport(
        snapshot: &ParallelEventStreamSnapshot,
        status_lines: Vec<Line<'static>>,
    ) -> Self {
        Self {
            stream_generation: snapshot.generation(),
            after_cursor: ParallelDeliveryCursor::initial(snapshot.generation()),
            events: snapshot.events().to_vec(),
            status_lines,
            layout: ParallelLiveLayout::Pending,
        }
    }

    fn planned(
        snapshot: &ParallelEventStreamSnapshot,
        after_cursor: ParallelDeliveryCursor,
        events: Vec<ProjectedParallelEvent>,
        status_lines: Vec<Line<'static>>,
        title_visible: bool,
        scroll_offset: u16,
    ) -> Self {
        Self {
            stream_generation: snapshot.generation(),
            after_cursor,
            events,
            status_lines,
            layout: ParallelLiveLayout::Planned {
                title_visible,
                scroll_offset,
            },
        }
    }

    pub(super) fn finalize_pending_geometry(&mut self, event_area: Rect) {
        if !matches!(self.layout, ParallelLiveLayout::Pending) {
            return;
        }
        let lines = self.render_lines();
        let (title_visible, visible_rows) = fitted_title_and_visible_rows(&lines, event_area, true);
        let rendered_rows = rendered_rows(&lines, event_area.width);
        self.layout = ParallelLiveLayout::Planned {
            title_visible,
            scroll_offset: rendered_rows
                .saturating_sub(visible_rows)
                .min(usize::from(u16::MAX)) as u16,
        };
    }

    pub(super) fn into_render_parts(self) -> ParallelLiveRenderParts {
        let ParallelLiveLayout::Planned {
            title_visible,
            scroll_offset,
        } = self.layout
        else {
            panic!("parallel live stream geometry must be finalized before rendering");
        };
        let mut lines = self.status_lines;
        lines.extend(self.events.into_iter().map(|event| event.line().clone()));
        ParallelLiveRenderParts {
            title_visible,
            scroll_offset,
            lines,
        }
    }

    pub(super) fn render_lines(&self) -> Vec<Line<'static>> {
        self.status_lines
            .iter()
            .cloned()
            .chain(self.events.iter().map(|event| event.line().clone()))
            .collect()
    }

    pub(super) fn fallback_status_lines(&self) -> Vec<Line<'static>> {
        self.status_lines.clone()
    }

    #[cfg(test)]
    fn event_ids(&self) -> Vec<ParallelStreamEventId> {
        self.events.iter().map(ProjectedParallelEvent::id).collect()
    }

    #[cfg(test)]
    fn title_visible(&self) -> bool {
        matches!(
            self.layout,
            ParallelLiveLayout::Planned {
                title_visible: true,
                ..
            }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParallelStreamDeliveryPlan {
    geometry: ParallelDeliveryGeometry,
    host_batch: Option<ParallelHostBatch>,
    live_stream: ParallelLiveStreamModel,
}

impl ParallelStreamDeliveryPlan {
    fn prepare(
        snapshot: &ParallelEventStreamSnapshot,
        cursor: ParallelDeliveryCursor,
        geometry: ParallelDeliveryGeometry,
        render_mode: InlineHistoryRenderMode,
        language: TuiLanguage,
        fallback_status_lines: &[Line<'static>],
    ) -> Self {
        assert_eq!(
            snapshot.generation(),
            cursor.stream_generation,
            "parallel stream snapshot and cursor generations must match"
        );
        validate_snapshot(snapshot);

        if render_mode == InlineHistoryRenderMode::ViewportReplay {
            return Self::viewport_replay(
                snapshot,
                cursor,
                geometry,
                language,
                fallback_status_lines,
                None,
            );
        }

        let gap = retention_gap(snapshot, cursor);
        let undelivered = snapshot
            .events()
            .iter()
            .filter(|event| !cursor.contains(event.id()))
            .cloned()
            .collect::<Vec<_>>();
        let idle_status = idle_status_lines(snapshot, language, fallback_status_lines);
        let has_durable_prefix = cursor.delivered_through.is_some() || gap.is_some();
        let candidate_lines = idle_status
            .iter()
            .cloned()
            .chain(undelivered.iter().map(|event| event.line().clone()))
            .collect::<Vec<_>>();
        let (title_visible, visible_rows) = fitted_title_and_visible_rows(
            &candidate_lines,
            geometry.event_area,
            !has_durable_prefix,
        );
        let status_rows = rendered_rows(&idle_status, geometry.event_area.width);
        let live_event_rows = visible_rows.saturating_sub(status_rows);
        let live_start =
            live_event_start_index(&undelivered, live_event_rows, geometry.event_area.width);
        let host_events = undelivered[..live_start].to_vec();
        let live_events = undelivered[live_start..].to_vec();
        let commit_through = host_events
            .last()
            .map(ProjectedParallelEvent::id)
            .or_else(|| {
                gap.as_ref().map(|gap| {
                    ParallelStreamEventId::new(gap.stream_generation, gap.missing_through)
                })
            });
        let host_batch = commit_through.map(|commit_through| ParallelHostBatch {
            expected_cursor: cursor,
            commit_through,
            retention_gap: gap,
            events: host_events,
        });
        let after_cursor = host_batch
            .as_ref()
            .map_or(cursor, ParallelHostBatch::proposed_cursor);
        let live_stream = ParallelLiveStreamModel::planned(
            snapshot,
            after_cursor,
            live_events,
            idle_status,
            title_visible,
            0,
        );
        debug_assert_partition(snapshot, cursor, host_batch.as_ref(), &live_stream);
        Self {
            geometry,
            host_batch,
            live_stream,
        }
    }

    fn blocked(
        snapshot: &ParallelEventStreamSnapshot,
        token: &ParallelHostDeliveryToken,
        geometry: ParallelDeliveryGeometry,
        language: TuiLanguage,
        fallback_status_lines: &[Line<'static>],
    ) -> Self {
        let effective_cursor = if token.proposed_cursor.stream_generation == snapshot.generation() {
            token.proposed_cursor
        } else {
            ParallelDeliveryCursor::initial(snapshot.generation())
        };
        Self::viewport_replay(
            snapshot,
            effective_cursor,
            geometry,
            language,
            fallback_status_lines,
            Some(uncertain_status_line(language)),
        )
    }

    fn viewport_replay(
        snapshot: &ParallelEventStreamSnapshot,
        cursor: ParallelDeliveryCursor,
        geometry: ParallelDeliveryGeometry,
        language: TuiLanguage,
        fallback_status_lines: &[Line<'static>],
        recovery_status: Option<Line<'static>>,
    ) -> Self {
        validate_snapshot(snapshot);
        let recovery_pending = recovery_status.is_some();
        let mut status_lines = recovery_status.into_iter().collect::<Vec<_>>();
        status_lines.extend(idle_status_lines(snapshot, language, fallback_status_lines));
        let events = if recovery_pending {
            snapshot
                .events()
                .iter()
                .filter(|event| !cursor.contains(event.id()))
                .cloned()
                .collect()
        } else {
            snapshot.events().to_vec()
        };
        let lines = status_lines
            .iter()
            .cloned()
            .chain(
                events
                    .iter()
                    .map(|event: &ProjectedParallelEvent| event.line().clone()),
            )
            .collect::<Vec<_>>();
        let (title_visible, visible_rows) =
            fitted_title_and_visible_rows(&lines, geometry.event_area, true);
        let scroll_offset = rendered_rows(&lines, geometry.event_area.width)
            .saturating_sub(visible_rows)
            .min(usize::from(u16::MAX)) as u16;
        Self {
            geometry,
            host_batch: None,
            live_stream: ParallelLiveStreamModel::planned(
                snapshot,
                cursor,
                events,
                status_lines,
                title_visible,
                scroll_offset,
            ),
        }
    }

    pub(super) fn host_batch(&self) -> Option<&ParallelHostBatch> {
        self.host_batch.as_ref()
    }

    pub(super) fn into_parts(self) -> (Option<ParallelHostBatch>, ParallelLiveStreamModel) {
        (self.host_batch, self.live_stream)
    }

    #[cfg(test)]
    fn live_stream(&self) -> &ParallelLiveStreamModel {
        &self.live_stream
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ParallelHostDeliveryAttemptId(u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParallelHostDeliveryToken {
    terminal_surface_generation: u64,
    stream_generation: u64,
    expected_cursor: ParallelDeliveryCursor,
    proposed_cursor: ParallelDeliveryCursor,
    attempt_id: ParallelHostDeliveryAttemptId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParallelHostDeliveryReceipt {
    token: ParallelHostDeliveryToken,
}

impl ParallelHostDeliveryToken {
    pub(super) fn receipt(&self) -> ParallelHostDeliveryReceipt {
        ParallelHostDeliveryReceipt {
            token: self.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ParallelDeliveryState {
    Ready { cursor: ParallelDeliveryCursor },
    Writing { token: ParallelHostDeliveryToken },
    Uncertain { token: ParallelHostDeliveryToken },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParallelHostReceiptSettlement {
    Applied,
    Duplicate,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParallelHostWriteTransition {
    Applied,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // Clear/replace are explicit policies for terminal hosts that expose those operations.
pub(super) enum TerminalSurfaceTransition {
    PreserveHostScrollback,
    ClearHostScrollback,
    ReplaceSurface,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParallelHostWriteStartError {
    DeliveryBlocked,
    StalePlan,
    StaleSurface,
}

#[derive(Debug, Clone)]
pub(super) struct ParallelTerminalDeliveryState {
    state: ParallelDeliveryState,
    terminal_surface_generation: u64,
    next_attempt_id: u64,
    last_committed_token: Option<ParallelHostDeliveryToken>,
}

impl Default for ParallelTerminalDeliveryState {
    fn default() -> Self {
        Self {
            state: ParallelDeliveryState::Ready {
                cursor: ParallelDeliveryCursor::initial(0),
            },
            terminal_surface_generation: 0,
            next_attempt_id: 0,
            last_committed_token: None,
        }
    }
}

impl ParallelTerminalDeliveryState {
    pub(super) fn prepare_plan(
        &mut self,
        snapshot: &ParallelEventStreamSnapshot,
        render_mode: InlineHistoryRenderMode,
        event_area: Rect,
        language: TuiLanguage,
        fallback_status_lines: &[Line<'static>],
    ) -> ParallelStreamDeliveryPlan {
        let geometry = ParallelDeliveryGeometry {
            terminal_surface_generation: self.terminal_surface_generation,
            event_area,
        };
        if let ParallelDeliveryState::Ready { cursor } = &mut self.state
            && cursor.stream_generation != snapshot.generation()
        {
            *cursor = ParallelDeliveryCursor::initial(snapshot.generation());
        }
        match &self.state {
            ParallelDeliveryState::Ready { cursor } => ParallelStreamDeliveryPlan::prepare(
                snapshot,
                *cursor,
                geometry,
                render_mode,
                language,
                fallback_status_lines,
            ),
            ParallelDeliveryState::Writing { token }
            | ParallelDeliveryState::Uncertain { token } => ParallelStreamDeliveryPlan::blocked(
                snapshot,
                token,
                geometry,
                language,
                fallback_status_lines,
            ),
        }
    }

    pub(super) fn begin_host_write(
        &mut self,
        plan: &ParallelStreamDeliveryPlan,
    ) -> Result<ParallelHostDeliveryToken, ParallelHostWriteStartError> {
        let Some(batch) = plan.host_batch() else {
            return Err(ParallelHostWriteStartError::StalePlan);
        };
        if plan.geometry.terminal_surface_generation != self.terminal_surface_generation {
            return Err(ParallelHostWriteStartError::StaleSurface);
        }
        let ParallelDeliveryState::Ready { cursor } = self.state else {
            return Err(ParallelHostWriteStartError::DeliveryBlocked);
        };
        if cursor != batch.expected_cursor() {
            return Err(ParallelHostWriteStartError::StalePlan);
        }
        self.next_attempt_id = self
            .next_attempt_id
            .checked_add(1)
            .expect("parallel host delivery attempt exhausted");
        let token = ParallelHostDeliveryToken {
            terminal_surface_generation: self.terminal_surface_generation,
            stream_generation: cursor.stream_generation,
            expected_cursor: cursor,
            proposed_cursor: batch.proposed_cursor(),
            attempt_id: ParallelHostDeliveryAttemptId(self.next_attempt_id),
        };
        self.state = ParallelDeliveryState::Writing {
            token: token.clone(),
        };
        Ok(token)
    }

    pub(super) fn abort_before_write(
        &mut self,
        token: &ParallelHostDeliveryToken,
    ) -> ParallelHostWriteTransition {
        let ParallelDeliveryState::Writing { token: active } = &self.state else {
            return ParallelHostWriteTransition::Rejected;
        };
        if active != token {
            return ParallelHostWriteTransition::Rejected;
        }
        self.state = ParallelDeliveryState::Ready {
            cursor: token.expected_cursor,
        };
        ParallelHostWriteTransition::Applied
    }

    pub(super) fn mark_uncertain(
        &mut self,
        token: &ParallelHostDeliveryToken,
    ) -> ParallelHostWriteTransition {
        let ParallelDeliveryState::Writing { token: active } = &self.state else {
            return ParallelHostWriteTransition::Rejected;
        };
        if active != token {
            return ParallelHostWriteTransition::Rejected;
        }
        self.state = ParallelDeliveryState::Uncertain {
            token: token.clone(),
        };
        ParallelHostWriteTransition::Applied
    }

    pub(super) fn commit_host_receipt(
        &mut self,
        receipt: ParallelHostDeliveryReceipt,
    ) -> ParallelHostReceiptSettlement {
        let token = receipt.token;
        if token.terminal_surface_generation != self.terminal_surface_generation {
            return ParallelHostReceiptSettlement::Rejected;
        }
        if self
            .last_committed_token
            .as_ref()
            .is_some_and(|committed| committed == &token)
        {
            return ParallelHostReceiptSettlement::Duplicate;
        }
        let ParallelDeliveryState::Writing { token: active } = &self.state else {
            return ParallelHostReceiptSettlement::Rejected;
        };
        if active != &token
            || token.stream_generation != token.expected_cursor.stream_generation
            || token.stream_generation != token.proposed_cursor.stream_generation
        {
            return ParallelHostReceiptSettlement::Rejected;
        }
        self.state = ParallelDeliveryState::Ready {
            cursor: token.proposed_cursor,
        };
        self.last_committed_token = Some(token);
        ParallelHostReceiptSettlement::Applied
    }

    pub(super) fn transition_terminal_surface(
        &mut self,
        transition: TerminalSurfaceTransition,
        active_stream_generation: u64,
    ) {
        if transition == TerminalSurfaceTransition::PreserveHostScrollback {
            return;
        }
        self.terminal_surface_generation = self
            .terminal_surface_generation
            .checked_add(1)
            .expect("terminal surface generation exhausted");
        self.state = ParallelDeliveryState::Ready {
            cursor: ParallelDeliveryCursor::initial(active_stream_generation),
        };
    }

    #[cfg(test)]
    fn cursor(&self) -> ParallelDeliveryCursor {
        match &self.state {
            ParallelDeliveryState::Ready { cursor } => *cursor,
            ParallelDeliveryState::Writing { token }
            | ParallelDeliveryState::Uncertain { token } => token.expected_cursor,
        }
    }

    #[cfg(test)]
    fn is_uncertain(&self) -> bool {
        matches!(self.state, ParallelDeliveryState::Uncertain { .. })
    }

    #[cfg(test)]
    pub(super) fn has_delivered(&self, event_id: ParallelStreamEventId) -> bool {
        matches!(
            self.state,
            ParallelDeliveryState::Ready { cursor } if cursor.contains(event_id)
        )
    }
}

fn validate_snapshot(snapshot: &ParallelEventStreamSnapshot) {
    let mut previous = None;
    for event in snapshot.events() {
        assert_eq!(
            event.id().stream_generation(),
            snapshot.generation(),
            "parallel event generation must match its snapshot"
        );
        assert!(
            previous.is_none_or(|ordinal| event.id().ordinal() > ordinal),
            "parallel event ordinals must be strictly increasing"
        );
        previous = Some(event.id().ordinal());
    }
    if let Some(first) = snapshot.events().first() {
        assert_eq!(
            first.id().ordinal(),
            snapshot.first_ordinal(),
            "parallel snapshot first ordinal must match the retained window"
        );
    }
}

fn retention_gap(
    snapshot: &ParallelEventStreamSnapshot,
    cursor: ParallelDeliveryCursor,
) -> Option<ParallelRetentionGap> {
    let missing_from = cursor.next_ordinal();
    let missing_through = snapshot.first_ordinal().checked_sub(1)?;
    (missing_from <= missing_through).then_some(ParallelRetentionGap {
        stream_generation: snapshot.generation(),
        missing_from,
        missing_through,
    })
}

fn live_event_start_index(
    events: &[ProjectedParallelEvent],
    live_rows: usize,
    width: u16,
) -> usize {
    if events.is_empty() {
        return 0;
    }
    if live_rows == 0 || width == 0 {
        return events.len();
    }
    let mut used_rows = 0usize;
    let mut start = events.len();
    for (index, event) in events.iter().enumerate().rev() {
        let rows = rendered_parallel_event_line_rows(event.line(), width);
        if used_rows.saturating_add(rows) > live_rows {
            break;
        }
        used_rows += rows;
        start = index;
    }
    start
}

fn idle_status_lines(
    snapshot: &ParallelEventStreamSnapshot,
    language: TuiLanguage,
    fallback_status_lines: &[Line<'static>],
) -> Vec<Line<'static>> {
    if !snapshot.events().is_empty() {
        return Vec::new();
    }
    if !fallback_status_lines.is_empty() {
        return fallback_status_lines.to_vec();
    }
    vec![Line::from(language.no_parallel_events())]
}

fn uncertain_status_line(language: TuiLanguage) -> Line<'static> {
    Line::from(language.parallel_delivery_uncertain())
}

fn fitted_title_and_visible_rows(
    lines: &[Line<'static>],
    area: Rect,
    title_allowed: bool,
) -> (bool, usize) {
    let titled_rows = usize::from(area.height.saturating_sub(1));
    let title_visible = title_allowed && rendered_rows(lines, area.width) <= titled_rows;
    (
        title_visible,
        usize::from(if title_visible {
            area.height.saturating_sub(1)
        } else {
            area.height
        }),
    )
}

fn rendered_rows(lines: &[Line<'static>], width: u16) -> usize {
    lines
        .iter()
        .map(|line| rendered_parallel_event_line_rows(line, width))
        .sum()
}

fn debug_assert_partition(
    snapshot: &ParallelEventStreamSnapshot,
    cursor: ParallelDeliveryCursor,
    host_batch: Option<&ParallelHostBatch>,
    live_stream: &ParallelLiveStreamModel,
) {
    #[cfg(debug_assertions)]
    {
        let host_ids = host_batch
            .into_iter()
            .flat_map(|batch| batch.events.iter())
            .map(ProjectedParallelEvent::id)
            .collect::<std::collections::BTreeSet<_>>();
        let live_ids = live_stream
            .events
            .iter()
            .map(ProjectedParallelEvent::id)
            .collect::<std::collections::BTreeSet<_>>();
        debug_assert!(host_ids.is_disjoint(&live_ids));
        let visible_retained = snapshot
            .events()
            .iter()
            .map(ProjectedParallelEvent::id)
            .filter(|event_id| !cursor.contains(*event_id))
            .collect::<std::collections::BTreeSet<_>>();
        debug_assert_eq!(
            host_ids
                .union(&live_ids)
                .copied()
                .collect::<std::collections::BTreeSet<_>>(),
            visible_retained
        );
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::adapter::inbound::tui::app::parallel_supervisor_events::ParallelEventStreamState;

    fn stream_with_events(count: usize) -> ParallelEventStreamState {
        let mut stream = ParallelEventStreamState::default();
        for index in 0..count {
            stream.push_for_test("12:00:00", "Supervisor", format!("event-{index:03}"));
        }
        stream
    }

    fn plan(
        state: &mut ParallelTerminalDeliveryState,
        snapshot: &ParallelEventStreamSnapshot,
        mode: InlineHistoryRenderMode,
        width: u16,
        height: u16,
    ) -> ParallelStreamDeliveryPlan {
        state.prepare_plan(
            snapshot,
            mode,
            Rect::new(0, 0, width, height),
            TuiLanguage::English,
            &[],
        )
    }

    fn commit(
        state: &mut ParallelTerminalDeliveryState,
        plan: &ParallelStreamDeliveryPlan,
    ) -> ParallelHostDeliveryToken {
        let token = state
            .begin_host_write(plan)
            .expect("host batch should start from the current frontier");
        assert_eq!(
            state.commit_host_receipt(token.receipt()),
            ParallelHostReceiptSettlement::Applied
        );
        token
    }

    #[test]
    fn planner_partitions_retained_ids_without_overlap() {
        let stream = stream_with_events(7);
        let snapshot = stream.snapshot();
        let mut state = ParallelTerminalDeliveryState::default();
        let plan = plan(
            &mut state,
            &snapshot,
            InlineHistoryRenderMode::HostScrollback,
            80,
            3,
        );
        let host_ids = plan
            .host_batch()
            .expect("overflow should create a durable prefix")
            .event_ids()
            .into_iter()
            .collect::<BTreeSet<_>>();
        let live_ids = plan
            .live_stream()
            .event_ids()
            .into_iter()
            .collect::<BTreeSet<_>>();
        assert!(host_ids.is_disjoint(&live_ids));
        assert_eq!(host_ids.len() + live_ids.len(), snapshot.events().len());
        assert_eq!(live_ids.len(), 3);
    }

    #[test]
    fn committed_frontier_is_monotonic_and_frame_failure_cannot_replay_host_ids() {
        let stream = stream_with_events(7);
        let snapshot = stream.snapshot();
        let mut state = ParallelTerminalDeliveryState::default();
        let first = plan(
            &mut state,
            &snapshot,
            InlineHistoryRenderMode::HostScrollback,
            80,
            3,
        );
        let token = commit(&mut state, &first);
        let committed = token.proposed_cursor.delivered_through();

        // A frame failure has no host-delivery receipt and therefore no state transition.
        let retry = plan(
            &mut state,
            &snapshot,
            InlineHistoryRenderMode::HostScrollback,
            80,
            3,
        );
        assert!(retry.host_batch().is_none());
        assert_eq!(state.cursor().delivered_through(), committed);
        assert!(
            retry
                .live_stream()
                .event_ids()
                .iter()
                .all(|event_id| event_id.ordinal() > committed.unwrap())
        );
    }

    #[test]
    fn duplicate_and_stale_receipts_cannot_mutate_the_frontier() {
        let stream = stream_with_events(7);
        let snapshot = stream.snapshot();
        let mut state = ParallelTerminalDeliveryState::default();
        let first = plan(
            &mut state,
            &snapshot,
            InlineHistoryRenderMode::HostScrollback,
            80,
            3,
        );
        let token = state.begin_host_write(&first).expect("write should start");
        let receipt = token.receipt();
        assert_eq!(
            state.commit_host_receipt(receipt.clone()),
            ParallelHostReceiptSettlement::Applied
        );
        let committed = state.cursor();
        assert_eq!(
            state.commit_host_receipt(receipt),
            ParallelHostReceiptSettlement::Duplicate
        );
        assert_eq!(state.cursor(), committed);
        assert_eq!(
            state.begin_host_write(&first),
            Err(ParallelHostWriteStartError::StalePlan)
        );

        state.transition_terminal_surface(TerminalSurfaceTransition::ReplaceSurface, 0);
        assert_eq!(
            state.commit_host_receipt(token.receipt()),
            ParallelHostReceiptSettlement::Rejected
        );
        assert_eq!(state.cursor().delivered_through(), None);

        state.transition_terminal_surface(TerminalSurfaceTransition::ClearHostScrollback, 0);
        assert_eq!(state.cursor().delivered_through(), None);
    }

    #[test]
    fn prewrite_abort_is_retryable_with_a_new_exact_attempt() {
        let stream = stream_with_events(7);
        let snapshot = stream.snapshot();
        let mut state = ParallelTerminalDeliveryState::default();
        let first = plan(
            &mut state,
            &snapshot,
            InlineHistoryRenderMode::HostScrollback,
            80,
            3,
        );
        let token = state.begin_host_write(&first).expect("write should start");
        assert_eq!(
            state.abort_before_write(&token),
            ParallelHostWriteTransition::Applied
        );
        assert_eq!(state.cursor().delivered_through(), None);
        let retry = state
            .begin_host_write(&first)
            .expect("same range is retryable");
        assert!(retry.attempt_id > token.attempt_id);
    }

    #[test]
    fn ambiguous_write_blocks_replay_and_exposes_recovery_status() {
        let stream = stream_with_events(7);
        let snapshot = stream.snapshot();
        let mut state = ParallelTerminalDeliveryState::default();
        let first = plan(
            &mut state,
            &snapshot,
            InlineHistoryRenderMode::HostScrollback,
            80,
            3,
        );
        let token = state.begin_host_write(&first).expect("write should start");
        assert_eq!(
            state.mark_uncertain(&token),
            ParallelHostWriteTransition::Applied
        );
        assert!(state.is_uncertain());

        let blocked = plan(
            &mut state,
            &snapshot,
            InlineHistoryRenderMode::HostScrollback,
            80,
            3,
        );
        assert!(blocked.host_batch().is_none());
        assert!(
            blocked
                .live_stream()
                .render_lines()
                .first()
                .expect("recovery status should be visible")
                .to_string()
                .contains("delivery uncertain")
        );
        assert!(blocked.live_stream().event_ids().iter().all(|event_id| {
            event_id.ordinal() > token.proposed_cursor.delivered_through().unwrap()
        }));
    }

    #[test]
    fn retention_loss_produces_one_typed_gap_before_advancing() {
        let stream = stream_with_events(520);
        let snapshot = stream.snapshot();
        assert_eq!(snapshot.first_ordinal(), 8);
        let mut state = ParallelTerminalDeliveryState::default();
        let plan = plan(
            &mut state,
            &snapshot,
            InlineHistoryRenderMode::HostScrollback,
            200,
            5,
        );
        let gap = plan
            .host_batch()
            .and_then(ParallelHostBatch::retention_gap)
            .expect("expired undelivered events require a typed gap");
        assert_eq!(gap.range(), 0..=7);
        assert_eq!(
            plan.host_batch()
                .expect("gap plan has one durable batch")
                .lines(TuiLanguage::English)
                .iter()
                .filter(|line| line.to_string().contains("retention gap"))
                .count(),
            1
        );
    }

    #[test]
    fn viewport_mode_switches_preserve_the_host_cursor() {
        let stream = stream_with_events(7);
        let snapshot = stream.snapshot();
        let mut state = ParallelTerminalDeliveryState::default();
        let host = plan(
            &mut state,
            &snapshot,
            InlineHistoryRenderMode::HostScrollback,
            80,
            3,
        );
        commit(&mut state, &host);
        let committed = state.cursor();

        let replay = plan(
            &mut state,
            &snapshot,
            InlineHistoryRenderMode::ViewportReplay,
            80,
            3,
        );
        assert!(replay.host_batch().is_none());
        assert_eq!(replay.live_stream().event_ids().len(), 7);
        assert_eq!(state.cursor(), committed);

        let host_again = plan(
            &mut state,
            &snapshot,
            InlineHistoryRenderMode::HostScrollback,
            80,
            3,
        );
        assert!(host_again.host_batch().is_none());
        assert_eq!(state.cursor(), committed);
    }

    #[test]
    fn deterministic_operation_sequence_never_decreases_the_frontier() {
        let mut stream = stream_with_events(12);
        let mut state = ParallelTerminalDeliveryState::default();
        let mut observed = None;
        for step in 0..24 {
            if step % 4 == 0 {
                stream.push_for_test("12:00:01", "Supervisor", format!("append-{step}"));
            }
            let snapshot = stream.snapshot();
            let mode = if step % 5 == 0 {
                InlineHistoryRenderMode::ViewportReplay
            } else {
                InlineHistoryRenderMode::HostScrollback
            };
            let height = if step % 3 == 0 { 2 } else { 5 };
            let plan = plan(&mut state, &snapshot, mode, 24, height);
            if mode == InlineHistoryRenderMode::HostScrollback && plan.host_batch().is_some() {
                if step % 7 == 0 {
                    let token = state.begin_host_write(&plan).expect("write should start");
                    assert_eq!(
                        state.abort_before_write(&token),
                        ParallelHostWriteTransition::Applied
                    );
                } else {
                    commit(&mut state, &plan);
                }
            }
            let current = state.cursor().delivered_through();
            if let (Some(previous), Some(current)) = (observed, current) {
                assert!(current >= previous);
            }
            observed = current.or(observed);
        }
    }

    #[test]
    fn pending_viewport_model_finalizes_layout_before_rendering() {
        let stream = stream_with_events(4);
        let snapshot = stream.snapshot();
        let mut model = ParallelLiveStreamModel::pending_viewport(&snapshot, Vec::new());
        model.finalize_pending_geometry(Rect::new(0, 0, 80, 6));
        assert!(model.title_visible());
        assert_eq!(model.into_render_parts().lines.len(), 4);
    }
}
