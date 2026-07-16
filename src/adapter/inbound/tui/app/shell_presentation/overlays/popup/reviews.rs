use super::super::super::{AkraTheme, Line};
use super::{ReviewOverlayView, ReviewsOverlayView};
use crate::adapter::inbound::tui::app::reviews_overlay_ui::{
    ReviewsOverlayAuthoritySnapshot, ReviewsOverlayContext, ReviewsOverlayScreenModel,
};
use crate::application::port::outbound::review_center_repository_port::{
    ReviewCenterHistoryEntry, ReviewCenterInboxItem, ReviewCenterThreadProjection,
};
use crate::domain::text::compact_whitespace_detail;

const REVIEW_ENTRY_LIMIT: usize = 3;
const REVIEW_ID_DETAIL_LIMIT: usize = 28;
const REVIEW_SUMMARY_DETAIL_LIMIT: usize = 52;
const REVIEW_WORKSPACE_DETAIL_LIMIT: usize = 44;

pub(crate) fn build_reviews_overlay_view(
    screen_model: ReviewsOverlayScreenModel<'_>,
) -> ReviewsOverlayView {
    match screen_model {
        ReviewsOverlayScreenModel::Idle => build_idle_view(),
        ReviewsOverlayScreenModel::Loading(request) => {
            let current_thread_state = if request.context.active_thread.is_some() {
                ReviewSectionState::Pending {
                    label: "loading",
                    message: "Loading current-thread reviews...",
                }
            } else {
                ReviewSectionState::Loaded {
                    total_count: 0,
                    entries: Vec::new(),
                }
            };
            build_view(
                &request.context,
                current_thread_state,
                ReviewSectionState::Pending {
                    label: "loading",
                    message: "Loading pending inbox...",
                },
                ReviewSectionState::Pending {
                    label: "loading",
                    message: "Loading recent review history...",
                },
            )
        }
        ReviewsOverlayScreenModel::Ready { request, authority } => {
            build_ready_view(&request.context, authority)
        }
    }
}

fn build_idle_view() -> ReviewsOverlayView {
    ReviewsOverlayView {
        header_lines: header_lines(),
        summary_lines: vec![
            Line::from("workspace: unavailable"),
            Line::from("thread: draft  |  inbox: not loaded  |  history: not loaded"),
        ],
        current_thread_reviews: pending_entries("Review data has not been requested."),
        inbox_reviews: pending_entries("Review data has not been requested."),
        history_reviews: pending_entries("Review data has not been requested."),
        key_lines: key_lines(),
    }
}

fn build_ready_view(
    context: &ReviewsOverlayContext,
    authority: &ReviewsOverlayAuthoritySnapshot,
) -> ReviewsOverlayView {
    build_view(
        context,
        load_section(&authority.current_thread_reviews, build_thread_review_views),
        load_section(&authority.pending_inbox, build_inbox_review_views),
        load_section(&authority.recent_history, build_history_review_views),
    )
}

fn build_view(
    context: &ReviewsOverlayContext,
    current_thread_state: ReviewSectionState,
    inbox_state: ReviewSectionState,
    history_state: ReviewSectionState,
) -> ReviewsOverlayView {
    let summary_lines = build_summary_lines(context, &inbox_state, &history_state);
    ReviewsOverlayView {
        header_lines: header_lines(),
        summary_lines,
        current_thread_reviews: current_thread_state.into_entries(),
        inbox_reviews: inbox_state.into_entries(),
        history_reviews: history_state.into_entries(),
        key_lines: key_lines(),
    }
}

fn build_summary_lines(
    context: &ReviewsOverlayContext,
    inbox_state: &ReviewSectionState,
    history_state: &ReviewSectionState,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(format!(
        "workspace: {}",
        compact_whitespace_detail(&context.workspace_directory, REVIEW_WORKSPACE_DETAIL_LIMIT)
    ))];
    lines.push(Line::from(format!(
        "thread: {}  |  inbox: {}  |  history: {}",
        context
            .active_thread
            .as_ref()
            .map(|thread| compact_whitespace_detail(&thread.thread_id, REVIEW_ID_DETAIL_LIMIT))
            .unwrap_or_else(|| "draft".to_string()),
        inbox_state.count_label("pending"),
        history_state.count_label("recent")
    )));
    if let Some(active_thread) = context.active_thread.as_ref() {
        if let Some(summary) = active_thread.review_summary.as_deref() {
            lines.push(Line::from(format!(
                "active: {}",
                compact_whitespace_detail(summary, REVIEW_SUMMARY_DETAIL_LIMIT)
            )));
        }
        if let Some(handoff_context) = active_thread.manual_handoff_context.as_deref() {
            lines.push(Line::from(format!(
                "handoff: {}",
                compact_whitespace_detail(handoff_context, REVIEW_SUMMARY_DETAIL_LIMIT)
            )));
        }
    }
    lines
}

fn header_lines() -> Vec<Line<'static>> {
    vec![
        AkraTheme::title_line("Review Center", " / shell inspection"),
        Line::from("Check inbox pressure, active-thread review context, and recent outcomes."),
    ]
}

fn key_lines() -> Vec<Line<'static>> {
    vec![AkraTheme::key_line(
        "Esc/Ctrl+C: close  |  read-only review status",
    )]
}

enum ReviewSectionState {
    Loaded {
        total_count: usize,
        entries: Vec<ReviewOverlayView>,
    },
    Pending {
        label: &'static str,
        message: &'static str,
    },
    Unavailable(String),
}

impl ReviewSectionState {
    fn count_label(&self, noun: &str) -> String {
        match self {
            Self::Loaded { total_count, .. } => format!("{total_count} {noun}"),
            Self::Pending { label, .. } => (*label).to_string(),
            Self::Unavailable(_) => "unavailable".to_string(),
        }
    }

    fn into_entries(self) -> Vec<ReviewOverlayView> {
        match self {
            Self::Loaded { entries, .. } => entries,
            Self::Pending { message, .. } => pending_entries(message),
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

fn pending_entries(message: &str) -> Vec<ReviewOverlayView> {
    vec![ReviewOverlayView {
        summary_line: Line::from(message.to_string()),
        detail_lines: Vec::new(),
    }]
}

fn load_section<T>(
    result: &Result<Vec<T>, String>,
    build_entries: fn(&[T]) -> (usize, Vec<ReviewOverlayView>),
) -> ReviewSectionState {
    match result {
        Ok(items) => {
            let (total_count, entries) = build_entries(items);
            ReviewSectionState::Loaded {
                total_count,
                entries,
            }
        }
        Err(error) => ReviewSectionState::Unavailable(error.clone()),
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
                compact_whitespace_detail(
                    review.review_summary.trim(),
                    REVIEW_SUMMARY_DETAIL_LIMIT
                )
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

fn build_history_review_views(
    history: &[ReviewCenterHistoryEntry],
) -> (usize, Vec<ReviewOverlayView>) {
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
    let handoff_target = handoff_target
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let handoff_note = handoff_note
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match (handoff_target, handoff_note) {
        (Some(target), Some(note)) => Some(format!("{target}: {note}")),
        (Some(target), None) => Some(target.to_string()),
        (None, Some(note)) => Some(note.to_string()),
        (None, None) => None,
    }
}
