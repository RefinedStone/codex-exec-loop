use std::fmt;

use crate::domain::conversation_item_lifecycle::{
    ConversationCommandAction, ConversationCommandActionProjection, ConversationItemKind,
    ConversationItemLifecycleConsistency, ConversationItemLifecyclePhase,
    ConversationItemLifecycleProjectionSnapshot, ConversationItemOutcome,
};
use crate::domain::conversation_progressive_activity::{
    ConversationProgressiveActivityKind, ConversationProgressiveActivityPayload,
    ConversationProgressiveActivityProjectionSnapshot, ConversationProgressiveFileChangeKind,
    ConversationProgressivePlanStepStatus,
};

const MAX_EXPANDED_CARD_KEYS: usize = 32;
const MAX_CARD_TITLE_CHARS: usize = 96;
const MAX_CARD_FACT_CHARS: usize = 32;
const MAX_SYNTHESIZED_DETAIL_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ProgressiveActivityCardKind {
    Command,
    Patch,
    Mcp,
    Plan,
    Reason,
    Agent,
    Terminal,
    Diff,
    Token,
    Moderation,
    Guardian,
    Unknown,
}

impl ProgressiveActivityCardKind {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Patch => "patch",
            Self::Mcp => "mcp",
            Self::Plan => "plan",
            Self::Reason => "reason",
            Self::Agent => "agent",
            Self::Terminal => "terminal",
            Self::Diff => "diff",
            Self::Token => "token",
            Self::Moderation => "moderation",
            Self::Guardian => "guardian",
            Self::Unknown => "unknown",
        }
    }

    pub(crate) const fn from_activity_kind(kind: &ConversationProgressiveActivityKind) -> Self {
        match kind {
            ConversationProgressiveActivityKind::CommandOutput => Self::Command,
            ConversationProgressiveActivityKind::FileChangePatch => Self::Patch,
            ConversationProgressiveActivityKind::McpProgress => Self::Mcp,
            ConversationProgressiveActivityKind::PlanDelta
            | ConversationProgressiveActivityKind::TurnPlan => Self::Plan,
            ConversationProgressiveActivityKind::ReasoningSummaryTextDelta
            | ConversationProgressiveActivityKind::ReasoningSummaryPartAdded
            | ConversationProgressiveActivityKind::ReasoningTextDelta => Self::Reason,
            ConversationProgressiveActivityKind::AgentMessageDelta => Self::Agent,
            ConversationProgressiveActivityKind::TerminalInteraction => Self::Terminal,
            ConversationProgressiveActivityKind::TurnDiff => Self::Diff,
            ConversationProgressiveActivityKind::TokenUsage => Self::Token,
            ConversationProgressiveActivityKind::Moderation => Self::Moderation,
            ConversationProgressiveActivityKind::GuardianWarning => Self::Guardian,
            ConversationProgressiveActivityKind::Unknown(_) => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ProgressiveActivityCardSource {
    Progressive,
    Lifecycle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ProgressiveActivityCardKey {
    pub(crate) source: ProgressiveActivityCardSource,
    pub(crate) sequence: u64,
    pub(crate) kind: ProgressiveActivityCardKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProgressiveActivityCard {
    pub(crate) key: ProgressiveActivityCardKey,
    pub(crate) item_id: Option<String>,
    pub(crate) activity_label: &'static str,
    pub(crate) title: String,
    pub(crate) summary: String,
    pub(crate) fact: String,
    pub(crate) outcome: ProgressiveActivityCardOutcome,
    pub(crate) elapsed_ms: Option<u64>,
    pub(crate) expandable: bool,
    pub(crate) record_index: Option<usize>,
    pub(crate) lifecycle_sequence: Option<u64>,
    pub(crate) source_bytes: u64,
    pub(crate) retained_bytes: u64,
    pub(crate) truncated_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProgressiveActivityCardOutcome {
    Observed,
    Active,
    Completed,
    Failed,
    Declined,
    Interrupted,
    Unknown,
}

impl ProgressiveActivityCardOutcome {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::Active => "active",
            Self::Completed => "complete",
            Self::Failed => "failed",
            Self::Declined => "declined",
            Self::Interrupted => "stopped",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProgressiveActivityWaitKind {
    Approval,
    ModelResponse,
    Subagent,
    TaskOutput,
    Retrying,
}

impl ProgressiveActivityWaitKind {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Approval => "approval",
            Self::ModelResponse => "model response",
            Self::Subagent => "subagent",
            Self::TaskOutput => "task output",
            Self::Retrying => "retry",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProgressiveActivityWaitStatus {
    pub(crate) kind: ProgressiveActivityWaitKind,
    pub(crate) summary: Option<String>,
}

impl ProgressiveActivityCard {
    #[cfg(test)]
    pub(crate) fn header_line(&self, expanded: bool) -> String {
        let indicator = if !self.expandable {
            "  "
        } else if expanded {
            "▼ "
        } else {
            "› "
        };
        let fact = if self.fact.is_empty() {
            String::new()
        } else {
            format!("  {}", self.fact)
        };
        format!(
            "{indicator}◆ {:<9} {}{fact}",
            self.activity_label, self.title
        )
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct ProgressiveActivityExpandState {
    expanded: Vec<ProgressiveActivityCardKey>,
    expanded_tool_digests: Vec<[u8; 32]>,
}

impl ProgressiveActivityExpandState {
    pub(crate) fn is_card_expanded(&self, key: ProgressiveActivityCardKey) -> bool {
        self.expanded.contains(&key)
    }

    pub(crate) fn expand_card(&mut self, key: ProgressiveActivityCardKey) {
        if self.is_card_expanded(key) {
            return;
        }
        if self.expanded.len() >= MAX_EXPANDED_CARD_KEYS {
            self.expanded.remove(0);
        }
        self.expanded.push(key);
    }

    pub(crate) fn is_tool_expanded(&self, digest: [u8; 32]) -> bool {
        self.expanded_tool_digests.contains(&digest)
    }

    pub(crate) fn expand_tool(&mut self, digest: [u8; 32]) {
        if self.is_tool_expanded(digest) {
            return;
        }
        if self.expanded_tool_digests.len() >= MAX_EXPANDED_CARD_KEYS {
            self.expanded_tool_digests.remove(0);
        }
        self.expanded_tool_digests.push(digest);
    }

    pub(crate) fn toggle_card(&mut self, key: ProgressiveActivityCardKey) -> bool {
        if let Some(index) = self.expanded.iter().position(|entry| *entry == key) {
            self.expanded.remove(index);
            return false;
        }
        self.expand_card(key);
        true
    }

    pub(crate) fn toggle_tool(&mut self, digest: [u8; 32]) -> bool {
        if let Some(index) = self
            .expanded_tool_digests
            .iter()
            .position(|entry| *entry == digest)
        {
            self.expanded_tool_digests.remove(index);
            return false;
        }
        self.expand_tool(digest);
        true
    }
}

impl fmt::Debug for ProgressiveActivityExpandState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProgressiveActivityExpandState")
            .field("expanded_cards", &self.expanded.len())
            .field("expanded_tools", &self.expanded_tool_digests.len())
            .finish()
    }
}

pub(crate) fn project_activity_cards(
    snapshot: &ConversationProgressiveActivityProjectionSnapshot,
) -> Vec<ProgressiveActivityCard> {
    project_activity_timeline_cards(snapshot, None, None)
}

pub(crate) fn project_activity_timeline_cards(
    snapshot: &ConversationProgressiveActivityProjectionSnapshot,
    lifecycle: Option<&ConversationItemLifecycleProjectionSnapshot>,
    rendered_at_ms: Option<i64>,
) -> Vec<ProgressiveActivityCard> {
    let mut cards = Vec::with_capacity(snapshot.records.len());
    for (record_index, record) in snapshot.records.iter().enumerate() {
        let observation = record.observation();
        let kind = ProgressiveActivityCardKind::from_activity_kind(&observation.kind);
        let (payload_title, fact, payload_expandable, payload_retained_bytes) =
            project_payload_summary(&observation.payload);
        let card_lifecycle = observation.item_id.as_deref().and_then(|item_id| {
            lifecycle
                .and_then(|lifecycle| project_card_lifecycle(lifecycle, item_id, rendered_at_ms))
        });
        let summary = card_lifecycle
            .as_ref()
            .map(|lifecycle| lifecycle.summary.as_str())
            .filter(|summary| !summary.is_empty())
            .unwrap_or(&payload_title)
            .to_string();
        let command_actions = card_lifecycle
            .as_ref()
            .and_then(|lifecycle| lifecycle.command_actions.as_ref());
        let title = command_actions
            .map(|actions| actions.title.clone())
            .unwrap_or(payload_title);
        let activity_label = command_actions.map_or(kind.label(), |actions| actions.activity_label);
        let action_source_bytes = command_actions.map_or(0, |actions| actions.source_bytes);
        let action_retained_bytes = command_actions.map_or(0, |actions| actions.retained_bytes);
        let action_truncated_bytes = command_actions.map_or(0, |actions| actions.truncated_bytes);
        cards.push(ProgressiveActivityCard {
            key: ProgressiveActivityCardKey {
                source: ProgressiveActivityCardSource::Progressive,
                sequence: record.last_sequence(),
                kind,
            },
            item_id: observation.item_id.clone(),
            activity_label,
            title,
            summary,
            fact,
            outcome: card_lifecycle
                .as_ref()
                .map_or(ProgressiveActivityCardOutcome::Observed, |lifecycle| {
                    lifecycle.outcome
                }),
            elapsed_ms: card_lifecycle
                .as_ref()
                .and_then(|lifecycle| lifecycle.elapsed_ms),
            expandable: payload_expandable || command_actions.is_some(),
            record_index: Some(record_index),
            lifecycle_sequence: command_actions.map(|actions| actions.sequence),
            source_bytes: observation
                .source_bytes()
                .saturating_add(action_source_bytes),
            retained_bytes: payload_retained_bytes.saturating_add(action_retained_bytes),
            truncated_bytes: observation
                .truncated_bytes()
                .saturating_add(action_truncated_bytes),
        });
    }
    if let Some(lifecycle) = lifecycle {
        append_lifecycle_only_action_cards(&mut cards, snapshot, lifecycle, rendered_at_ms);
    }
    cards
}

fn append_lifecycle_only_action_cards(
    cards: &mut Vec<ProgressiveActivityCard>,
    progressive: &ConversationProgressiveActivityProjectionSnapshot,
    lifecycle: &ConversationItemLifecycleProjectionSnapshot,
    rendered_at_ms: Option<i64>,
) {
    let represented_item_ids = progressive
        .records
        .iter()
        .filter_map(|record| record.observation().item_id.as_deref())
        .collect::<Vec<_>>();
    let mut seen_item_ids = Vec::new();
    let mut lifecycle_only = Vec::new();

    for record in lifecycle.records.iter().rev() {
        let item_id = record.observation.item_id.as_str();
        if seen_item_ids.contains(&item_id) {
            continue;
        }
        seen_item_ids.push(item_id);
        if represented_item_ids.contains(&item_id)
            || !matches!(
                record.observation.kind,
                ConversationItemKind::CommandExecution
            )
        {
            continue;
        }
        let Some(projected) = project_card_lifecycle(lifecycle, item_id, rendered_at_ms) else {
            continue;
        };
        let Some(actions) = projected.command_actions.as_ref() else {
            continue;
        };
        lifecycle_only.push(ProgressiveActivityCard {
            key: ProgressiveActivityCardKey {
                source: ProgressiveActivityCardSource::Lifecycle,
                sequence: actions.sequence,
                kind: ProgressiveActivityCardKind::Command,
            },
            item_id: Some(item_id.to_string()),
            activity_label: actions.activity_label,
            title: actions.title.clone(),
            summary: projected.summary.clone(),
            fact: actions.fact.clone(),
            outcome: projected.outcome,
            elapsed_ms: projected.elapsed_ms,
            expandable: true,
            record_index: None,
            lifecycle_sequence: Some(actions.sequence),
            source_bytes: actions.source_bytes,
            retained_bytes: actions.retained_bytes,
            truncated_bytes: actions.truncated_bytes,
        });
    }
    lifecycle_only.reverse();
    cards.extend(lifecycle_only);
}

pub(crate) fn project_activity_wait_status(
    lifecycle: Option<&ConversationItemLifecycleProjectionSnapshot>,
    retry_summary: Option<&str>,
    approval_pending: bool,
) -> Option<ProgressiveActivityWaitStatus> {
    if approval_pending {
        return Some(ProgressiveActivityWaitStatus {
            kind: ProgressiveActivityWaitKind::Approval,
            summary: None,
        });
    }
    if let Some(summary) = retry_summary {
        return Some(ProgressiveActivityWaitStatus {
            kind: ProgressiveActivityWaitKind::Retrying,
            summary: non_empty_summary(summary),
        });
    }
    let lifecycle = lifecycle?;
    let mut seen_item_ids = Vec::new();
    lifecycle.records.iter().rev().find_map(|record| {
        let observation = &record.observation;
        if !lifecycle_record_is_authoritative(record.consistency)
            || seen_item_ids
                .iter()
                .any(|item_id: &&str| *item_id == observation.item_id)
        {
            return None;
        }
        seen_item_ids.push(observation.item_id.as_str());
        if !lifecycle_observation_is_active(observation.phase, &observation.outcome) {
            return None;
        }
        Some(ProgressiveActivityWaitStatus {
            kind: wait_kind_for_item(&observation.kind)?,
            summary: non_empty_summary(&observation.summary),
        })
    })
}

struct ProjectedCardLifecycle {
    outcome: ProgressiveActivityCardOutcome,
    summary: String,
    elapsed_ms: Option<u64>,
    command_actions: Option<ProjectedCommandActions>,
}

struct ProjectedCommandActions {
    sequence: u64,
    activity_label: &'static str,
    title: String,
    fact: String,
    source_bytes: u64,
    retained_bytes: u64,
    truncated_bytes: u64,
}

fn project_card_lifecycle(
    snapshot: &ConversationItemLifecycleProjectionSnapshot,
    item_id: &str,
    rendered_at_ms: Option<i64>,
) -> Option<ProjectedCardLifecycle> {
    let mut outcome = ProgressiveActivityCardOutcome::Observed;
    let mut summary = String::new();
    let mut action_summary = None;
    let mut command_actions = None;
    let mut started_at_ms = None;
    let mut completed_at_ms = None;
    let mut matched = false;

    for record in snapshot
        .records
        .iter()
        .filter(|record| record.observation.item_id == item_id)
    {
        if !lifecycle_record_is_authoritative(record.consistency) {
            continue;
        }
        let observation = &record.observation;
        matched = true;
        if !observation.summary.trim().is_empty() {
            summary = observation.summary.clone();
        }
        if !observation.command_actions.is_empty() {
            action_summary = non_empty_summary(&observation.summary);
            command_actions = Some(project_command_actions(
                record.sequence,
                &observation.command_actions,
            ));
        }
        match observation.phase {
            ConversationItemLifecyclePhase::Started => {
                outcome = ProgressiveActivityCardOutcome::Active;
                started_at_ms = observation.observed_at_ms;
                completed_at_ms = None;
            }
            ConversationItemLifecyclePhase::Completed => {
                outcome = completed_outcome(&observation.outcome);
                completed_at_ms = (record.consistency
                    != ConversationItemLifecycleConsistency::TimestampRegression)
                    .then_some(observation.observed_at_ms)
                    .flatten();
            }
            ConversationItemLifecyclePhase::SnapshotObserved => {
                outcome = snapshot_outcome(&observation.outcome);
                started_at_ms = None;
                completed_at_ms = None;
            }
        }
    }

    matched.then(|| ProjectedCardLifecycle {
        outcome,
        summary: action_summary.unwrap_or(summary),
        elapsed_ms: projected_elapsed_ms(outcome, started_at_ms, completed_at_ms, rendered_at_ms),
        command_actions,
    })
}

fn project_command_actions(
    sequence: u64,
    projection: &ConversationCommandActionProjection,
) -> ProjectedCommandActions {
    ProjectedCommandActions {
        sequence,
        activity_label: command_action_activity_label(projection),
        title: command_action_title(projection),
        fact: command_action_fact(projection),
        source_bytes: projection.source_bytes,
        retained_bytes: u64::try_from(projection.retained_dynamic_bytes()).unwrap_or(u64::MAX),
        truncated_bytes: projection.truncated_bytes,
    }
}

fn command_action_activity_label(projection: &ConversationCommandActionProjection) -> &'static str {
    let count = projection.action_count();
    if projection.read_action_count == count {
        "read"
    } else if projection.list_files_action_count == count {
        "list"
    } else if projection.search_action_count == count {
        "search"
    } else {
        "explore"
    }
}

fn command_action_title(projection: &ConversationCommandActionProjection) -> String {
    if projection.action_count() == 1 {
        return projection
            .actions
            .first()
            .map(command_action_target)
            .unwrap_or_else(|| "workspace".to_string());
    }
    format!("{} targets", projection.action_count())
}

fn command_action_fact(projection: &ConversationCommandActionProjection) -> String {
    let count = projection.action_count();
    if projection.omitted_action_count > 0 {
        format!(
            "{}+{} actions",
            projection.actions.len(),
            projection.omitted_action_count
        )
    } else if count == 1 {
        "1 action".to_string()
    } else {
        format!("{count} actions")
    }
}

fn command_action_target(action: &ConversationCommandAction) -> String {
    match action {
        ConversationCommandAction::Read { name, path } => non_empty_text(name)
            .unwrap_or_else(|| non_empty_text(path).unwrap_or("file"))
            .to_string(),
        ConversationCommandAction::ListFiles { path } => path
            .as_deref()
            .and_then(non_empty_text)
            .unwrap_or("workspace")
            .to_string(),
        ConversationCommandAction::Search { query, path } => {
            let query = query.as_deref().and_then(non_empty_text);
            let path = path.as_deref().and_then(non_empty_text);
            match (query, path) {
                (Some(query), Some(path)) => format!("\"{query}\" in {path}"),
                (Some(query), None) => format!("\"{query}\""),
                (None, Some(path)) => format!("in {path}"),
                (None, None) => "workspace".to_string(),
            }
        }
    }
}

fn non_empty_text(value: &str) -> Option<&str> {
    (!value.trim().is_empty()).then_some(value)
}

fn lifecycle_record_is_authoritative(consistency: ConversationItemLifecycleConsistency) -> bool {
    matches!(
        consistency,
        ConversationItemLifecycleConsistency::Accepted
            | ConversationItemLifecycleConsistency::SnapshotObserved
            | ConversationItemLifecycleConsistency::CompletionWithoutStart
            | ConversationItemLifecycleConsistency::TimestampRegression
    )
}

fn lifecycle_observation_is_active(
    phase: ConversationItemLifecyclePhase,
    outcome: &ConversationItemOutcome,
) -> bool {
    phase == ConversationItemLifecyclePhase::Started
        || (phase == ConversationItemLifecyclePhase::SnapshotObserved
            && matches!(outcome, ConversationItemOutcome::InProgress))
}

fn completed_outcome(outcome: &ConversationItemOutcome) -> ProgressiveActivityCardOutcome {
    match outcome {
        ConversationItemOutcome::Failed => ProgressiveActivityCardOutcome::Failed,
        ConversationItemOutcome::Declined => ProgressiveActivityCardOutcome::Declined,
        ConversationItemOutcome::Interrupted => ProgressiveActivityCardOutcome::Interrupted,
        ConversationItemOutcome::Unknown(_) => ProgressiveActivityCardOutcome::Unknown,
        ConversationItemOutcome::NotReported
        | ConversationItemOutcome::InProgress
        | ConversationItemOutcome::Completed => ProgressiveActivityCardOutcome::Completed,
    }
}

fn snapshot_outcome(outcome: &ConversationItemOutcome) -> ProgressiveActivityCardOutcome {
    match outcome {
        ConversationItemOutcome::NotReported => ProgressiveActivityCardOutcome::Observed,
        ConversationItemOutcome::InProgress => ProgressiveActivityCardOutcome::Active,
        ConversationItemOutcome::Completed => ProgressiveActivityCardOutcome::Completed,
        ConversationItemOutcome::Failed => ProgressiveActivityCardOutcome::Failed,
        ConversationItemOutcome::Declined => ProgressiveActivityCardOutcome::Declined,
        ConversationItemOutcome::Interrupted => ProgressiveActivityCardOutcome::Interrupted,
        ConversationItemOutcome::Unknown(_) => ProgressiveActivityCardOutcome::Unknown,
    }
}

fn projected_elapsed_ms(
    outcome: ProgressiveActivityCardOutcome,
    started_at_ms: Option<i64>,
    completed_at_ms: Option<i64>,
    rendered_at_ms: Option<i64>,
) -> Option<u64> {
    let started_at_ms = started_at_ms?;
    let ended_at_ms = match outcome {
        ProgressiveActivityCardOutcome::Active => rendered_at_ms?,
        ProgressiveActivityCardOutcome::Observed
        | ProgressiveActivityCardOutcome::Completed
        | ProgressiveActivityCardOutcome::Failed
        | ProgressiveActivityCardOutcome::Declined
        | ProgressiveActivityCardOutcome::Interrupted
        | ProgressiveActivityCardOutcome::Unknown => completed_at_ms?,
    };
    u64::try_from(ended_at_ms.checked_sub(started_at_ms)?).ok()
}

fn wait_kind_for_item(kind: &ConversationItemKind) -> Option<ProgressiveActivityWaitKind> {
    match kind {
        ConversationItemKind::CollaborationAgentToolCall
        | ConversationItemKind::SubAgentActivity => Some(ProgressiveActivityWaitKind::Subagent),
        ConversationItemKind::CommandExecution
        | ConversationItemKind::FileChange
        | ConversationItemKind::McpToolCall
        | ConversationItemKind::DynamicToolCall
        | ConversationItemKind::WebSearch
        | ConversationItemKind::ImageView
        | ConversationItemKind::Sleep
        | ConversationItemKind::ImageGeneration => Some(ProgressiveActivityWaitKind::TaskOutput),
        ConversationItemKind::UserMessage
        | ConversationItemKind::HookPrompt
        | ConversationItemKind::AgentMessage
        | ConversationItemKind::Plan
        | ConversationItemKind::Reasoning
        | ConversationItemKind::EnteredReviewMode
        | ConversationItemKind::ExitedReviewMode
        | ConversationItemKind::ContextCompaction => {
            Some(ProgressiveActivityWaitKind::ModelResponse)
        }
        ConversationItemKind::Unknown(_) => None,
    }
}

fn non_empty_summary(summary: &str) -> Option<String> {
    let summary = summary.trim();
    (!summary.is_empty()).then(|| summary.to_string())
}

pub(crate) fn card_detail_text(
    snapshot: &ConversationProgressiveActivityProjectionSnapshot,
    record_index: usize,
) -> Option<String> {
    let record = snapshot.records.get(record_index)?;
    Some(synthesize_payload_detail(&record.observation().payload))
}

pub(crate) fn command_action_detail_text(
    snapshot: &ConversationItemLifecycleProjectionSnapshot,
    sequence: u64,
) -> Option<String> {
    let record = snapshot
        .records
        .iter()
        .find(|record| record.sequence == sequence)?;
    let projection = &record.observation.command_actions;
    if projection.is_empty() {
        return None;
    }

    let mut lines = vec![format!(
        "{} details",
        command_action_activity_label(projection)
    )];
    for (index, action) in projection.actions.iter().enumerate() {
        match action {
            ConversationCommandAction::Read { name, path } => {
                let target = non_empty_text(name)
                    .or_else(|| non_empty_text(path))
                    .unwrap_or("file");
                lines.push(format!("{}. Read {target}", index + 1));
                if path != target {
                    lines.push(format!("   path: {path}"));
                }
            }
            ConversationCommandAction::ListFiles { path } => {
                lines.push(format!("{}. List files", index + 1));
                lines.push(format!(
                    "   path: {}",
                    path.as_deref()
                        .and_then(non_empty_text)
                        .unwrap_or("workspace")
                ));
            }
            ConversationCommandAction::Search { query, path } => {
                lines.push(format!(
                    "{}. Search {}",
                    index + 1,
                    query
                        .as_deref()
                        .and_then(non_empty_text)
                        .map(|query| format!("\"{query}\""))
                        .unwrap_or_else(|| "workspace".to_string())
                ));
                if let Some(path) = path.as_deref().and_then(non_empty_text) {
                    lines.push(format!("   path: {path}"));
                }
            }
        }
    }
    if projection.omitted_action_count > 0 {
        lines.push(format!(
            "… {} more actions were not retained",
            projection.omitted_action_count
        ));
    }
    Some(bound_synthesized_detail(lines.join("\n")))
}

pub(crate) fn tool_message_digest(item_id: Option<&str>, text: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"akra-tool-card-v1\0");
    if let Some(item_id) = item_id {
        hasher.update(b"item\0");
        hasher.update(item_id.as_bytes());
    } else {
        hasher.update(b"text\0");
    }
    hasher.update(b"\0");
    hasher.update(text.as_bytes());
    hasher.finalize().into()
}

pub(crate) fn tool_message_is_expandable(text: &str) -> bool {
    text.lines().count() > 1 || text.chars().count() > MAX_CARD_TITLE_CHARS
}

pub(crate) fn tool_message_title(text: &str) -> String {
    let first = text.lines().next().unwrap_or("").trim();
    truncate_chars(first, MAX_CARD_TITLE_CHARS)
}

pub(crate) fn tool_message_fact(text: &str) -> String {
    let lines = text.lines().count();
    if lines <= 1 {
        return String::new();
    }
    format!("{lines} lines")
}

fn project_payload_summary(
    payload: &ConversationProgressiveActivityPayload,
) -> (String, String, bool, u64) {
    match payload {
        ConversationProgressiveActivityPayload::CommandOutput {
            tail,
            truncated_bytes,
            ..
        } => {
            let title = first_non_empty_line(tail).unwrap_or("command output");
            let lines = tail.lines().count().max(1);
            let fact = if *truncated_bytes > 0 {
                format!("{lines} lines · trunc")
            } else {
                format!("{lines} lines")
            };
            (
                truncate_chars(title, MAX_CARD_TITLE_CHARS),
                truncate_chars(&fact, MAX_CARD_FACT_CHARS),
                true,
                u64::try_from(tail.len()).unwrap_or(u64::MAX),
            )
        }
        ConversationProgressiveActivityPayload::FileChangePatch {
            changes,
            omitted_change_count,
            truncated_bytes,
            ..
        } => {
            let title = changes
                .first()
                .map(|change| change.path.as_str())
                .unwrap_or("file changes");
            let total = changes.len().saturating_add(*omitted_change_count as usize);
            let fact = if *truncated_bytes > 0 {
                format!("{total} files · trunc")
            } else {
                format!("{total} files")
            };
            (
                truncate_chars(title, MAX_CARD_TITLE_CHARS),
                truncate_chars(&fact, MAX_CARD_FACT_CHARS),
                !changes.is_empty() || *omitted_change_count > 0,
                u64::try_from(
                    changes
                        .iter()
                        .map(|change| change.path.len().saturating_add(change.diff.len()))
                        .sum::<usize>(),
                )
                .unwrap_or(u64::MAX),
            )
        }
        ConversationProgressiveActivityPayload::McpProgress {
            message,
            update_count,
            truncated_bytes,
            ..
        } => {
            let title = first_non_empty_line(message).unwrap_or("mcp progress");
            let fact = if *truncated_bytes > 0 {
                format!("{update_count} updates · trunc")
            } else {
                format!("{update_count} updates")
            };
            (
                truncate_chars(title, MAX_CARD_TITLE_CHARS),
                truncate_chars(&fact, MAX_CARD_FACT_CHARS),
                !message.trim().is_empty(),
                u64::try_from(message.len()).unwrap_or(u64::MAX),
            )
        }
        ConversationProgressiveActivityPayload::TurnDiff {
            detail,
            addition_count,
            deletion_count,
            hunk_count,
            truncated_bytes,
            ..
        } => {
            let title = first_non_empty_line(detail).unwrap_or("turn diff");
            let fact = if *truncated_bytes > 0 {
                format!("+{addition_count}/-{deletion_count} h{hunk_count} · trunc")
            } else {
                format!("+{addition_count}/-{deletion_count} h{hunk_count}")
            };
            (
                truncate_chars(title, MAX_CARD_TITLE_CHARS),
                truncate_chars(&fact, MAX_CARD_FACT_CHARS),
                !detail.is_empty(),
                u64::try_from(detail.len()).unwrap_or(u64::MAX),
            )
        }
        ConversationProgressiveActivityPayload::TurnPlan {
            explanation,
            steps,
            omitted_step_count,
            truncated_bytes,
            ..
        } => {
            let completed = steps
                .iter()
                .filter(|step| step.status == ConversationProgressivePlanStepStatus::Completed)
                .count();
            let retained = steps.len();
            let title = explanation
                .as_deref()
                .and_then(first_non_empty_line)
                .or_else(|| steps.first().map(|step| step.text.as_str()))
                .unwrap_or("plan");
            let fact = if *omitted_step_count > 0 || *truncated_bytes > 0 {
                format!("{completed}/{retained}+{omitted_step_count}? steps")
            } else {
                format!("{completed}/{retained} steps")
            };
            (
                truncate_chars(title, MAX_CARD_TITLE_CHARS),
                truncate_chars(&fact, MAX_CARD_FACT_CHARS),
                true,
                u64::try_from(
                    explanation.as_ref().map_or(0, String::len)
                        + steps.iter().map(|step| step.text.len()).sum::<usize>(),
                )
                .unwrap_or(u64::MAX),
            )
        }
        ConversationProgressiveActivityPayload::PlanDelta {
            chunk_count,
            source_bytes,
        } => (
            "plan delta".to_string(),
            format!("{chunk_count} chunks"),
            *source_bytes > 0,
            *source_bytes,
        ),
        ConversationProgressiveActivityPayload::AgentMessageDelta {
            phase,
            text,
            truncated_bytes,
            ..
        } => {
            let title = first_non_empty_line(text).unwrap_or("agent message");
            let phase_label = phase.as_deref().unwrap_or("delta");
            let fact = if *truncated_bytes > 0 {
                format!("{phase_label} · trunc")
            } else {
                phase_label.to_string()
            };
            (
                truncate_chars(title, MAX_CARD_TITLE_CHARS),
                truncate_chars(&fact, MAX_CARD_FACT_CHARS),
                !text.trim().is_empty(),
                u64::try_from(text.len()).unwrap_or(u64::MAX),
            )
        }
        ConversationProgressiveActivityPayload::ReasoningSummaryTextDelta {
            summary_index,
            chunk_count,
            source_bytes,
        }
        | ConversationProgressiveActivityPayload::ReasoningTextDelta {
            content_index: summary_index,
            chunk_count,
            source_bytes,
        } => (
            format!("reasoning #{summary_index}"),
            format!("{chunk_count} chunks"),
            *source_bytes > 0,
            *source_bytes,
        ),
        ConversationProgressiveActivityPayload::ReasoningSummaryPartAdded {
            summary_index,
            part_count,
        } => (
            format!("reasoning part #{summary_index}"),
            format!("{part_count} parts"),
            true,
            0,
        ),
        ConversationProgressiveActivityPayload::TerminalInteraction {
            process_id,
            interaction_count,
            input_bytes,
        } => (
            truncate_chars(process_id, MAX_CARD_TITLE_CHARS),
            format!("{interaction_count} ix · {input_bytes} B"),
            true,
            *input_bytes,
        ),
        ConversationProgressiveActivityPayload::TokenUsage { usage } => (
            "token usage".to_string(),
            format!("{} total", usage.total.total_tokens),
            true,
            0,
        ),
        ConversationProgressiveActivityPayload::Moderation {
            update_count,
            metadata_bytes,
        } => (
            "moderation".to_string(),
            format!("{update_count} updates"),
            *metadata_bytes > 0 || *update_count > 0,
            *metadata_bytes,
        ),
        ConversationProgressiveActivityPayload::GuardianWarning {
            message,
            update_count,
            truncated_bytes,
            ..
        } => {
            let title = first_non_empty_line(message).unwrap_or("guardian warning");
            let fact = if *truncated_bytes > 0 {
                format!("{update_count} · trunc")
            } else {
                format!("{update_count} updates")
            };
            (
                truncate_chars(title, MAX_CARD_TITLE_CHARS),
                truncate_chars(&fact, MAX_CARD_FACT_CHARS),
                !message.trim().is_empty(),
                u64::try_from(message.len()).unwrap_or(u64::MAX),
            )
        }
        ConversationProgressiveActivityPayload::Unknown { payload_bytes } => (
            "unknown activity".to_string(),
            format!("{payload_bytes} B"),
            *payload_bytes > 0,
            *payload_bytes,
        ),
    }
}

fn synthesize_payload_detail(payload: &ConversationProgressiveActivityPayload) -> String {
    let raw = match payload {
        ConversationProgressiveActivityPayload::CommandOutput { tail, .. } => tail.clone(),
        ConversationProgressiveActivityPayload::TurnDiff { detail, .. } => detail.clone(),
        ConversationProgressiveActivityPayload::McpProgress { message, .. } => message.clone(),
        ConversationProgressiveActivityPayload::AgentMessageDelta { text, .. } => text.clone(),
        ConversationProgressiveActivityPayload::GuardianWarning { message, .. } => message.clone(),
        ConversationProgressiveActivityPayload::FileChangePatch {
            changes,
            omitted_change_count,
            ..
        } => {
            let mut lines = Vec::new();
            for change in changes {
                let kind = match change.kind {
                    ConversationProgressiveFileChangeKind::Add => "add",
                    ConversationProgressiveFileChangeKind::Delete => "delete",
                    ConversationProgressiveFileChangeKind::Update { .. } => "update",
                };
                lines.push(format!("[{kind}] {}", change.path));
                if !change.diff.is_empty() {
                    lines.push(change.diff.clone());
                }
            }
            if *omitted_change_count > 0 {
                lines.push(format!("(+{omitted_change_count} omitted changes)"));
            }
            lines.join("\n")
        }
        ConversationProgressiveActivityPayload::TurnPlan {
            explanation,
            steps,
            omitted_step_count,
            ..
        } => {
            let mut lines = Vec::new();
            if let Some(explanation) = explanation
                .as_deref()
                .filter(|text| !text.trim().is_empty())
            {
                lines.push(explanation.to_string());
                lines.push(String::new());
            }
            for (index, step) in steps.iter().enumerate() {
                let marker = match step.status {
                    ConversationProgressivePlanStepStatus::Completed => "x",
                    ConversationProgressivePlanStepStatus::InProgress => ">",
                    ConversationProgressivePlanStepStatus::Pending => " ",
                };
                lines.push(format!("{}. [{marker}] {}", index + 1, step.text));
            }
            if *omitted_step_count > 0 {
                lines.push(format!("(+{omitted_step_count} omitted steps)"));
            }
            lines.join("\n")
        }
        ConversationProgressiveActivityPayload::PlanDelta {
            chunk_count,
            source_bytes,
        } => format!("plan delta: {chunk_count} chunks · {source_bytes} B"),
        ConversationProgressiveActivityPayload::ReasoningSummaryTextDelta {
            summary_index,
            chunk_count,
            source_bytes,
        } => format!(
            "reasoning summary #{summary_index}: {chunk_count} chunks · {source_bytes} B retained"
        ),
        ConversationProgressiveActivityPayload::ReasoningTextDelta {
            content_index,
            chunk_count,
            source_bytes,
        } => format!(
            "reasoning text #{content_index}: {chunk_count} chunks · {source_bytes} B retained"
        ),
        ConversationProgressiveActivityPayload::ReasoningSummaryPartAdded {
            summary_index,
            part_count,
        } => format!("reasoning part added #{summary_index}: {part_count} parts"),
        ConversationProgressiveActivityPayload::TerminalInteraction {
            process_id,
            interaction_count,
            input_bytes,
        } => format!(
            "terminal {process_id}\ninteractions: {interaction_count}\ninput: {input_bytes} B"
        ),
        ConversationProgressiveActivityPayload::TokenUsage { usage } => format!(
            "last input: {}\nlast cached input: {}\nlast output: {}\nlast reasoning output: {}\nlast total: {}\ntotal input: {}\ntotal output: {}\ntotal: {}",
            usage.last.input_tokens,
            usage.last.cached_input_tokens,
            usage.last.output_tokens,
            usage.last.reasoning_output_tokens,
            usage.last.total_tokens,
            usage.total.input_tokens,
            usage.total.output_tokens,
            usage.total.total_tokens
        ),
        ConversationProgressiveActivityPayload::Moderation {
            update_count,
            metadata_bytes,
        } => format!("moderation updates: {update_count}\nmetadata: {metadata_bytes} B"),
        ConversationProgressiveActivityPayload::Unknown { payload_bytes } => {
            format!("unknown progressive activity · {payload_bytes} B")
        }
    };
    bound_synthesized_detail(raw)
}

pub(crate) fn bound_synthesized_detail(raw: String) -> String {
    if raw.len() <= MAX_SYNTHESIZED_DETAIL_BYTES {
        raw
    } else {
        // Find a UTF-8 boundary before truncating; String::truncate panics mid-character.
        let mut limit = MAX_SYNTHESIZED_DETAIL_BYTES.min(raw.len());
        while limit > 0 && !raw.is_char_boundary(limit) {
            limit -= 1;
        }
        let mut truncated = raw;
        truncated.truncate(limit);
        truncated.push_str("\n[truncated by Akra activity card detail bound]");
        truncated
    }
}

fn first_non_empty_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|line| !line.is_empty())
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let kept: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{kept}…")
    } else {
        kept
    }
}

pub(crate) fn filter_cards_by_kind(
    cards: &[ProgressiveActivityCard],
    filter: Option<ProgressiveActivityCardKind>,
) -> Vec<usize> {
    cards
        .iter()
        .enumerate()
        .filter_map(|(index, card)| match filter {
            None => Some(index),
            Some(kind) if card.key.kind == kind => Some(index),
            Some(_) => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::conversation_item_lifecycle::{
        ConversationItemLifecycleObservation, ConversationItemLifecycleRecord,
        ConversationItemLifecycleSource,
    };
    use crate::domain::conversation_progressive_activity::{
        ConversationProgressiveActivityBatch, ConversationProgressiveActivityObservation,
        ConversationProgressiveActivityProjection, ConversationProgressiveFileChange,
    };
    use std::sync::Arc;

    fn snapshot_from(
        observations: Vec<ConversationProgressiveActivityObservation>,
    ) -> Arc<ConversationProgressiveActivityProjectionSnapshot> {
        let mut projection = ConversationProgressiveActivityProjection::default();
        for observation in observations {
            let turn_id = observation.turn_id.clone();
            let batch = ConversationProgressiveActivityBatch::single(observation)
                .expect("card test observation valid");
            projection
                .apply_batch_correlated(Some("thread-cards"), turn_id.as_deref(), batch)
                .expect("card test batch projects");
        }
        projection.snapshot()
    }

    fn obs(
        sequence: u64,
        kind: ConversationProgressiveActivityKind,
        payload: ConversationProgressiveActivityPayload,
        item_id: Option<&str>,
    ) -> ConversationProgressiveActivityObservation {
        let turn_id = match kind {
            ConversationProgressiveActivityKind::GuardianWarning => None,
            _ => Some("turn-cards".to_string()),
        };
        ConversationProgressiveActivityObservation {
            sequence,
            thread_id: "thread-cards".to_string(),
            turn_id,
            item_id: item_id.map(str::to_string),
            kind,
            payload,
        }
    }

    struct LifecycleRecordFixture<'a> {
        sequence: u64,
        item_id: &'a str,
        kind: ConversationItemKind,
        phase: ConversationItemLifecyclePhase,
        observed_at_ms: Option<i64>,
        outcome: ConversationItemOutcome,
        summary: &'a str,
        consistency: ConversationItemLifecycleConsistency,
    }

    fn lifecycle_record(fixture: LifecycleRecordFixture<'_>) -> ConversationItemLifecycleRecord {
        ConversationItemLifecycleRecord {
            sequence: fixture.sequence,
            observation: ConversationItemLifecycleObservation {
                thread_id: "thread-cards".to_string(),
                turn_id: "turn-cards".to_string(),
                item_id: fixture.item_id.to_string(),
                kind: fixture.kind,
                phase: fixture.phase,
                source: if fixture.phase == ConversationItemLifecyclePhase::SnapshotObserved {
                    ConversationItemLifecycleSource::Snapshot
                } else {
                    ConversationItemLifecycleSource::Live
                },
                observed_at_ms: fixture.observed_at_ms,
                outcome: fixture.outcome,
                summary: fixture.summary.to_string(),
                command_actions: Default::default(),
            },
            consistency: fixture.consistency,
        }
    }

    fn read_actions(name: &str, path: &str) -> ConversationCommandActionProjection {
        let retained_bytes = name.len().saturating_add(path.len());
        ConversationCommandActionProjection {
            actions: vec![ConversationCommandAction::Read {
                name: name.to_string(),
                path: path.to_string(),
            }],
            read_action_count: 1,
            source_bytes: u64::try_from(retained_bytes).unwrap_or(u64::MAX),
            ..ConversationCommandActionProjection::default()
        }
    }

    fn command_snapshot(item_id: &str) -> Arc<ConversationProgressiveActivityProjectionSnapshot> {
        snapshot_from(vec![obs(
            1,
            ConversationProgressiveActivityKind::CommandOutput,
            ConversationProgressiveActivityPayload::CommandOutput {
                tail: "cargo test\nok".to_string(),
                chunk_count: 1,
                source_bytes: 13,
                newline_count: 1,
                ends_with_newline: false,
                truncated_bytes: 0,
            },
            Some(item_id),
        )])
    }

    #[test]
    fn projects_all_major_payload_kinds_into_cards() {
        let command_tail = "cargo test\nok";
        let patch_diff = "+fn x() {}";
        let mcp_message = "searching docs";
        let snapshot = snapshot_from(vec![
            obs(
                1,
                ConversationProgressiveActivityKind::CommandOutput,
                ConversationProgressiveActivityPayload::CommandOutput {
                    tail: command_tail.to_string(),
                    chunk_count: 1,
                    source_bytes: command_tail.len() as u64,
                    newline_count: 1,
                    ends_with_newline: false,
                    truncated_bytes: 0,
                },
                Some("cmd-1"),
            ),
            obs(
                2,
                ConversationProgressiveActivityKind::FileChangePatch,
                ConversationProgressiveActivityPayload::FileChangePatch {
                    changes: vec![ConversationProgressiveFileChange {
                        path: "src/lib.rs".to_string(),
                        diff: patch_diff.to_string(),
                        kind: ConversationProgressiveFileChangeKind::Update {
                            move_path_present: false,
                        },
                    }],
                    omitted_change_count: 0,
                    source_bytes: ("src/lib.rs".len() + patch_diff.len()) as u64,
                    truncated_bytes: 0,
                },
                Some("patch-1"),
            ),
            obs(
                3,
                ConversationProgressiveActivityKind::McpProgress,
                ConversationProgressiveActivityPayload::McpProgress {
                    message: mcp_message.to_string(),
                    update_count: 1,
                    source_bytes: mcp_message.len() as u64,
                    truncated_bytes: 0,
                },
                Some("mcp-1"),
            ),
            obs(
                4,
                ConversationProgressiveActivityKind::TurnDiff,
                ConversationProgressiveActivityPayload::TurnDiff {
                    detail: "@@ -1 +1 @@\n+line".to_string(),
                    source_bytes: "@@ -1 +1 @@\n+line".len() as u64,
                    line_count: 2,
                    addition_count: 1,
                    deletion_count: 0,
                    hunk_count: 1,
                    truncated_bytes: 0,
                },
                None,
            ),
            obs(
                5,
                ConversationProgressiveActivityKind::GuardianWarning,
                ConversationProgressiveActivityPayload::GuardianWarning {
                    message: "be careful".to_string(),
                    update_count: 1,
                    source_bytes: "be careful".len() as u64,
                    truncated_bytes: 0,
                },
                None,
            ),
        ]);

        let cards = project_activity_cards(&snapshot);
        assert_eq!(cards.len(), 5);
        assert_eq!(cards[0].key.kind, ProgressiveActivityCardKind::Command);
        assert!(cards[0].header_line(false).starts_with("› ◆ command"));
        assert!(cards[0].expandable);
        assert_eq!(cards[1].key.kind, ProgressiveActivityCardKind::Patch);
        assert!(
            card_detail_text(&snapshot, 1)
                .unwrap()
                .contains("src/lib.rs")
        );
        assert_eq!(cards[2].key.kind, ProgressiveActivityCardKind::Mcp);
        assert_eq!(cards[3].key.kind, ProgressiveActivityCardKind::Diff);
        assert_eq!(cards[4].key.kind, ProgressiveActivityCardKind::Guardian);
    }

    #[test]
    fn timeline_cards_join_exact_lifecycle_outcome_summary_and_elapsed_time() {
        let progressive = command_snapshot("cmd-timeline");
        let lifecycle = ConversationItemLifecycleProjectionSnapshot {
            records: vec![
                lifecycle_record(LifecycleRecordFixture {
                    sequence: 0,
                    item_id: "cmd-timeline",
                    kind: ConversationItemKind::CommandExecution,
                    phase: ConversationItemLifecyclePhase::Started,
                    observed_at_ms: Some(1_000),
                    outcome: ConversationItemOutcome::InProgress,
                    summary: "running cargo test",
                    consistency: ConversationItemLifecycleConsistency::Accepted,
                }),
                lifecycle_record(LifecycleRecordFixture {
                    sequence: 1,
                    item_id: "cmd-timeline",
                    kind: ConversationItemKind::CommandExecution,
                    phase: ConversationItemLifecyclePhase::Completed,
                    observed_at_ms: Some(3_500),
                    outcome: ConversationItemOutcome::Completed,
                    summary: "test suite passed",
                    consistency: ConversationItemLifecycleConsistency::Accepted,
                }),
            ],
            ..ConversationItemLifecycleProjectionSnapshot::default()
        };

        let started_only = ConversationItemLifecycleProjectionSnapshot {
            records: vec![lifecycle.records[0].clone()],
            ..ConversationItemLifecycleProjectionSnapshot::default()
        };
        let active =
            project_activity_timeline_cards(&progressive, Some(&started_only), Some(2_000));
        assert_eq!(active[0].outcome, ProgressiveActivityCardOutcome::Active);
        assert_eq!(active[0].summary, "running cargo test");
        assert_eq!(active[0].elapsed_ms, Some(1_000));

        let cards = project_activity_timeline_cards(&progressive, Some(&lifecycle), Some(9_000));

        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].outcome, ProgressiveActivityCardOutcome::Completed);
        assert_eq!(cards[0].summary, "test suite passed");
        assert_eq!(cards[0].elapsed_ms, Some(2_500));
    }

    #[test]
    fn read_actions_replace_opaque_command_copy_and_expand_to_exact_path() {
        let progressive = command_snapshot("cmd-read");
        let mut started = lifecycle_record(LifecycleRecordFixture {
            sequence: 0,
            item_id: "cmd-read",
            kind: ConversationItemKind::CommandExecution,
            phase: ConversationItemLifecyclePhase::Started,
            observed_at_ms: Some(1_000),
            outcome: ConversationItemOutcome::InProgress,
            summary: "Reading src/lib.rs",
            consistency: ConversationItemLifecycleConsistency::Accepted,
        });
        started.observation.command_actions = read_actions("src/lib.rs", "C:/dev/akra/src/lib.rs");
        let mut completed = lifecycle_record(LifecycleRecordFixture {
            sequence: 1,
            item_id: "cmd-read",
            kind: ConversationItemKind::CommandExecution,
            phase: ConversationItemLifecyclePhase::Completed,
            observed_at_ms: Some(1_500),
            outcome: ConversationItemOutcome::Completed,
            summary: "Read src/lib.rs",
            consistency: ConversationItemLifecycleConsistency::Accepted,
        });
        completed.observation.command_actions = started.observation.command_actions.clone();
        let lifecycle = ConversationItemLifecycleProjectionSnapshot {
            records: vec![started, completed],
            ..ConversationItemLifecycleProjectionSnapshot::default()
        };

        let cards = project_activity_timeline_cards(&progressive, Some(&lifecycle), Some(2_000));

        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].activity_label, "read");
        assert_eq!(cards[0].title, "src/lib.rs");
        assert_eq!(cards[0].summary, "Read src/lib.rs");
        assert_eq!(cards[0].lifecycle_sequence, Some(1));
        assert!(cards[0].expandable);
        let detail = command_action_detail_text(&lifecycle, 1).unwrap();
        assert!(detail.contains("1. Read src/lib.rs"), "{detail}");
        assert!(detail.contains("path: C:/dev/akra/src/lib.rs"), "{detail}");
    }

    #[test]
    fn action_without_command_output_still_materializes_one_stable_activity_card() {
        let progressive = snapshot_from(Vec::new());
        let mut record = lifecycle_record(LifecycleRecordFixture {
            sequence: 7,
            item_id: "cmd-empty-read",
            kind: ConversationItemKind::CommandExecution,
            phase: ConversationItemLifecyclePhase::SnapshotObserved,
            observed_at_ms: None,
            outcome: ConversationItemOutcome::Completed,
            summary: "Read Cargo.toml",
            consistency: ConversationItemLifecycleConsistency::SnapshotObserved,
        });
        record.observation.command_actions = read_actions("Cargo.toml", "C:/dev/akra/Cargo.toml");
        let lifecycle = ConversationItemLifecycleProjectionSnapshot {
            records: vec![record],
            ..ConversationItemLifecycleProjectionSnapshot::default()
        };

        let cards = project_activity_timeline_cards(&progressive, Some(&lifecycle), None);

        assert_eq!(cards.len(), 1);
        assert_eq!(
            cards[0].key.source,
            ProgressiveActivityCardSource::Lifecycle
        );
        assert_eq!(cards[0].key.sequence, 7);
        assert_eq!(cards[0].activity_label, "read");
        assert_eq!(cards[0].record_index, None);
        assert_eq!(cards[0].lifecycle_sequence, Some(7));
    }

    #[test]
    fn active_failed_and_clock_regression_states_never_invent_elapsed_time() {
        let progressive = command_snapshot("cmd-state");
        let active = ConversationItemLifecycleProjectionSnapshot {
            records: vec![lifecycle_record(LifecycleRecordFixture {
                sequence: 0,
                item_id: "cmd-state",
                kind: ConversationItemKind::CommandExecution,
                phase: ConversationItemLifecyclePhase::Started,
                observed_at_ms: Some(4_000),
                outcome: ConversationItemOutcome::InProgress,
                summary: "running command",
                consistency: ConversationItemLifecycleConsistency::Accepted,
            })],
            ..ConversationItemLifecycleProjectionSnapshot::default()
        };
        let cards = project_activity_timeline_cards(&progressive, Some(&active), Some(5_250));
        assert_eq!(cards[0].outcome, ProgressiveActivityCardOutcome::Active);
        assert_eq!(cards[0].elapsed_ms, Some(1_250));

        let regressed = ConversationItemLifecycleProjectionSnapshot {
            records: vec![
                active.records[0].clone(),
                lifecycle_record(LifecycleRecordFixture {
                    sequence: 1,
                    item_id: "cmd-state",
                    kind: ConversationItemKind::CommandExecution,
                    phase: ConversationItemLifecyclePhase::Completed,
                    observed_at_ms: Some(3_000),
                    outcome: ConversationItemOutcome::Failed,
                    summary: "command failed",
                    consistency: ConversationItemLifecycleConsistency::TimestampRegression,
                }),
            ],
            ..ConversationItemLifecycleProjectionSnapshot::default()
        };
        let cards = project_activity_timeline_cards(&progressive, Some(&regressed), Some(6_000));
        assert_eq!(cards[0].outcome, ProgressiveActivityCardOutcome::Failed);
        assert_eq!(cards[0].elapsed_ms, None);
    }

    #[test]
    fn exact_wait_state_prefers_approval_then_retry_and_classifies_active_items() {
        let subagent = ConversationItemLifecycleProjectionSnapshot {
            records: vec![lifecycle_record(LifecycleRecordFixture {
                sequence: 0,
                item_id: "subagent-1",
                kind: ConversationItemKind::SubAgentActivity,
                phase: ConversationItemLifecyclePhase::Started,
                observed_at_ms: Some(1_000),
                outcome: ConversationItemOutcome::InProgress,
                summary: "reviewing implementation",
                consistency: ConversationItemLifecycleConsistency::Accepted,
            })],
            ..ConversationItemLifecycleProjectionSnapshot::default()
        };
        assert_eq!(
            project_activity_wait_status(Some(&subagent), None, false),
            Some(ProgressiveActivityWaitStatus {
                kind: ProgressiveActivityWaitKind::Subagent,
                summary: Some("reviewing implementation".to_string()),
            })
        );
        assert_eq!(
            project_activity_wait_status(Some(&subagent), Some("server overloaded"), false)
                .map(|status| status.kind),
            Some(ProgressiveActivityWaitKind::Retrying)
        );
        assert_eq!(
            project_activity_wait_status(Some(&subagent), Some("server overloaded"), true)
                .map(|status| status.kind),
            Some(ProgressiveActivityWaitKind::Approval)
        );

        let task = ConversationItemLifecycleProjectionSnapshot {
            records: vec![lifecycle_record(LifecycleRecordFixture {
                sequence: 0,
                item_id: "command-1",
                kind: ConversationItemKind::CommandExecution,
                phase: ConversationItemLifecyclePhase::SnapshotObserved,
                observed_at_ms: None,
                outcome: ConversationItemOutcome::InProgress,
                summary: "waiting on shell",
                consistency: ConversationItemLifecycleConsistency::SnapshotObserved,
            })],
            ..ConversationItemLifecycleProjectionSnapshot::default()
        };
        assert_eq!(
            project_activity_wait_status(Some(&task), None, false).map(|status| status.kind),
            Some(ProgressiveActivityWaitKind::TaskOutput)
        );

        let unknown = ConversationItemLifecycleProjectionSnapshot {
            records: vec![lifecycle_record(LifecycleRecordFixture {
                sequence: 0,
                item_id: "unknown-1",
                kind: ConversationItemKind::Unknown("futureItem".to_string()),
                phase: ConversationItemLifecyclePhase::SnapshotObserved,
                observed_at_ms: None,
                outcome: ConversationItemOutcome::InProgress,
                summary: "future activity",
                consistency: ConversationItemLifecycleConsistency::SnapshotObserved,
            })],
            ..ConversationItemLifecycleProjectionSnapshot::default()
        };
        assert_eq!(
            project_activity_wait_status(Some(&unknown), None, false),
            None,
            "unknown lifecycle kinds must not invent a wait category"
        );
    }

    #[test]
    fn expand_state_is_bounded_and_toggles() {
        let mut state = ProgressiveActivityExpandState::default();
        let key = ProgressiveActivityCardKey {
            source: ProgressiveActivityCardSource::Progressive,
            sequence: 1,
            kind: ProgressiveActivityCardKind::Command,
        };
        assert!(state.toggle_card(key));
        assert!(state.is_card_expanded(key));
        assert!(!state.toggle_card(key));
        assert!(!state.is_card_expanded(key));

        for sequence in 0..40 {
            state.expand_card(ProgressiveActivityCardKey {
                source: ProgressiveActivityCardSource::Progressive,
                sequence,
                kind: ProgressiveActivityCardKind::Diff,
            });
        }
        assert_eq!(state.expanded.len(), MAX_EXPANDED_CARD_KEYS);
    }

    #[test]
    fn tool_message_helpers_detect_expandable_bodies() {
        assert!(!tool_message_is_expandable("short"));
        assert!(tool_message_is_expandable("one\ntwo"));
        assert_eq!(tool_message_title("one\ntwo"), "one");
        assert_eq!(tool_message_fact("one\ntwo\nthree"), "3 lines");
        assert_ne!(
            tool_message_digest(None, "a"),
            tool_message_digest(None, "b")
        );
        assert_ne!(
            tool_message_digest(Some("read-1"), "same read"),
            tool_message_digest(Some("read-2"), "same read")
        );
        let mut state = ProgressiveActivityExpandState::default();
        let digest = tool_message_digest(None, "one\ntwo");
        state.expand_tool(digest);
        assert!(state.is_tool_expanded(digest));
    }

    #[test]
    fn synthesized_detail_truncates_on_utf8_boundary_without_panic() {
        // Build a payload whose synthesized detail exceeds the bound and ends with multi-byte chars.
        let mut tail = "a".repeat(MAX_SYNTHESIZED_DETAIL_BYTES - 2);
        tail.push('한');
        tail.push('글');
        let payload = ConversationProgressiveActivityPayload::CommandOutput {
            tail,
            chunk_count: 1,
            source_bytes: (MAX_SYNTHESIZED_DETAIL_BYTES + 4) as u64,
            newline_count: 0,
            ends_with_newline: false,
            truncated_bytes: 0,
        };
        let detail = synthesize_payload_detail(&payload);
        assert!(detail.len() <= MAX_SYNTHESIZED_DETAIL_BYTES + 80);
        assert!(detail.ends_with("[truncated by Akra activity card detail bound]"));
        assert!(detail.is_char_boundary(detail.len()));
    }
}
