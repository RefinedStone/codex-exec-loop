use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::application::port::inbound::planning_projection_port::PlanningApplicationProjection;

use super::super::{
    AkraTheme, AutoFollowSnapshotPresentation, ConversationScreenModel, ConversationViewModel,
    SHELL_TAIL_AUTO_FOLLOW_DETAIL_LIMIT, ShellActionAvailability, ShellOverlay, StartupState,
    TuiLanguage, compact_shell_detail,
};
use super::tail_shared::compact_auto_follow_status_summary;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RibbonTone {
    Normal,
    Ready,
    Warning,
    Danger,
}

impl RibbonTone {
    fn style(self) -> Style {
        match self {
            Self::Normal => AkraTheme::muted(),
            Self::Ready => AkraTheme::success(),
            Self::Warning => AkraTheme::warning(),
            Self::Danger => AkraTheme::danger(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RibbonMetric {
    text: String,
    tone: RibbonTone,
}

impl RibbonMetric {
    fn new(text: String, tone: RibbonTone) -> Self {
        Self { text, tone }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OperatorAttentionProjection {
    availability: ShellActionAvailability,
    warning_details: Vec<String>,
    runtime_notice_details: Vec<String>,
}

impl OperatorAttentionProjection {
    fn from_screen_model(
        screen_model: &ConversationScreenModel<'_>,
        conversation: Option<&ConversationViewModel>,
    ) -> Self {
        let startup_warnings = match screen_model.startup_state {
            StartupState::Ready(ready) => ready.warnings.iter().map(String::as_str).collect(),
            StartupState::Idle | StartupState::Loading | StartupState::Failed(_) => Vec::new(),
        };
        let conversation_warnings = conversation
            .into_iter()
            .flat_map(|conversation| conversation.base_warnings.iter().map(String::as_str));
        let warning_details =
            deduplicated_safe_details(startup_warnings.into_iter().chain(conversation_warnings));
        let runtime_notice_details = deduplicated_safe_details(
            conversation
                .into_iter()
                .flat_map(|conversation| conversation.runtime_notices.iter().map(String::as_str))
                .chain(
                    screen_model
                        .global_runtime_notices
                        .iter()
                        .map(String::as_str),
                ),
        );
        Self {
            availability: screen_model.shell_action_availability,
            warning_details,
            runtime_notice_details,
        }
    }

    fn requires_attention(&self) -> bool {
        self.availability != ShellActionAvailability::Ready
            || !self.warning_details.is_empty()
            || !self.runtime_notice_details.is_empty()
    }

    fn warning_count(&self) -> usize {
        self.warning_details.len()
    }

    fn runtime_notice_count(&self) -> usize {
        self.runtime_notice_details.len()
    }

    fn label(&self, language: TuiLanguage) -> &'static str {
        language.operator_attention_label(self.availability, self.warning_count())
    }

    fn impact(&self, language: TuiLanguage) -> &'static str {
        language.operator_attention_impact(self.availability, self.warning_count())
    }

    fn action(&self, language: TuiLanguage) -> &'static str {
        language.operator_attention_action(self.availability)
    }

    fn tone(&self) -> RibbonTone {
        match self.availability {
            ShellActionAvailability::Blocked => RibbonTone::Danger,
            ShellActionAvailability::Pending => RibbonTone::Warning,
            ShellActionAvailability::Ready if self.warning_count() > 0 => RibbonTone::Warning,
            ShellActionAvailability::Ready => RibbonTone::Normal,
        }
    }

    fn diagnostic_lines(&self) -> Vec<Line<'static>> {
        let mut lines = self
            .warning_details
            .iter()
            .map(|detail| {
                Line::from(vec![
                    Span::styled("operator warning: ", AkraTheme::warning()),
                    Span::raw(detail.clone()),
                ])
            })
            .collect::<Vec<_>>();
        lines.extend(self.runtime_notice_details.iter().map(|detail| {
            Line::from(vec![
                Span::styled("runtime notice: ", AkraTheme::accent()),
                Span::raw(detail.clone()),
            ])
        }));
        lines
    }
}

pub(super) fn build_operator_ribbon_line(
    screen_model: &ConversationScreenModel<'_>,
    conversation: Option<&ConversationViewModel>,
    content_width: u16,
) -> Line<'static> {
    let effective_width = if content_width == 0 {
        160
    } else {
        content_width
    };
    let compact_metrics = effective_width < 64;
    let planning = PlanningApplicationProjection::from_runtime_projection(
        &screen_model.planning_runtime_projection,
    );
    let queue = queue_metric(screen_model.tui_language, &planning, compact_metrics);
    let agents = agent_metric(screen_model, compact_metrics);
    let terminals = terminal_metric(screen_model, conversation, compact_metrics);
    let readiness = screen_model
        .tui_language
        .startup_axis_status(screen_model.shell_action_availability)
        .to_uppercase();
    let readiness_style = match screen_model.shell_action_availability {
        ShellActionAvailability::Ready => AkraTheme::success(),
        ShellActionAvailability::Pending => AkraTheme::warning(),
        ShellActionAvailability::Blocked => AkraTheme::danger(),
    }
    .add_modifier(Modifier::BOLD);
    let workspace_label = compact_workspace_label(&screen_model.workspace_directory);
    let workspace_limit = if effective_width >= 120 { 20 } else { 12 };

    let mut spans = vec![
        Span::styled("Akra", AkraTheme::brand()),
        Span::raw(" / "),
        Span::styled(
            compact_shell_detail(&workspace_label, workspace_limit),
            AkraTheme::accent(),
        ),
        separator(compact_metrics),
        Span::styled(readiness, readiness_style),
    ];
    append_metric(&mut spans, queue, compact_metrics);
    append_metric(&mut spans, agents, compact_metrics);
    append_metric(&mut spans, terminals, compact_metrics);

    if Line::from(spans.clone()).width() > usize::from(effective_width) {
        spans = vec![
            Span::styled("Akra", AkraTheme::brand()),
            separator(true),
            Span::styled(
                screen_model
                    .tui_language
                    .startup_axis_status(screen_model.shell_action_availability)
                    .to_uppercase(),
                readiness_style,
            ),
        ];
        append_metric(
            &mut spans,
            queue_metric(screen_model.tui_language, &planning, true),
            true,
        );
        append_metric(&mut spans, agent_metric(screen_model, true), true);
        append_metric(
            &mut spans,
            terminal_metric(screen_model, conversation, true),
            true,
        );
    }

    if effective_width >= 100 {
        append_optional_detail(
            &mut spans,
            compact_turn_options_hud_label(&screen_model.turn_options_hud_label),
            AkraTheme::muted(),
            effective_width,
        );
    }
    if effective_width >= 140
        && let Some(context_pressure_basis_points) = screen_model.context_pressure_basis_points
    {
        append_optional_detail(
            &mut spans,
            format!("ctx {}%", context_pressure_basis_points / 100),
            AkraTheme::muted(),
            effective_width,
        );
    }
    if effective_width >= 140 && conversation.is_some_and(should_show_auto_follow_status) {
        let conversation = conversation.expect("checked conversation must exist");
        append_optional_detail(
            &mut spans,
            format!(
                "auto: {} · done: {}",
                compact_auto_follow_status_summary(
                    conversation,
                    SHELL_TAIL_AUTO_FOLLOW_DETAIL_LIMIT,
                ),
                conversation.auto_follow_state().progress_label(),
            ),
            AkraTheme::muted(),
            effective_width,
        );
    }

    Line::from(spans)
}

pub(super) fn build_operator_attention_line(
    screen_model: &ConversationScreenModel<'_>,
    conversation: Option<&ConversationViewModel>,
    content_width: u16,
) -> Option<Line<'static>> {
    let attention = OperatorAttentionProjection::from_screen_model(screen_model, conversation);
    if !attention.requires_attention() {
        return None;
    }
    let effective_width = if content_width == 0 {
        160
    } else {
        content_width
    };
    let tone = attention.tone();
    let marker = if attention.availability == ShellActionAvailability::Pending {
        "…"
    } else {
        "!"
    };
    let mut count_segments = Vec::new();
    if attention.warning_count() > 0 {
        count_segments.push(format!(
            "{} {}",
            screen_model
                .tui_language
                .operator_warning_label(attention.warning_count()),
            attention.warning_count()
        ));
    }
    if attention.runtime_notice_count() > 0 {
        count_segments.push(format!(
            "{} {}",
            screen_model
                .tui_language
                .operator_runtime_notice_label(attention.runtime_notice_count()),
            attention.runtime_notice_count()
        ));
    }
    let mut segments = vec![attention.impact(screen_model.tui_language).to_string()];
    segments.extend(count_segments.iter().cloned());
    let action = (screen_model.shell_overlay == ShellOverlay::Hidden
        && !screen_model.dialog_visible())
    .then(|| attention.action(screen_model.tui_language));

    let mut spans = build_attention_spans(
        marker,
        attention.label(screen_model.tui_language),
        tone,
        Some(segments.join("  ·  ")),
        action,
    );
    if Line::from(spans.clone()).width() > usize::from(effective_width) {
        spans = build_attention_spans(
            marker,
            attention.label(screen_model.tui_language),
            tone,
            Some(count_segments.join("  ·  ")),
            action,
        );
    }
    if Line::from(spans.clone()).width() > usize::from(effective_width) {
        let mut compact = Vec::new();
        if attention.warning_count() > 0 {
            compact.push(format!("w{}", attention.warning_count()));
        }
        if attention.runtime_notice_count() > 0 {
            compact.push(format!("n{}", attention.runtime_notice_count()));
        }
        spans = build_attention_spans(
            marker,
            attention.label(screen_model.tui_language),
            tone,
            (!compact.is_empty()).then(|| compact.join(" ")),
            action,
        );
    }
    Some(Line::from(spans))
}

fn build_attention_spans(
    marker: &'static str,
    label: &'static str,
    tone: RibbonTone,
    detail: Option<String>,
    action: Option<&'static str>,
) -> Vec<Span<'static>> {
    let mut spans = vec![Span::styled(
        format!("{marker} {label}"),
        tone.style().add_modifier(Modifier::BOLD),
    )];
    if let Some(detail) = detail.filter(|detail| !detail.is_empty()) {
        spans.extend([
            Span::styled("  ·  ", AkraTheme::subtle()),
            Span::styled(detail, tone.style()),
        ]);
    }
    if let Some(action) = action {
        spans.extend([
            Span::styled("  ·  ", AkraTheme::subtle()),
            Span::styled(action, AkraTheme::shortcut()),
        ]);
    }
    spans
}

pub(in crate::adapter::inbound::tui::app) fn build_operator_diagnostic_lines(
    screen_model: &ConversationScreenModel<'_>,
) -> Vec<Line<'static>> {
    OperatorAttentionProjection::from_screen_model(screen_model, screen_model.ready_conversation())
        .diagnostic_lines()
}

pub(super) fn should_show_auto_follow_status(conversation: &ConversationViewModel) -> bool {
    !conversation.has_post_turn_settlement_in_flight()
        && (conversation.auto_follow_state().has_live_activity()
            || conversation.auto_follow_state().continuation_paused
            || conversation.auto_follow_state().completed_auto_turns > 0)
}

fn queue_metric(
    language: TuiLanguage,
    projection: &PlanningApplicationProjection,
    compact: bool,
) -> RibbonMetric {
    let label = metric_label(language, "queue", compact);
    if projection.has_structured_queue_projection {
        return RibbonMetric::new(
            format!("{label} {}", projection.visible_tasks.len()),
            if projection.visible_tasks.is_empty() {
                RibbonTone::Normal
            } else {
                RibbonTone::Ready
            },
        );
    }
    if projection.queue_head.is_some() {
        return RibbonMetric::new(format!("{label} 1"), RibbonTone::Ready);
    }
    let (state, tone) = if !projection.workspace_present {
        (localized_state(language, "off"), RibbonTone::Normal)
    } else {
        match projection.status_label.as_str() {
            "blocked" => (localized_state(language, "blocked"), RibbonTone::Danger),
            "pending" | "loading" => (localized_state(language, "pending"), RibbonTone::Warning),
            status => (localized_state(language, status), RibbonTone::Normal),
        }
    };
    RibbonMetric::new(format!("{label} {state}"), tone)
}

fn agent_metric(screen_model: &ConversationScreenModel<'_>, compact: bool) -> RibbonMetric {
    let label = metric_label(screen_model.tui_language, "agents", compact);
    let configured = screen_model.parallel_mode_supervisor.pool.configured_size;
    if configured > 0 {
        return RibbonMetric::new(
            format!(
                "{label} {}/{}",
                screen_model.parallel_mode_supervisor.roster.active_count(),
                configured
            ),
            if screen_model.parallel_mode_supervisor.pool.blocked_slots > 0
                || screen_model.parallel_mode_supervisor.pool.missing_slots > 0
                || screen_model.parallel_mode_supervisor.pool.unavailable_slots > 0
            {
                RibbonTone::Danger
            } else if screen_model.parallel_mode_supervisor.roster.active_count() > 0 {
                RibbonTone::Ready
            } else {
                RibbonTone::Normal
            },
        );
    }
    RibbonMetric::new(
        format!(
            "{label} {}",
            localized_state(
                screen_model.tui_language,
                if screen_model.parallel_mode_enabled {
                    "pending"
                } else {
                    "off"
                },
            )
        ),
        if screen_model.parallel_mode_enabled {
            RibbonTone::Warning
        } else {
            RibbonTone::Normal
        },
    )
}

fn terminal_metric(
    screen_model: &ConversationScreenModel<'_>,
    conversation: Option<&ConversationViewModel>,
    compact: bool,
) -> RibbonMetric {
    let label = metric_label(screen_model.tui_language, "terminals", compact);
    let Some(conversation) = conversation else {
        return RibbonMetric::new(format!("{label} --"), RibbonTone::Normal);
    };
    let active_count = conversation.progressive_activity.active_terminal_count();
    RibbonMetric::new(
        format!("{label} {active_count}"),
        if active_count > 0 {
            RibbonTone::Ready
        } else {
            RibbonTone::Normal
        },
    )
}

fn append_metric(spans: &mut Vec<Span<'static>>, metric: RibbonMetric, compact: bool) {
    spans.push(separator(compact));
    spans.push(Span::styled(metric.text, metric.tone.style()));
}

fn append_optional_detail(
    spans: &mut Vec<Span<'static>>,
    detail: String,
    style: Style,
    content_width: u16,
) {
    let mut candidate = spans.clone();
    candidate.push(separator(false));
    candidate.push(Span::styled(detail, style));
    if Line::from(candidate.clone()).width() <= usize::from(content_width) {
        *spans = candidate;
    }
}

fn separator(compact: bool) -> Span<'static> {
    Span::styled(if compact { " · " } else { "  ·  " }, AkraTheme::subtle())
}

fn metric_label(language: TuiLanguage, metric: &str, compact: bool) -> &'static str {
    match metric {
        "queue" => language.operator_queue_metric_label(compact),
        "agents" => language.operator_agent_metric_label(compact),
        "terminals" => language.operator_terminal_metric_label(compact),
        _ => "?",
    }
}

fn localized_state(language: TuiLanguage, state: &str) -> String {
    language.operator_state_label(state)
}

fn compact_workspace_label(workspace_directory: &str) -> String {
    workspace_directory
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .find(|segment| !segment.is_empty())
        .unwrap_or(workspace_directory)
        .to_string()
}

fn compact_turn_options_hud_label(summary: &str) -> String {
    summary
        .strip_prefix("model: ")
        .unwrap_or(summary)
        .replace("  |  think: ", "/")
}

fn deduplicated_safe_details<'a>(details: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut unique = Vec::new();
    for detail in details {
        let detail = safe_diagnostic_detail(detail);
        if !detail.is_empty() && !unique.contains(&detail) {
            unique.push(detail);
        }
    }
    unique
}

fn safe_diagnostic_detail(detail: &str) -> String {
    let mut safe = String::new();
    for character in detail.chars() {
        if character.is_whitespace() {
            if !safe.ends_with(' ') && !safe.is_empty() {
                safe.push(' ');
            }
        } else if character.is_control() {
            safe.extend(character.escape_default());
        } else {
            safe.push(character);
        }
    }
    safe.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_details_deduplicate_equal_causes_and_escape_controls() {
        let details = deduplicated_safe_details([
            "app-server sent notification remoteControl",
            "app-server  sent\nnotification remoteControl",
            "unsafe\u{1b}[31m payload",
        ]);

        assert_eq!(details.len(), 2);
        assert_eq!(details[0], "app-server sent notification remoteControl");
        assert_eq!(details[1], "unsafe\\u{1b}[31m payload");
    }
}
