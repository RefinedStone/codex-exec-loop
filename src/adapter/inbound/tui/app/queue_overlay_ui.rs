use std::collections::BTreeMap;

use crate::application::service::planning::{
    PlanningApplicationProjection, PlanningApplicationQueueTask, PlanningApplicationSkippedTask,
};
use crate::domain::planning::TaskStatus;

use super::NativeTuiApp;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QueueOverlayActionTask {
    pub(super) task_id: String,
    pub(super) status: TaskStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct QueueOverlayAuthorityToken {
    pub(super) status: TaskStatus,
    pub(super) updated_at: String,
}

impl From<PlanningApplicationQueueTask> for QueueOverlayActionTask {
    fn from(task: PlanningApplicationQueueTask) -> Self {
        Self {
            task_id: task.task_id,
            status: task.status,
        }
    }
}

impl From<PlanningApplicationSkippedTask> for QueueOverlayActionTask {
    fn from(task: PlanningApplicationSkippedTask) -> Self {
        Self {
            task_id: task.task_id,
            status: task.status,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct QueueOverlayUiState {
    selected_task_id: Option<String>,
    feedback: Option<String>,
    authority_revision: Option<i64>,
    authority_tokens: BTreeMap<String, QueueOverlayAuthorityToken>,
}

impl QueueOverlayUiState {
    pub(super) fn selected_task_id(&self) -> Option<&str> {
        self.selected_task_id.as_deref()
    }

    pub(super) fn feedback(&self) -> Option<&str> {
        self.feedback.as_deref()
    }

    pub(super) fn set_feedback(&mut self, feedback: impl Into<String>) {
        self.feedback = Some(feedback.into());
    }

    pub(super) fn bind_authority(
        &mut self,
        planning_revision: i64,
        authority_tokens: BTreeMap<String, QueueOverlayAuthorityToken>,
    ) {
        self.authority_revision = Some(planning_revision);
        self.authority_tokens = authority_tokens;
    }

    pub(super) fn clear_authority_binding(&mut self) {
        self.authority_revision = None;
        self.authority_tokens.clear();
    }

    pub(super) fn authority_revision(&self) -> Option<i64> {
        self.authority_revision
    }

    pub(super) fn selected_authority_token(
        &self,
    ) -> Option<(i64, &str, &QueueOverlayAuthorityToken)> {
        let task_id = self.selected_task_id.as_deref()?;
        Some((
            self.authority_revision?,
            task_id,
            self.authority_tokens.get(task_id)?,
        ))
    }

    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(super) fn sync(&mut self, task_ids: &[String]) {
        if task_ids.is_empty() {
            self.selected_task_id = None;
            return;
        }
        if self
            .selected_task_id
            .as_ref()
            .is_none_or(|selected| !task_ids.contains(selected))
        {
            self.selected_task_id = task_ids.first().cloned();
        }
    }

    pub(super) fn move_selection(&mut self, task_ids: &[String], delta: isize) {
        self.sync(task_ids);
        let Some(selected) = self.selected_task_id.as_ref() else {
            return;
        };
        let current = task_ids
            .iter()
            .position(|task_id| task_id == selected)
            .unwrap_or(0);
        let next = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current.saturating_add(delta as usize)
        }
        .min(task_ids.len().saturating_sub(1));
        self.selected_task_id = task_ids.get(next).cloned();
        self.feedback = None;
    }
}

impl NativeTuiApp {
    pub(super) fn queue_action_tasks(&self) -> Vec<QueueOverlayActionTask> {
        let projection = PlanningApplicationProjection::from_runtime_projection(
            &self.planning_runtime_projection_snapshot(),
        );
        projection
            .visible_tasks
            .into_iter()
            .map(QueueOverlayActionTask::from)
            .chain(
                projection
                    .proposed_tasks
                    .into_iter()
                    .map(QueueOverlayActionTask::from),
            )
            .chain(
                projection
                    .skipped_tasks
                    .into_iter()
                    .map(QueueOverlayActionTask::from),
            )
            .filter(|task| matches!(task.status, TaskStatus::Ready | TaskStatus::Proposed))
            .collect()
    }

    pub(super) fn sync_queue_overlay_selection(&mut self) {
        let task_ids = self
            .queue_action_tasks()
            .into_iter()
            .map(|task| task.task_id)
            .collect::<Vec<_>>();
        self.queue_overlay_ui_state.sync(&task_ids);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{QueueOverlayAuthorityToken, QueueOverlayUiState};
    use crate::domain::planning::TaskStatus;

    #[test]
    fn selection_tracks_task_identity_when_rows_change() {
        let mut state = QueueOverlayUiState::default();
        let tasks = vec!["task-a".to_string(), "task-b".to_string()];
        state.sync(&tasks);
        assert_eq!(state.selected_task_id(), Some("task-a"));

        state.set_feedback("old failure");
        state.move_selection(&tasks, 1);
        assert_eq!(state.selected_task_id(), Some("task-b"));
        assert_eq!(state.feedback(), None);

        state.sync(&["task-b".to_string(), "task-c".to_string()]);
        assert_eq!(state.selected_task_id(), Some("task-b"));

        state.sync(&["task-c".to_string()]);
        assert_eq!(state.selected_task_id(), Some("task-c"));

        state.sync(&[]);
        assert_eq!(state.selected_task_id(), None);
    }

    #[test]
    fn reset_drops_displayed_authority_tokens() {
        let mut state = QueueOverlayUiState::default();
        state.sync(&["task-a".to_string()]);
        state.bind_authority(
            7,
            BTreeMap::from([(
                "task-a".to_string(),
                QueueOverlayAuthorityToken {
                    status: TaskStatus::Ready,
                    updated_at: "2026-07-15T00:00:00Z".to_string(),
                },
            )]),
        );
        assert!(state.selected_authority_token().is_some());

        state.reset();

        assert!(state.selected_authority_token().is_none());
    }
}
