use super::super::super::{AkraTheme, ConversationState, Line, NativeTuiApp};
use super::{ReviewOverlayView, ReviewsOverlayView};
use crate::application::port::outbound::review_center_repository_port::{
    ReviewCenterHistoryEntry, ReviewCenterInboxItem, ReviewCenterThreadProjection,
};
use crate::domain::text::compact_whitespace_detail;

const REVIEW_ENTRY_LIMIT: usize = 3;
const REVIEW_ID_DETAIL_LIMIT: usize = 28;
const REVIEW_SUMMARY_DETAIL_LIMIT: usize = 52;
const REVIEW_WORKSPACE_DETAIL_LIMIT: usize = 44;

pub(crate) fn build_reviews_overlay_view(app: &NativeTuiApp) -> ReviewsOverlayView {
    app.build_reviews_overlay_view()
}

impl NativeTuiApp {
    pub(in crate::adapter::inbound::tui::app) fn build_reviews_overlay_view(
        &self,
    ) -> ReviewsOverlayView {
        let workspace_directory = self.planning_workspace_directory();
        let header_lines = vec![
            AkraTheme::title_line("Review Center", " / shell inspection"),
            Line::from("Check inbox pressure, active-thread review context, and recent outcomes."),
        ];
        let active_thread = match &self.conversation_state {
            ConversationState::Ready(conversation) if conversation.has_active_thread() => {
                Some((
                    conversation.thread_id.clone(),
                    conversation.resumed_thread_review_summary().map(str::to_string),
                    conversation
                        .resumed_thread_review_manual_handoff_context()
                        .map(str::to_string),
                ))
            }
            _ => None,
        };
        let current_thread_state = match active_thread.as_ref() {
            Some((thread_id, _, _)) => load_section(
                self.application
                    .load_review_center_thread_reviews_for_workspace(&workspace_directory, thread_id)
                    .map_err(anyhow::Error::msg)
                    .map(|reviews| build_thread_review_views(&reviews)),
            ),
            None => ReviewSectionState::Loaded {
                total_count: 0,
                entries: Vec::new(),
            },
        };
        let inbox_state = load_section(
            self.application
                .load_review_center_pending_inbox()
                .map_err(anyhow::Error::msg)
                .map(|inbox| build_inbox_review_views(&inbox)),
        );
        let history_state = load_section(
            self.application
                .load_review_center_recent_history()
                .map_err(anyhow::Error::msg)
                .map(|history| build_history_review_views(&history)),
        );

        let mut summary_lines = vec![Line::from(format!(
            "workspace: {}",
            compact_whitespace_detail(&workspace_directory, REVIEW_WORKSPACE_DETAIL_LIMIT)
        ))];
        summary_lines.push(Line::from(format!(
            "thread: {}  |  inbox: {}  |  history: {}",
            active_thread
                .as_ref()
                .map(|(thread_id, _, _)| compact_whitespace_detail(thread_id, REVIEW_ID_DETAIL_LIMIT))
                .unwrap_or_else(|| "draft".to_string()),
            inbox_state.count_label("pending"),
            history_state.count_label("recent")
        )));
        if let Some((_, summary, handoff_context)) = active_thread.as_ref() {
            if let Some(summary) = summary.as_deref() {
                summary_lines.push(Line::from(format!(
                    "active: {}",
                    compact_whitespace_detail(summary, REVIEW_SUMMARY_DETAIL_LIMIT)
                )));
            }
            if let Some(handoff_context) = handoff_context.as_deref() {
                summary_lines.push(Line::from(format!(
                    "handoff: {}",
                    compact_whitespace_detail(handoff_context, REVIEW_SUMMARY_DETAIL_LIMIT)
                )));
            }
        }

        ReviewsOverlayView {
            header_lines,
            summary_lines,
            current_thread_reviews: current_thread_state.into_entries(),
            inbox_reviews: inbox_state.into_entries(),
            history_reviews: history_state.into_entries(),
            key_lines: vec![AkraTheme::key_line(
                "Esc/Ctrl+C: close  |  read-only: use admin/telegram for actions",
            )],
        }
    }
}

enum ReviewSectionState {
    Loaded {
        total_count: usize,
        entries: Vec<ReviewOverlayView>,
    },
    Unavailable(String),
}

impl ReviewSectionState {
    fn count_label(&self, noun: &str) -> String {
        match self {
            Self::Loaded { total_count, .. } => format!("{total_count} {noun}"),
            Self::Unavailable(_) => "unavailable".to_string(),
        }
    }

    fn into_entries(self) -> Vec<ReviewOverlayView> {
        match self {
            Self::Loaded { entries, .. } => entries,
            Self::Unavailable(message) => vec![ReviewOverlayView {
                summary_line: Line::from("Review data unavailable"),
                detail_lines: vec![Line::from(format!(
                    "detail: {}",
                    compact_whitespace_detail(&message, REVIEW_SUMMARY_DETAIL_LIMIT)
                ))],
            }],
        }
    }
}

fn load_section(result: anyhow::Result<(usize, Vec<ReviewOverlayView>)>) -> ReviewSectionState {
    match result {
        Ok((total_count, entries)) => ReviewSectionState::Loaded {
            total_count,
            entries,
        },
        Err(error) => ReviewSectionState::Unavailable(error.to_string()),
    }
}

fn build_thread_review_views(
    reviews: &[ReviewCenterThreadProjection],
) -> (usize, Vec<ReviewOverlayView>) {
    let mut entries = reviews
        .iter()
        .rev()
        .take(REVIEW_ENTRY_LIMIT)
        .map(|review| ReviewOverlayView {
            summary_line: Line::from(format!(
                "{} [{}] {}",
                compact_whitespace_detail(review.review_label.trim(), REVIEW_ID_DETAIL_LIMIT),
                compact_whitespace_detail(review.review_state.trim(), 18),
                compact_whitespace_detail(review.review_summary.trim(), REVIEW_SUMMARY_DETAIL_LIMIT)
            )),
            detail_lines: build_thread_review_detail_lines(review),
        })
        .collect::<Vec<_>>();
    push_hidden_count_line(&mut entries, reviews.len());
    (reviews.len(), entries)
}

fn build_thread_review_detail_lines(review: &ReviewCenterThreadProjection) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(format!(
        "thread: {}  |  updated: {}",
        compact_whitespace_detail(review.thread_id.trim(), REVIEW_ID_DETAIL_LIMIT),
        compact_whitespace_detail(review.updated_at.trim(), REVIEW_ID_DETAIL_LIMIT)
    ))];
    if let Some(handoff) = format_handoff(
        review.handoff_target.as_deref(),
        review.handoff_note.as_deref(),
    ) {
        lines.push(Line::from(format!(
            "handoff: {}",
            compact_whitespace_detail(&handoff, REVIEW_SUMMARY_DETAIL_LIMIT)
        )));
    }
    lines
}

fn build_inbox_review_views(inbox: &[ReviewCenterInboxItem]) -> (usize, Vec<ReviewOverlayView>) {
    let mut entries = inbox
        .iter()
        .take(REVIEW_ENTRY_LIMIT)
        .map(|item| ReviewOverlayView {
            summary_line: Line::from(format!(
                "{} [{}] {}",
                compact_whitespace_detail(item.review_id.trim(), REVIEW_ID_DETAIL_LIMIT),
                compact_whitespace_detail(item.inbox_state.trim(), 18),
                compact_whitespace_detail(item.summary.trim(), REVIEW_SUMMARY_DETAIL_LIMIT)
            )),
            detail_lines: build_inbox_review_detail_lines(item),
        })
        .collect::<Vec<_>>();
    push_hidden_count_line(&mut entries, inbox.len());
    (inbox.len(), entries)
}

fn build_inbox_review_detail_lines(item: &ReviewCenterInboxItem) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(format!(
        "thread: {}  |  requested: {}",
        compact_whitespace_detail(item.thread_id.trim(), REVIEW_ID_DETAIL_LIMIT),
        compact_whitespace_detail(item.requested_at.trim(), REVIEW_ID_DETAIL_LIMIT)
    ))];
    lines.push(Line::from(format!(
        "last activity: {}",
        compact_whitespace_detail(item.last_activity_at.trim(), REVIEW_ID_DETAIL_LIMIT)
    )));
    if let Some(handoff_target) = item
        .handoff_target
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(Line::from(format!(
            "handoff: {}",
            compact_whitespace_detail(handoff_target, REVIEW_SUMMARY_DETAIL_LIMIT)
        )));
    }
    lines
}

fn build_history_review_views(history: &[ReviewCenterHistoryEntry]) -> (usize, Vec<ReviewOverlayView>) {
    let mut entries = history
        .iter()
        .take(REVIEW_ENTRY_LIMIT)
        .map(|entry| ReviewOverlayView {
            summary_line: Line::from(format!(
                "{} {}",
                compact_whitespace_detail(entry.event_kind.trim(), REVIEW_ID_DETAIL_LIMIT),
                compact_whitespace_detail(entry.summary.trim(), REVIEW_SUMMARY_DETAIL_LIMIT)
            )),
            detail_lines: vec![Line::from(format!(
                "thread: {}  |  at: {}",
                compact_whitespace_detail(entry.thread_id.trim(), REVIEW_ID_DETAIL_LIMIT),
                compact_whitespace_detail(entry.recorded_at.trim(), REVIEW_ID_DETAIL_LIMIT)
            ))],
        })
        .collect::<Vec<_>>();
    push_hidden_count_line(&mut entries, history.len());
    (history.len(), entries)
}

fn push_hidden_count_line(entries: &mut Vec<ReviewOverlayView>, total_count: usize) {
    let hidden_count = total_count.saturating_sub(REVIEW_ENTRY_LIMIT);
    if hidden_count == 0 {
        return;
    }

    entries.push(ReviewOverlayView {
        summary_line: Line::from(format!(
            "+{hidden_count} more item{} hidden for readability",
            if hidden_count == 1 { "" } else { "s" }
        )),
        detail_lines: Vec::new(),
    });
}

fn format_handoff(handoff_target: Option<&str>, handoff_note: Option<&str>) -> Option<String> {
    let handoff_target = handoff_target.map(str::trim).filter(|value| !value.is_empty());
    let handoff_note = handoff_note.map(str::trim).filter(|value| !value.is_empty());
    match (handoff_target, handoff_note) {
        (Some(target), Some(note)) => Some(format!("{target}: {note}")),
        (Some(target), None) => Some(target.to_string()),
        (None, Some(note)) => Some(note.to_string()),
        (None, None) => None,
    }
}
