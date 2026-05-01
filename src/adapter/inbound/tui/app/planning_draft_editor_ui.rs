use std::path::Path;

use crate::application::service::planning::{PlanningDraftEditorFile, PlanningDraftEditorSession};
use crate::domain::planning::PlanningValidationReport;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct PlanningDraftEditorUiState {
    session: Option<PlanningDraftEditorSessionState>,
    close_guard: PlanningDraftEditorCloseGuardState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PlanningDraftEditorSessionState {
    draft_name: String,
    draft_directory: String,
    buffers: Vec<PlanningDraftEditorBufferState>,
    selected_file_index: usize,
    validation_report: PlanningValidationReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PlanningDraftEditorBufferState {
    active_path: String,
    staged_path: String,
    lines: Vec<String>,
    cursor_line_index: usize,
    cursor_column: usize,
    preferred_column: usize,
    editor_scroll: u16,
    dirty: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PlanningDraftEditorCloseRisk {
    has_dirty_buffers: bool,
    has_invalid_staged_draft: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum PlanningDraftEditorCloseGuardState {
    #[default]
    Inactive,
    ConfirmationPending(PlanningDraftEditorCloseRisk),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlanningDraftEditorCloseRequest {
    CloseImmediately,
    ConfirmationRequired(PlanningDraftEditorCloseRisk),
    Confirmed(PlanningDraftEditorCloseRisk),
}

impl PlanningDraftEditorUiState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn open_session(&mut self, session: PlanningDraftEditorSession) {
        self.session = Some(PlanningDraftEditorSessionState::from(session));
        self.close_guard = PlanningDraftEditorCloseGuardState::Inactive;
    }

    pub fn draft_name(&self) -> Option<&str> {
        self.session
            .as_ref()
            .map(|session| session.draft_name.as_str())
    }

    pub fn draft_directory(&self) -> Option<&str> {
        self.session
            .as_ref()
            .map(|session| session.draft_directory.as_str())
    }

    pub fn selected_file_index(&self) -> Option<usize> {
        self.session
            .as_ref()
            .map(|session| session.selected_file_index)
    }

    pub fn buffers(&self) -> Option<&[PlanningDraftEditorBufferState]> {
        self.session
            .as_ref()
            .map(|session| session.buffers.as_slice())
    }

    pub fn selected_buffer(&self) -> Option<&PlanningDraftEditorBufferState> {
        let session = self.session.as_ref()?;
        session.buffers.get(session.selected_file_index)
    }

    pub fn move_file_selection(&mut self, delta: isize) {
        self.clear_close_confirmation();
        let Some(session) = self.session.as_mut() else {
            return;
        };
        if session.buffers.is_empty() {
            session.selected_file_index = 0;
            return;
        }
        let max_index = session.buffers.len().saturating_sub(1) as isize;
        let next_index = (session.selected_file_index as isize + delta).clamp(0, max_index);
        session.selected_file_index = next_index as usize;
    }

    pub fn insert_character(&mut self, character: char) {
        self.clear_close_confirmation();
        if let Some(buffer) = self.selected_buffer_mut() {
            buffer.insert_character(character);
        }
    }

    pub fn insert_newline(&mut self) {
        self.clear_close_confirmation();
        if let Some(buffer) = self.selected_buffer_mut() {
            buffer.insert_newline();
        }
    }

    pub fn backspace(&mut self) {
        self.clear_close_confirmation();
        if let Some(buffer) = self.selected_buffer_mut() {
            buffer.backspace();
        }
    }

    pub fn delete_previous_word(&mut self) {
        self.clear_close_confirmation();
        if let Some(buffer) = self.selected_buffer_mut() {
            buffer.delete_previous_word();
        }
    }

    pub fn move_cursor_left(&mut self) {
        self.clear_close_confirmation();
        if let Some(buffer) = self.selected_buffer_mut() {
            buffer.move_cursor_left();
        }
    }

    pub fn move_cursor_right(&mut self) {
        self.clear_close_confirmation();
        if let Some(buffer) = self.selected_buffer_mut() {
            buffer.move_cursor_right();
        }
    }

    pub fn move_cursor_up(&mut self) {
        self.clear_close_confirmation();
        if let Some(buffer) = self.selected_buffer_mut() {
            buffer.move_cursor_up();
        }
    }

    pub fn move_cursor_down(&mut self) {
        self.clear_close_confirmation();
        if let Some(buffer) = self.selected_buffer_mut() {
            buffer.move_cursor_down();
        }
    }

    pub fn sync_editor_scroll(&mut self, visible_height: u16) {
        if let Some(buffer) = self.selected_buffer_mut() {
            buffer.sync_editor_scroll(visible_height);
        }
    }

    pub fn collect_editable_files(&self) -> Vec<PlanningDraftEditorFile> {
        self.buffers()
            .unwrap_or(&[])
            .iter()
            .map(|buffer| PlanningDraftEditorFile {
                active_path: buffer.active_path.clone(),
                staged_path: buffer.staged_path.clone(),
                body: buffer.body(),
            })
            .collect()
    }

    pub fn apply_save_result(&mut self, validation_report: PlanningValidationReport) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        session.validation_report = validation_report;
        self.close_guard = PlanningDraftEditorCloseGuardState::Inactive;
        for buffer in &mut session.buffers {
            buffer.dirty = false;
        }
    }

    pub fn validation_report(&self) -> Option<&PlanningValidationReport> {
        self.session
            .as_ref()
            .map(|session| &session.validation_report)
    }

    pub fn has_dirty_buffers(&self) -> bool {
        self.buffers()
            .unwrap_or(&[])
            .iter()
            .any(PlanningDraftEditorBufferState::is_dirty)
    }

    pub fn has_invalid_staged_draft(&self) -> bool {
        self.validation_report()
            .is_some_and(|report| !report.is_valid())
    }

    pub fn dirty_file_labels(&self) -> Vec<String> {
        self.buffers()
            .unwrap_or(&[])
            .iter()
            .filter(|buffer| buffer.is_dirty())
            .map(|buffer| buffer.file_label())
            .collect()
    }

    pub fn close_risk(&self) -> Option<PlanningDraftEditorCloseRisk> {
        self.current_close_risk()
    }

    pub fn pending_close_risk(&self) -> Option<PlanningDraftEditorCloseRisk> {
        match self.close_guard {
            PlanningDraftEditorCloseGuardState::Inactive => None,
            PlanningDraftEditorCloseGuardState::ConfirmationPending(risk) => Some(risk),
        }
    }

    pub fn is_close_confirmation_pending(&self) -> bool {
        self.pending_close_risk().is_some()
    }

    pub fn clear_close_confirmation(&mut self) {
        self.close_guard = PlanningDraftEditorCloseGuardState::Inactive;
    }

    pub fn request_close(&mut self) -> PlanningDraftEditorCloseRequest {
        let Some(risk) = self.current_close_risk() else {
            self.close_guard = PlanningDraftEditorCloseGuardState::Inactive;
            return PlanningDraftEditorCloseRequest::CloseImmediately;
        };

        if self.pending_close_risk() == Some(risk) {
            self.close_guard = PlanningDraftEditorCloseGuardState::Inactive;
            PlanningDraftEditorCloseRequest::Confirmed(risk)
        } else {
            self.close_guard = PlanningDraftEditorCloseGuardState::ConfirmationPending(risk);
            PlanningDraftEditorCloseRequest::ConfirmationRequired(risk)
        }
    }

    fn current_close_risk(&self) -> Option<PlanningDraftEditorCloseRisk> {
        let has_dirty_buffers = self.has_dirty_buffers();
        let has_invalid_staged_draft = self.has_invalid_staged_draft();
        if !has_dirty_buffers && !has_invalid_staged_draft {
            return None;
        }

        Some(PlanningDraftEditorCloseRisk {
            has_dirty_buffers,
            has_invalid_staged_draft,
        })
    }

    fn selected_buffer_mut(&mut self) -> Option<&mut PlanningDraftEditorBufferState> {
        let session = self.session.as_mut()?;
        session.buffers.get_mut(session.selected_file_index)
    }
}

impl PlanningDraftEditorCloseRisk {
    pub fn has_dirty_buffers(&self) -> bool {
        self.has_dirty_buffers
    }

    pub fn has_invalid_staged_draft(&self) -> bool {
        self.has_invalid_staged_draft
    }
}

impl PlanningDraftEditorSessionState {
    fn from(session: PlanningDraftEditorSession) -> Self {
        let buffers = session
            .editable_files
            .into_iter()
            .map(PlanningDraftEditorBufferState::from)
            .collect::<Vec<_>>();
        Self {
            draft_name: session.draft_name,
            draft_directory: session.draft_directory,
            buffers,
            selected_file_index: 0,
            validation_report: session.validation_report,
        }
    }
}

impl PlanningDraftEditorBufferState {
    pub fn active_path(&self) -> &str {
        self.active_path.as_str()
    }

    pub fn staged_path(&self) -> &str {
        self.staged_path.as_str()
    }

    pub fn lines(&self) -> &[String] {
        self.lines.as_slice()
    }

    pub fn cursor_line_index(&self) -> usize {
        self.cursor_line_index
    }

    pub fn cursor_column(&self) -> usize {
        self.cursor_column
    }

    pub fn editor_scroll(&self) -> u16 {
        self.editor_scroll
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn file_label(&self) -> String {
        Path::new(self.active_path())
            .file_name()
            .and_then(|file_name| file_name.to_str())
            .unwrap_or(self.active_path())
            .to_string()
    }

    pub fn body(&self) -> String {
        self.lines.join("\n")
    }

    fn insert_character(&mut self, character: char) {
        let byte_index =
            char_to_byte_index(&self.lines[self.cursor_line_index], self.cursor_column);
        self.lines[self.cursor_line_index].insert(byte_index, character);
        self.cursor_column += 1;
        self.preferred_column = self.cursor_column;
        self.dirty = true;
    }

    fn insert_newline(&mut self) {
        let byte_index =
            char_to_byte_index(&self.lines[self.cursor_line_index], self.cursor_column);
        let remainder = self.lines[self.cursor_line_index].split_off(byte_index);
        self.lines.insert(self.cursor_line_index + 1, remainder);
        self.cursor_line_index += 1;
        self.cursor_column = 0;
        self.preferred_column = 0;
        self.dirty = true;
    }

    fn backspace(&mut self) {
        if self.cursor_column > 0 {
            let line = &mut self.lines[self.cursor_line_index];
            let current_byte = char_to_byte_index(line, self.cursor_column);
            let previous_byte = char_to_byte_index(line, self.cursor_column - 1);
            line.replace_range(previous_byte..current_byte, "");
            self.cursor_column -= 1;
        } else if self.cursor_line_index > 0 {
            let current_line = self.lines.remove(self.cursor_line_index);
            self.cursor_line_index -= 1;
            let previous_line = &mut self.lines[self.cursor_line_index];
            self.cursor_column = previous_line.chars().count();
            previous_line.push_str(&current_line);
        } else {
            return;
        }

        self.preferred_column = self.cursor_column;
        self.dirty = true;
    }

    fn delete_previous_word(&mut self) {
        let original_position = (self.cursor_line_index, self.cursor_column);

        while self
            .character_before_cursor()
            .is_some_and(|character| character.is_whitespace())
        {
            self.backspace();
        }
        while self
            .character_before_cursor()
            .is_some_and(|character| !character.is_whitespace())
        {
            self.backspace();
        }

        if original_position != (self.cursor_line_index, self.cursor_column) {
            self.dirty = true;
        }
    }

    fn move_cursor_left(&mut self) {
        if self.cursor_column > 0 {
            self.cursor_column -= 1;
        } else if self.cursor_line_index > 0 {
            self.cursor_line_index -= 1;
            self.cursor_column = self.lines[self.cursor_line_index].chars().count();
        } else {
            return;
        }
        self.preferred_column = self.cursor_column;
    }

    fn move_cursor_right(&mut self) {
        let line_length = self.lines[self.cursor_line_index].chars().count();
        if self.cursor_column < line_length {
            self.cursor_column += 1;
        } else if self.cursor_line_index + 1 < self.lines.len() {
            self.cursor_line_index += 1;
            self.cursor_column = 0;
        } else {
            return;
        }
        self.preferred_column = self.cursor_column;
    }

    fn move_cursor_up(&mut self) {
        if self.cursor_line_index == 0 {
            return;
        }
        self.cursor_line_index -= 1;
        self.cursor_column = self
            .preferred_column
            .min(self.lines[self.cursor_line_index].chars().count());
    }

    fn move_cursor_down(&mut self) {
        if self.cursor_line_index + 1 >= self.lines.len() {
            return;
        }
        self.cursor_line_index += 1;
        self.cursor_column = self
            .preferred_column
            .min(self.lines[self.cursor_line_index].chars().count());
    }

    fn character_before_cursor(&self) -> Option<char> {
        if self.cursor_column > 0 {
            return self.lines[self.cursor_line_index]
                .chars()
                .nth(self.cursor_column - 1);
        }
        if self.cursor_line_index > 0 {
            return Some('\n');
        }

        None
    }

    fn sync_editor_scroll(&mut self, visible_height: u16) {
        let visible_height = visible_height.max(1) as usize;
        let max_scroll = self.lines.len().saturating_sub(visible_height);
        let current_scroll = self.editor_scroll as usize;
        let next_scroll = if self.cursor_line_index < current_scroll {
            self.cursor_line_index
        } else if self.cursor_line_index >= current_scroll + visible_height {
            self.cursor_line_index + 1 - visible_height
        } else {
            current_scroll
        };
        self.editor_scroll = next_scroll.min(max_scroll).min(u16::MAX as usize) as u16;
    }
}

impl From<PlanningDraftEditorFile> for PlanningDraftEditorBufferState {
    fn from(file: PlanningDraftEditorFile) -> Self {
        let lines = if file.body.is_empty() {
            vec![String::new()]
        } else {
            file.body.split('\n').map(|line| line.to_string()).collect()
        };

        Self {
            active_path: file.active_path,
            staged_path: file.staged_path,
            lines,
            cursor_line_index: 0,
            cursor_column: 0,
            preferred_column: 0,
            editor_scroll: 0,
            dirty: false,
        }
    }
}

fn char_to_byte_index(line: &str, column: usize) -> usize {
    line.char_indices()
        .nth(column)
        .map(|(index, _)| index)
        .unwrap_or(line.len())
}

#[cfg(test)]
#[path = "planning_draft_editor_ui/tests.rs"]
mod tests;
